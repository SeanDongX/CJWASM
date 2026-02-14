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
    let mut program = parser.parse_program().expect("语法分析应成功");
    cjwasm::optimizer::optimize_program(&mut program);
    cjwasm::monomorph::monomorphize_program(&mut program);
    let mut codegen = CodeGen::new();
    codegen.compile(&program)
}

fn compile_source_result(source: &str) -> Result<Vec<u8>, String> {
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("词法错误: {}", e))?;
    let mut parser = Parser::new(tokens);
    let program = parser
        .parse_program()
        .map_err(|e| format!("语法错误: {:?}", e))?;
    let mut program = program;
    cjwasm::optimizer::optimize_program(&mut program);
    cjwasm::monomorph::monomorphize_program(&mut program);
    let mut codegen = CodeGen::new();
    Ok(codegen.compile(&program))
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
        func main() : Int64 {
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "hello_snippet");
}

#[test]
fn test_compile_arithmetic() {
    let source = r#"
        func add(a: Int64, b: Int64) : Int64 {
            return a + b
        }
        func main() : Int64 {
            return add(1, 2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "arithmetic");
}

#[test]
fn test_compile_pow() {
    let source = r#"
        func main() : Int64 {
            return 2 ** 10
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "pow");
}

#[test]
fn test_compile_cast() {
    let source = r#"
        func main() : Int32 {
            return (100 as Int64) as Int32
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cast");
}

#[test]
fn test_compile_bitwise() {
    let source = r#"
        func main() : Int64 {
            return (1 << 4) | 2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "bitwise");
}

#[test]
fn test_compile_float32() {
    let source = r#"
        func main() : Float32 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func Color.disc(self: Color) : Int64 {
            match self {
                Color.Red => 1,
                Color.Green => 2,
                Color.Blue => 3,
                _ => 0
            }
        }
        func main() : Int64 {
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
        func power(base: Int64, exp: Int64 = 2) : Int64 {
            return base ** exp
        }
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
func main() : Int64 {
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
func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func Rect.area(self: Rect) : Int64 {
            return self.width * self.height
        }
        func main() : Int64 {
            let r = Rect { width: 5, height: 10 }
            return r.area()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "struct_method");
}

/// cjc-style struct: init constructor, methods inside body, field default values
#[test]
fn test_compile_cjc_style_struct() {
    let source = r#"
        struct Point {
            var x: Int64 = 0
            var y: Int64 = 0
            init(x: Int64, y: Int64) { }
            func area(this: Point) : Int64 {
                return this.x * this.y
            }
        }
        func main() : Int64 {
            let p = Point { x: 5, y: 10 }
            return p.area()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cjc_style_struct");
}

#[test]
fn test_compile_array_and_for() {
    let source = r#"
        func main() : Int64 {
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
        func classify(n: Int64) : Int64 {
            match n {
                0 => 2,
                1..10 => 3,
                _ => 4
            }
        }
        func main() : Int64 {
            return classify(0)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_and_guard");
}

#[test]
fn test_compile_logical_ops() {
    let source = r#"
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func get_val() : Int64 { return 42 }
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
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
        foreign func hostPrint(ptr: Int32, len: Int32)
        func main() : Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "extern_import");
    // WASM 应包含 Import 段（section id 2）；简单检查二进制中含 "env" 或 "print" 表示导入存在
    assert!(
        wasm.windows(3).any(|w| w == b"env") || wasm.windows(5).any(|w| w == b"print"),
        "foreign 导入应生成包含模块/函数名的 WASM"
    );
}

#[test]
fn test_compile_example_files() {
    let examples_dir = Path::new("examples");
    if !examples_dir.exists() {
        return;
    }
    let mut files: Vec<_> = std::fs::read_dir(examples_dir)
        .expect("读取 examples 目录")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "cj").unwrap_or(false))
        .collect();
    files.sort_by_key(|e| e.file_name());
    for entry in files {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();
        let source = std::fs::read_to_string(&path).expect("读取示例源文件");
        let wasm = compile_source_result(&source)
            .unwrap_or_else(|e| panic!("编译失败 ({}): {}", name, e));
        assert_valid_wasm(&wasm, &name);
    }
}

#[test]
fn test_compile_variadic_params() {
    let source = r#"
        func sum(args: Int64...) : Int64 {
            var total: Int64 = 0
            for x in args {
                total = total + x
            }
            return total
        }
        func main() : Int64 {
            return sum(1, 2, 3, 4, 5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "variadic_params");
}

#[test]
fn test_compile_function_overload() {
    let source = r#"
        func add(a: Int64, b: Int64) : Int64 {
            return a + b
        }
        func add(a: Float64, b: Float64) : Float64 {
            return a + b
        }
        func main() : Int64 {
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
        func main() : Int64 {
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
        func main() : Int64 {
            let x: Result<Int64, String> = Ok(42)
            let y: Result<Int64, String> = Err("error")
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "result_type");
}

#[test]
fn test_compile_interface_and_class() {
    let source = r#"
        interface Drawable { func area() : Int64; }
        class Rect {
            private var width: Int64;
            private var height: Int64;
            func area(self: Rect) : Int64 { return self.width * self.height }
        }
        func main() : Int64 {
            let r = Rect { width: 5, height: 10 }
            return r.area()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "interface_and_class");
}

#[test]
fn test_compile_generic() {
    let source = r#"
        struct Pair<T, U> { first: T, second: U }
        func identity<T>(value: T): T { return value }
        func main() : Int64 {
            let p = Pair<Int64, Int64> { first: 1, second: 2 }
            let x = identity<Int64>(42)
            return p.first + x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic");
}

#[test]
fn test_compile_string_interpolation() {
    let source = r#"
        func main() : Int64 {
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

// ====================================================================
// 覆盖率补充测试 — 集中覆盖未测试的代码路径
// ====================================================================

// --- 类继承 + vtable + super + ~init ---
#[test]
fn test_compile_class_inheritance_super_deinit() {
    let source = r#"
        open class Animal {
            var kind: Int64;
            var age: Int64;

            init(kind: Int64, age: Int64) {
                this.kind = kind
                this.age = age
            }

            ~init { }

            func speak(self: Animal) : Int64 {
                return self.kind
            }

            func getAge(self: Animal) : Int64 {
                return self.age
            }
        }

        class Dog <: Animal {
            var breed: Int64;

            init(age: Int64, breed: Int64) {
                super(1, age)
                this.breed = breed
            }

            override func speak(self: Dog) : Int64 {
                return 100 + self.breed
            }

            func superSpeak(self: Dog) : Int64 {
                return super.speak()
            }
        }

        func main() : Int64 {
            let d = Dog(5, 42)
            let s = d.speak()
            let a = d.getAge()
            return s + a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_inheritance_super_deinit");
}

// --- class prop (getter/setter) ---
#[test]
fn test_compile_class_prop_getter_setter() {
    let source = r#"
        class Counter {
            var count: Int64;

            init(n: Int64) {
                this.count = n
            }

            prop value: Int64 {
                get() { return this.count }
                set(v) { this.count = v }
            }

            func inc(self: Counter) : Int64 {
                self.count = self.count + 1
                return self.count
            }
        }

        func main() : Int64 {
            let c = Counter(10)
            let v1 = c.inc()
            return v1
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_prop_getter_setter");
}

// --- try-catch-finally + throw ---
#[test]
fn test_compile_try_catch_finally_throw() {
    let source = r#"
        func safeDivide(a: Int64, b: Int64) : Int64 {
            var result: Int64 = 0
            var cleaned: Int64 = 0
            try {
                if b == 0 {
                    throw 0
                }
                result = a / b
            } catch(e) {
                result = -1
            } finally {
                cleaned = 1
            }
            return result + cleaned
        }

        func tryCatchOnly(x: Int64) : Int64 {
            try {
                if x < 0 {
                    throw 0
                }
                return x * 2
            } catch(e) {
                return 0
            }
        }

        func main() : Int64 {
            let a = safeDivide(10, 2)
            let b = safeDivide(10, 0)
            let c = tryCatchOnly(7)
            let d = tryCatchOnly(-1)
            return a + b + c + d
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "try_catch_finally_throw");
}

// --- throws 声明 ---
#[test]
fn test_compile_throws_declaration() {
    let source = r#"
        func validate(x: Int64) : Int64 {
            if x < 0 {
                throw 0
            }
            return x
        }

        func process(x: Int64) : Int64 {
            return validate(x) + 1
        }

        func main() : Int64 {
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "throws_declaration");
}

// --- import + package ---
#[test]
fn test_compile_import_module() {
    let source = r#"
        package test.main

        import bar.baz.foo
        import std.io
        import std.math

        func main() : Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "import_module");
}

// --- interface 继承 + 默认方法 + 关联类型 ---
#[test]
fn test_compile_interface_inheritance_default_assoc() {
    let source = r#"
        interface Base {
            func id() : Int64;
        }

        interface Extended: Base {
            type Element;

            func doubled() : Int64 {
                return 0
            }
        }

        class MyClass {
            var x: Int64;
            init(x: Int64) { this.x = x }
            func id(self: MyClass) : Int64 { return self.x }
        }

        extend MyClass: Extended {
            type Element = Int64;
            func doubled(self: MyClass) : Int64 {
                return self.x * 2
            }
        }

        func main() : Int64 {
            let c = MyClass(21)
            let a = c.id()
            let b = c.doubled()
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "interface_inheritance_default_assoc");
}

// --- 泛型类 + 约束检查 ---
#[test]
fn test_compile_generic_class_constraints() {
    let source = r#"
        class Box<T> {
            var value: T;
            init(value: T) { this.value = value }
            func get(self: Box<T>): T { return self.value }
        }

        func identity<T: Comparable>(x: T): T { return x }

        struct Wrapper<T: Hashable> { inner: T }

        func main() : Int64 {
            let b = Box<Int64>(42)
            let v = b.get()
            let w = Wrapper<Int64> { inner: 5 }
            let id_val = identity<Int64>(99)
            return v + id_val + w.inner
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_class_constraints");
}

// --- 非泛型枚举 + 关联值 ---
#[test]
fn test_compile_enum_with_payload() {
    let source = r#"
        enum Action {
            Move(Int64)
            Stop
            Jump(Int64)
        }

        func handle(a: Action) : Int64 {
            return match a {
                Action.Move(dist) => dist,
                Action.Stop => 0,
                Action.Jump(h) => h * 2,
                _ => -1
            }
        }

        func main() : Int64 {
            let a = Action.Move(10)
            let b = Action.Stop
            let c = Action.Jump(5)
            return handle(a) + handle(b) + handle(c)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_with_payload");
}

// --- 泛型多用途 ---
#[test]
fn test_compile_generic_multi_instantiation() {
    let source = r#"
        func wrap<T>(x: T): T { return x }

        func main() : Int64 {
            let a = wrap<Int64>(42)
            let b = wrap<Float64>(3.14)
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_multi_instantiation");
}

// --- where 子句 ---
#[test]
fn test_compile_where_clause() {
    let source = r#"
        func compare<T>(a: T, b: T) : Int64 where T: Comparable {
            return 0
        }

        enum Holder<T> where T: Comparable {
            Val(T)
            Empty
        }

        func main() : Int64 {
            let r = compare<Int64>(1, 2)
            return r
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "where_clause");
}

// --- 小整数 / 无符号类型运算 ---
#[test]
fn test_compile_small_int_unsigned_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 10
            let b: Int8 = 20
            let c = a + b

            let d: UInt8 = 200
            let e: UInt8 = 50
            let f = d + e

            let g: Int16 = 1000
            let h: Int16 = 2000
            let i = g + h

            let j: UInt16 = 60000
            let k: UInt16 = 5000
            let l = j + k

            let m: UInt32 = 100000
            let n: UInt64 = 999999

            let o: Int32 = 42
            let p = o * o

            return (c as Int64) + (f as Int64)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "small_int_unsigned_ops");
}

// --- 一元运算符（BitNot, Neg on various types）---
#[test]
fn test_compile_unary_ops_all() {
    let source = r#"
        func main() : Int64 {
            let a = ~1
            let b = ~(1 as Int32)
            let c = -1
            let d = -(1.0 as Float32)
            let e = -2.0
            let f = !true
            return a + (b as Int64) + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "unary_ops_all");
}

// --- Cast (类型转换) 各种路径 ---
#[test]
fn test_compile_cast_various() {
    let source = r#"
        func main() : Int64 {
            let a = 100 as Int8
            let b = 100 as Int16
            let c = 100 as UInt8
            let d = 100 as UInt16
            let e = 100 as UInt32
            let f = 100 as UInt64
            let g = 100 as Float32
            let h = 100 as Float64
            let i = 1.5 as Int64
            let j = 1.5 as Int32
            let k = 1.5 as Float32
            let l = (1 as Int32) as Int64
            let m = (1 as Int32) as Float64
            return (a as Int64) + (b as Int64) + (c as Int64) + i
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cast_various");
}

// --- Match: 所有模式类型 ---
#[test]
fn test_compile_match_all_patterns() {
    let source = r#"
        enum Color {
            Red
            Green
            Blue
        }

        struct Point { x: Int64, y: Int64 }

        func matchLiteral(n: Int64) : Int64 {
            return match n {
                0 => 100,
                1 => 200,
                x if x > 10 => 999,
                _ => 0
            }
        }

        func matchEnum(c: Color) : Int64 {
            return match c {
                Color.Red => 1,
                Color.Green => 2,
                Color.Blue => 3,
                _ => 0
            }
        }

        func matchStruct(p: Point) : Int64 {
            return match p {
                Point { x, y } => x + y
            }
        }

        func matchOr(n: Int64) : Int64 {
            return match n {
                1 | 2 | 3 => 10,
                4 | 5 => 20,
                _ => 0
            }
        }

        func main() : Int64 {
            let a = matchLiteral(0)
            let b = matchLiteral(20)
            let c = matchLiteral(5)
            let d = matchEnum(Color.Red)
            let i = matchOr(2)
            let k = matchStruct(Point { x: 3, y: 4 })
            return a + d + i + k
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_all_patterns");
}

// --- Match: Option + Result ---
#[test]
fn test_compile_match_option_result() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            let a = match o {
                Some(v) => v,
                None => 0
            }

            let r: Result<Int64, String> = Ok(10)
            let b = match r {
                Ok(v) => v,
                Err(_) => -1
            }

            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_option_result");
}

// --- Match: Range pattern ---
#[test]
fn test_compile_match_range_pattern() {
    let source = r#"
        func classify(n: Int64) : Int64 {
            return match n {
                0..10 => 1,
                10..100 => 2,
                _ => 0
            }
        }

        func main() : Int64 {
            return classify(5) + classify(50) + classify(200)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_range_pattern");
}

// --- Tuple 操作 ---
#[test]
fn test_compile_tuple_ops() {
    let source = r#"
        func main() : Int64 {
            let t = (10, 20)
            let a = t.0
            let b = t.1
            let t2 = (1, 2, 3)
            let c = t2.2
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "tuple_ops");
}

// --- 默认参数 + 可变参数 ---
#[test]
fn test_compile_default_and_variadic_params() {
    let source = r#"
        func greet(name: String, times: Int64 = 1) : Int64 {
            return times
        }

        func sum(args: Int64...) : Int64 {
            var total: Int64 = 0
            for x in args {
                total = total + x
            }
            return total
        }

        func main() : Int64 {
            let a = greet("Alice")
            let b = greet("Bob", 3)
            let c = sum(1, 2, 3, 4, 5)
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "default_and_variadic_params");
}

// --- Null 合并 (??) ---
#[test]
fn test_compile_null_coalesce() {
    let source = r#"
        func main() : Int64 {
            let a: Option<Int64> = Some(42)
            let b: Option<Int64> = None
            let x = a ?? 0
            let y = b ?? 99
            return x + y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "null_coalesce");
}

// --- if-let / while-let ---
#[test]
fn test_compile_if_let_while_let() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            var result: Int64 = 0

            if let Some(v) = o {
                result = v
            }

            var count: Int64 = 0
            var opt: Option<Int64> = Some(1)
            while let Some(v) = opt {
                count = count + v
                opt = None
            }

            return result + count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "if_let_while_let");
}

// --- Loop + break + continue ---
#[test]
fn test_compile_loop_break_continue() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            var sum: Int64 = 0
            loop {
                if i >= 10 {
                    break
                }
                i = i + 1
                if i % 2 == 0 {
                    continue
                }
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "loop_break_continue");
}

// --- 字符串插值（覆盖 Interpolate 编译）---
#[test]
fn test_compile_string_interpolation_types() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 42
            let y: Float64 = 3.14
            let z: Bool = true
            let s1 = "value=${x}"
            let s2 = "pi=${y}"
            let s3 = "flag=${z}"
            let s4 = "combo: ${x} and ${y}"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "string_interpolation_types");
}

// --- Char 类型 ---
#[test]
fn test_compile_char_type() {
    let source = r#"
        func main() : Int64 {
            let c: Rune = 'A'
            let d: Rune = 'Z'
            let n = (c as Int64) + (d as Int64)
            return n
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "char_type");
}

// --- Range 作为值 ---
#[test]
fn test_compile_range_value_and_for() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..10 {
                sum = sum + i
            }
            var sum2: Int64 = 0
            for j in 0..=5 {
                sum2 = sum2 + j
            }
            let arr = [10, 20, 30]
            var sum3: Int64 = 0
            for v in arr {
                sum3 = sum3 + v
            }
            return sum + sum2 + sum3
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "range_value_and_for");
}

// --- 位运算 ---
#[test]
fn test_compile_bitwise_all() {
    let source = r#"
        func main() : Int64 {
            let a = 0xFF & 0x0F
            let b = 0xF0 | 0x0F
            let c = 0xFF ^ 0x0F
            let d = 1 << 4
            let e = 256 >> 2
            let f = 256 >> 2
            return a + b + c + d + e + f
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "bitwise_all");
}

// --- 常量折叠: 浮点 + 比较 + 除零 ---
#[test]
fn test_compile_const_fold_float_cmp() {
    let source = r#"
        func main() : Int64 {
            let a = 1.0 + 2.0
            let b = 3.0 * 4.0
            let c = 10.0 / 3.0
            let d = 10.0 - 3.0
            let e = 1 == 1
            let f = 2 != 3
            let g = 1 < 2
            let h = 2 > 1
            let i = 1 <= 1
            let j = 2 >= 2
            let k = 1 & 1
            let l = 0 | 1
            let m = 1 ^ 0
            let n = 1 << 2
            let o = 8 >> 1
            let p = 8 >> 1
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "const_fold_float_cmp");
}

// --- 赋值：数组下标赋值 + 结构体字段赋值 ---
#[test]
fn test_compile_assign_index_field() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }

        func main() : Int64 {
            var arr = [1, 2, 3]
            arr[0] = 100

            var p = Point { x: 1, y: 2 }
            p.x = 10
            p.y = 20

            return arr[0] + p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "assign_index_field");
}

// --- struct 解构 let ---
#[test]
fn test_compile_struct_destructure() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }

        func main() : Int64 {
            let p = Point { x: 3, y: 4 }
            let Point { x, y } = p
            return x + y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "struct_destructure");
}

// --- Lambda 表达式 ---
#[test]
fn test_compile_lambda_expression() {
    let source = r#"
        func main() : Int64 {
            let f = (x: Int64) : Int64 { x * 2 }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "lambda_expression");
}

// --- abstract class / sealed class ---
#[test]
fn test_compile_abstract_sealed_class() {
    let source = r#"
        abstract class Shape {
            var name: Int64;
        }

        sealed class Container {
            var size: Int64;
            init(size: Int64) { this.size = size }
        }

        class Box {
            var width: Int64;
            var height: Int64;
            init(w: Int64, h: Int64) {
                this.width = w
                this.height = h
            }
            func area(self: Box) : Int64 {
                return self.width * self.height
            }
        }

        func main() : Int64 {
            let b = Box(3, 4)
            let c = Container(10)
            return b.area() + c.size
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "abstract_sealed_class");
}

// --- 枚举方法 + 关联值 ---
#[test]
fn test_compile_enum_methods_associated() {
    let source = r#"
        enum Shape {
            Circle(Int64)
            Rect(Int64)
            Unknown
        }

        func area(s: Shape) : Int64 {
            return match s {
                Shape.Circle(r) => r * r * 3,
                Shape.Rect(side) => side * side,
                Shape.Unknown => 0,
                _ => 0
            }
        }

        func main() : Int64 {
            let c = Shape.Circle(5)
            let r = Shape.Rect(4)
            let u = Shape.Unknown
            return area(c) + area(r) + area(u)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_methods_associated");
}

// --- Compound assignment (+=, -=, *=, /=, %=) ---
#[test]
fn test_compile_compound_assign_all() {
    let source = r#"
        func main() : Int64 {
            var a: Int64 = 10
            a += 5
            a -= 2
            a *= 3
            a /= 2
            a %= 7
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compound_assign_all");
}

// --- 错误处理: ? 操作符 + Result ---
#[test]
fn test_compile_try_operator_result() {
    let source = r#"
        func divide(a: Int64, b: Int64) : Result<Int64, String> {
            if b == 0 {
                return Err("div by zero")
            }
            return Ok(a / b)
        }

        func compute(x: Int64, y: Int64) : Result<Int64, String> {
            let r = divide(x, y)?
            return Ok(r * 2)
        }

        func main() : Int64 {
            let r = divide(10, 2)
            return match r {
                Ok(v) => v,
                Err(_) => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "try_operator_result");
}

// --- 多文件特性 (package + import) ---
#[test]
fn test_compile_module_declaration() {
    let source = r#"
        package my.app

        import std.io
        import std.math

        func helper() : Int64 {
            return 42
        }

        func main() : Int64 {
            return helper()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "module_declaration");
}

// --- 多重约束 <T: A & B> ---
#[test]
fn test_compile_multi_constraint() {
    let source = r#"
        func process<T: Comparable & Hashable>(x: T): T {
            return x
        }

        func main() : Int64 {
            return process<Int64>(42)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "multi_constraint");
}

// --- 多泛型参数的结构体 ---
#[test]
fn test_compile_multi_type_param_struct() {
    let source = r#"
        struct Pair<T, U> { first: T, second: U }

        func main() : Int64 {
            let p = Pair<Int64, Int64> { first: 10, second: 20 }
            return p.first + p.second
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "multi_type_param_struct");
}

// --- Float32 + mixed float ops ---
#[test]
fn test_compile_float32_mixed() {
    let source = r#"
        func main() : Int64 {
            let a: Float32 = 1.5f
            let b: Float32 = 2.5f
            let c = a + b
            let d = a * b
            let e = a - b
            let f = a / b
            let g: Float64 = 3.14
            let h = g + 1.0
            let i = g * 2.0
            let j = g - 1.0
            let k = g / 2.0
            return (c as Int64) + (h as Int64)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "float32_mixed");
}

// --- 比较运算 (各种类型) ---
#[test]
fn test_compile_comparison_ops_types() {
    let source = r#"
        func main() : Int64 {
            let a = 1 < 2
            let b = 1 > 2
            let c = 1 <= 1
            let d = 1 >= 1
            let e = 1 == 1
            let f = 1 != 2
            let g = 1.0 < 2.0
            let h = 1.0 > 2.0
            let i = 1.0 == 1.0
            let j = 1.0 != 2.0
            let k = (1 as Int32) < (2 as Int32)
            let l = (1 as UInt32) < (2 as UInt32)
            let m = (1 as UInt64) < (2 as UInt64)
            var result: Int64 = 0
            if a { result = result + 1 }
            if c { result = result + 1 }
            if e { result = result + 1 }
            if f { result = result + 1 }
            if g { result = result + 1 }
            if i { result = result + 1 }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "comparison_ops_types");
}

// --- 逻辑运算短路 ---
#[test]
fn test_compile_logical_short_circuit() {
    let source = r#"
        func main() : Int64 {
            let a = true && false
            let b = false || true
            let c = true && true
            let d = false || false
            var result: Int64 = 0
            if a { result = result + 1 }
            if b { result = result + 10 }
            if c { result = result + 100 }
            if !d { result = result + 1000 }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "logical_short_circuit");
}

// --- 多行字符串 + 原始字符串 + 转义 ---
#[test]
fn test_compile_string_varieties() {
    let source = r#"
        func main() : Int64 {
            let s1 = "hello\nworld"
            let s2 = "tab\there"
            let s3 = "quote\"inside"
            let s4 = "backslash\\"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "string_varieties");
}

// --- Error 类继承体系 ---
#[test]
fn test_compile_error_class_hierarchy() {
    let source = r#"
        open class CustomError <: Error {
            var code: Int64;
            init(code: Int64) {
                this.code = code
            }
        }

        class SpecificError <: CustomError {
            init() {
                super(404)
            }
        }

        func main() : Int64 {
            let e = CustomError(500)
            return e.code
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "error_class_hierarchy");
}

// --- Unsigned 除法 / 比较 ---
#[test]
fn test_compile_unsigned_div_cmp() {
    let source = r#"
        func main() : Int64 {
            let a: UInt32 = 100
            let b: UInt32 = 10
            let c = a / b
            let d = a % b
            let e = a > b
            let f = a < b
            let g: UInt64 = 1000
            let h: UInt64 = 100
            let i = g / h
            let j = g > h
            return (c as Int64) + (d as Int64) + (i as Int64)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "unsigned_div_cmp");
}

// --- Pow 运算 ---
#[test]
fn test_compile_pow_ops() {
    let source = r#"
        func main() : Int64 {
            let a = 2 ** 10
            let b = 3 ** 3
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "pow_ops");
}

// --- min/max/abs 内建函数 ---
#[test]
fn test_compile_builtin_min_max_abs() {
    let source = r#"
        func main() : Int64 {
            let a: Int64 = min(10, 20)
            let b: Int64 = max(10, 20)
            let c: Int64 = abs(-42)
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "builtin_min_max_abs");
}

// --- 块表达式 (block expr) ---
#[test]
fn test_compile_block_expr_complex() {
    let source = r#"
        func main() : Int64 {
            let result = {
                let a = 10
                let b = 20
                a + b
            }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "block_expr_complex");
}

// --- For-in array + for-in range inclusive ---
#[test]
fn test_compile_for_in_array_range() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30, 40]
            var sum: Int64 = 0
            for v in arr {
                sum = sum + v
            }
            for i in 0..=3 {
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "for_in_array_range");
}

// --- 复杂嵌套表达式 ---
#[test]
fn test_compile_complex_nested_expressions() {
    let source = r#"
        func fib(n: Int64) : Int64 {
            if n <= 1 {
                return n
            }
            return fib(n - 1) + fib(n - 2)
        }

        func main() : Int64 {
            let a = fib(10)
            let b = if a > 50 { a } else { 0 }
            let c = match b {
                0 => 0,
                _ => b * 2
            }
            return c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "complex_nested_expressions");
}

// ====================================================================
// 额外覆盖率补充 — 深度覆盖 codegen/parser/monomorph 特定路径
// ====================================================================

// --- WhileLet with Variant pattern ---
#[test]
fn test_compile_while_let_variant() {
    let source = r#"
        enum OptVal {
            Val(Int64)
            Empty
        }

        func main() : Int64 {
            var count: Int64 = 0
            var opt = OptVal.Val(10)
            while let OptVal.Val(v) = opt {
                count = count + v
                opt = OptVal.Empty
            }
            return count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "while_let_variant");
}

// --- for in inclusive range ---
#[test]
fn test_compile_for_inclusive_range() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..=10 {
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "for_inclusive_range");
}

// --- Var 语句 + Assign 语句 ---
#[test]
fn test_compile_var_assign_patterns() {
    let source = r#"
        struct Pt { x: Int64, y: Int64 }

        func main() : Int64 {
            var a: Int64 = 0
            a = 42
            var p = Pt { x: 1, y: 2 }
            p.x = 100
            p.y = 200
            return a + p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "var_assign_patterns");
}

// --- 数组操作 ---
#[test]
fn test_compile_array_index_assign() {
    let source = r#"
        func main() : Int64 {
            var arr = [10, 20, 30]
            arr[0] = 100
            arr[1] = 200
            let a = arr[0]
            let b = arr[1]
            let c = arr[2]
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "array_index_assign");
}

// --- While 语句 ---
#[test]
fn test_compile_while_statement() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            var sum: Int64 = 0
            while i < 10 {
                sum = sum + i
                i = i + 1
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "while_statement");
}

// --- Return 语句在不同位置 ---
#[test]
fn test_compile_early_return() {
    let source = r#"
        func check(n: Int64) : Int64 {
            if n < 0 {
                return -1
            }
            if n == 0 {
                return 0
            }
            return n * 2
        }

        func main() : Int64 {
            let a = check(-5)
            let b = check(0)
            let c = check(10)
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "early_return");
}

// --- 混合类型的二元运算 (Int32, UInt32, Float32) ---
#[test]
fn test_compile_mixed_type_binary_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int32 = 10
            let b: Int32 = 20
            let c = a + b
            let d = a - b
            let e = a * b
            let f = a / b

            let g: UInt32 = 100
            let h: UInt32 = 50
            let i = g + h
            let j = g - h

            let k: Float32 = 1.5f
            let l: Float32 = 2.5f
            let m = k + l
            let n = k * l

            return (c as Int64) + (i as Int64)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "mixed_type_binary_ops");
}

