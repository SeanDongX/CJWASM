//! 编译流水线工具函数：文件解析、AST合并、import依赖解析。
//!
//! L1 模块（纯 Cangjie，Vendor 优先）：std.io, std.binary, std.console,
//! std.overflow, std.crypto, std.deriving, std.ast, std.argopt, std.sort,
//! std.ref, std.unicode — 从 third_party/cangjie_runtime/std/libs/std 解析。

use crate::ast::Program;
use crate::codegen::CodeGen;
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// L1 顶层 std 模块名（纯 Cangjie 实现，Vendor 优先）
const L1_STD_TOP: &[&str] = &[
    "io", "binary", "console", "overflow", "crypto", "deriving",
    "ast", "argopt", "sort", "ref", "unicode",
];

/// 返回 L1 顶层模块名列表，供测试或工具使用
pub fn l1_std_top_modules() -> &'static [&'static str] {
    L1_STD_TOP
}

/// 将 quote(...) 宏调用内容替换为空（quote()），避免词法错误（如 \( \) 和关键字）
/// quote 内部是原始 token 模板，cjwasm 不执行宏展开，只需保留调用外壳
fn strip_quote_contents(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        // 检查是否以 "quote(" 开始（quote 后紧跟括号）
        if i + 6 <= len && &bytes[i..i + 6] == b"quote(" {
            out.push_str("quote()");
            i += 6; // skip "quote("
            // 消费直到匹配的 ')'，跟踪嵌套深度
            let mut depth = 1usize;
            while i < len && depth > 0 {
                let ch = bytes[i];
                if ch == b'(' {
                    depth += 1;
                } else if ch == b')' {
                    depth -= 1;
                } else if ch == b'"' {
                    // 跳过字符串字面量
                    i += 1;
                    while i < len {
                        let sc = bytes[i];
                        if sc == b'\\' {
                            i += 2;
                            continue;
                        }
                        if sc == b'"' {
                            break;
                        }
                        i += 1;
                    }
                }
                i += 1;
            }
            continue;
        }
        // 处理字符串字面量（跳过，防止把字符串内的 "quote(" 替换掉）
        if bytes[i] == b'"' {
            out.push('"');
            i += 1;
            while i < len {
                // 按 UTF-8 字符复制，避免把多字节字符（如中文 Rune/字符串）按字节拆坏
                let c = match source[i..].chars().next() {
                    Some(c) => c,
                    None => break,
                };
                if c == '\\' {
                    out.push('\\');
                    i += 1;
                    if i < len {
                        let esc = match source[i..].chars().next() {
                            Some(c) => c,
                            None => break,
                        };
                        out.push(esc);
                        i += esc.len_utf8();
                    }
                    continue;
                }
                out.push(c);
                i += c.len_utf8();
                if c == '"' {
                    break;
                }
            }
            continue;
        }
        // 普通字符同样按 UTF-8 字符复制（保持 CJK 标识符 / Rune 字面量原样）
        let c = match source[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// 找到块注释的关闭位置（返回 */ 之后的位置），支持嵌套块注释（规范 1.5(2)）
///
/// 仓颉规范 A.1.1(1)：`DelimitedComment : '/*' ( DelimitedComment | . )*? '*/'`，
/// 注释内容中引号不具特殊含义，只有 `/*` / `*/` 参与嵌套计数。
fn find_comment_close(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    let mut depth = 1usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
            continue;
        }
        i += 1;
    }
    bytes.len()
}

