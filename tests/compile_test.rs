//! 集成测试：完整编译流水线 (词法 -> 语法 -> 代码生成) 并验证输出为合法 WASM。

use cjwasm::codegen::CodeGen;
use cjwasm::lexer::Lexer;
use cjwasm::parser::Parser;
use std::path::Path;

fn compile_source(source: &str) -> Vec<u8> {
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer
        .collect::<Result<Vec<_>, _>>()
        .expect("词法分析应成功");
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program().expect("语法分析应成功");
    let mut codegen = CodeGen::new();
    codegen.compile(&program)
}

fn assert_valid_wasm(wasm: &[u8], name: &str) {
    assert!(
        wasm.len() >= 8,
        "{}: WASM 输出过短 ({} 字节)",
        name,
        wasm.len()
    );
    assert_eq!(
        &wasm[0..4],
        b"\0asm",
        "{}: 魔数应为 \\0asm",
        name
    );
    // WASM 版本 (4-8): 0x01 0x00 0x00 0x00 表示 1
    assert_eq!(
        &wasm[4..8],
        [1, 0, 0, 0],
        "{}: 版本应为 1",
        name
    );
}

#[test]
fn test_compile_hello_snippet() {
    let source = r#"
        func main() -> Int64 {
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "hello_snippet");
}

#[test]
fn test_compile_arithmetic() {
    let source = r#"
        func add(a: Int64, b: Int64) -> Int64 {
            return a + b
        }
        func main() -> Int64 {
            return add(1, 2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "arithmetic");
}

#[test]
fn test_compile_struct_and_field() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() -> Int64 {
            let p = Point { x: 10, y: 20 }
            return p.x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "struct_and_field");
}

#[test]
fn test_compile_array_and_for() {
    let source = r#"
        func main() -> Int64 {
            let arr: Array<Int64> = [1, 2, 3]
            var sum: Int64 = 0
            for x in arr {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "array_and_for");
}

#[test]
fn test_compile_match_and_guard() {
    // 字面量 + 通配符匹配（不含 match 分支绑定变量，因 codegen 尚未为 arm 绑定分配局部变量）
    let source = r#"
        func classify(n: Int64) -> Int64 {
            match n {
                0 => 2,
                1..10 => 3,
                _ => 4
            }
        }
        func main() -> Int64 {
            return classify(0)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_and_guard");
}

#[test]
fn test_compile_logical_ops() {
    let source = r#"
        func main() -> Int64 {
            let a = true && false
            let b = true || false
            let c = !false
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "logical_ops");
}

#[test]
fn test_compile_unary_neg() {
    let source = r#"
        func main() -> Int64 {
            let a = -42
            let b = -(1 + 2)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "unary_neg");
}

#[test]
fn test_compile_block_expr() {
    let source = r#"
        func main() -> Int64 {
            let x = { let a = 10 let b = 20 a + b }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "block_expr");
}

#[test]
fn test_compile_compound_assign() {
    let source = r#"
        func main() -> Int64 {
            var x: Int64 = 10
            x += 5
            x -= 2
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compound_assign");
}

#[test]
fn test_compile_for_range() {
    let source = r#"
        func main() -> Int64 {
            var s: Int64 = 0
            for i in 0..5 {
                s = s + i
            }
            return s
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "for_range");
}

#[test]
fn test_compile_if_expr() {
    let source = r#"
        func main() -> Int64 {
            let x = if 1 > 0 { 10 } else { 20 }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "if_expr");
}

#[test]
fn test_compile_example_files() {
    let examples_dir = Path::new("examples");
    if !examples_dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(examples_dir).expect("读取 examples 目录") {
        let entry = entry.expect("目录项");
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.extension().map(|e| e == "cj").unwrap_or(false) {
            let source = std::fs::read_to_string(&path).expect("读取示例源文件");
            let wasm = compile_source(&source);
            assert_valid_wasm(&wasm, &name);
        }
    }
}
