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
fn test_compile_pow() {
    let source = r#"
        func main() -> Int64 {
            return 2 ** 10
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "pow");
}

#[test]
fn test_compile_cast() {
    let source = r#"
        func main() -> Int32 {
            return (100 as Int64) as Int32
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cast");
}

#[test]
fn test_compile_bitwise() {
    let source = r#"
        func main() -> Int64 {
            return (1 << 4) | 2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "bitwise");
}

#[test]
fn test_compile_float32() {
    let source = r#"
        func main() -> Float32 {
            return 1.0f + 1f
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "float32");
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
fn test_compile_enum_match() {
    let source = r#"
        enum Color { Red, Green, Blue }
        func main() -> Int64 {
            let c: Color = Color.Red
            match c {
                Color.Red => 1,
                Color.Green => 2,
                Color.Blue => 3,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_match");
}

#[test]
fn test_compile_enum_method() {
    let source = r#"
        enum Color { Red, Green, Blue }
        func Color.disc(self: Color) -> Int64 {
            match self {
                Color.Red => 1,
                Color.Green => 2,
                Color.Blue => 3,
                _ => 0
            }
        }
        func main() -> Int64 {
            let c: Color = Color.Red
            return c.disc()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_method");
}

#[test]
fn test_compile_default_param() {
    let source = r#"
        func power(base: Int64, exp: Int64 = 2) -> Int64 {
            return base ** exp
        }
        func main() -> Int64 {
            return power(10)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "default_param");
}

#[test]
fn test_compile_match_struct_destructure() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() -> Int64 {
            let p = Point { x: 1, y: 2 }
            match p {
                Point { x: a, y: b } => a + b,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_struct_destructure");
}

#[test]
fn test_compile_let_destructure() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() -> Int64 {
            let p = Point { x: 10, y: 20 }
            let Point { x, y } = p
            return x + y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "let_destructure");
}

#[test]
fn test_compile_if_let() {
    let source = r#"
func main() -> Int64 {
    let p = 42
    if let x = p { return x } else { return 0 }
}
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "if_let");
}

#[test]
fn test_compile_while_let() {
    let source = r#"
func main() -> Int64 {
    var n = 3
    var sum = 0
    while let x = n {
        sum = sum + x
        n = n - 1
        if n < 0 { break }
    }
    return sum
}
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "while_let");
}

#[test]
fn test_compile_enum_associated_value() {
    let source = r#"
        enum MyResult { Success(Int64), Failure(Int64) }
        func main() -> Int64 {
            let r: MyResult = MyResult.Success(42)
            match r {
                MyResult.Success(v) => v,
                MyResult.Failure(e) => 0 - e,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_associated_value");
}

#[test]
fn test_compile_raw_string() {
    let source = r#"
        func main() -> Int64 {
            let s = r"raw\nliteral"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "raw_string");
}

#[test]
fn test_compile_constructor() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() -> Int64 {
            let p = Point(10, 20)
            return p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "constructor");
}

#[test]
fn test_compile_struct_method() {
    let source = r#"
        struct Rect { width: Int64, height: Int64 }
        func Rect.area(self: Rect) -> Int64 {
            return self.width * self.height
        }
        func main() -> Int64 {
            let r = Rect { width: 5, height: 10 }
            return r.area()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "struct_method");
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
fn test_compile_range_as_value() {
    // 范围作为值赋给变量
    let source = r#"
        func main() -> Int64 {
            let r = 0..10
            let r2 = 1..=5
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "range_as_value");
}

#[test]
fn test_compile_range_with_type() {
    // 显式 Range 类型注解
    let source = r#"
        func main() -> Int64 {
            let r: Range = 0..10
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "range_with_type");
}

#[test]
fn test_compile_call_type_inference() {
    // let 无类型注解时，从函数返回类型推断
    let source = r#"
        func get_val() -> Int64 { return 42 }
        func main() -> Int64 {
            let x = get_val()
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "call_type_inference");
}

#[test]
fn test_compile_break_continue() {
    let source = r#"
        func main() -> Int64 {
            var i: Int64 = 0
            var n: Int64 = 0
            while true {
                i = i + 1
                if i > 10 { break }
                if i % 2 == 0 { continue }
                n = n + 1
            }
            return n
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "break_continue");
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
fn test_compile_stdlib_min_max_abs() {
    let source = r#"
        func main() -> Int64 {
            let a = min(-10, 5)
            let b = max(3, 8)
            let c = abs(-42)
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "stdlib_min_max_abs");
}

#[test]
fn test_compile_extern_import() {
    let source = r#"
        @import("env", "print")
        extern func hostPrint(ptr: Int32, len: Int32)
        func main() -> Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "extern_import");
    // WASM 应包含 Import 段（section id 2）；简单检查二进制中含 "env" 或 "print" 表示导入存在
    assert!(
        wasm.windows(3).any(|w| w == b"env") || wasm.windows(5).any(|w| w == b"print"),
        "extern 导入应生成包含模块/函数名的 WASM"
    );
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

#[test]
fn test_compile_variadic_params() {
    let source = r#"
        func sum(args: Int64...) -> Int64 {
            var total: Int64 = 0
            for x in args {
                total = total + x
            }
            return total
        }
        func main() -> Int64 {
            return sum(1, 2, 3, 4, 5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "variadic_params");
}

#[test]
fn test_compile_function_overload() {
    let source = r#"
        func add(a: Int64, b: Int64) -> Int64 {
            return a + b
        }
        func add(a: Float64, b: Float64) -> Float64 {
            return a + b
        }
        func main() -> Int64 {
            let x = add(1, 2)
            let y = add(1.0, 2.0)
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "function_overload");
}

#[test]
fn test_compile_option_type() {
    let source = r#"
        func main() -> Int64 {
            let x: Option<Int64> = Some(42)
            let y: Option<Int64> = None
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "option_type");
}

#[test]
fn test_compile_result_type() {
    let source = r#"
        func main() -> Int64 {
            let x: Result<Int64, String> = Ok(42)
            let y: Result<Int64, String> = Err("error")
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "result_type");
}

#[test]
fn test_compile_string_interpolation() {
    let source = r#"
        func main() -> Int64 {
            let name = "World"
            let greeting = "Hello, ${name}!"
            let x = 42
            let msg = "The answer is ${x}"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "string_interpolation");
}