// --- 枚举 match 全路径覆盖 ---
#[test]
fn test_compile_match_enum_variant_payload() {
    let source = r#"
        enum Msg {
            Text(Int64)
            Number(Int64)
            Empty
        }

        func process(m: Msg) : Int64 {
            return match m {
                Msg.Text(t) => t * 10,
                Msg.Number(n) => n,
                Msg.Empty => 0,
                _ => -1
            }
        }

        func main() : Int64 {
            let a = process(Msg.Text(5))
            let b = process(Msg.Number(42))
            let c = process(Msg.Empty)
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_enum_variant_payload");
}

// --- 结构体 match ---
#[test]
fn test_compile_match_struct_pattern() {
    let source = r#"
        struct Vec2 { x: Int64, y: Int64 }

        func magnitude(v: Vec2) : Int64 {
            return match v {
                Vec2 { x, y } => x * x + y * y
            }
        }

        func main() : Int64 {
            let v = Vec2 { x: 3, y: 4 }
            return magnitude(v)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_struct_pattern");
}

// --- 深层嵌套 if-else ---
#[test]
fn test_compile_nested_if_else() {
    let source = r#"
        func classify(n: Int64) : Int64 {
            if n < 0 {
                if n < -100 {
                    return -2
                } else {
                    return -1
                }
            } else {
                if n > 100 {
                    return 2
                } else {
                    if n > 0 {
                        return 1
                    } else {
                        return 0
                    }
                }
            }
        }

        func main() : Int64 {
            let a = classify(-200)
            let b = classify(-50)
            let c = classify(0)
            let d = classify(50)
            let e = classify(200)
            return a + b + c + d + e
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "nested_if_else");
}

// --- 类实现接口 ---
#[test]
fn test_compile_class_implements_interface() {
    let source = r#"
        interface Calculable {
            func compute() : Int64;
        }

        class Calculator <: Calculable {
            var value: Int64;

            init(v: Int64) {
                this.value = v
            }

            func compute(self: Calculator) : Int64 {
                return self.value * 2
            }
        }

        func main() : Int64 {
            let c = Calculator(21)
            return c.compute()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_implements_interface");
}

// --- extend 语法 ---
#[test]
fn test_compile_extend_struct() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }

        extend Point {
            func magnitude(self: Point) : Int64 {
                return self.x * self.x + self.y * self.y
            }
        }

        func main() : Int64 {
            let p = Point { x: 3, y: 4 }
            return p.magnitude()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "extend_struct");
}

// --- 复杂类场景：open + 多重继承链 ---
#[test]
fn test_compile_class_chain() {
    let source = r#"
        open class A {
            var x: Int64;
            init(x: Int64) { this.x = x }
            func val(self: A) : Int64 { return self.x }
        }

        open class B <: A {
            var y: Int64;
            init(x: Int64, y: Int64) {
                super(x)
                this.y = y
            }
            override func val(self: B) : Int64 { return self.x + self.y }
        }

        class C <: B {
            init(x: Int64, y: Int64) {
                super(x, y)
            }
            override func val(self: C) : Int64 { return self.x * self.y }
        }

        func main() : Int64 {
            let a = A(10)
            let b = B(10, 20)
            let c = C(10, 20)
            return a.val() + b.val() + c.val()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_chain");
}

// --- 常量折叠: 更多运算 ---
#[test]
fn test_compile_const_fold_int_ops() {
    let source = r#"
        func main() : Int64 {
            let a = 10 / 3
            let b = 10 % 3
            let c = 10 / 0
            let d = 1 == 2
            let e = 1 != 2
            let f = 2 > 1
            let g = 1 < 2
            let h = 2 >= 2
            let i = 1 <= 1
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "const_fold_int_ops");
}

// --- Type infer 各种表达式 ---
#[test]
fn test_compile_type_infer_various() {
    let source = r#"
        struct Vec2 { x: Int64, y: Int64 }

        func main() : Int64 {
            let arr = [1, 2, 3]
            let elem = arr[0]
            let v = Vec2 { x: 10, y: 20 }
            let fx = v.x
            let t = (1, 2)
            let t0 = t.0
            let casted = 42 as Float64
            let back = casted as Int64
            return elem + fx + t0 + back
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "type_infer_various");
}

// --- 类静态方法 vs 实例方法 ---
#[test]
fn test_compile_class_static_and_instance() {
    let source = r#"
        class Counter {
            var count: Int64;
            init(n: Int64) { this.count = n }
            func inc(self: Counter) : Int64 {
                self.count = self.count + 1
                return self.count
            }
            func reset(self: Counter) : Int64 {
                self.count = 0
                return 0
            }
        }

        func main() : Int64 {
            let c = Counter(0)
            let a = c.inc()
            let b = c.inc()
            let r = c.reset()
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_static_and_instance");
}

// --- 嵌套 try-catch ---
#[test]
fn test_compile_nested_try_catch() {
    let source = r#"
        func main() : Int64 {
            var result: Int64 = 0
            try {
                try {
                    throw 1
                } catch(inner) {
                    result = 10
                }
                result = result + 5
            } catch(outer) {
                result = -1
            } finally {
                result = result + 100
            }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "nested_try_catch");
}

// --- 更多 Cast 路径 ---
#[test]
fn test_compile_cast_int32_float32() {
    let source = r#"
        func main() : Int64 {
            let a: Int32 = 42
            let b = a as Float32
            let c = b as Int32
            let d: Float32 = 3.14f
            let e = d as Float64
            let f = e as Float32
            let g: Int32 = 100
            let h = g as Int64
            let i: UInt32 = 200
            let j = i as Int64
            let k: UInt64 = 300
            let l = k as Int64
            return h + j + l
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cast_int32_float32");
}

// --- 递归 + 复杂控制流 ---
#[test]
fn test_compile_recursive_control_flow() {
    let source = r#"
        func gcd(a: Int64, b: Int64) : Int64 {
            if b == 0 {
                return a
            }
            return gcd(b, a % b)
        }

        func main() : Int64 {
            return gcd(48, 18)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "recursive_control_flow");
}

// --- 多个结构体 + 方法 ---
#[test]
fn test_compile_multi_struct_methods() {
    let source = r#"
        struct Circle { radius: Int64 }
        struct Rect { width: Int64, height: Int64 }

        func circleArea(c: Circle) : Int64 {
            return c.radius * c.radius * 3
        }

        func rectArea(r: Rect) : Int64 {
            return r.width * r.height
        }

        func main() : Int64 {
            let c = Circle { radius: 5 }
            let r = Rect { width: 4, height: 6 }
            return circleArea(c) + rectArea(r)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "multi_struct_methods");
}

// --- While + 复杂条件 ---
#[test]
fn test_compile_while_complex_condition() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 0
            var y: Int64 = 100
            while x < 10 && y > 0 {
                x = x + 1
                y = y - 10
            }
            return x + y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "while_complex_condition");
}

// --- Var without type annotation ---
#[test]
fn test_compile_var_no_annotation() {
    let source = r#"
        func main() : Int64 {
            var a = 10
            var b = 20
            a = a + b
            b = a - b
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "var_no_annotation");
}

// --- 综合覆盖：类 + 枚举 + 泛型 + match ---
#[test]
fn test_compile_comprehensive_coverage() {
    let source = r#"
        struct Pair { a: Int64, b: Int64 }

        enum Status {
            Active(Int64)
            Inactive
        }

        func identity<T>(x: T): T { return x }

        open class Vehicle {
            var speed: Int64;
            init(s: Int64) { this.speed = s }
            func getSpeed(self: Vehicle) : Int64 { return self.speed }
        }

        class Car <: Vehicle {
            var fuel: Int64;
            init(s: Int64, f: Int64) {
                super(s)
                this.fuel = f
            }
            override func getSpeed(self: Car) : Int64 {
                return self.speed + self.fuel
            }
        }

        func checkStatus(s: Status) : Int64 {
            return match s {
                Status.Active(v) => v,
                Status.Inactive => 0,
                _ => -1
            }
        }

        func main() : Int64 {
            let p = Pair { a: 10, b: 20 }
            let sum = p.a + p.b

            let s1 = Status.Active(42)
            let s2 = Status.Inactive
            let r1 = checkStatus(s1)
            let r2 = checkStatus(s2)

            let id = identity<Int64>(99)

            let car = Car(100, 50)
            let speed = car.getSpeed()

            var total: Int64 = 0
            for i in 0..5 {
                total = total + i
            }

            return sum + r1 + r2 + id + speed + total
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "comprehensive_coverage");
}

// === 覆盖率补充：针对 codegen 未覆盖路径 ===

// --- UInt64 完整算术/比较/位运算 ---
#[test]
fn test_compile_uint64_full_ops() {
    let source = r#"
        func main() : Int64 {
            let a: UInt64 = 100
            let b: UInt64 = 7
            let sum = a + b
            let diff = a - b
            let prod = a * b
            let quot = a / b
            let rem = a % b
            let lt = a < b
            let gt = a > b
            let le = a <= b
            let ge = a >= b
            let eq = a == b
            let ne = a != b
            let shr = a >> b
            let band = a & b
            let bor = a | b
            let bxor = a ^ b
            let shl = a << b
            let ushr = a >> b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "uint64_full_ops");
}

// --- Int32 完整算术/比较/位运算 ---
#[test]
fn test_compile_int32_full_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int32 = 50
            let b: Int32 = 3
            let sum = a + b
            let diff = a - b
            let prod = a * b
            let quot = a / b
            let rem = a % b
            let lt = a < b
            let gt = a > b
            let le = a <= b
            let ge = a >= b
            let eq = a == b
            let ne = a != b
            let band = a & b
            let bor = a | b
            let bxor = a ^ b
            let shl = a << b
            let shr = a >> b
            let ushr = a >> b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "int32_full_ops");
}