/// 移除源码中的块注释 /* ... */，便于解析含版权头的 vendor 文件
/// 通过引号计数正确处理注释内含 */ 的字符串（如 /* e.g. "/*xx*/" */）
fn strip_block_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // 行注释：`//` 到行尾整体保留（词法器会跳过），避免把 `//*/` 中的 `/*` 误判为块注释
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(bytes[i]);
                i += 1;
            }
            continue;
        }
        // 多行字符串 """..."""：整体保留
        if i + 2 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"'
        {
            out.extend_from_slice(b"\"\"\"");
            i += 3;
            while i < bytes.len() {
                if i + 2 < bytes.len()
                    && bytes[i] == b'"'
                    && bytes[i + 1] == b'"'
                    && bytes[i + 2] == b'"'
                {
                    out.extend_from_slice(b"\"\"\"");
                    i += 3;
                    break;
                }
                out.push(bytes[i]);
                i += 1;
            }
            continue;
        }
        // 普通字符串：整体保留，避免字符串内的 /* */ 被误判
        if bytes[i] == b'"' {
            out.push(bytes[i]);
            i += 1;
            while i < bytes.len() {
                out.push(bytes[i]);
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let close = find_comment_close(bytes, i);
            i = close;
            out.push(b' ');
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// 十六进制字面量紧邻范围运算符（如 `0x1..5`）时，logos 的十六进制浮点模式会误吞 `.`。
/// 在十六进制整数与 `..` 之间插入一个空格（不改变语义），避免词法错误。
fn separate_hex_range(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // 跳过字符串（普通/多行），保持原样
        if i + 2 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"'
        {
            out.extend_from_slice(b"\"\"\"");
            i += 3;
            while i < bytes.len() {
                if i + 2 < bytes.len()
                    && bytes[i] == b'"'
                    && bytes[i + 1] == b'"'
                    && bytes[i + 2] == b'"'
                {
                    out.extend_from_slice(b"\"\"\"");
                    i += 3;
                    break;
                }
                out.push(bytes[i]);
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'"' {
            out.push(bytes[i]);
            i += 1;
            while i < bytes.len() {
                out.push(bytes[i]);
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // 十六进制整数 0x... 后紧跟 `..`
        if i + 1 < bytes.len()
            && bytes[i] == b'0'
            && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            let mut j = i + 2;
            while j < bytes.len()
                && ((bytes[j] as char).is_ascii_hexdigit() || bytes[j] == b'_')
            {
                j += 1;
            }
            if j + 1 < bytes.len() && bytes[j] == b'.' && bytes[j + 1] == b'.' {
                out.extend_from_slice(&bytes[i..j]);
                out.push(b' ');
                i = j;
                continue;
            }
            out.extend_from_slice(&bytes[i..j]);
            i = j;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// 解析源代码字符串为 Program AST（仅词法 + 语法，**不做语义校验**）。
///
/// 供多文件编译：逐文件弦解析后再合并、对合并结果统一校验，避免把同包其它
/// 文件里的顶层名误判为未定义。
pub fn parse_source_ast(source: &str) -> Result<Program, String> {
    let source = strip_block_comments(source);
    let source = strip_quote_contents(&source);
    let source = separate_hex_range(&source);
    let lexer = Lexer::new(&source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens.map_err(|e| format!("词法错误: {}", e))?;

    let mut parser = Parser::new(tokens).with_source(&source);
    parser
        .parse_program()
        .map_err(|e| format!("语法错误: {}", e))
}

/// 解析源代码字符串为 Program AST
pub fn parse_source(source: &str) -> Result<Program, String> {
    let program = parse_source_ast(source)?;

    // 语义校验：重定义 / static+实例同名重载 / override 签名 / 接口实现完整性。
    crate::sema::validate_program(&program).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| e.message.clone()).collect();
        format!("语义错误: {}", msgs.join("; "))
    })?;

    Ok(program)
}

/// 解析文件为 Program AST（仅词法 + 语法，不做语义校验），返回 (program, source)。
pub fn parse_file_ast(path: &str) -> Result<(Program, String), String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取文件 '{}': {}", path, e))?;
    let program = parse_source_ast(&source).map_err(|e| format!("{}: {}", path, e))?;
    Ok((program, source))
}

/// 解析文件为 Program AST，返回 (program, source)
pub fn parse_file(path: &str) -> Result<(Program, String), String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取文件 '{}': {}", path, e))?;
    let program = parse_source(&source).map_err(|e| format!("{}: {}", path, e))?;
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
        constants: vec![],
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
        merged.constants.extend(prog.constants);
    }

    merged
}

/// 判断是否为 L1 std 模块（含子包，如 std.crypto.digest）
fn is_l1_std_module(module_path: &[String]) -> bool {
    if module_path.is_empty() || module_path[0] != "std" {
        return false;
    }
    if module_path.len() == 1 {
        return false;
    }
    L1_STD_TOP.contains(&module_path[1].as_str())
}

/// 获取 vendor 标准库根目录（L1 解析用）
/// 优先 project_dir 及其父目录下的 third_party/...，其次环境变量 CJWASM_STD_PATH
pub fn get_vendor_std_dir(project_dir: &Path) -> Option<PathBuf> {
    let mut dir = project_dir.to_path_buf();
    for _ in 0..8 {
        let vendor = dir.join("third_party/cangjie_runtime/std/libs/std");
        if vendor.exists() && vendor.is_dir() {
            return Some(vendor);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            break;
        }
    }
    std::env::var_os("CJWASM_STD_PATH").map(PathBuf::from)
}

/// 根据 import 路径解析为单个文件（用于非 std 或非 L1）
fn resolve_import_path_single(module_path: &[String], base_dir: &Path) -> Option<PathBuf> {
    let dir_path = base_dir
        .join(module_path.iter().cloned().collect::<Vec<_>>().join("/"))
        .with_extension("cj");
    if dir_path.exists() {
        return Some(dir_path);
    }
    let underscore_path = base_dir
        .join(module_path.iter().cloned().collect::<Vec<_>>().join("_"))
        .with_extension("cj");
    if underscore_path.exists() {
        return Some(underscore_path);
    }
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

/// 根据 import 路径解析文件路径（单文件，兼容旧逻辑）
/// 例如: import math.utils -> 搜索 math/utils.cj 或 math_utils.cj
pub fn resolve_import_path(module_path: &[String], base_dir: &Path) -> Option<PathBuf> {
    resolve_import_path_single(module_path, base_dir)
}

/// L1：将 import 解析为若干文件（std L1 包可能对应目录下多个 .cj）
pub fn resolve_import_to_files(
    module_path: &[String],
    base_dirs: &[&Path],
    vendor_std_dir: Option<&Path>,
) -> Vec<PathBuf> {
    // L1 Vendor 优先：std.io / std.crypto.digest 等
    if let Some(vendor) = vendor_std_dir {
        if is_l1_std_module(module_path) && module_path.len() >= 2 {
            let rel: PathBuf = module_path[1..].iter().cloned().collect::<Vec<_>>().join("/").into();
            let dir = vendor.join(rel);
            if dir.exists() && dir.is_dir() {
                let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
                    Ok(rd) => rd
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.extension().map_or(false, |e| e == "cj"))
                        .collect(),
                    Err(_) => vec![],
                };
                files.sort();
                if !files.is_empty() {
                    return files;
                }
            }
        }
    }

    // 回退：在 base_dirs 中按单文件解析
    for base in base_dirs {
        if let Some(p) = resolve_import_path_single(module_path, base) {
            return vec![p];
        }
    }
    vec![]
}

/// 递归解析 import 依赖；支持 L1 vendor 多文件解析
pub fn collect_import_files(
    program: &Program,
    base_dirs: &[&Path],
    visited: &mut HashSet<PathBuf>,
    vendor_std_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut import_files = Vec::new();
    for import in &program.imports {
        let resolved = resolve_import_to_files(&import.module_path, base_dirs, vendor_std_dir);
        for path in resolved {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !visited.contains(&canonical) {
                visited.insert(canonical.clone());
                import_files.push(path);
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

    // 默认使用 CHIR 路径，设置 NO_CHIR=1 可回退到旧路径
    if std::env::var("NO_CHIR").is_err() {
        // 新路径: AST → CHIR → WASM
        let mut chir_program = crate::chir::lower_program(&program)?;
        if std::env::var("NO_CHIR_OPT").is_err() {
            crate::chir::optimize::optimize_chir(&mut chir_program);
        }
        let mut codegen = crate::codegen::chir_codegen::CHIRCodeGen::new();
        Ok(codegen.generate(&chir_program))
    } else {
        // 旧路径: AST → WASM
        let mut codegen = CodeGen::new();
        Ok(codegen.compile(&program))
    }
}

/// 多文件编译流水线
pub fn compile_files_to_wasm(files: &[&str]) -> Result<Vec<u8>, String> {
    let mut programs = Vec::new();
    if files.len() <= 1 {
        for file in files {
            let (prog, _source) = parse_file(file)?;
            programs.push(prog);
        }
    } else {
        // 多文件：先仅解析，合并后统一做语义校验（同包其它文件的顶层名可见）。
        for file in files {
            let source = std::fs::read_to_string(file)
                .map_err(|e| format!("无法读取文件 '{}': {}", file, e))?;
            let prog = parse_source_ast(&source).map_err(|e| format!("{}: {}", file, e))?;
            programs.push(prog);
        }
    }

    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        merge_programs(programs)
    };

    if files.len() > 1 {
        crate::sema::validate_program(&program).map_err(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| e.message.clone()).collect();
            format!("语义错误: {}", msgs.join("; "))
        })?;
    }

    crate::optimizer::optimize_program(&mut program);
    crate::monomorph::monomorphize_program(&mut program);

    // 默认使用 CHIR 路径，设置 NO_CHIR=1 可回退到旧路径
    if std::env::var("NO_CHIR").is_err() {
        // 新路径: AST → CHIR → WASM
        let mut chir_program = crate::chir::lower_program(&program)?;
        if std::env::var("NO_CHIR_OPT").is_err() {
            crate::chir::optimize::optimize_chir(&mut chir_program);
        }
        let mut codegen = crate::codegen::chir_codegen::CHIRCodeGen::new();
        Ok(codegen.generate(&chir_program))
    } else {
        // 旧路径: AST → WASM
        let mut codegen = CodeGen::new();
        Ok(codegen.compile(&program))
    }
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
    fn test_parse_extend_interface_with_type_args() {
        let source = "package std.sort
extend<T> Array<T> <: SortByExtension<T> {
}
";
        let result = parse_source(source);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        let program = result.unwrap();
        assert_eq!(program.extends.len(), 1);
        assert_eq!(program.extends[0].target_type, "Array");
        assert_eq!(program.extends[0].interface.as_deref(), Some("SortByExtension"));
    }

    #[test]
    #[ignore] // 仅需时手动运行：cargo test --release test_parse_sort_cj -- --ignored
    fn test_parse_sort_cj() {
        let path = "third_party/cangjie_runtime/std/libs/std/sort/sort.cj";
        let source = fs::read_to_string(path).unwrap();
        let result = parse_source(&source);
        assert!(result.is_ok(), "parse sort.cj failed: {:?}", result);
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
            struct Point { var x: Int64 = 0, var y: Int64 = 0 }
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
    fn test_get_vendor_std_dir_not_found() {
        let tmp = std::env::temp_dir().join("cjwasm_no_vendor");
        let _ = fs::create_dir_all(&tmp).ok();
        let _out = get_vendor_std_dir(&tmp);
        // 无 third_party 时可为 None 或由环境变量决定
        let _ = fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_resolve_l1_std_vendor() {
        let tmp = std::env::temp_dir().join("cjwasm_test_l1_vendor");
        let _ = fs::remove_dir_all(&tmp).ok();
        let vendor = tmp.join("third_party/cangjie_runtime/std/libs/std");
        let io_dir = vendor.join("overflow");
        let _ = fs::create_dir_all(&io_dir).ok();
        let _ = fs::write(io_dir.join("wrapping_op.cj"), "package std.overflow\n");
        let _ = fs::write(io_dir.join("checked_op.cj"), "package std.overflow\n");

        let vendor_ref = vendor.as_path();
        let bases: &[&Path] = &[tmp.as_path()];
        let files = resolve_import_to_files(
            &["std".to_string(), "overflow".to_string()],
            bases,
            Some(vendor_ref),
        );
        assert!(!files.is_empty(), "L1 std.overflow 应解析到 vendor 下多个 .cj");
        assert!(files.iter().all(|p| p.extension().map_or(false, |e| e == "cj")));

        let _ = fs::remove_dir_all(&tmp).ok();
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
        let bases = [tmp.as_path()];
        let files = collect_import_files(&program, &bases, &mut visited, None);
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
        let bases = [tmp.as_path()];
        let files = collect_import_files(&program, &bases, &mut visited, None);
        assert_eq!(files.len(), 1);

        // 再次收集不会重复
        let files2 = collect_import_files(&program, &bases, &mut visited, None);
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
            struct Point { var x: Int64 = 0, var y: Int64 = 0 }
            
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
    fn test_l1_std_top_modules() {
        let modules = l1_std_top_modules();
        assert!(!modules.is_empty());
        assert!(modules.contains(&"io"));
        assert!(modules.contains(&"console"));
    }

    #[test]
    fn test_compile_if_else() {
        let source = "func main(): Int64 { let x = 10; return if x > 5 { 1 } else { 0 } }";
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
        assert!(result.unwrap().len() >= 8);
    }

    #[test]
    fn test_compile_while_loop() {
        let source = r#"
            func main(): Int64 {
                var i = 0
                while i < 5 { i = i + 1 }
                return i
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_for_range() {
        let source = r#"
            func main(): Int64 {
                var sum = 0
                for i in 0..5 { sum = sum + i }
                return sum
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_match_expr() {
        let source = r#"
            func main(): Int64 {
                let n = 2
                return match n {
                    case 0 => 10
                    case 1 => 20
                    case _ => 30
                }
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_try_catch() {
        let source = r#"
            func main(): Int64 {
                return try { 42 } catch { 0 }
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_interface_and_extend() {
        let source = r#"
            struct Point { var x: Int64 = 0, var y: Int64 = 0 }
            interface Printable { func print(): Unit; }
            extend Point: Printable {
                func print(): Unit { }
            }
            func main(): Int64 { return 0 }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_class_inheritance() {
        let source = r#"
            open class Base { var id: Int64; init(i: Int64) { this.id = i } }
            class Derived <: Base { init(i: Int64) { super(i) } }
            func main(): Int64 {
                let d = Derived(1)
                return d.id
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_string_interpolation() {
        let source = r#"
            func main(): Int64 {
                let x = 42
                let s = "value: ${x}"
                return 0
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compile_tuple_and_array() {
        let source = r#"
            func main(): Int64 {
                let t = (1, 2)
                let arr = [10, 20, 30]
                return t.0 + arr[0]
            }
        "#;
        let result = compile_source_to_wasm(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_source_with_block_comment() {
        let source = "/* comment */ func main(): Int64 { return 0 }";
        let result = parse_source(source);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().functions.len(), 1);
    }

    #[test]
    fn test_parse_source_with_nested_block_comment() {
        // 规范 1.5(2)：嵌套块注释整体视为一个注释
        let source = "/* outer /* inner */ still comment */ func main(): Int64 { return 0 }";
        let result = parse_source(source);
        assert!(result.is_ok(), "nested comment should parse: {:?}", result.err());
        assert_eq!(result.unwrap().functions.len(), 1);
    }

    #[test]
    fn test_parse_source_with_deeply_nested_block_comment() {
        let source =
            "/* a /* b /* c */ b */ a */ func main(): Int64 { return 0 }";
        let result = parse_source(source);
        assert!(result.is_ok(), "3-level comment should parse: {:?}", result.err());
        assert_eq!(result.unwrap().functions.len(), 1);
    }

    #[test]
    fn test_parse_source_preserves_cjk_rune_literal() {
        // strip_quote_contents / strip_block_comments 必须按 UTF-8 字符复制，不能按字节拆坏 CJK
        let source = "func main(): Int64 { let c = r'严'; let d = '文'; let s = \"中文\"; return 0 }";
        let result = parse_source(source);
        assert!(result.is_ok(), "CJK literals should parse: {:?}", result.err());
    }

    #[test]
    fn test_strip_quote_contents_preserves_utf8() {
        let source = "let a = \"中文\" let b = r'严'";
        let out = strip_quote_contents(source);
        assert!(out.contains("中文"));
        assert!(out.contains("r'严'"));
    }

    #[test]
    fn test_find_comment_close_nested() {
        let src = b"/* outer /* inner */ tail */x";
        let end = find_comment_close(src, 2);
        assert_eq!(end, src.len() - 1);
        assert_eq!(&src[end..], b"x");
    }
}

#[cfg(test)]
mod batch04_tests {
    use super::*;

    #[test]
    fn test_line_comment_containing_block_open() {
        // `//*/` 是行注释，不能把其中的 `/*` 当作块注释起始
        let src = "//*/. tail words\nmain() { }\n";
        let out = strip_block_comments(src);
        assert!(out.contains("//*/. tail words"), "line comment preserved: {:?}", out);
        assert!(out.contains("main()"), "code preserved: {:?}", out);
    }

    #[test]
    fn test_block_comment_with_quotes_and_nesting() {
        // 注释内的引号不参与嵌套计数，`/* /* */` 仍按嵌套处理
        let src = "/* a \" /* b */ still */ main() { }\n";
        assert!(!strip_block_comments(src).contains("still"));
    }

    #[test]
    fn test_separate_hex_range() {
        let out = separate_hex_range("let a = 0x1..5\nlet b = 0x1.8p3\n");
        assert!(out.contains("0x1 ..5"), "hex range separated: {:?}", out);
        assert!(out.contains("0x1.8p3"), "hex float untouched: {:?}", out);
    }

    #[test]
    fn test_hex_range_parses() {
        assert!(parse_source("main() { let a = 0x1..5 }").is_ok());
    }

    #[test]
    fn test_string_with_block_comment_text_preserved() {
        let src = "main() { let s = \"/* not a comment */\" }\n";
        let out = strip_block_comments(src);
        assert!(out.contains("/* not a comment */"), "string content preserved: {:?}", out);
    }
}
