//! 编译流水线工具函数：文件解析、AST合并、import依赖解析。

use crate::ast::Program;
use crate::codegen::CodeGen;
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
        structs: vec![],
        interfaces: vec![],
        classes: vec![],
        enums: vec![],
        functions: vec![],
        extends: vec![],
        type_aliases: vec![],
        macros: vec![],
        is_macro_package: false,
        global_vars: vec![],
    };

    for prog in programs {
        if merged.package_name.is_none() && prog.package_name.is_some() {
            merged.package_name = prog.package_name;
        }
        merged.imports.extend(prog.imports);
        merged.structs.extend(prog.structs);
        merged.interfaces.extend(prog.interfaces);
        merged.classes.extend(prog.classes);
        merged.enums.extend(prog.enums);
        merged.functions.extend(prog.functions);
        merged.extends.extend(prog.extends);
        merged.type_aliases.extend(prog.type_aliases);
        merged.macros.extend(prog.macros);
        merged.global_vars.extend(prog.global_vars);
    }

    merged
}

/// 根据 import 路径解析文件路径
/// 例如: import math.utils -> 搜索 math/utils.cj 或 math_utils.cj
pub fn resolve_import_path(module_path: &[String], base_dir: &Path) -> Option<PathBuf> {
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

/// 递归解析 import 依赖
pub fn collect_import_files(
    program: &Program,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut import_files = Vec::new();

    for import in &program.imports {
        if let Some(resolved) = resolve_import_path(&import.module_path, base_dir) {
            let canonical = resolved.canonicalize().unwrap_or(resolved.clone());
            if !visited.contains(&canonical) {
                visited.insert(canonical.clone());
                import_files.push(resolved);
            }
        }
    }

    import_files
}

/// 完整编译流水线：源代码 -> WASM 字节码
pub fn compile_source_to_wasm(source: &str) -> Result<Vec<u8>, String> {
    let mut program = parse_source(source)?;

    // 注入 JSON 标准库（如果有 import stdx.encoding.json）
    let needs_json_stdlib = program.imports.iter().any(|imp| {
        let path = imp.module_path.join(".");
        path.starts_with("stdx.encoding.json")
    });
    if needs_json_stdlib {
        let json_stdlib = crate::stdlib::json::generate_json_stdlib();
        json_stdlib.inject_into(&mut program);
    }
    if program.imports.iter().any(|imp| imp.module_path.join(".").starts_with("std.time")) {
        crate::stdlib::time::generate_time_stdlib().inject_into(&mut program);
    }

    // M5: 宏展开阶段 — 在优化和单态化之前执行
    if crate::macro_expand::program_has_macros(&program) {
        let expander = crate::macro_expand::MacroExpander::new(&program);
        expander
            .expand_program(&mut program)
            .map_err(|errs| {
                errs.iter()
                    .map(|e| format!("{}", e))
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
    }

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

    // 注入 JSON 标准库（如果有 import stdx.encoding.json）
    let needs_json_stdlib = program.imports.iter().any(|imp| {
        let path = imp.module_path.join(".");
        path.starts_with("stdx.encoding.json")
    });
    if needs_json_stdlib {
        let json_stdlib = crate::stdlib::json::generate_json_stdlib();
        json_stdlib.inject_into(&mut program);
    }
    if program.imports.iter().any(|imp| imp.module_path.join(".").starts_with("std.time")) {
        crate::stdlib::time::generate_time_stdlib().inject_into(&mut program);
    }

    // M5: 宏展开阶段
    if crate::macro_expand::program_has_macros(&program) {
        let expander = crate::macro_expand::MacroExpander::new(&program);
        expander
            .expand_program(&mut program)
            .map_err(|errs| {
                errs.iter()
                    .map(|e| format!("{}", e))
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
    }

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

    #[test]
    fn test_parse_macro_def() {
        let source = r#"
            public macro func MyLog(args: String): String {
                return quote(
                    println("log")
                )
            }
            func main(): Int64 { return 0 }
        "#;
        let program = parse_source(source).unwrap();
        assert_eq!(program.macros.len(), 1);
        assert_eq!(program.macros[0].name, "MyLog");
        assert_eq!(program.macros[0].params.len(), 1);
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_macro_call_bracket() {
        let source = r#"
            func main(): Int64 {
                @MyLog["hello"]
                return 0
            }
        "#;
        let program = parse_source(source).unwrap();
        assert_eq!(program.functions.len(), 1);
        let body = &program.functions[0].body;
        assert!(body.len() >= 2);
    }

    #[test]
    fn test_parse_macro_call_paren() {
        let source = r#"
            func main(): Int64 {
                @MyLog("hello", 42)
                return 0
            }
        "#;
        let program = parse_source(source).unwrap();
        let body = &program.functions[0].body;
        assert!(body.len() >= 2);
    }
}