// --- Float32 完整算术/比较 ---
#[test]
fn test_compile_float32_full_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Float32 = 3.14
            let b: Float32 = 2.71
            let sum = a + b
            let diff = a - b
            let prod = a * b
            let quot = a / b
            let lt = a < b
            let gt = a > b
            let le = a <= b
            let ge = a >= b
            let eq = a == b
            let ne = a != b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "float32_full_ops");
}

// --- Float64 完整算术/比较 ---
#[test]
fn test_compile_float64_full_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Float64 = 10.5
            let b: Float64 = 3.2
            let sum = a + b
            let diff = a - b
            let prod = a * b
            let quot = a / b
            let lt = a < b
            let gt = a > b
            let le = a <= b
            let ge = a >= b
            let eq = a == b
            let ne = a != b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "float64_full_ops");
}

// --- UInt8/UInt16 算术运算掩码 ---
#[test]
fn test_compile_uint8_uint16_mask() {
    let source = r#"
        func main() : Int64 {
            let a: UInt8 = 200
            let b: UInt8 = 100
            let sum8 = a + b
            let diff8 = a - b
            let prod8 = a * b

            let c: UInt16 = 60000
            let d: UInt16 = 10000
            let sum16 = c + d
            let diff16 = c - d
            let prod16 = c * d
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "uint8_uint16_mask");
}

