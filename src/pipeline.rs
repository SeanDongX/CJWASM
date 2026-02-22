//! 编译流水线工具函数：文件解析、AST合并、import依赖解析。

use crate::ast::Program;
use crate::codegen::CodeGen;
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 标准库 overlay 根目录（cjwasm 提供的精简实现，优先于 vendor 使用）
/// 路径为 CARGO_MANIFEST_DIR/stdlib_overlay
pub fn get_stdlib_overlay_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("stdlib_overlay");
    if root.exists() {
        Some(root)
    } else {
        None
    }
}

/// 标准库根目录（cangjie_runtime 的 .cj 源码）
/// 优先使用环境变量 CJWASM_STDLIB，否则为 CARGO_MANIFEST_DIR/third_party/cangjie_runtime/std/libs/std
pub fn get_stdlib_root() -> Option<PathBuf> {
    if let Ok(ref s) = std::env::var("CJWASM_STDLIB") {
        let p = PathBuf::from(s);
        if p.exists() {
            return Some(p);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .join("third_party")
        .join("cangjie_runtime")
        .join("std")
        .join("libs")
        .join("std");
    if root.exists() {
        Some(root)
    } else {
        None
    }
}

/// 收集标准库模块目录下的所有 .cj 文件（不包含 native 子目录）
fn collect_cj_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return out;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.file_name().map_or(false, |n| n == "native") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map_or(false, |e| e == "cj") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// 解析源代码字符串为 Program AST
pub fn parse_source(source: &str) -> Result<Program, String> {
    let lexer = Lexer::new(source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens.map_err(|e| format!("词法错误: {}", e))?;

    let mut parser = Parser::new(tokens);
    let program = parser
        .parse_program()
        .map_err(|e| format!("语法错误: {}", e))?;

    Ok(program)
}

/// 解析文件为 Program AST，返回 (program, source)
pub fn parse_file(path: &str) -> Result<(Program, String), String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取文件 '{}': {}", path, e))?;

    let program = parse_source(&source)?;
    Ok((program, source))
}

/// 合并多个 Program AST 为一个
pub fn merge_programs(programs: Vec<Program>) -> Program {
    let mut merged = Program {
        package_name: None,
        imports: vec![],
        global_constants: vec![],
        global_vars: vec![],
        structs: vec![],
        interfaces: vec![],
        classes: vec![],
        enums: vec![],
        functions: vec![],
        extends: vec![],
        type_aliases: vec![],
    };

    for prog in programs {
        if merged.package_name.is_none() && prog.package_name.is_some() {
            merged.package_name = prog.package_name;
        }
        merged.imports.extend(prog.imports);
        merged.global_constants.extend(prog.global_constants);
        merged.global_vars.extend(prog.global_vars);
        merged.structs.extend(prog.structs);
        merged.interfaces.extend(prog.interfaces);
        merged.classes.extend(prog.classes);
        merged.enums.extend(prog.enums);
        merged.functions.extend(prog.functions);
        merged.extends.extend(prog.extends);
        merged.type_aliases.extend(prog.type_aliases);
    }

    merged
}

/// 三层模块分类策略
enum StdModuleLayer {
    /// L1: 纯 Cangjie 实现，vendor 优先
    VendorFirst,
    /// L2: 轻量 native 依赖，vendor + overlay 回退
    VendorWithFallback,
    /// L3: 重度 native 依赖，仅 overlay
    OverlayOnly,
    /// 不支持的模块
    Unsupported,
}

/// 分类 std 子模块
fn classify_std_module(module_name: &str) -> StdModuleLayer {
    match module_name {
        // L1: 纯 Cangjie 实现 - vendor 优先
        "io" | "binary" | "console" | "overflow" | "crypto" | "deriving" | "ast"
        | "argopt" | "sort" | "ref" | "unicode" => StdModuleLayer::VendorFirst,

        // L2: 轻量 native - vendor + overlay 回退
        "collection" | "convert" | "time" | "core" | "reflect" | "objectpool" => {
            StdModuleLayer::VendorWithFallback
        }

        // L3: 重度 native - 仅 overlay
        "env" | "runtime" | "random" | "fs" | "math" => StdModuleLayer::OverlayOnly,

        // 不支持
        "net" | "posix" | "process" | "database" | "sync" | "unittest" => {
            StdModuleLayer::Unsupported
        }

        // 未知模块，尝试 vendor + overlay
        _ => StdModuleLayer::VendorWithFallback,
    }
}

/// 在指定根目录下查找模块（支持单文件和目录）
fn try_resolve_in_root(root: &Path, rel_path: &str) -> Option<PathBuf> {
    // 尝试单文件
    let file_path = root.join(rel_path).with_extension("cj");
    if file_path.exists() {
        return Some(file_path);
    }
    // 尝试目录
    let dir_path = root.join(rel_path);
    if dir_path.is_dir() {
        return Some(dir_path);
    }
    None
}

/// 根据 import 路径解析文件路径
/// 对 std.* 使用三层分类策略；否则在 base_dir 下解析。
/// 例如: import math.utils -> 搜索 math/utils.cj 或 math_utils.cj
pub fn resolve_import_path(module_path: &[String], base_dir: &Path) -> Option<PathBuf> {
    // std.* 解析：使用三层分类策略
    if module_path.first().map(|s| s.as_str()) == Some("std") && module_path.len() >= 2 {
        let module_name = &module_path[1];
        let rest: Vec<&str> = module_path.iter().skip(1).map(String::as_str).collect();
        let rel = rest.join("/");

        let layer = classify_std_module(module_name);

        match layer {
            StdModuleLayer::VendorFirst => {
                // L1: 优先 vendor，回退 overlay
                if let Some(stdlib_root) = get_stdlib_root() {
                    if let Some(path) = try_resolve_in_root(&stdlib_root, &rel) {
                        return Some(path);
                    }
                }
                if let Some(overlay_root) = get_stdlib_overlay_root() {
                    if let Some(path) = try_resolve_in_root(&overlay_root, &rel) {
                        return Some(path);
                    }
                }
            }

            StdModuleLayer::VendorWithFallback => {
                // L2: vendor + overlay 回退
                if let Some(stdlib_root) = get_stdlib_root() {
                    if let Some(path) = try_resolve_in_root(&stdlib_root, &rel) {
                        return Some(path);
                    }
                }
                if let Some(overlay_root) = get_stdlib_overlay_root() {
                    if let Some(path) = try_resolve_in_root(&overlay_root, &rel) {
                        return Some(path);
                    }
                }
            }

            StdModuleLayer::OverlayOnly => {
                // L3: 仅 overlay
                if let Some(overlay_root) = get_stdlib_overlay_root() {
                    if let Some(path) = try_resolve_in_root(&overlay_root, &rel) {
                        return Some(path);
                    }
                }
            }

            StdModuleLayer::Unsupported => {
                // 不支持的模块
                eprintln!("警告: std.{} 模块在 WASM 环境下不支持", module_name);
                return None;
            }
        }
    }

    // 策略 1: 将模块路径转为目录路径 math.utils -> math/utils.cj
    let dir_path = base_dir
        .join(module_path.iter().cloned().collect::<Vec<_>>().join("/"))
        .with_extension("cj");
    if dir_path.exists() {
        return Some(dir_path);
    }

    // 策略 2: 使用下划线连接 math.utils -> math_utils.cj
    let underscore_path = base_dir
        .join(module_path.iter().cloned().collect::<Vec<_>>().join("_"))
        .with_extension("cj");
    if underscore_path.exists() {
        return Some(underscore_path);
    }

    // 策略 3: 在 src/ 子目录中查找
    let src_dir = base_dir.join("src");
    if src_dir.exists() {
        let src_path = src_dir
            .join(module_path.iter().cloned().collect::<Vec<_>>().join("/"))
            .with_extension("cj");
        if src_path.exists() {
            return Some(src_path);
        }
    }

    None
}

/// 递归解析 import 依赖；对标准库目录会展开为目录下所有 .cj 文件（排除 native/）
pub fn collect_import_files(
    program: &Program,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut import_files = Vec::new();

    for import in &program.imports {
        if let Some(resolved) = resolve_import_path(&import.module_path, base_dir) {
            if resolved.is_dir() {
                for p in collect_cj_files_in_dir(&resolved) {
                    let canonical = p.canonicalize().unwrap_or(p.clone());
                    if !visited.contains(&canonical) {
                        visited.insert(canonical.clone());
                        import_files.push(p);
                    }
                }
            } else {
                let canonical = resolved.canonicalize().unwrap_or(resolved.clone());
                if !visited.contains(&canonical) {
                    visited.insert(canonical.clone());
                    import_files.push(resolved);
                }
            }
        }
    }

    import_files
}

/// 完整编译流水线：源代码 -> WASM 字节码
pub fn compile_source_to_wasm(source: &str) -> Result<Vec<u8>, String> {
    let mut program = parse_source(source)?;
    crate::optimizer::optimize_program(&mut program);
    crate::monomorph::monomorphize_program(&mut program);
    let mut codegen = CodeGen::new();
    Ok(codegen.compile(&program))
}

/// 多文件编译流水线
pub fn compile_files_to_wasm(files: &[&str]) -> Result<Vec<u8>, String> {
    let mut programs = Vec::new();
    for file in files {
        let (prog, _source) = parse_file(file)?;
        programs.push(prog);
    }

    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        merge_programs(programs)
    };

    crate::optimizer::optimize_program(&mut program);
    crate::monomorph::monomorphize_program(&mut program);
    let mut codegen = CodeGen::new();
    Ok(codegen.compile(&program))
}

/// 从入口文件递归解析 import 并编译为 WASM（用于验证 std.io 等多模块工程）
/// base_dir：解析非 std 模块时的根目录，通常为入口文件所在目录或项目根
pub fn compile_entry_with_imports(entry_path: &str, base_dir: &Path) -> Result<Vec<u8>, String> {
    let entry = PathBuf::from(entry_path);
    let entry_canon = entry
        .canonicalize()
        .map_err(|e| format!("入口文件 '{}': {}", entry_path, e))?;
    let mut visited: HashSet<PathBuf> = HashSet::new();
    visited.insert(entry_canon.clone());
    let mut queue = vec![entry_canon];
    let mut programs: Vec<Program> = Vec::new();

    while let Some(p) = queue.pop() {
        let path_str = p.to_string_lossy().to_string();
        let (prog, _) = parse_file(&path_str)?;
        programs.push(prog);
        for f in collect_import_files(programs.last().unwrap(), base_dir, &mut visited) {
            queue.push(f.canonicalize().unwrap_or(f));
        }
    }

    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        merge_programs(programs)
    };

    crate::optimizer::optimize_program(&mut program);
    crate::monomorph::monomorphize_program(&mut program);
    let mut codegen = CodeGen::new();
    Ok(codegen.compile(&program))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_source_success() {
        let source = "func main(): Int64 { return 42 }";
        let program = parse_source(source).unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_source_lex_error() {
        let source = "func main() { let x = ` }";
        let result = parse_source(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("词法错误"));
    }

    #[test]
    fn test_parse_source_syntax_error() {
        let source = "func main( { return 42 }";
        let result = parse_source(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("语法错误"));
    }

    #[test]
    fn test_merge_programs_empty() {
        let merged = merge_programs(vec![]);
        assert!(merged.functions.is_empty());
        assert!(merged.package_name.is_none());
    }

    #[test]
    fn test_merge_programs_single() {
        let source = "func main(): Int64 { return 0 }";
        let prog = parse_source(source).unwrap();
        let merged = merge_programs(vec![prog]);
        assert_eq!(merged.functions.len(), 1);
    }

    #[test]
    fn test_merge_programs_multiple() {
        let source1 = "func foo(): Int64 { return 1 }";
        let source2 = "func bar(): Int64 { return 2 }";
        let prog1 = parse_source(source1).unwrap();
        let prog2 = parse_source(source2).unwrap();
        let merged = merge_programs(vec![prog1, prog2]);
        assert_eq!(merged.functions.len(), 2);
    }

    #[test]
    fn test_merge_programs_package_name() {
        let source1 = "package app.main\nfunc main(): Int64 { return 0 }";
        let source2 = "func helper(): Int64 { return 1 }";
        let prog1 = parse_source(source1).unwrap();
        let prog2 = parse_source(source2).unwrap();
        let merged = merge_programs(vec![prog1, prog2]);
        assert!(merged.package_name.is_some());
        assert_eq!(merged.functions.len(), 2);
    }

    #[test]
    fn test_merge_programs_structs_classes_enums() {
        let source1 = r#"
            struct Point { x: Int64, y: Int64 }
            class Box { var size: Int64; init(s: Int64) { this.size = s } }
        "#;
        let source2 = r#"
            enum Color { Red Green Blue }
            interface Drawable { func draw(): Int64; }
        "#;
        let prog1 = parse_source(source1).unwrap();
        let prog2 = parse_source(source2).unwrap();
        let merged = merge_programs(vec![prog1, prog2]);
        assert_eq!(merged.structs.len(), 1);
        assert_eq!(merged.classes.len(), 1);
        assert_eq!(merged.enums.len(), 1);
        assert_eq!(merged.interfaces.len(), 1);
    }

    #[test]
    fn test_resolve_import_path_not_found() {
        let tmp = std::env::temp_dir();
        let result = resolve_import_path(&["nonexistent".to_string(), "module".to_string()], &tmp);
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_import_path_dir_strategy() {
        // 创建临时目录结构
        let tmp = std::env::temp_dir().join("cjwasm_test_resolve_dir");
        let _ = fs::create_dir_all(&tmp.join("math"));
        let test_file = tmp.join("math/utils.cj");
        let _ = fs::write(&test_file, "func foo(): Int64 { return 0 }");

        let result = resolve_import_path(
            &["math".to_string(), "utils".to_string()],
            &tmp,
        );
        assert!(result.is_some());

        // 清理
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_import_path_underscore_strategy() {
        let tmp = std::env::temp_dir().join("cjwasm_test_resolve_underscore");
        let _ = fs::create_dir_all(&tmp);
        let test_file = tmp.join("math_utils.cj");
        let _ = fs::write(&test_file, "func foo(): Int64 { return 0 }");

        let result = resolve_import_path(
            &["math".to_string(), "utils".to_string()],
            &tmp,
        );
        assert!(result.is_some());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_import_path_src_strategy() {
        let tmp = std::env::temp_dir().join("cjwasm_test_resolve_src");
        let _ = fs::create_dir_all(&tmp.join("src").join("math"));
        let test_file = tmp.join("src/math/utils.cj");
        let _ = fs::write(&test_file, "func foo(): Int64 { return 0 }");

        let result = resolve_import_path(
            &["math".to_string(), "utils".to_string()],
            &tmp,
        );
        assert!(result.is_some());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_import_files() {
        let source = r#"
            import nonexistent.mylib
            func main(): Int64 { return 0 }
        "#;
        let program = parse_source(source).unwrap();
        let tmp = std::env::temp_dir();
        let mut visited = HashSet::new();
        let files = collect_import_files(&program, &tmp, &mut visited);
        // 找不到文件时返回空
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_import_files_with_existing() {
        let tmp = std::env::temp_dir().join("cjwasm_test_collect_imports");
        let _ = fs::create_dir_all(&tmp);
        let lib_file = tmp.join("mylib.cj");
        let _ = fs::write(&lib_file, "func helper(): Int64 { return 1 }");

        let source = "import mylib\nfunc main(): Int64 { return 0 }";
        let program = parse_source(source).unwrap();
        let mut visited = HashSet::new();
        let files = collect_import_files(&program, &tmp, &mut visited);
        assert_eq!(files.len(), 1);

        // 再次收集不会重复
        let files2 = collect_import_files(&program, &tmp, &mut visited);
        assert!(files2.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_compile_source_to_wasm() {
        let source = "func main(): Int64 { return 42 }";
        let wasm = compile_source_to_wasm(source).unwrap();
        assert!(wasm.len() >= 8);
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_compile_source_to_wasm_error() {
        let source = "func main( { }"; // syntax error
        let result = compile_source_to_wasm(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_files_to_wasm() {
        let tmp = std::env::temp_dir().join("cjwasm_test_compile_files");
        let _ = fs::create_dir_all(&tmp);
        let file1 = tmp.join("main.cj");
        let _ = fs::write(&file1, "func main(): Int64 { return helper() }");
        let file2 = tmp.join("helper.cj");
        let _ = fs::write(&file2, "func helper(): Int64 { return 42 }");

        let wasm = compile_files_to_wasm(&[
            file1.to_str().unwrap(),
            file2.to_str().unwrap(),
        ])
        .unwrap();
        assert!(wasm.len() >= 8);
        assert_eq!(&wasm[0..4], b"\0asm");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_file_not_found() {
        let result = parse_file("/nonexistent/path/file.cj");
        assert!(result.is_err());
    }

    /// 验证能成功解析 std.time constants.cj 且 program.global_constants.len() > 0（完整复用 std.time 阶段 1）
    #[test]
    fn test_parse_std_time_constants_cj() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("third_party")
            .join("cangjie_runtime")
            .join("std")
            .join("libs")
            .join("std")
            .join("time")
            .join("constants.cj");
        if !path.exists() {
            return; // vendor 未克隆时跳过
        }
        let (program, _) = parse_file(path.to_str().unwrap()).unwrap();
        assert!(
            !program.global_constants.is_empty(),
            "constants.cj 应解析出顶层 const"
        );
        assert!(program.global_constants.iter().any(|(n, _, _)| n == "MIN_INT64"));
        assert!(program.global_constants.iter().any(|(n, _, _)| n == "NS_PER_SEC"));
    }

    #[test]
    fn test_parse_file_success() {
        let tmp = std::env::temp_dir().join("cjwasm_test_parse_file");
        let _ = fs::create_dir_all(&tmp);
        let file = tmp.join("test.cj");
        let _ = fs::write(&file, "func main(): Int64 { return 0 }");

        let (program, source) = parse_file(file.to_str().unwrap()).unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(!source.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    /// Phase 6：std.time overlay（nowNs）与入口合并后可编译出 WASM
    #[test]
    fn test_compile_std_time_overlay_with_now_ns() {
        let Some(overlay_root) = get_stdlib_overlay_root() else { return; };
        let time_cj = overlay_root.join("time.cj");
        if !time_cj.exists() {
            return;
        }
        let tmp = std::env::temp_dir().join("cjwasm_phase6_test");
        let _ = fs::create_dir_all(&tmp);
        let entry_path = tmp.join("main.cj");
        let entry_src = "func main(): Int64 { return nowNs() }";
        let _ = fs::write(&entry_path, entry_src);
        let time_str = time_cj.to_string_lossy().to_string();
        let entry_str = entry_path.to_string_lossy().to_string();
        let wasm = compile_files_to_wasm(&[&time_str, &entry_str]).unwrap();
        assert!(!wasm.is_empty(), "std.time overlay + nowNs() 应能编译出 WASM");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Phase 6：std.time 优先解析 vendor 目录（存在则返回 time 目录）
    #[test]
    fn test_resolve_std_time_prefer_vendor() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let result = resolve_import_path(&["std".to_string(), "time".to_string()], &base);
        if let Some(ref stdlib_root) = get_stdlib_root() {
            let time_dir = stdlib_root.join("time");
            if time_dir.is_dir() {
                assert_eq!(result.as_ref(), Some(&time_dir), "std.time 应解析到 vendor time 目录");
                return;
            }
        }
        // 无 vendor 时解析到 overlay 或 None
        if get_stdlib_overlay_root().is_some() {
            assert!(result.is_some(), "无 vendor 时 std.time 可回退到 overlay");
        }
    }

    /// std.math overlay：sqrt/ceil/floor 通过 __math_sqrt/__math_ceil/__math_floor 运行时编译
    #[test]
    fn test_compile_std_math_overlay_sqrt_ceil_floor() {
        let Some(overlay_root) = get_stdlib_overlay_root() else { return; };
        let math_cj = overlay_root.join("math.cj");
        if !math_cj.exists() {
            return;
        }
        let tmp = std::env::temp_dir().join("cjwasm_math_overlay_test");
        let _ = fs::create_dir_all(&tmp);
        let entry_path = tmp.join("main.cj");
        let entry_src = r#"
            func main(): Int64 {
                let _ = sqrt(2.0)
                let _ = ceil(3.2)
                let _ = floor(3.8)
                return 0
            }
        "#;
        let _ = fs::write(&entry_path, entry_src);
        let math_str = math_cj.to_string_lossy().to_string();
        let entry_str = entry_path.to_string_lossy().to_string();
        let wasm = compile_files_to_wasm(&[&math_str, &entry_str]).unwrap();
        assert!(!wasm.is_empty(), "std.math overlay sqrt/ceil/floor 应能编译出 WASM");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// 验证 std.io：从入口文件递归解析 import（L1 vendor）并编译
    #[test]
    fn test_compile_std_io_entry_with_imports() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entry = manifest.join("examples").join("std_io_simple.cj");
        if !entry.exists() {
            return;
        }
        let base = manifest.clone();
        let result = compile_entry_with_imports(entry.to_str().unwrap(), &base);
        let wasm = result.expect("std.io 入口应能解析并编译");
        assert!(!wasm.is_empty(), "应生成 WASM");
    }

    /// std.fs overlay：能解析并编译出 WASM（SEEK_* 常量、Path、exists）
    #[test]
    fn test_compile_std_fs_overlay() {
        let Some(overlay_root) = get_stdlib_overlay_root() else { return; };
        let fs_cj = overlay_root.join("fs.cj");
        if !fs_cj.exists() {
            return;
        }
        let tmp = std::env::temp_dir().join("cjwasm_fs_overlay_test");
        let _ = fs::create_dir_all(&tmp);
        let entry_path = tmp.join("main.cj");
        let entry_src = r#"
            func main(): Int64 {
                let _ = exists(".")
                let p = Path("x")
                return 0
            }
        "#;
        let _ = fs::write(&entry_path, entry_src);
        let fs_str = fs_cj.to_string_lossy().to_string();
        let entry_str = entry_path.to_string_lossy().to_string();
        let wasm = compile_files_to_wasm(&[&fs_str, &entry_str]).unwrap();
        assert!(!wasm.is_empty(), "std.fs overlay 应能编译出 WASM");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Phase 5：能解析 std.convert 与 std.collection.ArrayList overlay 桩
    #[test]
    fn test_parse_std_convert_and_collection_overlay() {
        let Some(overlay_root) = get_stdlib_overlay_root() else { return; };
        let convert_cj = overlay_root.join("convert.cj");
        if convert_cj.exists() {
            let (prog, _) = parse_file(convert_cj.to_str().unwrap()).unwrap();
            assert!(!prog.interfaces.is_empty(), "convert.cj 应解析出接口");
        }
        let arraylist_cj = overlay_root.join("collection").join("ArrayList.cj");
        if arraylist_cj.exists() {
            let (prog, _) = parse_file(arraylist_cj.to_str().unwrap()).unwrap();
            assert!(!prog.classes.is_empty(), "ArrayList.cj 应解析出类");
        }
    }

    #[test]
    fn test_compile_complex_pipeline() {
        // 测试完整流水线，覆盖多种功能
        let source = r#"
            struct Point { x: Int64, y: Int64 }
            
            enum Direction {
                North
                South
                East
                West
            }
            
            func add(a: Int64, b: Int64): Int64 { return a + b }
            
            func main(): Int64 {
                let p = Point { x: 10, y: 20 }
                let d = Direction.North
                let sum = add(p.x, p.y)
                return sum
            }
        "#;
        let wasm = compile_source_to_wasm(source).unwrap();
        assert!(wasm.len() >= 8);
    }

    /// 入口仅单文件、无 import 时，compile_entry_with_imports 仍能编译
    #[test]
    fn test_compile_entry_with_imports_single_file_no_import() {
        let tmp = std::env::temp_dir().join("cjwasm_entry_single");
        let _ = fs::create_dir_all(&tmp);
        let entry = tmp.join("main.cj");
        let src = r#"
            func main(): Int64 {
                return 42
            }
        "#;
        let _ = fs::write(&entry, src);
        let result = compile_entry_with_imports(entry.to_str().unwrap(), &tmp);
        let wasm = result.expect("单文件入口应能编译");
        assert!(!wasm.is_empty(), "应生成 WASM");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// 多文件合并编译：两文件各定义函数，合并后 main 调用另一文件函数
    #[test]
    fn test_compile_two_files_merged() {
        let tmp = std::env::temp_dir().join("cjwasm_two_files");
        let _ = fs::create_dir_all(&tmp);
        let a = tmp.join("a.cj");
        let b = tmp.join("b.cj");
        let _ = fs::write(
            &a,
            r#"
            func fromA(): Int64 { return 1 }
        "#,
        );
        let _ = fs::write(
            &b,
            r#"
            func main(): Int64 {
                return fromA() + 1
            }
        "#,
        );
        let wasm = compile_files_to_wasm(&[a.to_str().unwrap(), b.to_str().unwrap()]).unwrap();
        assert!(!wasm.is_empty(), "两文件合并应生成 WASM");
        let _ = fs::remove_dir_all(&tmp);
    }
}