// --- UInt32 完整算术/比较 ---
#[test]
fn test_compile_uint32_ops() {
    let source = r#"
        func main() : Int64 {
            let a: UInt32 = 1000
            let b: UInt32 = 7
            let sum = a + b
            let diff = a - b
            let prod = a * b
            let quot = a / b
            let rem = a % b
            let lt = a < b
            let gt = a > b
            let le = a <= b
            let ge = a >= b
            let shr = a >> b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "uint32_ops");
}

// --- Match enum with payload binding + guard ---
#[test]
fn test_compile_match_enum_payload_guard() {
    let source = r#"
        enum Action {
            Move(Int64)
            Stop
        }

        func process(a: Action) : Int64 {
            return match a {
                Action.Move(dist) if dist > 10 => dist * 2,
                Action.Move(dist) => dist,
                Action.Stop => 0,
                _ => -1
            }
        }

        func main() : Int64 {
            let a1 = Action.Move(20)
            let a2 = Action.Move(5)
            let a3 = Action.Stop
            return process(a1) + process(a2) + process(a3)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_enum_payload_guard");
}

// --- Option match pattern ---
#[test]
fn test_compile_option_match_pattern() {
    let source = r#"
        func unwrap(opt: Option<Int64>) : Int64 {
            return match opt {
                Some(v) => v,
                None => -1
            }
        }

        func main() : Int64 {
            let a: Option<Int64> = Some(42)
            let b: Option<Int64> = None
            return unwrap(a) + unwrap(b)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "option_match_pattern");
}

// --- Result match pattern ---
#[test]
fn test_compile_result_match_pattern() {
    let source = r#"
        func extract(r: Result<Int64, String>) : Int64 {
            return match r {
                Ok(v) => v,
                Err(e) => -1
            }
        }

        func main() : Int64 {
            let ok: Result<Int64, String> = Ok(100)
            let err: Result<Int64, String> = Err("bad")
            return extract(ok) + extract(err)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "result_match_pattern");
}

// --- Return without value ---
#[test]
fn test_compile_return_no_value() {
    let source = r#"
        func doStuff() {
            let x: Int64 = 1
            return
        }

        func main() : Int64 {
            doStuff()
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "return_no_value");
}

// --- Break/Continue in loop ---
#[test]
fn test_compile_break_continue_in_loop() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            var i: Int64 = 0
            loop {
                if i >= 10 {
                    break
                }
                i = i + 1
                if i % 2 == 0 {
                    continue
                }
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "break_continue_in_loop");
}

// --- Match with range pattern (inclusive and exclusive) ---
#[test]
fn test_compile_match_range_incl_excl() {
    let source = r#"
        func classify(x: Int64) : Int64 {
            return match x {
                0..10 => 1,
                10..=20 => 2,
                _ => 0
            }
        }

        func main() : Int64 {
            return classify(5) + classify(15) + classify(25)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_range_incl_excl");
}

// --- Struct pattern in match ---
#[test]
fn test_compile_match_struct_pattern_full() {
    let source = r#"
        struct Point {
            x: Int64
            y: Int64
        }

        func describe(p: Point) : Int64 {
            return match p {
                Point { x: px, y: py } => px + py,
                _ => 0
            }
        }

        func main() : Int64 {
            let p = Point { x: 3, y: 7 }
            return describe(p)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_struct_pattern_full");
}

// --- IfLet with struct/enum patterns for locals collection ---
#[test]
fn test_compile_if_let_pattern_coverage() {
    let source = r#"
        struct Point {
            x: Int64
            y: Int64
        }

        func main() : Int64 {
            let p = Point { x: 10, y: 20 }
            if let Point { x: px, y: py } = p {
                return px + py
            }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "if_let_pattern_coverage");
}

// --- For-in over array ---
#[test]
fn test_compile_for_in_array_full() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30, 40]
            var sum: Int64 = 0
            for x in arr {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "for_in_array_full");
}

// --- Stmt::Expr (expression statement with drop) ---
#[test]
fn test_compile_expr_statement_drop() {
    let source = r#"
        func sideEffect() : Int64 {
            return 42
        }

        func main() : Int64 {
            sideEffect()
            let x = sideEffect()
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "expr_statement_drop");
}

// --- NullCoalesce type inference ---
#[test]
fn test_compile_null_coalesce_infer() {
    let source = r#"
        func main() : Int64 {
            let a: Option<Int64> = Some(10)
            let b: Option<Int64> = None
            let v1 = a ?? 0
            let v2 = b ?? 99
            return v1 + v2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "null_coalesce_infer");
}

// --- Tuple type inference ---
#[test]
fn test_compile_tuple_type_infer() {
    let source = r#"
        func main() : Int64 {
            let t = (10, 20, 30)
            let a = t[0]
            let b = t[1]
            let c = t[2]
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "tuple_type_infer");
}

// --- Some/None/Ok/Err expressions ---
#[test]
fn test_compile_some_none_ok_err_exprs() {
    let source = r#"
        func main() : Int64 {
            let a: Option<Int64> = Some(5)
            let b: Option<Int64> = None
            let c: Result<Int64, String> = Ok(10)
            let d: Result<Int64, String> = Err("fail")
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "some_none_ok_err");
}

// --- Match on field access (parse_match_subject 字段访问) ---
#[test]
fn test_compile_match_field_access() {
    let source = r#"
        struct Config {
            mode: Int64
        }

        func main() : Int64 {
            let c = Config { mode: 2 }
            return match c.mode {
                1 => 10,
                2 => 20,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_field_access");
}

// --- Match on function call result (parse_match_subject 函数调用) ---
#[test]
fn test_compile_match_func_call_subject() {
    let source = r#"
        func getVal() : Int64 {
            return 3
        }

        func main() : Int64 {
            return match getVal() {
                1 => 10,
                2 => 20,
                3 => 30,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_func_call_subject");
}

// --- Match on parenthesized expression ---
#[test]
fn test_compile_match_paren_subject() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 5
            return match (x) {
                5 => 100,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_paren_subject");
}

// --- Match with bool pattern ---
#[test]
fn test_compile_match_bool_pattern() {
    let source = r#"
        func main() : Int64 {
            let b: Bool = true
            return match b {
                true => 1,
                false => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_bool_pattern");
}

// --- Match with string pattern ---
#[test]
fn test_compile_match_string_pattern() {
    let source = r#"
        func classify(s: String) : Int64 {
            return match s {
                "hello" => 1,
                "world" => 2,
                _ => 0
            }
        }

        func main() : Int64 {
            return classify("hello") + classify("world") + classify("other")
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_string_pattern");
}

// --- For-in with variable range ---
#[test]
fn test_compile_for_variable_range() {
    let source = r#"
        func main() : Int64 {
            let n: Int64 = 5
            var sum: Int64 = 0
            for i in 0..n {
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "for_variable_range");
}

// --- Return with no value in void function ---
#[test]
fn test_compile_void_func_return() {
    let source = r#"
        func doNothing() {
            return
        }

        func earlyReturn(x: Int64) {
            if x > 10 {
                return
            }
            let y = x + 1
        }

        func main() : Int64 {
            doNothing()
            earlyReturn(5)
            earlyReturn(15)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "void_func_return");
}

// --- WhileLet with struct pattern ---
#[test]
fn test_compile_while_let_struct_pattern() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            var result: Int64 = 0
            while let Some(v) = o {
                result = v
                break
            }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "while_let_struct_pattern");
}

// --- Try operator with Result type inference (codegen path) ---
#[test]
fn test_compile_try_operator_result_infer() {
    let source = r#"
        func risky(x: Int64) : Result<Int64, String> {
            if x < 0 {
                return Err("negative")
            }
            return Ok(x * 2)
        }

        func compute(x: Int64) : Result<Int64, String> {
            let v = risky(x)?
            return Ok(v + 1)
        }

        func main() : Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "try_operator_result");
}

// --- Lambda expression types ---
#[test]
fn test_compile_lambda_type_infer() {
    let source = r#"
        func main() : Int64 {
            let add = (a: Int64, b: Int64) : Int64 { a + b }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "lambda_type_infer");
}

// --- ConstructorCall (class with init) ---
#[test]
fn test_compile_class_constructor_call() {
    let source = r#"
        class Counter {
            var count: Int64;

            init(start: Int64) {
                this.count = start
            }

            func get(self: Counter) : Int64 {
                return self.count
            }

            func inc(self: Counter) {
                self.count = self.count + 1
            }
        }

        func main() : Int64 {
            let c = Counter(10)
            c.inc()
            c.inc()
            return c.get()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_constructor_call");
}

// --- Or pattern in match ---
#[test]
fn test_compile_match_or_pattern() {
    let source = r#"
        func classify(x: Int64) : Int64 {
            return match x {
                1 | 2 | 3 => 10,
                4 | 5 => 20,
                _ => 0
            }
        }

        func main() : Int64 {
            return classify(2) + classify(5) + classify(9)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_or_pattern");
}

// === 覆盖率补充：monomorph walk paths ===

// --- 泛型函数在 while 循环中调用 (StmtWalkExprs::While) ---
#[test]
fn test_compile_generic_in_while() {
    let source = r#"
        func identity<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            var sum: Int64 = 0
            var i: Int64 = 0
            while i < 3 {
                sum = sum + identity<Int64>(i)
                i = i + 1
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_while");
}

// --- 泛型函数在 for 循环中调用 (StmtWalkExprs::For) ---
#[test]
fn test_compile_generic_in_for() {
    let source = r#"
        func double<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..5 {
                sum = sum + double<Int64>(i)
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_for");
}

// --- 泛型函数在 loop 中调用 (StmtWalkExprs::Loop) ---
#[test]
fn test_compile_generic_in_loop() {
    let source = r#"
        func wrap<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            var result: Int64 = 0
            var i: Int64 = 0
            loop {
                if i >= 3 {
                    break
                }
                result = result + wrap<Int64>(i)
                i = i + 1
            }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_loop");
}

// --- 泛型函数在 match 分支中调用 ---
#[test]
fn test_compile_generic_in_match_body() {
    let source = r#"
        func convert<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            let x: Int64 = 2
            return match x {
                1 => convert<Int64>(10),
                2 => convert<Int64>(20),
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_match_body");
}

// --- 泛型函数在 if-let 中调用 ---
#[test]
fn test_compile_generic_in_if_let() {
    let source = r#"
        func wrap<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            if let Some(v) = o {
                return wrap<Int64>(v)
            }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_if_let");
}

// --- 泛型函数带约束 (check_constraints) ---
#[test]
fn test_compile_generic_with_constraint_check() {
    let source = r#"
        func compare<T: Comparable>(a: T, b: T): T {
            if a > b {
                return a
            }
            return b
        }

        func main() : Int64 {
            let r = compare<Int64>(3, 5)
            return r
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_constraint_check");
}

// --- 泛型函数带多个约束 (更多 check_constraints 路径) ---
#[test]
fn test_compile_generic_multi_constraint_check() {
    let source = r#"
        func process<T: Comparable & Hashable>(x: T): T {
            return x
        }

        func main() : Int64 {
            let r1 = process<Int64>(42)
            let r2 = process<String>("hello")
            return r1
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_multi_constraint_check");
}

// --- 泛型函数用 Bool 类型实例化 (constraint check Bool 路径) ---
#[test]
fn test_compile_generic_bool_constraint() {
    let source = r#"
        func check<T: Equatable>(x: T): T {
            return x
        }

        func main() : Int64 {
            let b = check<Bool>(true)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_bool_constraint");
}

// --- 泛型函数在 while-let 中调用 (StmtWalkExprs::WhileLet) ---
#[test]
fn test_compile_generic_in_while_let() {
    let source = r#"
        func id<T>(x: T): T {
            return x
        }

        func main() : Int64 {
            var sum: Int64 = 0
            var o: Option<Int64> = Some(10)
            while let Some(v) = o {
                sum = sum + id<Int64>(v)
                o = None
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_in_while_let");
}

// --- 泛型 struct 带约束 ---
#[test]
fn test_compile_generic_struct_constraint() {
    let source = r#"
        struct Wrapper<T: Comparable> {
            value: T
        }

        func main() : Int64 {
            let w = Wrapper<Int64> { value: 42 }
            return w.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "generic_struct_constraint");
}

// ============================================================
// Phase 8: 内存管理测试
// ============================================================

/// 验证 WASM 模块导出了内存管理函数（通过在二进制中搜索导出名字符串）
fn assert_has_memory_exports(wasm: &[u8]) {
    let wasm_str = String::from_utf8_lossy(wasm);
    assert!(wasm_str.contains("__alloc"), "应导出 __alloc 函数");
    assert!(wasm_str.contains("__free"), "应导出 __free 函数");
    assert!(wasm_str.contains("__rc_inc"), "应导出 __rc_inc 函数");
    assert!(wasm_str.contains("__rc_dec"), "应导出 __rc_dec 函数");
    assert!(wasm_str.contains("__gc_collect"), "应导出 __gc_collect 函数");
}

// --- 内存管理函数导出验证 ---
#[test]
fn test_memory_management_exports() {
    let source = r#"
        func main() : Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_exports");
    assert_has_memory_exports(&wasm);
}

// --- 结构体分配通过 __alloc ---
#[test]
fn test_memory_struct_alloc() {
    let source = r#"
        struct Point {
            x: Int64
            y: Int64
        }

        func main() : Int64 {
            let p = Point { x: 10, y: 20 }
            return p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_struct_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 数组分配通过 __alloc ---
#[test]
fn test_memory_array_alloc() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3, 4, 5]
            return arr[0] + arr[4]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_array_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 元组分配通过 __alloc ---
#[test]
fn test_memory_tuple_alloc() {
    let source = r#"
        func main() : Int64 {
            let t = (10, 20, 30)
            return t.0 + t.2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_tuple_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 枚举带关联值分配通过 __alloc ---
#[test]
fn test_memory_enum_alloc() {
    let source = r#"
        enum Shape {
            Circle(Int64)
            Rect(Int64)
        }

        func area(s: Shape) : Int64 {
            return match s {
                Shape.Circle(r) => r * r,
                Shape.Rect(side) => side * side,
                _ => 0
            }
        }

        func main() : Int64 {
            let c = Shape.Circle(5)
            return area(c)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_enum_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 类实例分配通过 __alloc ---
#[test]
fn test_memory_class_alloc() {
    let source = r#"
        class Counter {
            var count: Int64;
            init(start: Int64) {
                this.count = start
            }
            func get(self: Counter) : Int64 {
                return self.count
            }
        }

        func main() : Int64 {
            let c = Counter(10)
            return c.get()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_class_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 字符串拼接通过 __alloc ---
#[test]
fn test_memory_string_concat_alloc() {
    let source = r#"
        func main() : Int64 {
            let a = "hello"
            let b = " world"
            let c = a + b
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_string_concat_alloc");
    assert_has_memory_exports(&wasm);
}

// --- 多次分配和赋值（RC dec 覆盖） ---
#[test]
fn test_memory_rc_on_reassignment() {
    let source = r#"
        struct Box {
            value: Int64
        }

        func main() : Int64 {
            var b = Box { value: 1 }
            b = Box { value: 2 }
            b = Box { value: 3 }
            return b.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_rc_reassignment");
}

// --- Range 分配通过 __alloc ---
#[test]
fn test_memory_range_alloc() {
    let source = r#"
        func main() : Int64 {
            let r = 1..10
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_range_alloc");
}

// --- 多个堆对象同时使用 ---
#[test]
fn test_memory_multiple_heap_objects() {
    let source = r#"
        struct Point {
            x: Int64
            y: Int64
        }

        func main() : Int64 {
            let p1 = Point { x: 1, y: 2 }
            let p2 = Point { x: 3, y: 4 }
            let p3 = Point { x: 5, y: 6 }
            let arr = [p1.x, p2.x, p3.x]
            return p1.x + p2.y + p3.x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_multiple_heap_objects");
}

// --- 类继承 + ~init + RC ---
#[test]
fn test_memory_class_deinit() {
    let source = r#"
        class Base {
            var value: Int64;
            init(v: Int64) {
                this.value = v
            }
            ~init {
                let x = 0
            }
        }

        func main() : Int64 {
            let b = Base(42)
            return b.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_class_deinit");
}

// --- 复杂场景：循环中分配 ---
#[test]
fn test_memory_alloc_in_loop() {
    let source = r#"
        struct Item {
            value: Int64
        }

        func main() : Int64 {
            var sum: Int64 = 0
            var i: Int64 = 0
            while i < 5 {
                let item = Item { value: i * 10 }
                sum = sum + item.value
                i = i + 1
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_alloc_in_loop");
}

// --- Option/Result 内存分配 ---
#[test]
fn test_memory_option_result_alloc() {
    let source = r#"
        func maybe(x: Int64) : Int64 {
            let opt = Some(x)
            match opt {
                Some(v) => v,
                None => 0,
            }
        }

        func main() : Int64 {
            return maybe(42)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_option_alloc");
}

// --- 综合内存管理测试 ---
#[test]
fn test_memory_comprehensive() {
    let source = r#"
        struct Node {
            value: Int64
            next: Int64
        }

        enum Color {
            Red
            Green
            Blue
            Custom(Int64)
        }

        class Container {
            var data: Int64;
            init(d: Int64) {
                this.data = d
            }
            func getData(self: Container) : Int64 {
                return self.data
            }
        }

        func main() : Int64 {
            let n1 = Node { value: 1, next: 0 }
            let n2 = Node { value: 2, next: 0 }
            let arr = [n1.value, n2.value, 3]
            let t = (10, 20)
            let c = Container(100)
            let color = Color.Custom(255)
            return n1.value + arr[1] + t.0 + c.getData()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "memory_comprehensive");
    assert_has_memory_exports(&wasm);
}
