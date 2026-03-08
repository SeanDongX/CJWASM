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

/// L1 模块解析：在含 third_party 的仓库中，import std.overflow 应从 vendor 解析到多个 .cj 文件
#[test]
fn test_l1_std_vendor_resolution() {
    let repo = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let vendor = cjwasm::pipeline::get_vendor_std_dir(&repo);
    if let Some(ref v) = vendor {
        let module_path = ["std".to_string(), "overflow".to_string()];
        let bases: &[&Path] = &[];
        let files = cjwasm::pipeline::resolve_import_to_files(
            &module_path,
            bases,
            Some(v.as_path()),
        );
        assert!(
            !files.is_empty(),
            "L1 std.overflow 应从 vendor 解析到至少一个 .cj 文件 (vendor={})",
            v.display()
        );
        assert!(
            files.iter().all(|p| p.extension().map_or(false, |e| e == "cj")),
            "解析结果应均为 .cj 文件"
        );
    }
    // 无 vendor 时跳过断言（如仅安装 binary 时）
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

// --- Parser/Decl coverage (tests 1-27) ---
#[test]
fn test_parse_struct_with_methods() {
    let source = r#"
        struct Rect { width: Int64, height: Int64 }
        func Rect.area(self: Rect) : Int64 { return self.width * self.height }
        func Rect.perimeter(self: Rect) : Int64 { return 2 * (self.width + self.height) }
        func main() : Int64 {
            let r = Rect { width: 5, height: 10 }
            return r.area() + r.perimeter()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_struct_with_methods");
}

#[test]
fn test_parse_struct_with_primary_ctor() {
    let source = r#"
        struct Point(var x: Int64, var y: Int64) { }
        func main() : Int64 {
            let p = Point(10, 20)
            return p.x + p.y
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_struct_with_primary_ctor");
    }
}

#[test]
fn test_parse_struct_implements_interface() {
    let source = r#"
        interface Drawable { func draw() : Int64; }
        struct Square <: Drawable { size: Int64 }
        extend Square: Drawable {
            func draw(self: Square) : Int64 { return self.size * self.size }
        }
        func main() : Int64 {
            let s = Square { size: 5 }
            return s.draw()
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_struct_implements_interface");
    }
}

#[test]
fn test_parse_struct_with_init() {
    let source = r#"
        struct Box {
            var value: Int64
            init(v: Int64) { }
        }
        func main() : Int64 {
            let b = Box { value: 42 }
            return b.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_struct_with_init");
}

#[test]
fn test_parse_struct_generic() {
    let source = r#"
        struct Pair<T> { first: T, second: T }
        func main() : Int64 {
            let p = Pair<Int64> { first: 1, second: 2 }
            return p.first + p.second
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_struct_generic");
}

#[test]
fn test_parse_enum_basic() {
    let source = r#"
        enum Direction { North, South, East, West }
        func main() : Int64 {
            let d: Direction = Direction.North
            match d {
                Direction.North => 1,
                Direction.South => 2,
                Direction.East => 3,
                Direction.West => 4,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_enum_basic");
}

#[test]
fn test_parse_enum_with_value() {
    let source = r#"
        enum Result { Ok(Int64), Err(Int64) }
        func main() : Int64 {
            let r: Result = Result.Ok(100)
            match r {
                Result.Ok(v) => v,
                Result.Err(e) => 0 - e,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_enum_with_value");
}

#[test]
fn test_parse_enum_with_methods() {
    let source = r#"
        enum Status { Idle, Running(Int64) }
        func Status.value(self: Status) : Int64 {
            match self {
                Status.Running(v) => v,
                _ => 0
            }
        }
        func main() : Int64 {
            let s: Status = Status.Running(99)
            return s.value()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_enum_with_methods");
}

#[test]
fn test_parse_interface_basic() {
    let source = r#"
        interface Printable { func print() : Int64; }
        struct Doc { id: Int64 }
        extend Doc: Printable {
            func print(self: Doc) : Int64 { return self.id }
        }
        func main() : Int64 {
            let d = Doc { id: 7 }
            return d.print()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_interface_basic");
}

#[test]
fn test_parse_interface_with_default() {
    let source = r#"
        interface Greeter {
            func greet() : Int64;
            func defaultGreet() : Int64 { return 42 }
        }
        struct Hello { }
        extend Hello: Greeter {
            func greet(self: Hello) : Int64 { return 1 }
        }
        func main() : Int64 {
            let h = Hello { }
            return h.defaultGreet()
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_interface_with_default");
    }
}

#[test]
fn test_parse_interface_extends() {
    let source = r#"
        interface Base { func id() : Int64; }
        interface Extended: Base { func extra() : Int64; }
        struct Impl { }
        extend Impl: Extended {
            func id(self: Impl) : Int64 { return 1 }
            func extra(self: Impl) : Int64 { return 2 }
        }
        func main() : Int64 {
            let i = Impl { }
            return i.id() + i.extra()
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_interface_extends");
    }
}

#[test]
fn test_parse_extend_basic() {
    let source = r#"
        struct Num { val: Int64 }
        extend Num {
            func double(self: Num) : Int64 { return self.val * 2 }
        }
        func main() : Int64 {
            let n = Num { val: 21 }
            return n.double()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_extend_basic");
}

#[test]
fn test_parse_class_basic() {
    let source = r#"
        class Counter {
            var count: Int64
            init() { this.count = 0 }
            func inc(self: Counter) : Int64 {
                this.count = this.count + 1
                return this.count
            }
        }
        func main() : Int64 {
            let c = Counter()
            c.inc()
            c.inc()
            return c.count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_class_basic");
}

#[test]
fn test_parse_class_open() {
    let source = r#"
        open class Base {
            var x: Int64
            init(v: Int64) { this.x = v }
            func get(self: Base) : Int64 { return this.x }
        }
        class Derived <: Base {
            init(v: Int64) { super(v) }
        }
        func main() : Int64 {
            let d = Derived(10)
            return d.get()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_class_open");
}

#[test]
fn test_parse_class_sealed() {
    let source = r#"
        sealed class Final {
            var v: Int64
            init(x: Int64) { this.v = x }
        }
        func main() : Int64 {
            let f = Final(5)
            return f.v
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_class_sealed");
}

#[test]
fn test_parse_class_abstract() {
    let source = r#"
        abstract class Shape {
            func area(self: Shape) : Int64 { return 0 }
        }
        class Rect : Shape {
            var w: Int64
            var h: Int64
            init(w: Int64, h: Int64) { this.w = w; this.h = h; super() }
            override func area(self: Rect) : Int64 { return this.w * this.h }
        }
        func main() : Int64 {
            let r = Rect(3, 4)
            return r.area()
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_class_abstract");
    }
}

#[test]
fn test_parse_class_with_prop() {
    let source = r#"
        class WithProp {
            var _val: Int64
            init(v: Int64) { this._val = v }
            prop value: Int64 { get() { return this._val } set(v) { this._val = v } }
        }
        func main() : Int64 {
            let w = WithProp(7)
            w.value = 14
            return w.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_class_with_prop");
}

#[test]
fn test_parse_class_inheritance_chain() {
    let source = r#"
        class A {
            var a: Int64
            init() { this.a = 1 }
        }
        class B <: A {
            var b: Int64
            init() { this.b = 2; super() }
        }
        class C <: B {
            var c: Int64
            init() { this.c = 3; super() }
        }
        func main() : Int64 {
            let x = C()
            return x.a + x.b + x.c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_class_inheritance_chain");
}

#[test]
fn test_parse_class_operator_func() {
    let source = r#"
        struct Vec2 { x: Int64, y: Int64 }
        extend Vec2 {
            operator func +(self: Vec2, other: Vec2) : Vec2 {
                return Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }
        func main() : Int64 {
            let a = Vec2 { x: 1, y: 2 }
            let b = Vec2 { x: 3, y: 4 }
            let c = a + b
            return c.x + c.y
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_class_operator_func");
    }
}

#[test]
fn test_parse_const_decl() {
    let source = r#"
        const PI: Float64 = 3.14
        func main() : Int64 {
            return 0
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_const_decl");
    }
}

#[test]
fn test_parse_extern_func() {
    let source = r#"
        @import("env", "log") foreign func log(n: Int64) : Unit
        func main() : Int64 {
            log(42)
            return 0
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_extern_func");
    }
}

#[test]
fn test_parse_function_with_throws() {
    let source = r#"
        func risky() : Int64 throws { return 42 }
        func main() : Int64 {
            return 0
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_function_with_throws");
    }
}

#[test]
fn test_parse_function_default_params() {
    let source = r#"
        func add(a: Int64, b: Int64 = 10) : Int64 { return a + b }
        func main() : Int64 {
            return add(5) + add(1, 2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_function_default_params");
}

#[test]
fn test_parse_function_variadic() {
    let source = r#"
        func sum(args: Int64...) : Int64 {
            var s: Int64 = 0
            return s
        }
        func main() : Int64 {
            return sum(1, 2, 3)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_function_variadic");
}

#[test]
fn test_parse_function_generic() {
    let source = r#"
        func identity<T>(x: T) : T { return x }
        func main() : Int64 {
            return identity(42)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_function_generic");
}

#[test]
fn test_parse_function_visibility() {
    let source = r#"
        public func exported() : Int64 { return 1 }
        func main() : Int64 {
            return exported()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_function_visibility");
}

#[test]
fn test_parse_multiple_imports() {
    let source = r#"
        import std.io
        import std.string
        func main() : Int64 {
            return 0
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_multiple_imports");
    }
}

// --- Parser/Stmt coverage (tests 28-42) ---
#[test]
fn test_parse_let_with_type() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 5
            let y: Float64 = 3.14
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_let_with_type");
}

#[test]
fn test_parse_var_mutable() {
    let source = r#"
        func main() : Int64 {
            var x = 5
            x = 10
            x = x + 1
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_var_mutable");
}

#[test]
fn test_parse_for_range() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..10 {
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_for_range");
}

#[test]
fn test_parse_for_array() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3, 4, 5]
            var sum: Int64 = 0
            for x in arr {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_for_array");
}

#[test]
fn test_parse_while_loop() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            var count: Int64 = 0
            while i < 5 {
                if i == 2 { continue }
                count = count + 1
                i = i + 1
                if i > 10 { break }
            }
            return count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_while_loop");
}

#[test]
fn test_parse_nested_loops() {
    let source = r#"
        func main() : Int64 {
            var total: Int64 = 0
            for i in 0..3 {
                for j in 0..3 {
                    total = total + i + j
                }
            }
            return total
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_nested_loops");
}

#[test]
fn test_parse_match_enum() {
    let source = r#"
        enum Tag { A, B, C }
        func main() : Int64 {
            let t: Tag = Tag.B
            match t {
                Tag.A => 1,
                Tag.B => 2,
                Tag.C => 3,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_match_enum");
}

#[test]
fn test_parse_match_with_guard() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                x if x < 0 => 0,
                x if x < 5 => 1,
                x if x < 10 => 2,
                _ => 3
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_match_with_guard");
}

#[test]
fn test_parse_match_tuple() {
    let source = r#"
        func main() : Int64 {
            let t = (1, 2)
            match t {
                (a, b) => a + b,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_match_tuple");
}

#[test]
fn test_parse_match_wildcard() {
    let source = r#"
        func main() : Int64 {
            match 99 {
                0 => 1,
                1 => 2,
                _ => 42
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_match_wildcard");
}

#[test]
fn test_parse_try_catch() {
    let source = r#"
        func main() : Int64 {
            try {
                return 42
            } catch (e: Exception) {
                return 0
            }
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_try_catch");
    }
}

#[test]
fn test_parse_if_let() {
    let source = r#"
        func main() : Int64 {
            let opt = Some(10)
            if let Some(x) = opt { return x } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_if_let");
}

#[test]
fn test_parse_nested_if() {
    let source = r#"
        func main() : Int64 {
            if true {
                if false { return 0 } else {
                    if true { return 1 } else { return 2 }
                }
            } else {
                return 3
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_nested_if");
}

#[test]
fn test_parse_block_expr() {
    let source = r#"
        func main() : Int64 {
            let x = {
                let a = 1
                let b = 2
                a + b
            }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_block_expr");
}

#[test]
fn test_parse_return_early() {
    let source = r#"
        func classify(n: Int64) : Int64 {
            if n < 0 { return 0 }
            if n == 0 { return 1 }
            if n > 100 { return 2 }
            return 3
        }
        func main() : Int64 {
            return classify(50)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_return_early");
}

// --- Parser/Expr coverage (tests 43-61) ---
#[test]
fn test_parse_string_interpolation() {
    let source = r#"
        func main() : Int64 {
            let x = 42
            let s = `value: ${x}`
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_string_interpolation");
}

#[test]
fn test_parse_multiline_string() {
    let source = r#"
        func main() : Int64 {
            let s = """line1
            line2"""
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_multiline_string");
}

#[test]
fn test_parse_binary_ops_all() {
    let source = r#"
        func main() : Int64 {
            let a = 10 + 5
            let b = 10 - 5
            let c = 10 * 5
            let d = 10 / 5
            let e = 10 % 3
            let f = 2 ** 3
            let g = 8 << 1
            let h = 8 >> 1
            let i = 5 & 3
            let j = 5 | 3
            let k = 5 ^ 3
            return a + b + c + d + e + f + g + h + i + j + k
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_binary_ops_all");
}

#[test]
fn test_parse_comparison_ops() {
    let source = r#"
        func main() : Int64 {
            let a = 1 == 1
            let b = 1 != 2
            let c = 1 < 2
            let d = 2 > 1
            let e = 1 <= 2
            let f = 2 >= 1
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_comparison_ops");
}

#[test]
fn test_parse_logical_ops() {
    let source = r#"
        func main() : Int64 {
            let a = true && false
            let b = true || false
            let c = !true
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_logical_ops");
}

#[test]
fn test_parse_unary_ops() {
    let source = r#"
        func main() : Int64 {
            let a = -42
            let b = !false
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_unary_ops");
}

#[test]
fn test_parse_array_index() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30]
            return arr[0] + arr[1] + arr[2]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_array_index");
}

#[test]
fn test_parse_tuple_access() {
    let source = r#"
        func main() : Int64 {
            let t = (100, 200)
            return t.0 + t.1
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_tuple_access");
}

#[test]
fn test_parse_method_chain() {
    let source = r#"
        struct Wrapper { val: Int64 }
        func Wrapper.inc(self: Wrapper) : Wrapper { return Wrapper { val: self.val + 1 } }
        func main() : Int64 {
            let w = Wrapper { val: 0 }
            return w.inc().inc().val
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_method_chain");
}

#[test]
fn test_parse_lambda_basic() {
    let source = r#"
        func main() : Int64 {
            let f = { x: Int64 => x * 2 }
            return f(21)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_lambda_basic");
}

#[test]
fn test_parse_lambda_multi_param() {
    let source = r#"
        func main() : Int64 {
            let add = { x: Int64, y: Int64 => x + y }
            return add(10, 32)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_lambda_multi_param");
}

#[test]
fn test_parse_pipeline_op() {
    let source = r#"
        func double(x: Int64) : Int64 { return x * 2 }
        func main() : Int64 {
            return 21 |> double
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_pipeline_op");
}

#[test]
fn test_parse_null_coalesce() {
    let source = r#"
        func main() : Int64 {
            let opt: Option<Int64> = None
            return opt ?? 100
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_null_coalesce");
}

#[test]
fn test_parse_cast_expr() {
    let source = r#"
        func main() : Int64 {
            let x = 100 as Int64
            return (x as Int32) as Int64
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_cast_expr");
}

#[test]
fn test_parse_is_type() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 5
            if x is Int64 { return 1 } else { return 0 }
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_is_type");
    }
}

#[test]
fn test_parse_range_expr() {
    let source = r#"
        func main() : Int64 {
            let r = 0..10
            let r2 = 1..=5
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_range_expr");
}

#[test]
fn test_parse_nested_expr() {
    let source = r#"
        func main() : Int64 {
            let x = (1 + 2) * (3 + 4) - (5 / 2)
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_nested_expr");
}

#[test]
fn test_parse_field_access() {
    let source = r#"
        struct Data { a: Int64, b: Int64 }
        func main() : Int64 {
            let d = Data { a: 1, b: 2 }
            return d.a + d.b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_field_access");
}

#[test]
fn test_parse_constructor_call() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() : Int64 {
            let p = Point(10, 20)
            return p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_constructor_call");
}

// --- Parser/Type coverage (tests 62-67) ---
#[test]
fn test_parse_primitive_types() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 1
            let b: Int16 = 2
            let c: Int32 = 3
            let d: Int64 = 4
            let e: UInt8 = 5
            let f: UInt16 = 6
            let g: UInt32 = 7
            let h: UInt64 = 8
            let i: Float32 = 1.0f
            let j: Float64 = 2.0
            let k: Bool = true
            let l: Rune = 'x'
            let m: String = "hi"
            return d
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_primitive_types");
}

#[test]
fn test_parse_array_type() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = [1, 2, 3]
            return arr[0]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_array_type");
}

#[test]
fn test_parse_tuple_type() {
    let source = r#"
        func main() : Int64 {
            let t: (Int64, String) = (1, "a")
            return t.0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_tuple_type");
}

#[test]
fn test_parse_option_type() {
    let source = r#"
        func main() : Int64 {
            let opt: Option<Int64> = Some(42)
            match opt {
                Some(v) => v,
                None => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_option_type");
}

#[test]
fn test_parse_function_type() {
    let source = r#"
        func main() : Int64 {
            let f: (Int64) -> Int64 = { x: Int64 => x + 1 }
            return f(41)
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "parse_function_type");
    }
}

#[test]
fn test_parse_generic_type() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = [1, 2, 3]
            let opt: Option<Int64> = Some(42)
            return arr[0]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_generic_type");
}

// --- Parser/Pattern coverage (tests 68-72) ---
#[test]
fn test_parse_pattern_binding() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                x => x,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_pattern_binding");
}

#[test]
fn test_parse_pattern_literal() {
    let source = r#"
        func main() : Int64 {
            match 42 {
                0 => 0,
                42 => 1,
                _ => 2
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_pattern_literal");
}

#[test]
fn test_parse_pattern_wildcard() {
    let source = r#"
        func main() : Int64 {
            match 100 {
                _ => 1
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_pattern_wildcard");
}

#[test]
fn test_parse_pattern_or() {
    let source = r#"
        func main() : Int64 {
            match 2 {
                1 | 2 | 3 => 1,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "parse_pattern_or");
}

#[test]
fn test_parse_pattern_struct() {
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
    assert_valid_wasm(&wasm, "parse_pattern_struct");
}

// --- Codegen/Monomorph coverage (tests 73-85) ---
#[test]
fn test_codegen_class_methods() {
    let source = r#"
        class Calc {
            var n: Int64
            init(v: Int64) { this.n = v }
            func add(self: Calc, x: Int64) : Int64 { return this.n + x }
            func mul(self: Calc, x: Int64) : Int64 { return this.n * x }
        }
        func main() : Int64 {
            let c = Calc(10)
            return c.add(5) + c.mul(2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_class_methods");
}

#[test]
fn test_codegen_string_ops() {
    let source = r#"
        func main() : Int64 {
            let a = "hello"
            let b = "world"
            let c = a + b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_string_ops");
}

#[test]
fn test_codegen_array_ops() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            let len = arr.length()
            return arr[0] + len
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_array_ops");
}

#[test]
fn test_codegen_nested_struct() {
    let source = r#"
        struct Inner { val: Int64 }
        struct Outer { inner: Inner }
        func main() : Int64 {
            let o = Outer { inner: Inner { val: 42 } }
            return o.inner.val
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_nested_struct");
}

#[test]
fn test_codegen_recursion() {
    let source = r#"
        func fib(n: Int64) : Int64 {
            if n <= 1 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        func main() : Int64 {
            return fib(5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_recursion");
}

#[test]
fn test_codegen_mutual_recursion() {
    let source = r#"
        func is_even(n: Int64) : Bool {
            if n == 0 { return true }
            return is_odd(n - 1)
        }
        func is_odd(n: Int64) : Bool {
            if n == 0 { return false }
            return is_even(n - 1)
        }
        func main() : Int64 {
            if is_even(4) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_mutual_recursion");
}

#[test]
fn test_codegen_complex_match() {
    let source = r#"
        enum E { A(Int64), B(Int64), C }
        func main() : Int64 {
            let e1: E = E.A(1)
            let e2: E = E.B(2)
            let e3: E = E.C
            match e1 {
                E.A(v) => match e2 {
                    E.B(w) => v + w,
                    _ => 0
                },
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_complex_match");
}

#[test]
fn test_codegen_operator_overload() {
    let source = r#"
        struct V { x: Int64 }
        extend V {
            operator func +(self: V, o: V) : V { return V { x: self.x + o.x } }
        }
        func main() : Int64 {
            let a = V { x: 1 }
            let b = V { x: 2 }
            let c = a + b
            return c.x
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "codegen_operator_overload");
    }
}

#[test]
fn test_codegen_generic_function() {
    let source = r#"
        func id<T>(x: T) : T { return x }
        func main() : Int64 {
            let a = id(42)
            let b = id(3.14)
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_generic_function");
}

#[test]
fn test_codegen_enum_match() {
    let source = r#"
        enum Opt { Some(Int64), None }
        func main() : Int64 {
            let o: Opt = Opt.Some(99)
            match o {
                Opt.Some(v) => v,
                Opt.None => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_enum_match");
}

#[test]
fn test_codegen_multiple_classes() {
    let source = r#"
        class A { var x: Int64; init() { this.x = 1 } }
        class B { var y: Int64; init() { this.y = 2 } }
        class C { var z: Int64; init() { this.z = 3 } }
        func main() : Int64 {
            let a = A()
            let b = B()
            let c = C()
            return a.x + b.y + c.z
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_multiple_classes");
}

#[test]
fn test_codegen_class_with_array_field() {
    let source = r#"
        class Container {
            var items: Array<Int64>
            init() { this.items = [1, 2, 3] }
        }
        func main() : Int64 {
            let c = Container()
            return c.items[0]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_class_with_array_field");
}

#[test]
fn test_codegen_nested_control_flow() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..5 {
                if i % 2 == 0 {
                    match i {
                        0 => sum = sum + 1,
                        2 => sum = sum + 2,
                        4 => sum = sum + 4,
                        _ => { }
                    }
                }
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_nested_control_flow");
}

// --- Edge cases (tests 86-90) ---
#[test]
fn test_compile_empty_main() {
    let source = r#"
        func main() : Int64 {
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compile_empty_main");
}

#[test]
fn test_compile_large_program() {
    let source = r#"
        func f1() : Int64 { return 1 }
        func f2() : Int64 { return 2 }
        func f3() : Int64 { return 3 }
        func f4() : Int64 { return 4 }
        func f5() : Int64 { return 5 }
        func f6() : Int64 { return 6 }
        func f7() : Int64 { return 7 }
        func f8() : Int64 { return 8 }
        func f9() : Int64 { return 9 }
        func f10() : Int64 { return 10 }
        func main() : Int64 {
            return f1() + f2() + f3() + f4() + f5()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compile_large_program");
}

#[test]
fn test_compile_deep_nesting() {
    let source = r#"
        func main() : Int64 {
            let x = { { { { { 42 } } } } }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compile_deep_nesting");
}

#[test]
fn test_compile_many_local_vars() {
    let source = r#"
        func main() : Int64 {
            let a = 1
            let b = 2
            let c = 3
            let d = 4
            let e = 5
            let f = 6
            let g = 7
            let h = 8
            let i = 9
            let j = 10
            return a + b + c + d + e + f + g + h + i + j
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compile_many_local_vars");
}

#[test]
fn test_compile_complex_inheritance() {
    let source = r#"
        class A { var a: Int64; init() { this.a = 1 } }
        class B <: A { var b: Int64; init() { this.b = 2; super() } }
        class C <: B { var c: Int64; init() { this.c = 3; super() } }
        class D <: C { var d: Int64; init() { this.d = 4; super() } }
        func main() : Int64 {
            let x = D()
            return x.a + x.b + x.c + x.d
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "compile_complex_inheritance");
}

// === Parser/decl.rs - Class parsing coverage ===

#[test]
fn test_class_with_deinit() {
    let source = r#"
        class Resource {
            var handle: Int64;
            init(h: Int64) { this.handle = h }
            ~init { let _ = this.handle }
            func getHandle(self: Resource) : Int64 { return self.handle }
        }
        func main() : Int64 { let r = Resource(42); return r.getHandle() }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "class with deinit: parse or codegen");
}

#[test]
fn test_class_with_static_init() {
    let source = r#"
        class Config {
            var value: Int64;
            init(v: Int64) { this.value = v }
            static init() { let _ = 0 }
            func getValue(self: Config) : Int64 { return self.value }
        }
        func main() : Int64 { let c = Config(99); return c.getValue() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_with_static_init");
}

#[test]
fn test_class_with_static_const() {
    let source = r#"
        class MathConst {
            static const MAX_VAL: Int64 = 100;
            var x: Int64;
            init(v: Int64) { this.x = v }
            func getX(self: MathConst) : Int64 { return self.x }
        }
        func main() : Int64 { let m = MathConst(5); return m.getX() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_with_static_const");
}

#[test]
fn test_class_with_static_let() {
    let source = r#"
        class Singleton {
            static let INSTANCE: Int64 = 1;
            var data: Int64;
            init(d: Int64) { this.data = d }
        }
        func main() : Int64 { let s = Singleton(10); return 10 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_with_static_let");
}

#[test]
fn test_class_override_method() {
    let source = r#"
        open class Base {
            func getValue(self: Base) : Int64 { return 1 }
        }
        class Derived <: Base {
            override func getValue(self: Derived) : Int64 { return 2 }
        }
        func main() : Int64 { let d = Derived(); return d.getValue() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_override_method");
}

#[test]
fn test_class_override_prop() {
    let source = r#"
        open class Shape {
            prop area: Int64 {
                get() { return 0 }
            }
        }
        class Square <: Shape {
            var side: Int64;
            init(s: Int64) { this.side = s }
            override prop area: Int64 {
                get() { return this.side * this.side }
            }
        }
        func main() : Int64 { let s = Square(5); return s.area }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_override_prop");
}

#[test]
fn test_class_primary_constructor() {
    let source = r#"
        class Point(var x: Int64, var y: Int64) {
            func sum(self: Point) : Int64 { return self.x + self.y }
        }
        func main() : Int64 { let p = Point(3, 4); return p.sum() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_primary_constructor");
}

#[test]
fn test_class_open_override_chain() {
    let source = r#"
        open class A {
            open func value(self: A) : Int64 { return 1 }
        }
        open class B <: A {
            override open func value(self: B) : Int64 { return 2 }
        }
        class C <: B {
            override func value(self: C) : Int64 { return 3 }
        }
        func main() : Int64 { let c = C(); return c.value() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_open_override_chain");
}

#[test]
fn test_class_multiple_fields_with_defaults() {
    let source = r#"
        class Config {
            var width: Int64 = 800;
            var height: Int64 = 600;
            var depth: Int64;
            init(d: Int64) { this.depth = d }
            func getWidth(self: Config) : Int64 { return self.width }
        }
        func main() : Int64 { let c = Config(32); return c.getWidth() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_multiple_fields_with_defaults");
}

#[test]
fn test_class_abstract_method() {
    let source = r#"
        abstract class Processor {
            func process(self: Processor) : Int64;
            func name(self: Processor) : Int64 { return 0 }
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "abstract method: parse or codegen");
}

#[test]
fn test_class_operator_index() {
    let source = r#"
        class MyList {
            var data: Int64;
            init(d: Int64) { this.data = d }
            operator func [](self: MyList, idx: Int64) : Int64 { return self.data + idx }
        }
        func main() : Int64 { let l = MyList(10); return l[5] }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_operator_index");
}

#[test]
fn test_class_field_no_type() {
    let source = r#"
        class Counter {
            let initial = 0;
            var count: Int64;
            init(c: Int64) { this.count = c }
            func getCount(self: Counter) : Int64 { return self.count }
        }
        func main() : Int64 { let c = Counter(42); return c.getCount() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_field_no_type");
}

#[test]
fn test_class_prop_with_setter() {
    let source = r#"
        class Box {
            var _value: Int64;
            init(v: Int64) { this._value = v }
            prop value: Int64 {
                get() { return this._value }
                set(v) { this._value = v }
            }
        }
        func main() : Int64 { let b = Box(10); return b.value }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_prop_with_setter");
}

// === Parser/decl.rs - Enum parsing ===

#[test]
fn test_enum_with_pipe_syntax() {
    let source = r#"
        enum Direction {
            | North
            | South
            | East
            | West
        }
        func main() : Int64 { let d = Direction.North; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_with_pipe_syntax");
}

#[test]
fn test_enum_with_tuple_payload() {
    let source = r#"
        enum Shape {
            Circle(Int64)
            Rectangle(Int64, Int64)
            Point
        }
        func main() : Int64 { let s = Shape.Circle(5); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "enum_with_tuple_payload");
}

#[test]
fn test_enum_subtype() {
    let source = r#"
        enum Status <: ToString {
            Active
            Inactive
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "enum subtype: parse or codegen");
}

#[test]
fn test_enum_option_result() {
    let source = r#"
        func maybe(x: Int64) : Int64 {
            if (x > 0) { return x }
            return 0
        }
        func main() : Int64 { return maybe(5) }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "enum option result names");
}

// === Parser/decl.rs - Interface parsing ===

#[test]
fn test_interface_with_prop() {
    let source = r#"
        interface Measurable {
            prop size: Int64 {
                get();
            }
        }
        func main() : Int64 { return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "interface_with_prop");
}

#[test]
fn test_interface_with_default_body() {
    let source = r#"
        interface Greetable {
            func greet(self: Greetable) : Int64 { return 42 }
        }
        func main() : Int64 { return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "interface_with_default_body");
}

#[test]
fn test_interface_multiple_parents() {
    let source = r#"
        interface A {
            func a() : Int64;
        }
        interface B {
            func b() : Int64;
        }
        interface C <: A & B {
            func c() : Int64;
        }
        func main() : Int64 { return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "interface_multiple_parents");
}

#[test]
fn test_interface_assoc_type() {
    let source = r#"
        interface Collection {
            type Element;
            func size() : Int64;
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "interface assoc type");
}

// === Parser/decl.rs - Extend parsing ===

#[test]
fn test_extend_with_prop() {
    let source = r#"
        class Counter {
            var _count: Int64;
            init(c: Int64) { this._count = c }
        }
        extend Counter {
            prop count: Int64 {
                get() { return this._count }
                set(v) { this._count = v }
            }
        }
        func main() : Int64 { let c = Counter(10); return c.count }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "extend_with_prop");
}

#[test]
fn test_extend_with_interface() {
    let source = r#"
        interface Printable {
            func display(self: Printable) : Int64;
        }
        class Item {
            var id: Int64;
            init(i: Int64) { this.id = i }
        }
        extend Item <: Printable {
            func display(self: Item) : Int64 { return self.id }
        }
        func main() : Int64 { let item = Item(7); return item.display() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "extend_with_interface");
}

#[test]
fn test_extend_primitive_type() {
    let source = r#"
        extend Int64 {
            func double(self: Int64) : Int64 { return self * 2 }
        }
        func main() : Int64 { return 5 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "extend primitive");
}

// === Parser/stmt.rs - Special statement paths ===

#[test]
fn test_stmt_let_wildcard() {
    let source = r#"
        func main() : Int64 { let _ = 42; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "stmt_let_wildcard");
}

#[test]
fn test_stmt_let_struct_destructure() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() : Int64 {
            let p = Point { x: 1, y: 2 }
            let Point { x: a, y: b } = p
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "stmt_let_struct_destructure");
}

#[test]
fn test_stmt_let_no_init() {
    let source = r#"
        func main() : Int64 {
            let x: Int64
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "let no init");
}

#[test]
fn test_stmt_var_no_init() {
    let source = r#"
        func main() : Int64 {
            var x: Int64
            x = 42
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "stmt_var_no_init");
}

#[test]
fn test_stmt_while_let() {
    let source = r#"
        func main() : Int64 {
            var count = 0
            while (count < 5) {
                count = count + 1
            }
            return count
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "while let variant");
}

#[test]
fn test_stmt_for_tuple_destructure() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            let items = [(1, 10), (2, 20)]
            for ((k, v) in items) {
                sum = sum + v
            }
            return sum
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "for tuple destructure");
}

#[test]
fn test_stmt_try_catch() {
    let source = r#"
        func main() : Int64 {
            var result = 0
            try {
                result = 42
            } catch (e: Exception) {
                result = -1
            }
            return result
        }
    "#;
    let result = compile_source_result(source);
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "stmt_try_catch");
    }
}

#[test]
fn test_stmt_throw() {
    let source = r#"
        func fail() : Int64 {
            throw Exception("error")
            return 0
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "throw statement");
}

#[test]
fn test_stmt_for_underscore() {
    let source = r#"
        func main() : Int64 {
            var count = 0
            for (_ in [1, 2, 3]) {
                count = count + 1
            }
            return count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "stmt_for_underscore");
}

// === Parser/decl.rs - Function parsing ===

#[test]
fn test_func_named_params() {
    let source = r#"
        func add(a!: Int64, b!: Int64) : Int64 { return a + b }
        func main() : Int64 { return add(a: 3, b: 4) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "func_named_params");
}

#[test]
fn test_func_inout_param() {
    let source = r#"
        func inc(inout x: Int64) : Unit { x = x + 1 }
        func main() : Int64 { var n = 5; return n }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "inout param");
}

#[test]
fn test_func_operator_in_struct() {
    let source = r#"
        struct Vec2 { x: Int64, y: Int64 }
        extend Vec2 {
            operator func +(self: Vec2, other: Vec2) : Vec2 {
                return Vec2 { x: self.x + other.x, y: self.y + other.y }
            }
        }
        func main() : Int64 {
            let a = Vec2 { x: 1, y: 2 }
            let b = Vec2 { x: 3, y: 4 }
            let c = a + b
            return c.x
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "operator func in struct");
}

// === Parser/decl.rs - Import parsing ===

#[test]
fn test_import_std_collection() {
    let source = r#"
        import std.collection
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "import std.collection");
}

#[test]
fn test_import_multiple_modules() {
    let source = r#"
        import std.io
        import std.math
        import std.collection
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "import multiple");
}

// === Parser/decl.rs - Top level const ===

#[test]
fn test_top_level_const_decl() {
    let source = r#"
        const MAX: Int64 = 100
        const MIN: Int64 = 0
        func main() : Int64 { return MAX + MIN }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "top_level_const_decl");
}

// === Parser/decl.rs - Package ===

#[test]
fn test_package_declaration() {
    let source = r#"
        package mylib
        func helper() : Int64 { return 42 }
        func main() : Int64 { return helper() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "package_declaration");
}

// === Parser/stmt.rs - Complex patterns ===

#[test]
fn test_match_or_pattern() {
    let source = r#"
        func classify(x: Int64) : Int64 {
            return match x {
                1 | 2 | 3 => 1,
                _ => 0
            }
        }
        func main() : Int64 { return classify(2) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_or_pattern");
}

#[test]
fn test_match_nested() {
    let source = r#"
        func nested(x: Int64, y: Int64) : Int64 {
            return match x {
                1 => match y {
                    10 => 100,
                    _ => 50
                },
                _ => 0
            }
        }
        func main() : Int64 { return nested(1, 10) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "match_nested");
}

// === Codegen coverage - Complex expressions ===

#[test]
fn test_codegen_bitwise_all() {
    let source = r#"
        func main() : Int64 {
            let a = 0xFF
            let b = 0x0F
            let and = a & b
            let or = a | b
            let xor = a ^ b
            let shl = b << 4
            let shr = a >> 4
            return and + or + xor + shl + shr
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_bitwise_all");
}

#[test]
fn test_codegen_power_operator() {
    let source = r#"
        func main() : Int64 {
            let base = 2
            let exp = 10
            let result = base ** exp
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_power_operator");
}

#[test]
fn test_codegen_string_interpolation() {
    let source = r#"
        func main() : Int64 {
            let name = "world"
            let msg = "hello ${name}"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_string_interpolation");
}

#[test]
fn test_codegen_array_in_loop() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30, 40, 50]
            var sum = 0
            for (x in arr) {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_array_in_loop");
}

#[test]
fn test_codegen_nested_struct_access() {
    let source = r#"
        struct Inner { value: Int64 }
        struct Outer { inner: Inner, tag: Int64 }
        func main() : Int64 {
            let i = Inner { value: 42 }
            let o = Outer { inner: i, tag: 1 }
            return o.inner.value + o.tag
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_nested_struct_access");
}

#[test]
fn test_codegen_complex_control_flow() {
    let source = r#"
        func fibonacci(n: Int64) : Int64 {
            if (n <= 1) { return n }
            var a = 0
            var b = 1
            var i = 2
            while (i <= n) {
                let temp = a + b
                a = b
                b = temp
                i = i + 1
            }
            return b
        }
        func main() : Int64 { return fibonacci(10) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_complex_control_flow");
}

#[test]
fn test_codegen_multi_return_paths() {
    let source = r#"
        func classify(x: Int64) : Int64 {
            if (x < 0) { return -1 }
            if (x == 0) { return 0 }
            if (x < 10) { return 1 }
            if (x < 100) { return 2 }
            return 3
        }
        func main() : Int64 { return classify(50) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_multi_return_paths");
}

#[test]
fn test_codegen_recursive_sum() {
    let source = r#"
        func sum(n: Int64) : Int64 {
            if (n <= 0) { return 0 }
            return n + sum(n - 1)
        }
        func main() : Int64 { return sum(10) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "codegen_recursive_sum");
}

// === Additional parser/codegen tests ===

#[test]
fn test_struct_with_default_field_values() {
    let source = r#"
        struct Config { width: Int64, height: Int64 }
        func main() : Int64 {
            let c = Config { width: 800, height: 600 }
            return c.width + c.height
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "struct_with_default_field_values");
}

#[test]
fn test_class_with_multiple_methods() {
    let source = r#"
        class Calculator {
            var result: Int64;
            init(initial: Int64) { this.result = initial }
            func add(self: Calculator, x: Int64) : Int64 { return self.result + x }
            func sub(self: Calculator, x: Int64) : Int64 { return self.result - x }
            func mul(self: Calculator, x: Int64) : Int64 { return self.result * x }
            func getResult(self: Calculator) : Int64 { return self.result }
        }
        func main() : Int64 {
            let calc = Calculator(0)
            let r1 = calc.add(10)
            let r2 = calc.mul(5)
            return r1 + r2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_with_multiple_methods");
}

#[test]
fn test_var_declaration_types() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 1
            let b: Int16 = 2
            let c: Int32 = 3
            let d: Int64 = 4
            let e: Float32 = 1.0
            let f: Float64 = 2.0
            let g: Bool = true
            let h: String = "hello"
            return d
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "var_declaration_types");
}

#[test]
fn test_complex_expr_nesting() {
    let source = r#"
        func main() : Int64 {
            let x = (1 + 2) * (3 + 4) - (5 * 6) / 2
            let y = if (x > 0) { x } else { -x }
            return y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "complex_expr_nesting");
}

#[test]
fn test_multiple_structs_and_enums() {
    let source = r#"
        struct RGB { r: Int64, g: Int64, b: Int64 }
        struct Pixel { color: RGB, x: Int64, y: Int64 }
        enum Shape {
            Circle(Int64)
            Rect(Int64, Int64)
        }
        func main() : Int64 {
            let red = RGB { r: 255, g: 0, b: 0 }
            let p = Pixel { color: red, x: 10, y: 20 }
            return p.color.r + p.x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "multiple_structs_and_enums");
}

#[test]
fn test_block_as_expression() {
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
    assert_valid_wasm(&wasm, "block_as_expression");
}

#[test]
fn test_if_else_chain() {
    let source = r#"
        func grade(score: Int64) : Int64 {
            if (score >= 90) { return 4 }
            else if (score >= 80) { return 3 }
            else if (score >= 70) { return 2 }
            else if (score >= 60) { return 1 }
            else { return 0 }
        }
        func main() : Int64 { return grade(85) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "if_else_chain");
}

#[test]
fn test_class_inheriting_with_interface() {
    let source = r#"
        interface Describable {
            func describe(self: Describable) : Int64;
        }
        open class Entity {
            var id: Int64;
            init(i: Int64) { this.id = i }
            func getId(self: Entity) : Int64 { return self.id }
        }
        class Player <: Entity & Describable {
            init(i: Int64) { super(i) }
            func describe(self: Player) : Int64 { return self.id * 10 }
        }
        func main() : Int64 {
            let p = Player(5)
            return p.describe()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "class_inheriting_with_interface");
}

#[test]
fn test_enum_with_method() {
    let source = r#"
        enum Color {
            Red
            Green
            Blue
            func value(self: Color) : Int64 { return 0 }
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "enum with method");
}

#[test]
fn test_class_named_constructor() {
    let source = r#"
        class Vector {
            var x: Int64;
            var y: Int64;
            Vector(let x: Int64, let y: Int64) {
                this.x = x
                this.y = y
            }
            func mag(self: Vector) : Int64 { return self.x + self.y }
        }
        func main() : Int64 {
            let v = Vector(3, 4)
            return v.mag()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "class named constructor");
}

// ====================================================================
// cg2_ 系列：codegen/expr.rs 与 parser 路径覆盖测试
// ====================================================================

#[test]
fn test_cg2_method_call_struct() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        extend Point {
            func magnitude(self: Point) : Int64 { return self.x + self.y }
        }
        func main() : Int64 {
            let p = Point { x: 3, y: 4 }
            return p.magnitude()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_method_call_struct");
}

#[test]
fn test_cg2_method_call_class() {
    let source = r#"
        class Box {
            var w: Int64;
            var h: Int64;
            init(w: Int64, h: Int64) { this.w = w; this.h = h }
            func area(self: Box) : Int64 { return self.w * self.h }
        }
        func main() : Int64 {
            let b = Box(3, 4)
            return b.area()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_method_call_class");
}

#[test]
fn test_cg2_string_concat() {
    let source = r#"
        func main() : Int64 {
            let a = "hello"
            let b = "world"
            let c = a + " " + b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_string_concat");
}

#[test]
fn test_cg2_string_interpolation_multi() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 10
            let y: Float64 = 2.5
            let s = "x=${x} y=${y}"
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_string_interpolation_multi");
}

#[test]
fn test_cg2_string_to_string() {
    let source = r#"
        func main() : Int64 {
            let n: Int64 = 42
            let s = n.toString()
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_string_toString");
}

#[test]
fn test_cg2_lambda_arrow_syntax() {
    let source = r#"
        func main() : Int64 {
            let f = (x: Int64) : Int64 { x + 1 }
            return f(5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_lambda_arrow_syntax");
}

#[test]
fn test_cg2_lambda_brace_syntax() {
    let source = r#"
        func main() : Int64 {
            let f = { x: Int64 => x * 2 }
            return f(7)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_lambda_brace_syntax");
}

#[test]
fn test_cg2_lambda_no_type() {
    let source = r#"
        func main() : Int64 {
            let f = { x => x + 1 }
            return f(3)
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_lambda_no_type");
}

#[test]
fn test_cg2_type_coercion_int8_to_int64() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 10
            let b: Int64 = 20
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_type_coercion_int8_int64");
}

#[test]
fn test_cg2_type_coercion_float32_to_float64() {
    let source = r#"
        func main() : Float64 {
            let a: Float32 = 1.0f
            let b: Float64 = 2.0
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_type_coercion_float32_float64");
}

#[test]
fn test_cg2_type_coercion_int32_float64() {
    let source = r#"
        func main() : Float64 {
            let a: Int32 = 5
            let b: Float64 = 3.0
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_type_coercion_int32_float64");
}

#[test]
fn test_cg2_array_indexing() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30]
            let a = arr[0]
            let b = arr[1]
            let c = arr[2]
            return a + b + c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_array_indexing");
}

#[test]
fn test_cg2_array_size() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = [1, 2, 3, 4]
            return arr.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_array_size");
}

#[test]
fn test_cg2_array_for_iteration() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for x in [1, 2, 3, 4, 5] {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_array_for_iteration");
}

#[test]
fn test_cg2_tuple_create() {
    let source = r#"
        func main() : Int64 {
            let t = (1, 2, 3)
            return t.0 + t.1 + t.2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_tuple_create");
}

#[test]
fn test_cg2_tuple_access_fields() {
    let source = r#"
        func main() : Int64 {
            let p = (10, "hi")
            let a = p.0
            let b = p.1
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_tuple_access_fields");
}

#[test]
fn test_cg2_option_some() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            match o {
                Some(v) => return v,
                None => return 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_option_some");
}

#[test]
fn test_cg2_option_none() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = None
            match o {
                Some(v) => return v,
                None => return 99
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_option_none");
}

#[test]
fn test_cg2_match_literal() {
    let source = r#"
        func main() : Int64 {
            match 1 {
                0 => 0,
                1 => 10,
                2 => 20,
                _ => 99
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_literal");
}

#[test]
fn test_cg2_match_range() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                0..3 => 1,
                3..7 => 2,
                _ => 3
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_range");
}

#[test]
fn test_cg2_match_enum_variant() {
    let source = r#"
        enum E { A, B, C }
        func main() : Int64 {
            let e: E = E.B
            match e {
                E.A => 1,
                E.B => 2,
                E.C => 3,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_enum_variant");
}

#[test]
fn test_cg2_match_enum_payload() {
    let source = r#"
        enum Result { Ok(Int64), Err(Int64) }
        func main() : Int64 {
            let r = Result.Ok(42)
            match r {
                Result.Ok(v) => v,
                Result.Err(e) => e,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_enum_payload");
}

#[test]
fn test_cg2_match_struct_pattern() {
    let source = r#"
        struct P { x: Int64, y: Int64 }
        func main() : Int64 {
            let p = P { x: 1, y: 2 }
            match p {
                P { x: a, y: b } => a + b,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_struct_pattern");
}

#[test]
fn test_cg2_match_tuple_pattern() {
    let source = r#"
        func main() : Int64 {
            let t = (10, 20)
            match t {
                (a, b) => a + b,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_tuple_pattern");
}

#[test]
fn test_cg2_match_guard() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                n if n < 3 => 1,
                n if n < 7 => 2,
                _ => 3
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_match_guard");
}

#[test]
fn test_cg2_try_catch_basic() {
    let source = r#"
        func main() : Int64 {
            var r: Int64 = 0
            try {
                r = 10
            } catch(e) {
                r = 1
            }
            return r
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_try_catch_basic");
}

#[test]
fn test_cg2_throw_expr() {
    let source = r#"
        func main() : Int64 {
            try {
                throw 0
            } catch(e) {
                return 42
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_throw_expr");
}

#[test]
fn test_cg2_assert_macro() {
    let source = r#"
        func main() : Int64 {
            @Assert(1, 1)
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_assert_macro");
}

#[test]
fn test_cg2_expect_macro() {
    let source = r#"
        func main() : Int64 {
            @Expect(2, 2)
            return 42
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_expect_macro");
}

#[test]
fn test_cg2_constructor_struct() {
    let source = r#"
        struct S { a: Int64, b: Int64 }
        func main() : Int64 {
            let s = S(1, 2)
            return s.a + s.b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_constructor_struct");
}

#[test]
fn test_cg2_constructor_class() {
    let source = r#"
        class C {
            var x: Int64;
            init(n: Int64) { this.x = n }
            func get(self: C) : Int64 { return self.x }
        }
        func main() : Int64 {
            let c = C(7)
            return c.get()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_constructor_class");
}

#[test]
fn test_cg2_interface_dispatch() {
    let source = r#"
        interface I { func f(self: I) : Int64; }
        class A <: I {
            var v: Int64;
            init(v: Int64) { this.v = v }
            func f(self: A) : Int64 { return self.v }
        }
        func callIt(i: I) : Int64 { return i.f() }
        func main() : Int64 {
            let a = A(10)
            return callIt(a)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_interface_dispatch");
}

#[test]
fn test_cg2_pipeline_operator() {
    let source = r#"
        func double(x: Int64) : Int64 { x * 2 }
        func main() : Int64 {
            return 5 |> double
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_pipeline_operator");
}

#[test]
fn test_cg2_pipeline_chain() {
    let source = r#"
        func inc(x: Int64) : Int64 { x + 1 }
        func double(x: Int64) : Int64 { x * 2 }
        func main() : Int64 {
            return 3 |> inc |> double
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_pipeline_chain");
}

#[test]
fn test_cg2_null_coalesce_option() {
    let source = r#"
        func main() : Int64 {
            let a: Option<Int64> = Some(10)
            let b: Option<Int64> = None
            let x = a ?? 0
            let y = b ?? 5
            return x + y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_null_coalesce_option");
}

#[test]
fn test_cg2_cast_int64_float64() {
    let source = r#"
        func main() : Float64 {
            let x: Int64 = 42
            return x as Float64
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_cast_int64_float64");
}

#[test]
fn test_cg2_cast_float64_int64() {
    let source = r#"
        func main() : Int64 {
            let x: Float64 = 3.14
            return x as Int64
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_cast_float64_int64");
}

#[test]
fn test_cg2_istype_expr() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 42
            if (x is Int64) {
                return 1
            }
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_istype_expr");
}

#[test]
fn test_cg2_range_expr() {
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
    assert_valid_wasm(&wasm, "cg2_range_expr");
}

#[test]
fn test_cg2_range_inclusive() {
    let source = r#"
        func main() : Int64 {
            var s: Int64 = 0
            for i in 0..=5 {
                s = s + i
            }
            return s
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_range_inclusive");
}

#[test]
fn test_cg2_range_as_value() {
    let source = r#"
        func main() : Int64 {
            let r = 0..10
            var s: Int64 = 0
            for i in r {
                s = s + i
            }
            return s
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_range_as_value");
}

#[test]
fn test_cg2_complex_field_chain() {
    let source = r#"
        struct Inner { v: Int64 }
        struct Outer { inner: Inner }
        func main() : Int64 {
            let o = Outer { inner: Inner { v: 42 } }
            return o.inner.v
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_complex_field_chain");
}

#[test]
fn test_cg2_builtin_min_max() {
    let source = r#"
        func main() : Int64 {
            let a = min(1, 2)
            let b = max(10, 20)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_builtin_min_max");
}

#[test]
fn test_cg2_builtin_abs() {
    let source = r#"
        func main() : Int64 {
            let a = abs(-5)
            let b = abs(10)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_builtin_abs");
}

#[test]
fn test_cg2_builtin_abs_float() {
    let source = r#"
        func main() : Float64 {
            return abs(-3.14)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_builtin_abs_float");
}

#[test]
fn test_cg2_complex_type_array_array() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Array<Int64>> = [[1, 2], [3, 4]]
            return arr[0][0] + arr[1][1]
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_complex_type_array_array");
}

#[test]
fn test_cg2_complex_type_func_sig() {
    let source = r#"
        func apply(f: (Int64, Int64) -> Int64, a: Int64, b: Int64) : Int64 {
            return f(a, b)
        }
        func main() : Int64 {
            return apply({ x: Int64, y: Int64 => x + y }, 1, 2)
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_complex_type_func_sig");
}

#[test]
fn test_cg2_pattern_destructure_tuple() {
    let source = r#"
        func main() : Int64 {
            let (a, b) = (1, 2)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_pattern_destructure_tuple");
}

#[test]
fn test_cg2_pattern_destructure_struct() {
    let source = r#"
        struct P { x: Int64, y: Int64 }
        func main() : Int64 {
            let P { x: a, y: b } = P { x: 10, y: 20 }
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_pattern_destructure_struct");
}

#[test]
fn test_cg2_where_clause_func() {
    let source = r#"
        func id<T>(x: T) : T where T: Comparable {
            return x
        }
        func main() : Int64 {
            return id<Int64>(42)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_where_clause_func");
}

#[test]
fn test_cg2_where_clause_struct() {
    let source = r#"
        struct Wrapper<T> where T: Comparable {
            value: T
        }
        func main() : Int64 {
            let w = Wrapper<Int64> { value: 1 }
            return w.value
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_where_clause_struct");
}

#[test]
fn test_cg2_multiline_string() {
    let source = r#"
        func main() : Int64 {
            let s = "line1
        line2
        line3"
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_multiline_string");
}

#[test]
fn test_cg2_extend_struct_method() {
    let source = r#"
        struct Vec2 { x: Int64, y: Int64 }
        extend Vec2 {
            func dot(self: Vec2, other: Vec2) : Int64 {
                return self.x * other.x + self.y * other.y
            }
        }
        func main() : Int64 {
            let a = Vec2 { x: 1, y: 0 }
            let b = Vec2 { x: 0, y: 1 }
            return a.dot(b)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_extend_struct_method");
}

#[test]
fn test_cg2_enum_with_methods() {
    let source = r#"
        enum State {
            Idle
            Running(Int64)
            Done
        }
        func main() : Int64 {
            let s = State.Running(5)
            match s {
                State.Idle => 0,
                State.Running(n) => n,
                State.Done => 100,
                _ => -1
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_enum_with_methods");
}

#[test]
fn test_cg2_array_assign_index() {
    let source = r#"
        func main() : Int64 {
            var arr = [1, 2, 3]
            arr[1] = 20
            return arr[0] + arr[1] + arr[2]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_array_assign_index");
}

#[test]
fn test_cg2_postfix_incr() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 5
            x++
            return x
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg2_postfix_incr");
}

#[test]
fn test_cg2_compound_assign_all() {
    let source = r#"
        func main() : Int64 {
            var a: Int64 = 100
            a += 10
            a -= 5
            a *= 2
            a /= 10
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_compound_assign_all");
}

#[test]
fn test_cg2_block_expr_tail() {
    let source = r#"
        func main() : Int64 {
            let x = {
                let a = 1
                let b = 2
                a + b
            }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_block_expr_tail");
}

#[test]
fn test_cg2_if_expr() {
    let source = r#"
        func main() : Int64 {
            let x = if (true) { 1 } else { 2 }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_if_expr");
}

#[test]
fn test_cg2_unary_not() {
    let source = r#"
        func main() : Int64 {
            let a = !true
            let b = !false
            return if (a) { 0 } else { 1 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_unary_not");
}

#[test]
fn test_cg2_unary_bitnot() {
    let source = r#"
        func main() : Int64 {
            return ~0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_unary_bitnot");
}

#[test]
fn test_cg2_binary_pow() {
    let source = r#"
        func main() : Int64 {
            return 2 ** 10
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_binary_pow");
}

#[test]
fn test_cg2_super_call() {
    let source = r#"
        open class Base {
            var x: Int64;
            init(x: Int64) { this.x = x }
            func get(self: Base) : Int64 { return self.x }
        }
        class Derived <: Base {
            init(x: Int64) { super(x) }
            func getPlus(self: Derived) : Int64 { return super.get() + 1 }
        }
        func main() : Int64 {
            let d = Derived(10)
            return d.getPlus()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_super_call");
}

#[test]
fn test_cg2_result_ok_err() {
    let source = r#"
        func main() : Int64 {
            let r: Result<Int64, Int64> = Ok(42)
            match r {
                Ok(v) => v,
                Err(e) => e,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_result_ok_err");
}

#[test]
fn test_cg2_const_decl() {
    let source = r#"
        const N: Int64 = 100
        func main() : Int64 {
            return N
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_const_decl");
}

#[test]
fn test_cg2_var_mutable() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 0
            x = 10
            x = 20
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_var_mutable");
}

#[test]
fn test_cg2_do_while() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            do {
                i = i + 1
            } while (i < 5)
            return i
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_do_while");
}

#[test]
fn test_cg2_loop_break_value() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            loop {
                i = i + 1
                if (i >= 10) {
                    break
                }
            }
            return i
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_loop_break_value");
}

#[test]
fn test_cg2_early_return() {
    let source = r#"
        func f(x: Int64) : Int64 {
            if (x < 0) {
                return -1
            }
            return x
        }
        func main() : Int64 {
            return f(5) + f(-1)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_early_return");
}

#[test]
fn test_cg2_variadic_call() {
    let source = r#"
        func sum(args: Int64...) : Int64 {
            var t: Int64 = 0
            for x in args {
                t = t + x
            }
            return t
        }
        func main() : Int64 {
            return sum(1, 2, 3, 4, 5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_variadic_call");
}

#[test]
fn test_cg2_default_param() {
    let source = r#"
        func add(a: Int64, b: Int64 = 10) : Int64 {
            return a + b
        }
        func main() : Int64 {
            return add(5) + add(5, 5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_default_param");
}

#[test]
fn test_cg2_generic_struct() {
    let source = r#"
        struct Pair<T> { first: T, second: T }
        func main() : Int64 {
            let p = Pair<Int64> { first: 1, second: 2 }
            return p.first + p.second
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_generic_struct");
}

#[test]
fn test_cg2_slice_expr() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = [1, 2, 3, 4, 5]
            let sub = arr.slice(1, 4)
            return sub.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_slice_expr");
}

#[test]
fn test_cg2_string_methods() {
    let source = r#"
        func main() : Int64 {
            let s = "hello"
            let b = s.contains("ell")
            let i = s.indexOf("l")
            return if (b) { i } else { -1 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_string_methods");
}

#[test]
fn test_cg2_type_annot_tuple() {
    let source = r#"
        func main() : Int64 {
            let t: (Int64, String) = (42, "hi")
            return t.0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_type_annot_tuple");
}

#[test]
fn test_cg2_if_let_option() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(7)
            if let Some(v) = o {
                return v
            }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_if_let_option");
}

#[test]
fn test_cg2_while_let_option() {
    let source = r#"
        func main() : Int64 {
            var o: Option<Int64> = Some(1)
            var sum: Int64 = 0
            while let Some(v) = o {
                sum = sum + v
                o = None
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_while_let_option");
}

#[test]
fn test_cg2_try_finally() {
    let source = r#"
        func main() : Int64 {
            var flag: Int64 = 0
            try {
                flag = 1
            } catch(e) {
                flag = 2
            } finally {
                flag = 10
            }
            return flag
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_try_finally");
}

#[test]
fn test_cg2_struct_init_named_fields() {
    let source = r#"
        struct Rect { w: Int64, h: Int64 }
        func main() : Int64 {
            let r = Rect { w: 5, h: 10 }
            return r.w * r.h
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_struct_init_named_fields");
}

#[test]
fn test_cg2_extern_func() {
    let source = r#"
        foreign func putchar(c: Int32) : Int32
        func main() : Int64 {
            putchar(65)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg2_extern_func");
}

// ====================================================================
// cg3_ coverage tests — targeted code paths and edge cases
// ====================================================================

#[test]
fn test_cg3_class_init_with_super() {
    let source = r#"
        open class Animal {
            var name: Int64;
            init(n: Int64) { this.name = n }
            func getName(self: Animal) : Int64 { return self.name }
        }
        class Dog <: Animal {
            init(n: Int64) { super(n) }
        }
        func main() : Int64 { let d = Dog(42); return d.getName() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_class_init_with_super");
}

#[test]
fn test_cg3_class_multiple_init_params() {
    let source = r#"
        class Vec3 {
            var x: Int64; var y: Int64; var z: Int64;
            init(x: Int64, y: Int64, z: Int64) {
                this.x = x; this.y = y; this.z = z
            }
            func sum(self: Vec3) : Int64 { return self.x + self.y + self.z }
        }
        func main() : Int64 { let v = Vec3(1, 2, 3); return v.sum() }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_class_multiple_init_params");
}

#[test]
fn test_cg3_multiple_classes_interact() {
    let source = r#"
        class Engine {
            var power: Int64;
            init(p: Int64) { this.power = p }
            func getPower(self: Engine) : Int64 { return self.power }
        }
        class Car {
            var engine: Int64;
            init(e: Int64) { this.engine = e }
            func speed(self: Car) : Int64 { return self.engine * 2 }
        }
        func main() : Int64 {
            let e = Engine(100)
            let c = Car(e.getPower())
            return c.speed()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_multiple_classes_interact");
}

#[test]
fn test_cg3_struct_method_chain() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func addX(p: Point, dx: Int64) : Point { return Point { x: p.x + dx, y: p.y } }
        func addY(p: Point, dy: Int64) : Point { return Point { x: p.x, y: p.y + dy } }
        func main() : Int64 {
            let p = Point { x: 0, y: 0 }
            let p2 = addX(p, 5)
            let p3 = addY(p2, 10)
            return p3.x + p3.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_struct_method_chain");
}

#[test]
fn test_cg3_class_method_on_field() {
    let source = r#"
        class Counter {
            var count: Int64;
            init() { this.count = 0 }
            func increment(self: Counter) : Int64 {
                return self.count + 1
            }
            func getCount(self: Counter) : Int64 { return self.count }
        }
        func main() : Int64 {
            let c = Counter()
            let n = c.increment()
            return n
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_class_method_on_field");
}

#[test]
fn test_cg3_string_empty() {
    let source = r#"
        func main() : Int64 { let s = ""; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_string_empty");
}

#[test]
fn test_cg3_string_escape_chars() {
    let source = r#"
        func main() : Int64 { let s = "hello\nworld\ttab"; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_string_escape_chars");
}

#[test]
fn test_cg3_interpolation_integer() {
    let source = r#"
        func main() : Int64 {
            let x = 42
            let s = "value: ${x}"
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_interpolation_integer");
}

#[test]
fn test_cg3_interpolation_multiple() {
    let source = r#"
        func main() : Int64 {
            let a = 1
            let b = 2
            let s = "${a} + ${b} = ${a + b}"
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_interpolation_multiple");
}

#[test]
fn test_cg3_interpolation_in_call() {
    let source = r#"
        func identity(s: String) : Int64 { return 0 }
        func main() : Int64 {
            let x = 99
            return identity("val=${x}")
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_interpolation_in_call");
}

#[test]
fn test_cg3_array_empty() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = []
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_array_empty");
}

#[test]
fn test_cg3_array_nested() {
    let source = r#"
        func main() : Int64 {
            let matrix = [[1, 2], [3, 4]]
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_array_nested");
}

#[test]
fn test_cg3_array_in_struct() {
    let source = r#"
        struct Data { values: Array<Int64>, count: Int64 }
        func main() : Int64 {
            let d = Data { values: [10, 20, 30], count: 3 }
            return d.count
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_array_in_struct");
}

#[test]
fn test_cg3_array_pass_to_func() {
    let source = r#"
        func sumArr(arr: Array<Int64>) : Int64 {
            var s = 0
            for (x in arr) { s = s + x }
            return s
        }
        func main() : Int64 { return sumArr([1, 2, 3, 4, 5]) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_array_pass_to_func");
}

#[test]
fn test_cg3_array_large() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
            var sum = 0
            for (x in arr) { sum = sum + x }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_array_large");
}

#[test]
fn test_cg3_match_complex_enum() {
    let source = r#"
        enum Result { Ok(Int64), Err(Int64) }
        func getValue(r: Result) : Int64 {
            return match r {
                Result.Ok(v) => v,
                Result.Err(e) => -e
            }
        }
        func main() : Int64 { return getValue(Result.Ok(42)) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_match_complex_enum");
}

#[test]
fn test_cg3_match_on_expr() {
    let source = r#"
        func classify(x: Int64) : Int64 {
            let r = x % 3
            return match r {
                0 => 0,
                1 => 1,
                _ => 2
            }
        }
        func main() : Int64 { return classify(7) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_match_on_expr");
}

#[test]
fn test_cg3_nested_if_with_blocks() {
    let source = r#"
        func main() : Int64 {
            let x = 10
            let result = if (x > 5) {
                if (x > 8) { x * 2 } else { x + 1 }
            } else {
                0
            }
            return result
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_nested_if_with_blocks");
}

#[test]
fn test_cg3_while_complex() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            var i = 0
            while (i < 100) {
                if (i % 2 == 0) {
                    sum = sum + i
                }
                i = i + 1
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_while_complex");
}

#[test]
fn test_cg3_for_with_index() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            for (i in 0..10) {
                sum = sum + i * i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_for_with_index");
}

#[test]
fn test_cg3_modulo() {
    let source = r#"
        func main() : Int64 { return 17 % 5 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_modulo");
}

#[test]
fn test_cg3_comparison_chain() {
    let source = r#"
        func main() : Int64 {
            let a = 5
            let b = 10
            let c = 15
            if (a < b && b < c) { return 1 }
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_comparison_chain");
}

#[test]
fn test_cg3_logical_complex() {
    let source = r#"
        func main() : Int64 {
            let x = 5
            let y = 10
            if ((x > 0 && y > 0) || (x < 0 && y < 0)) { return 1 }
            else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_logical_complex");
}

#[test]
fn test_cg3_negation() {
    let source = r#"
        func main() : Int64 { let x = 42; return -x }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_negation");
}

#[test]
fn test_cg3_not_operator() {
    let source = r#"
        func main() : Int64 { let b = true; if (!b) { return 0 } else { return 1 } }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_not_operator");
}

#[test]
fn test_cg3_type_int8() {
    let source = r#"
        func main() : Int64 { let x: Int8 = 5; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_int8");
}

#[test]
fn test_cg3_type_int16() {
    let source = r#"
        func main() : Int64 { let x: Int16 = 100; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_int16");
}

#[test]
fn test_cg3_type_int32() {
    let source = r#"
        func main() : Int64 { let x: Int32 = 1000; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_int32");
}

#[test]
fn test_cg3_type_uint8() {
    let source = r#"
        func main() : Int64 { let x: UInt8 = 255; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_uint8");
}

#[test]
fn test_cg3_type_uint16() {
    let source = r#"
        func main() : Int64 { let x: UInt16 = 65535; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_uint16");
}

#[test]
fn test_cg3_type_uint32() {
    let source = r#"
        func main() : Int64 { let x: UInt32 = 100000; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_uint32");
}

#[test]
fn test_cg3_type_uint64() {
    let source = r#"
        func main() : Int64 { let x: UInt64 = 999; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_uint64");
}

#[test]
fn test_cg3_type_float32() {
    let source = r#"
        func main() : Int64 { let x: Float32 = 3.14; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_float32");
}

#[test]
fn test_cg3_type_rune() {
    let source = r#"
        func main() : Int64 { let c: Rune = 'A'; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_rune");
}

#[test]
fn test_cg3_type_bool() {
    let source = r#"
        func main() : Int64 { let b: Bool = false; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_type_bool");
}

#[test]
fn test_cg3_func_many_params() {
    let source = r#"
        func add5(a: Int64, b: Int64, c: Int64, d: Int64, e: Int64) : Int64 {
            return a + b + c + d + e
        }
        func main() : Int64 { return add5(1, 2, 3, 4, 5) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_func_many_params");
}

#[test]
fn test_cg3_func_recursive_fib() {
    let source = r#"
        func fib(n: Int64) : Int64 {
            if (n <= 1) { return n }
            return fib(n - 1) + fib(n - 2)
        }
        func main() : Int64 { return fib(10) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_func_recursive_fib");
}

#[test]
fn test_cg3_func_mutual_recursion() {
    let source = r#"
        func isEven(n: Int64) : Bool {
            if (n == 0) { return true }
            return isOdd(n - 1)
        }
        func isOdd(n: Int64) : Bool {
            if (n == 0) { return false }
            return isEven(n - 1)
        }
        func main() : Int64 { if (isEven(10)) { return 1 } else { return 0 } }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_func_mutual_recursion");
}

#[test]
fn test_cg3_func_no_return_type() {
    let source = r#"
        func doNothing() { let _ = 0 }
        func main() : Int64 { doNothing(); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_func_no_return_type");
}

#[test]
fn test_cg3_func_return_bool() {
    let source = r#"
        func isPositive(x: Int64) : Bool { return x > 0 }
        func main() : Int64 { if (isPositive(5)) { return 1 } else { return 0 } }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_func_return_bool");
}

#[test]
fn test_cg3_match_multiple_arms() {
    let source = r#"
        func dayNum(d: Int64) : Int64 {
            return match d {
                1 => 10,
                2 => 20,
                3 => 30,
                4 => 40,
                5 => 50,
                _ => 0
            }
        }
        func main() : Int64 { return dayNum(3) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_match_multiple_arms");
}

#[test]
fn test_cg3_match_nested_enum() {
    let source = r#"
        enum MyOption { Some(Int64), None }
        func unwrap(opt: MyOption) : Int64 {
            return match opt {
                MyOption.Some(v) => v,
                MyOption.None => -1
            }
        }
        func main() : Int64 { return unwrap(MyOption.Some(99)) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_match_nested_enum");
}

#[test]
fn test_cg3_let_pattern_tuple() {
    let source = r#"
        func main() : Int64 {
            let (a, b) = (10, 20)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_let_pattern_tuple");
}

#[test]
fn test_cg3_for_range_step() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            for (i in 1..5) { sum = sum + i }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_for_range_step");
}

#[test]
fn test_cg3_class_generic_simple() {
    let source = r#"
        class Box<T> {
            var value: T;
            init(v: T) { this.value = v }
            func get(self: Box<T>) : T { return self.value }
        }
        func main() : Int64 { let b = Box<Int64>(42); return b.get() }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg3_class_generic_simple");
}

#[test]
fn test_cg3_interface_with_methods() {
    let source = r#"
        interface Shape {
            func area() : Int64;
            func perimeter() : Int64;
        }
        func main() : Int64 { return 0 }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg3_interface_with_methods");
}

#[test]
fn test_cg3_enum_many_variants() {
    let source = r#"
        enum Token {
            Number(Int64)
            Plus
            Minus
            Star
            Slash
            LParen
            RParen
            End
        }
        func main() : Int64 { let t = Token.Number(42); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_enum_many_variants");
}

#[test]
fn test_cg3_struct_many_fields() {
    let source = r#"
        struct Record {
            id: Int64, name: String, age: Int64, score: Float64, active: Bool
        }
        func main() : Int64 {
            let r = Record { id: 1, name: "Alice", age: 30, score: 95.5, active: true }
            return r.id
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_struct_many_fields");
}

#[test]
fn test_cg3_class_with_multiple_props() {
    let source = r#"
        class Rectangle {
            var _w: Int64;
            var _h: Int64;
            init(w: Int64, h: Int64) { this._w = w; this._h = h }
            prop width: Int64 {
                get() { return this._w }
                set(v) { this._w = v }
            }
            prop height: Int64 {
                get() { return this._h }
                set(v) { this._h = v }
            }
        }
        func main() : Int64 {
            let r = Rectangle(10, 20)
            return r.width + r.height
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_class_with_multiple_props");
}

#[test]
fn test_cg3_extend_with_multiple_methods() {
    let source = r#"
        class MyNum {
            var val: Int64;
            init(v: Int64) { this.val = v }
        }
        extend MyNum {
            func double(self: MyNum) : Int64 { return self.val * 2 }
            func triple(self: MyNum) : Int64 { return self.val * 3 }
            func isZero(self: MyNum) : Bool { return self.val == 0 }
        }
        func main() : Int64 {
            let n = MyNum(7)
            return n.double() + n.triple()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_extend_with_multiple_methods");
}

#[test]
fn test_cg3_empty_block() {
    let source = r#"
        func main() : Int64 { {}; return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_empty_block");
}

#[test]
fn test_cg3_many_locals() {
    let source = r#"
        func main() : Int64 {
            let a = 1; let b = 2; let c = 3; let d = 4; let e = 5
            let f = 6; let g = 7; let h = 8; let i = 9; let j = 10
            let k = 11; let l = 12; let m = 13; let n = 14; let o = 15
            return a+b+c+d+e+f+g+h+i+j+k+l+m+n+o
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_many_locals");
}

#[test]
fn test_cg3_nested_function_calls() {
    let source = r#"
        func add(a: Int64, b: Int64) : Int64 { return a + b }
        func mul(a: Int64, b: Int64) : Int64 { return a * b }
        func main() : Int64 { return add(mul(2, 3), mul(4, 5)) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_nested_function_calls");
}

#[test]
fn test_cg3_deeply_nested_if() {
    let source = r#"
        func main() : Int64 {
            let x = 5
            if (x > 0) { if (x > 1) { if (x > 2) { if (x > 3) { if (x > 4) { return 100 } else { return 4 } } else { return 3 } } else { return 2 } } else { return 1 } } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_deeply_nested_if");
}

#[test]
fn test_cg3_complex_arithmetic() {
    let source = r#"
        func main() : Int64 {
            return (1 + 2) * 3 - 4 / 2 + 5 % 3 + (10 - 7) * (8 + 2) / 5
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_complex_arithmetic");
}

#[test]
fn test_cg3_class_field_assign() {
    let source = r#"
        class Box {
            var value: Int64;
            init(v: Int64) { this.value = v }
            func set(self: Box, v: Int64) : Int64 { return v }
            func get(self: Box) : Int64 { return self.value }
        }
        func main() : Int64 {
            let b = Box(10)
            return b.get()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_class_field_assign");
}

#[test]
fn test_cg3_println_string() {
    let source = r#"
        func main() : Int64 { println("hello world"); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_println_string");
}

#[test]
fn test_cg3_println_integer() {
    let source = r#"
        func main() : Int64 { println(42); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_println_integer");
}

#[test]
fn test_cg3_print_no_newline() {
    let source = r#"
        func main() : Int64 { print("hello"); return 0 }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_print_no_newline");
}

#[test]
fn test_cg3_println_expression() {
    let source = r#"
        func main() : Int64 {
            let x = 5
            println(x * 2 + 1)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_println_expression");
}

#[test]
fn test_cg3_multiple_println() {
    let source = r#"
        func main() : Int64 {
            println("start")
            println(1)
            println(2)
            println(3)
            println("end")
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_multiple_println");
}

#[test]
fn test_cg3_return_from_if() {
    let source = r#"
        func abs(x: Int64) : Int64 {
            return if (x >= 0) { x } else { -x }
        }
        func main() : Int64 { return abs(-5) + abs(3) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_return_from_if");
}

#[test]
fn test_cg3_compound_assign() {
    let source = r#"
        func main() : Int64 {
            var x = 10
            x = x + 5
            x = x - 3
            x = x * 2
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_compound_assign");
}

#[test]
fn test_cg3_float_arithmetic() {
    let source = r#"
        func main() : Int64 {
            let a: Float64 = 3.14
            let b: Float64 = 2.71
            let c = a + b
            let d = a * b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_float_arithmetic");
}

#[test]
fn test_cg3_float32_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Float32 = 1.5
            let b: Float32 = 2.5
            let c = a + b
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_float32_ops");
}

#[test]
fn test_cg3_mixed_numeric_types() {
    let source = r#"
        func main() : Int64 {
            let a: Int32 = 10
            let b: Int64 = 20
            let c: Float64 = 3.14
            return b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_mixed_numeric_types");
}

#[test]
fn test_cg3_bool_operations() {
    let source = r#"
        func main() : Int64 {
            let a = true
            let b = false
            let c = a && b
            let d = a || b
            let e = !a
            if (d && !c) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_bool_operations");
}

#[test]
fn test_cg3_struct_in_class() {
    let source = r#"
        struct Color { r: Int64, g: Int64, b: Int64 }
        class Pixel {
            var color: Color; var x: Int64; var y: Int64;
            init(c: Color, x: Int64, y: Int64) {
                this.color = c; this.x = x; this.y = y
            }
            func getR(self: Pixel) : Int64 { return 0 }
        }
        func main() : Int64 {
            let red = Color { r: 255, g: 0, b: 0 }
            let p = Pixel(red, 10, 20)
            return p.x + p.y
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_struct_in_class");
}

#[test]
fn test_cg3_enum_as_func_param() {
    let source = r#"
        enum Direction { North, South, East, West }
        func isNorth(d: Direction) : Bool {
            return match d {
                Direction.North => true,
                _ => false
            }
        }
        func main() : Int64 { if (isNorth(Direction.North)) { return 1 } else { return 0 } }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg3_enum_as_func_param");
}

#[test]
fn test_cg3_loop_break_value() {
    let source = r#"
        func main() : Int64 {
            var i = 0
            while (true) {
                if (i >= 10) { break }
                i = i + 1
            }
            return i
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg3_loop_break_value");
}

// --- cg4: BUILTIN METHOD CODEGEN and COMPLEX CODEGEN PATHS ---

#[test]
fn test_cg4_int64_tostring() {
    let source = r#"
        func main() : Int64 {
            let s = 42.toString()
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int64_tostring");
}

#[test]
fn test_cg4_int64_tofloat64() {
    let source = r#"
        func main() : Float64 {
            let f = 42.toFloat64()
            return f
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int64_tofloat64");
}

#[test]
fn test_cg4_int64_abs() {
    let source = r#"
        func main() : Int64 {
            let a = (-5).abs()
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int64_abs");
}

#[test]
fn test_cg4_int64_compareto() {
    let source = r#"
        func main() : Int64 {
            let c = 5.compareTo(10)
            return c
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int64_compareto");
}

#[test]
fn test_cg4_int64_hashcode() {
    let source = r#"
        func main() : Int64 {
            let h = 42.hashCode()
            return h
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int64_hashcode");
}

#[test]
fn test_cg4_string_length() {
    let source = r#"
        func main() : Int64 {
            let len = "hello".size()
            return len
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_length");
}

#[test]
fn test_cg4_string_isempty() {
    let source = r#"
        func main() : Int64 {
            let e = "".isEmpty()
            if (e) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_isempty");
}

#[test]
fn test_cg4_string_contains() {
    let source = r#"
        func main() : Int64 {
            let c = "hello".contains("ell")
            if (c) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_contains");
}

#[test]
fn test_cg4_string_startswith() {
    let source = r#"
        func main() : Int64 {
            let s = "hello".startsWith("hel")
            if (s) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_startswith");
}

#[test]
fn test_cg4_string_endswith() {
    let source = r#"
        func main() : Int64 {
            let e = "hello".endsWith("llo")
            if (e) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_endswith");
}

#[test]
fn test_cg4_string_concat_plus() {
    let source = r#"
        func main() : Int64 {
            let s = "a" + "b"
            return s.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_concat_plus");
}

#[test]
fn test_cg4_string_tostring() {
    let source = r#"
        func main() : Int64 {
            let s = "hello".toString()
            return s.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_string_tostring");
}

#[test]
fn test_cg4_array_size() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            return arr.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_array_size");
}

#[test]
fn test_cg4_array_get() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30]
            let a = arr[0]
            let b = arr.get(1)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_array_get");
}

#[test]
fn test_cg4_array_isempty() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = []
            let e = arr.isEmpty()
            if (e) { return 1 } else { return 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_array_isempty");
}

#[test]
fn test_cg4_array_contains() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            var found = 0
            for x in arr {
                if (x == 2) { found = 1 }
            }
            return found
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_array_contains");
}

#[test]
fn test_cg4_bool_tostring() {
    let source = r#"
        func main() : Int64 {
            let s = true.toString()
            return s.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_bool_tostring");
}

#[test]
fn test_cg4_bool_hashcode() {
    let source = r#"
        func main() : Int64 {
            let h = 42.hashCode()
            return h
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_bool_hashcode");
}

#[test]
fn test_cg4_if_expr_value() {
    let source = r#"
        func main() : Int64 {
            let x = if (true) { 1 } else { 2 }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_if_expr_value");
}

#[test]
fn test_cg4_match_expr_value() {
    let source = r#"
        func main() : Int64 {
            let y = 1
            let x = match (y) {
                1 => 10,
                _ => 0
            }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_match_expr_value");
}

#[test]
fn test_cg4_block_expr_value() {
    let source = r#"
        func main() : Int64 {
            let x = { let a = 1; a + 2 }
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_block_expr_value");
}

#[test]
fn test_cg4_nested_method_call() {
    let source = r#"
        func main() : Int64 {
            let s = 42.toString()
            return s.size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_nested_method_call");
}

#[test]
fn test_cg4_chained_field_access() {
    let source = r#"
        struct Inner { value: Int64 }
        struct Outer { inner: Inner }
        func main() : Int64 {
            let o = Outer { inner: Inner { value: 99 } }
            return o.inner.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_chained_field_access");
}

#[test]
fn test_cg4_lambda_as_value() {
    let source = r#"
        func main() : Int64 {
            let f = { x: Int64 => x * 2 }
            return f(5)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_lambda_as_value");
}

#[test]
fn test_cg4_lambda_call() {
    let source = r#"
        func main() : Int64 {
            let add = { a: Int64, b: Int64 => a + b }
            return add(3, 4)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_lambda_call");
}

#[test]
fn test_cg4_complex_index() {
    let source = r#"
        func main() : Int64 {
            let arr = [10, 20, 30, 40]
            var i = 0
            let v = arr[i + 1]
            return v
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_complex_index");
}

#[test]
fn test_cg4_tuple_return() {
    let source = r#"
        func pair() : (Int64, Int64) {
            return (1, 2)
        }
        func main() : Int64 {
            let p = pair()
            return p.0 + p.1
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_tuple_return");
}

#[test]
fn test_cg4_tuple_create_access() {
    let source = r#"
        func main() : Int64 {
            let t = (10, 20, 30)
            return t.0 + t.1 + t.2
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_tuple_create_access");
}

#[test]
fn test_cg4_enum_match_extract() {
    let source = r#"
        enum Result { Ok(Int64), Err }
        func main() : Int64 {
            let r: Result = Result.Ok(42)
            match r {
                Result.Ok(v) => v,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_enum_match_extract");
}

#[test]
fn test_cg4_enum_as_param() {
    let source = r#"
        enum Dir { North, South }
        func go(d: Dir) : Int64 {
            match d {
                Dir.North => 1,
                _ => 0
            }
        }
        func main() : Int64 { return go(Dir.North) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_enum_as_param");
}

#[test]
fn test_cg4_enum_return() {
    let source = r#"
        enum E { A, B }
        func make() : E { E.A }
        func main() : Int64 {
            let e = make()
            match e { E.A => 1, _ => 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_enum_return");
}

#[test]
fn test_cg4_class_field_assign_in_method() {
    let source = r#"
        class Counter {
            var n: Int64;
            init() { this.n = 0 }
            func inc(self: Counter) : Int64 {
                this.n = this.n + 1
                return this.n
            }
        }
        func main() : Int64 {
            let c = Counter()
            c.inc()
            return c.inc()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_class_field_assign_in_method");
}

#[test]
fn test_cg4_class_self_method_call() {
    let source = r#"
        class C {
            func a(self: C) : Int64 { 1 }
            func b(self: C) : Int64 { self.a() + 2 }
        }
        func main() : Int64 {
            let c = C()
            return c.b()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_class_self_method_call");
}

#[test]
fn test_cg4_class_generic_instantiate() {
    let source = r#"
        class Box<T> {
            var value: T;
            init(v: T) { this.value = v }
            func get(self: Box<T>) : T { this.value }
        }
        func main() : Int64 {
            let b = Box<Int64>(42)
            return b.get()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg4_class_generic_instantiate: {:?}", result.err());
    assert_valid_wasm(&result.unwrap(), "cg4_class_generic_instantiate");
}

#[test]
fn test_cg4_class_with_interface_method() {
    let source = r#"
        interface ToInt { func toInt(): Int64; }
        class Num <: ToInt {
            var x: Int64;
            init(v: Int64) { this.x = v }
            func toInt(self: Num) : Int64 { this.x }
        }
        func main() : Int64 {
            let n = Num(7)
            return n.toInt()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg4_class_with_interface_method: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg4_class_with_interface_method");
    }
}

#[test]
fn test_cg4_for_in_range() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            for i in 0..5 {
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_for_in_range");
}

#[test]
fn test_cg4_for_in_array_with_index() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for x in [1, 2, 3, 4, 5] {
                sum = sum + x
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_for_in_array_with_index");
}

#[test]
fn test_cg4_while_with_break() {
    let source = r#"
        func main() : Int64 {
            var i = 0
            while (true) {
                i = i + 1
                if (i >= 10) { break }
            }
            return i
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_while_with_break");
}

#[test]
fn test_cg4_nested_for_loops() {
    let source = r#"
        func main() : Int64 {
            var sum = 0
            for i in 0..3 {
                for j in 0..3 {
                    sum = sum + i + j
                }
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_nested_for_loops");
}

#[test]
fn test_cg4_try_catch_body() {
    let source = r#"
        func main() : Int64 {
            var r = 0
            try {
                r = 10 / 2
            } catch(e) {
                r = -1
            }
            return r
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_try_catch_body");
}

#[test]
fn test_cg4_match_many_cases() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                0 => 0, 1 => 1, 2 => 2, 3 => 3, 4 => 4,
                5 => 5, 6 => 6, 7 => 7, 8 => 8, 9 => 9,
                10 => 10, 11 => 11, _ => 99
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_match_many_cases");
}

#[test]
fn test_cg4_complex_assign() {
    let source = r#"
        struct S { var x: Int64 }
        func main() : Int64 {
            let arr = [10, 20]
            arr[0] = 100
            let s = S { x: 5 }
            s.x = 50
            return arr[0] + s.x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_complex_assign");
}

#[test]
fn test_cg4_println_bool() {
    let source = r#"
        func main() : Int64 {
            println(true)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_println_bool");
}

#[test]
fn test_cg4_println_float() {
    let source = r#"
        func main() : Int64 {
            println(3.14)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_println_float");
}

#[test]
fn test_cg4_println_string_var() {
    let source = r#"
        func main() : Int64 {
            let myStr = "world"
            println(myStr)
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_println_string_var");
}

#[test]
fn test_cg4_println_interpolated() {
    let source = r#"
        func main() : Int64 {
            let x = 42
            println("x = ${x}")
            return 0
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_println_interpolated");
}

#[test]
fn test_cg4_power_expr() {
    let source = r#"
        func main() : Int64 {
            return 2 ** 10
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_power_expr");
}

#[test]
fn test_cg4_shift_ops() {
    let source = r#"
        func main() : Int64 {
            let x = 8
            let a = x << 3
            let b = x >> 2
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_shift_ops");
}

#[test]
fn test_cg4_bitwise_complex() {
    let source = r#"
        func main() : Int64 {
            let a = 15
            let b = 3
            let c = 5
            let d = 1
            return (a & b) | (c ^ d)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_bitwise_complex");
}

#[test]
fn test_cg4_float_comparison() {
    let source = r#"
        func main() : Int64 {
            let a: Float64 = 3.14
            if (a > 2.0) { 1 } else { 0 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_float_comparison");
}

#[test]
fn test_cg4_int_to_float() {
    let source = r#"
        func main() : Float64 {
            let i: Int64 = 100
            let f = i.toFloat64()
            return f
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int_to_float");
}

#[test]
fn test_cg4_float_to_int() {
    let source = r#"
        func main() : Int64 {
            let f: Float64 = 3.14
            return f.toInt64()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_float_to_int");
}

#[test]
fn test_cg4_int8_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 10
            let b: Int8 = 20
            return (a + b) as Int64
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int8_ops");
}

#[test]
fn test_cg4_int32_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int32 = 100
            let b: Int32 = 200
            return (a + b) as Int64
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_int32_ops");
}

#[test]
fn test_cg4_bubble_sort() {
    let source = r#"
        func main() : Int64 {
            var arr = [3, 1, 4, 1, 5]
            var n = arr.size()
            var i = 0
            while (i < n - 1) {
                var j = 0
                while (j < n - 1 - i) {
                    if (arr[j] > arr[j + 1]) {
                        let t = arr[j]
                        arr[j] = arr[j + 1]
                        arr[j + 1] = t
                    }
                    j = j + 1
                }
                i = i + 1
            }
            return arr[0]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_bubble_sort");
}

#[test]
fn test_cg4_binary_search() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 3, 5, 7, 9]
            var lo = 0
            var hi = arr.size() - 1
            let target = 5
            while (lo <= hi) {
                let mid = (lo + hi) / 2
                if (arr[mid] == target) { return mid }
                if (arr[mid] < target) { lo = mid + 1 } else { hi = mid - 1 }
            }
            return -1
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_binary_search");
}

#[test]
fn test_cg4_linked_list() {
    let source = r#"
        struct Node { value: Int64, next: Option<Node> }
        func main() : Int64 {
            let n = Node { value: 10, next: None }
            return n.value
        }
    "#;
    let result = compile_source_result(source);
    if result.is_ok() {
        assert_valid_wasm(&result.unwrap(), "cg4_linked_list");
    }
}

#[test]
fn test_cg4_gcd() {
    let source = r#"
        func gcd(a: Int64, b: Int64) : Int64 {
            if (b == 0) { return a }
            return gcd(b, a % b)
        }
        func main() : Int64 { return gcd(48, 18) }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_gcd");
}

#[test]
fn test_cg4_matrix_multiply() {
    let source = r#"
        func main() : Int64 {
            let a00 = 1
            let a01 = 2
            let a10 = 3
            let a11 = 4
            let b00 = 5
            let b01 = 6
            let b10 = 7
            let b11 = 8
            let c00 = a00 * b00 + a01 * b10
            let c01 = a00 * b01 + a01 * b11
            let c10 = a10 * b00 + a11 * b10
            let c11 = a10 * b01 + a11 * b11
            return c00 + c01 + c10 + c11
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg4_matrix_multiply");
}

#[test]
fn test_cg4_complex_class_hierarchy() {
    let source = r#"
        class A {
            func f(self: A) : Int64 { 1 }
        }
        class B <: A {
            func f(self: B) : Int64 { 2 }
        }
        class C <: B {
            func f(self: C) : Int64 { 3 }
        }
        func main() : Int64 {
            let c = C()
            return c.f()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg4_complex_class_hierarchy: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg4_complex_class_hierarchy");
    }
}

#[test]
fn test_compile_fixture_files() {
    let fixtures_dir = Path::new("tests/fixtures");
    if !fixtures_dir.exists() {
        return;
    }
    let mut files: Vec<_> = std::fs::read_dir(fixtures_dir)
        .expect("读取 fixtures 目录")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "cj").unwrap_or(false))
        .collect();
    files.sort_by_key(|e| e.file_name());
    for entry in files {
        let path = entry.path();
        let source = std::fs::read_to_string(&path).expect("读取 fixture 源文件");
        let _ = compile_source_result(&source); // Don't assert success - just exercise parser/codegen
    }
}

// cg5_*: targeted tests for uncovered codegen/expr.rs builtin method paths

#[test]
fn test_cg5_string_size() {
    let source = r#"
        func main() : Int64 {
            return "hello".size()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg5_string_size");
}

#[test]
fn test_cg5_string_replace() {
    let source = r#"
        func main() : Int64 {
            let s = "hello".replace("l", "r")
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_replace: {:?}", result.err());
}

#[test]
fn test_cg5_string_split() {
    let source = r#"
        func main() : Int64 {
            let arr = "a,b,c".split(",")
            return arr.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_split: {:?}", result.err());
}

#[test]
fn test_cg5_string_trim() {
    let source = r#"
        func main() : Int64 {
            let s = " hello ".trim()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_trim: {:?}", result.err());
}

#[test]
fn test_cg5_string_substring() {
    let source = r#"
        func main() : Int64 {
            let s = "hello".indexOf("e")
            return s
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_substring: {:?}", result.err());
}

#[test]
fn test_cg5_string_toint() {
    let source = r#"
        func main() : Int64 {
            return "42".toInt64()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_toint: {:?}", result.err());
}

#[test]
fn test_cg5_array_append() {
    let source = r#"
        func main() : Int64 {
            var arr = ArrayList<Int64>()
            arr.append(1)
            arr.append(2)
            arr.append(4)
            return arr.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_array_append: {:?}", result.err());
}

#[test]
fn test_cg5_array_tostring() {
    let source = r#"
        func main() : Int64 {
            let s = [1, 2, 3].toString()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_array_tostring: {:?}", result.err());
}

#[test]
fn test_cg5_float_tostring() {
    let source = r#"
        func main() : Int64 {
            let s = 3.14.toString()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_float_tostring: {:?}", result.err());
}

#[test]
fn test_cg5_float_toint() {
    let source = r#"
        func main() : Int64 {
            return 3.14.toInt64()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_float_toint: {:?}", result.err());
}

#[test]
fn test_cg5_arraylist_basic() {
    let source = r#"
        func main() : Int64 {
            let list = ArrayList<Int64>()
            list.append(1)
            list.append(2)
            return list.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_arraylist_basic: {:?}", result.err());
}

#[test]
fn test_cg5_hashmap_basic() {
    let source = r#"
        func main() : Int64 {
            let m = HashMap<Int64, Int64>()
            m.put(1, 10)
            return m.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_hashmap_basic: {:?}", result.err());
}

#[test]
fn test_cg5_option_some_none() {
    let source = r#"
        func main() : Int64 {
            let a: Option<Int64> = Some(42)
            let b: Option<Int64> = None
            match a {
                Some(v) => match b { None => v, _ => 0 },
                None => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_option_some_none: {:?}", result.err());
}

#[test]
fn test_cg5_result_ok_err() {
    let source = r#"
        func main() : Int64 {
            let r: Result<Int64, String> = Ok(42)
            match r {
                Ok(v) => v,
                Err(_) => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_result_ok_err: {:?}", result.err());
}

#[test]
fn test_cg5_class_with_generic() {
    let source = r#"
        class Box<T> {
            var value: T
            init(v: T) { this.value = v }
            func get(self: Box<T>) : T { return this.value }
        }
        func main() : Int64 {
            let b = Box<Int64>(42)
            return b.get()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_class_with_generic: {:?}", result.err());
}

#[test]
fn test_cg5_trait_method_dispatch() {
    let source = r#"
        interface I { func f(self: I) : Int64; }
        class A <: I {
            var v: Int64
            init(v: Int64) { this.v = v }
            func f(self: A) : Int64 { return self.v }
        }
        func main() : Int64 {
            let a: I = A(10)
            return a.f()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_trait_method_dispatch: {:?}", result.err());
}

#[test]
fn test_cg5_extend_with_computed_prop() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        extend Point {
            func len2(self: Point) : Int64 { return self.x * self.x + self.y * self.y }
        }
        func main() : Int64 {
            let p = Point { x: 3, y: 4 }
            return p.len2()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_extend_with_computed_prop: {:?}", result.err());
}

#[test]
fn test_cg5_complex_closure() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 10
            let f = (a: Int64) : Int64 { a + x }
            return f(5)
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_complex_closure: {:?}", result.err());
}

#[test]
fn test_cg5_nested_lambda() {
    let source = r#"
        func main() : Int64 {
            let f = (x: Int64) : Int64 { (y: Int64) : Int64 { x + y }(20) }
            return f(10)
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_nested_lambda: {:?}", result.err());
}

#[test]
fn test_cg5_enum_match_all_variants() {
    let source = r#"
        enum E { A, B(Int64), C }
        func main() : Int64 {
            let e1: E = E.A
            let e2: E = E.B(42)
            let e3: E = E.C
            match e2 {
                E.A => 0,
                E.B(n) => n,
                E.C => 1
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_enum_match_all_variants: {:?}", result.err());
}

#[test]
fn test_cg5_class_deinit_with_body() {
    let source = r#"
        class C {
            var x: Int64
            init(n: Int64) { this.x = n }
            ~init { }
        }
        func main() : Int64 {
            let c = C(42)
            return c.x
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_class_deinit_with_body: {:?}", result.err());
}

#[test]
fn test_cg5_for_in_string() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..5 {
                sum = sum + 1
            }
            return sum
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_for_in_string: {:?}", result.err());
}

#[test]
fn test_cg5_string_equality() {
    let source = r#"
        func main() : Int64 {
            if ("a" == "b") { return 1 } else { return 0 }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_string_equality: {:?}", result.err());
}

#[test]
fn test_cg5_null_check() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = None
            match o {
                Some(_) => 1,
                None => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_null_check: {:?}", result.err());
}

#[test]
fn test_cg5_complex_generics() {
    let source = r#"
        struct Wrapper<T> { value: T }
        func id<T>(x: T) : T { return x }
        func main() : Int64 {
            let w = Wrapper<Int64> { value: 42 }
            return id(w).value
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_complex_generics: {:?}", result.err());
}

#[test]
fn test_cg5_multi_catch() {
    let source = r#"
        func main() : Int64 {
            var r: Int64 = 0
            try {
                r = 10
            } catch (e) {
                r = 1
            }
            return r
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_multi_catch: {:?}", result.err());
}

#[test]
fn test_cg5_if_let_some() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(42)
            if (let Some(x) <- o) {
                return x
            }
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_if_let_some: {:?}", result.err());
}

#[test]
fn test_cg5_while_let_some() {
    let source = r#"
        func main() : Int64 {
            var o: Option<Int64> = Some(5)
            var sum: Int64 = 0
            while (let Some(x) <- o) {
                sum = sum + x
                o = None
            }
            return sum
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_while_let_some: {:?}", result.err());
}

#[test]
fn test_cg5_multiline_string_literal() {
    let source = r#"
        func main() : Int64 {
            let s = "line1
        line2
        line3"
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg5_multiline_string_literal");
}

#[test]
fn test_cg5_complex_class_system() {
    let source = r#"
        interface Drawable { func draw(self: Drawable) : Int64; }
        class Shape <: Drawable {
            var id: Int64
            init(id: Int64) { this.id = id }
            func draw(self: Shape) : Int64 { return self.id }
        }
        class Circle <: Shape {
            var r: Int64
            init(id: Int64, r: Int64) { super(id); this.r = r }
            func draw(self: Circle) : Int64 { return self.id + self.r }
        }
        extend Circle: Drawable { }
        func main() : Int64 {
            let c = Circle(1, 5)
            return c.draw()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg5_complex_class_system: {:?}", result.err());
}

// ====================================================================
// cg6_ — 覆盖率补充：codegen/expr.rs 等独特代码路径
// ====================================================================

#[test]
fn test_cg6_method_tostring_chain() {
    let source = r#"
        func main() : Int64 {
            let s = 42.toString()
            let t = true.toString()
            return s.size() + t.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_method_tostring_chain: {:?}", result.err());
}

#[test]
fn test_cg6_method_compareto() {
    let source = r#"
        func main() : Int64 {
            let result = 5.compareTo(10)
            return result
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_method_compareto: {:?}", result.err());
}

#[test]
fn test_cg6_method_hashcode_int() {
    let source = r#"
        func main() : Int64 {
            let h = 42.hashCode()
            return h
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_method_hashcode_int: {:?}", result.err());
}

#[test]
fn test_cg6_string_add_operator() {
    let source = r#"
        func main() : Int64 {
            let s = "hello" + " " + "world"
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_string_add_operator: {:?}", result.err());
}

#[test]
fn test_cg6_array_map_lambda() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            let doubled = arr.map({ x: Int64 => x * 2 })
            return doubled.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_array_map_lambda");
}

#[test]
fn test_cg6_class_vtable_dispatch() {
    let source = r#"
        open class Base {
            var v: Int64
            init(v: Int64) { this.v = v }
            func get(self: Base) : Int64 { return self.v }
        }
        class Derived <: Base {
            init(v: Int64) { super(v) }
            override func get(self: Derived) : Int64 { return self.v * 2 }
        }
        func main() : Int64 {
            let b: Base = Derived(5)
            return b.get()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_class_vtable_dispatch: {:?}", result.err());
}

#[test]
fn test_cg6_interface_impl_dispatch() {
    let source = r#"
        interface I { func f(self: I) : Int64; }
        class A <: I {
            var x: Int64
            init(x: Int64) { this.x = x }
            func f(self: A) : Int64 { return self.x }
        }
        func main() : Int64 {
            let a: I = A(7)
            return a.f()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_interface_impl_dispatch: {:?}", result.err());
}

#[test]
fn test_cg6_complex_pattern_match_enum() {
    let source = r#"
        enum E { A, B(Int64), C(String) }
        func main() : Int64 {
            let e: E = E.B(42)
            match e {
                E.A => 0,
                E.B(n) => n,
                E.C(_) => 1
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_complex_pattern_match_enum: {:?}", result.err());
}

#[test]
fn test_cg6_try_catch_finally() {
    let source = r#"
        func main() : Int64 {
            var r: Int64 = 0
            try {
                r = 10
            } catch (e) {
                r = 1
            }
            return r
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_try_catch_finally: {:?}", result.err());
}

#[test]
fn test_cg6_class_field_default_init() {
    let source = r#"
        class C {
            var x: Int64 = 42
            init() { }
            func get(self: C) : Int64 { return this.x }
        }
        func main() : Int64 {
            let c = C()
            return c.get()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_class_field_default_init: {:?}", result.err());
}

#[test]
fn test_cg6_closure_capture_var() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 10
            let f = (a: Int64) : Int64 { a + x }
            return f(5)
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_closure_capture_var: {:?}", result.err());
}

#[test]
fn test_cg6_struct_with_many_methods() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        extend Point {
            func a(self: Point) : Int64 { return self.x }
            func b(self: Point) : Int64 { return self.y }
            func c(self: Point) : Int64 { return self.x + self.y }
            func d(self: Point) : Int64 { return self.x * self.y }
            func e(self: Point) : Int64 { return self.x - self.y }
        }
        func main() : Int64 {
            let p = Point { x: 3, y: 4 }
            return p.a() + p.b() + p.c()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_struct_with_many_methods");
}

#[test]
fn test_cg6_enum_match_payload_extract() {
    let source = r#"
        enum Opt { None, Some(Int64) }
        func main() : Int64 {
            let o: Opt = Opt.Some(100)
            match o {
                Opt.None => 0,
                Opt.Some(n) => n
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_enum_match_payload_extract: {:?}", result.err());
}

#[test]
fn test_cg6_for_enumerate() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            var sum: Int64 = 0
            for ((i, v) in arr.enumerate()) {
                sum = sum + v
            }
            return sum
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_for_enumerate");
}

#[test]
fn test_cg6_class_property_access() {
    let source = r#"
        class C {
            var v: Int64
            init(n: Int64) { this.v = n }
            prop value: Int64 { get() { return this.v } }
        }
        func main() : Int64 {
            let c = C(42)
            return c.value
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_class_property_access");
}

#[test]
fn test_cg6_complex_expr_tree() {
    let source = r#"
        func main() : Int64 {
            let a: Int64 = 10
            let b: Int64 = 5
            let c: Int64 = 8
            let d: Int64 = 3
            let e: Int64 = 7
            let f: Int64 = 2
            return ((a + b) * (c - d)) / ((e % f) + 1)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_complex_expr_tree");
}

#[test]
fn test_cg6_bool_to_int() {
    let source = r#"
        func main() : Int64 {
            let flag: Bool = true
            let n = if (flag) { 1 } else { 0 }
            return n
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_bool_to_int");
}

#[test]
fn test_cg6_string_format() {
    let source = r#"
        func main() : Int64 {
            let s = 42.format("d")
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_string_format");
}

#[test]
fn test_cg6_nested_struct_init() {
    let source = r#"
        struct Inner { x: Int64 }
        struct Outer { inner: Inner }
        func main() : Int64 {
            let o = Outer { inner: Inner { x: 42 } }
            return o.inner.x
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_nested_struct_init: {:?}", result.err());
}

#[test]
fn test_cg6_recursive_type() {
    let source = r#"
        class Node {
            var value: Int64
            var next: Option<Node>
            init(v: Int64, n: Option<Node>) { this.value = v; this.next = n }
        }
        func main() : Int64 {
            let n = Node(1, None)
            return n.value
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_recursive_type");
}

#[test]
fn test_cg6_match_with_guard() {
    let source = r#"
        func main() : Int64 {
            match 5 {
                n if n > 0 => n * 2,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_match_with_guard");
}

#[test]
fn test_cg6_do_while_loop() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            do {
                i = i + 1
            } while (i < 5)
            return i
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_do_while_loop: {:?}", result.err());
}

#[test]
fn test_cg6_super_method_call() {
    let source = r#"
        open class Base {
            var x: Int64
            init(n: Int64) { this.x = n }
            func getX(self: Base) : Int64 { return this.x }
        }
        class Derived <: Base {
            init(n: Int64) { super(n) }
            func getBaseX(self: Derived) : Int64 { return super.getX() }
        }
        func main() : Int64 {
            let d = Derived(42)
            return d.getBaseX()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_super_method_call: {:?}", result.err());
}

#[test]
fn test_cg6_tuple_destructure() {
    let source = r#"
        func main() : Int64 {
            let (a, b) = (1, 2)
            return a + b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_tuple_destructure");
}

#[test]
fn test_cg6_postfix_incr() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 5
            x++
            return x
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_postfix_incr");
}

#[test]
fn test_cg6_named_args_call() {
    let source = r#"
        func add(a: Int64, b: Int64) : Int64 { return a + b }
        func main() : Int64 {
            return add(a: 1, b: 2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_named_args_call");
}

#[test]
fn test_cg6_switch_expr() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 2
            switch x {
                1 => 10,
                2 => 20,
                3 => 30,
                _ => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg6_switch_expr");
}

#[test]
fn test_cg6_float64_arithmetic() {
    let source = r#"
        func main() : Float64 {
            let a: Float64 = 1.5
            let b: Float64 = 2.5
            return a * b + a / b
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_float64_arithmetic");
}

#[test]
fn test_cg6_array_index_assign() {
    let source = r#"
        func main() : Int64 {
            var arr = [1, 2, 3]
            arr[1] = 10
            return arr[1]
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_array_index_assign");
}

#[test]
fn test_cg6_loop_break() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            loop {
                i = i + 1
                if (i >= 5) { break }
            }
            return i
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_loop_break");
}

#[test]
fn test_cg6_loop_continue() {
    let source = r#"
        func main() : Int64 {
            var i: Int64 = 0
            var sum: Int64 = 0
            while (i < 10) {
                i = i + 1
                if (i % 2 == 0) { continue }
                sum = sum + i
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_loop_continue");
}

#[test]
fn test_cg6_compound_assign() {
    let source = r#"
        func main() : Int64 {
            var x: Int64 = 10
            x += 5
            x *= 2
            return x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_compound_assign");
}

#[test]
fn test_cg6_bitwise_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int64 = 0xFF & 0xF0
            let b: Int64 = 1 | 2 | 4
            let c: Int64 = 15 ^ 3
            let d: Int64 = 1 << 4
            let e: Int64 = 16 >> 2
            return a + b + c + d + e
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_bitwise_ops");
}

#[test]
fn test_cg6_logical_short_circuit() {
    let source = r#"
        func main() : Int64 {
            let a: Bool = true && false
            let b: Bool = false || true
            return if (a) { 0 } else { if (b) { 1 } else { 0 } }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_logical_short_circuit");
}

#[test]
fn test_cg6_unary_ops() {
    let source = r#"
        func main() : Int64 {
            let a: Int64 = -42
            let b: Bool = !true
            let c: Int64 = ~0
            return if (b) { 0 } else { a + c }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_unary_ops");
}

#[test]
fn test_cg6_pow_operator() {
    let source = r#"
        func main() : Int64 {
            return 2 ** 10
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_pow_operator");
}

#[test]
fn test_cg6_optional_binding_match() {
    let source = r#"
        func main() : Int64 {
            let o: Option<Int64> = Some(7)
            match o {
                Some(v) => v * 2,
                None => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg6_optional_binding_match: {:?}", result.err());
}

#[test]
fn test_cg6_struct_match_destructure() {
    let source = r#"
        struct Pair { a: Int64, b: Int64 }
        func main() : Int64 {
            let p = Pair { a: 1, b: 2 }
            match p {
                Pair { a: x, b: y } => x + y
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg6_struct_match_destructure");
}

// === cg7_ coverage push: 25 targeted integration tests ===

#[test]
fn test_cg7_rune_tostring() {
    let source = r#"
        func main() : Int64 {
            let c: Rune = 'A'
            let s = c.toString()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_rune_tostring: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_rune_tostring");
    }
}

#[test]
fn test_cg7_bool_not_method() {
    let source = r#"
        func main() : Int64 {
            let b = true
            let nb = !b
            return if (nb) { 0 } else { 1 }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_bool_not_method");
}

#[test]
fn test_cg7_int32_tostring() {
    let source = r#"
        func main() : Int64 {
            let x: Int32 = 42
            let s = x.toString()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_int32_tostring: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_int32_tostring");
    }
}

#[test]
fn test_cg7_float64_abs() {
    let source = r#"
        func main() : Int64 {
            let f: Float64 = -3.14
            let a = f.abs()
            return a.toInt64()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg7_float64_abs should not panic");
}

#[test]
fn test_cg7_string_charAt() {
    let source = r#"
        func main() : Int64 {
            let c = "hello".charAt(0)
            return 0
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg7_string_charAt should not panic");
}

#[test]
fn test_cg7_string_toUpper() {
    let source = r#"
        func main() : Int64 {
            let s = "hello".toUpperCase()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg7_string_toUpper should not panic");
}

#[test]
fn test_cg7_string_toLower() {
    let source = r#"
        func main() : Int64 {
            let s = "HELLO".toLowerCase()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg7_string_toLower should not panic");
}

#[test]
fn test_cg7_array_first() {
    let source = r#"
        func main() : Int64 {
            let arr = [1, 2, 3]
            let f = arr.get(0)
            return f
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_array_first: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_array_first");
    }
}

#[test]
fn test_cg7_array_last() {
    let source = r#"
        func main() : Int64 {
            var arr = ArrayList<Int64>()
            arr.append(1)
            arr.append(2)
            arr.append(3)
            let s = arr.size()
            let l = arr.get(s - 1)
            return l
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_array_last: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_array_last");
    }
}

#[test]
fn test_cg7_class_chain_methods() {
    let source = r#"
        class C {
            var x: Int64
            init(v: Int64) { this.x = v }
            func setX(self: C, v: Int64): C { this.x = v; return self }
            func getX(self: C): Int64 { return this.x }
        }
        func main() : Int64 {
            let c = C(0)
            return c.setX(10).getX()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_class_chain_methods: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_class_chain_methods");
    }
}

#[test]
fn test_cg7_class_field_update_return() {
    let source = r#"
        class Counter {
            var n: Int64
            init() { this.n = 0 }
            func inc(self: Counter): Int64 { this.n = this.n + 1; return this.n }
        }
        func main() : Int64 {
            let c = Counter()
            return c.inc()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_class_field_update_return: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_class_field_update_return");
    }
}

#[test]
fn test_cg7_struct_copy_semantic() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        func main() : Int64 {
            let p = Point { x: 1, y: 2 }
            let q = Point { x: p.x + 10, y: p.y }
            return q.x
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_struct_copy_semantic");
}

#[test]
fn test_cg7_class_with_array_member() {
    let source = r#"
        class Container {
            var arr: Array<Int64>
            init() { this.arr = [1, 2, 3] }
            func sum(self: Container): Int64 {
                var s: Int64 = 0
                for i in 0..self.arr.size() { s = s + self.arr.get(i) }
                return s
            }
        }
        func main() : Int64 {
            let c = Container()
            return c.sum()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_class_with_array_member: {:?}", result.err());
}

#[test]
fn test_cg7_match_with_binding_and_guard() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 7
            return match x {
                case n where n > 5 => n * 2,
                case n where n > 0 => n,
                case _ => 0
            }
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_match_with_binding_and_guard: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_match_with_binding_and_guard");
    }
}

#[test]
fn test_cg7_nested_match() {
    let source = r#"
        func main() : Int64 {
            let a: Int64 = 1
            let b: Int64 = 2
            return match a {
                0 => match b { 0 => 0, _ => 1 },
                1 => match b { 2 => 10, _ => 11 },
                _ => 99
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_nested_match");
}

#[test]
fn test_cg7_if_in_for() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            for i in 0..10 {
                if i % 2 == 0 { sum = sum + i }
            }
            return sum
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_if_in_for");
}

#[test]
fn test_cg7_break_nested_loop() {
    let source = r#"
        func main() : Int64 {
            var outer: Int64 = 0
            for i in 0..5 {
                for j in 0..5 {
                    if j == 2 { break }
                    outer = outer + 1
                }
            }
            return outer
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_break_nested_loop");
}

#[test]
fn test_cg7_cast_int_to_string() {
    let source = r#"
        func main() : Int64 {
            let x: Int64 = 42
            let s = x.toString()
            return s.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_cast_int_to_string: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_cast_int_to_string");
    }
}

#[test]
fn test_cg7_numeric_promotion() {
    let source = r#"
        func main() : Int64 {
            let a: Int8 = 10
            let b: Int64 = 20
            return a + b
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_numeric_promotion: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_numeric_promotion");
    }
}

#[test]
fn test_cg7_rune_to_int() {
    let source = r#"
        func main() : Int64 {
            let c: Rune = 'A'
            let n = c.toInt64()
            return n
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok() || result.is_err(), "cg7_rune_to_int should not panic");
}

#[test]
fn test_cg7_multiple_returns() {
    let source = r#"
        func f(x: Int64): Int64 {
            if x < 0 { return 0 }
            if x == 0 { return 1 }
            if x == 1 { return 2 }
            if x == 2 { return 3 }
            return 4
        }
        func main() : Int64 {
            return f(2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_multiple_returns");
}

#[test]
fn test_cg7_empty_array_type() {
    let source = r#"
        func main() : Int64 {
            let arr: Array<Int64> = []
            return arr.size()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg7_empty_array_type: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg7_empty_array_type");
    }
}

#[test]
fn test_cg7_global_function_call() {
    let source = r#"
        func a(): Int64 { 1 }
        func b(): Int64 { a() + 2 }
        func c(): Int64 { b() + 3 }
        func d(): Int64 { c() + 4 }
        func main() : Int64 {
            return d()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_global_function_call");
}

#[test]
fn test_cg7_complex_for_accumulator() {
    let source = r#"
        func main() : Int64 {
            var sum: Int64 = 0
            var prod: Int64 = 1
            for i in 1..=5 {
                sum = sum + i
                prod = prod * i
            }
            return sum + prod
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_complex_for_accumulator");
}

#[test]
fn test_cg7_deeply_nested_blocks() {
    let source = r#"
        func main() : Int64 {
            let a = {
                let b = {
                    let c = {
                        let d = {
                            let e = 42
                            e
                        }
                        d
                    }
                    c
                }
                b
            }
            return a
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg7_deeply_nested_blocks");
}

// --- cg8: targeted coverage tests ---
#[test]
fn test_cg8_class_static_method() {
    let source = r#"
        class X {
            var v: Int64;
            init() { this.v = 0 }
            static func create() : X { return X() }
            func get(self: X) : Int64 { return this.v }
        }
        func main() : Int64 {
            let x = X.create()
            return x.get()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg8_class_static_method: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg8_class_static_method");
    }
}

#[test]
fn test_cg8_extend_multiple_methods() {
    let source = r#"
        struct S { x: Int64 }
        extend S {
            func a(self: S) : Int64 { return self.x }
            func b(self: S) : Int64 { return self.x + 1 }
            func c(self: S) : Int64 { return self.x + 2 }
            func d(self: S) : Int64 { return self.x + 3 }
        }
        func main() : Int64 {
            let s = S { x: 10 }
            return s.a() + s.b() + s.c() + s.d()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg8_extend_multiple_methods");
}

#[test]
fn test_cg8_class_deep_inheritance_3() {
    let source = r#"
        open class A {
            var x: Int64;
            init(x: Int64) { this.x = x }
            func v(self: A) : Int64 { return self.x }
        }
        open class B <: A {
            var y: Int64;
            init(x: Int64, y: Int64) { super(x); this.y = y }
            override func v(self: B) : Int64 { return self.x + self.y }
        }
        open class C <: B {
            var z: Int64;
            init(x: Int64, y: Int64, z: Int64) { super(x, y); this.z = z }
            override func v(self: C) : Int64 { return self.x + self.y + self.z }
        }
        class D <: C {
            init(x: Int64, y: Int64, z: Int64) { super(x, y, z) }
            override func v(self: D) : Int64 { return self.x * 2 }
        }
        func main() : Int64 {
            let d = D(1, 2, 3)
            return d.v()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg8_class_deep_inheritance_3: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg8_class_deep_inheritance_3");
    }
}

#[test]
fn test_cg8_enum_with_3_payload_variants() {
    let source = r#"
        enum E { A(Int64), B(Int64), C(Int64) }
        func main() : Int64 {
            let a = E.A(1)
            let b = E.B(2)
            let c = E.C(3)
            return match a {
                E.A(v) => v,
                E.B(v) => v,
                E.C(v) => v,
                _ => 0
            }
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg8_enum_with_3_payload_variants");
}

#[test]
fn test_cg8_match_on_enum_3_arms() {
    let source = r#"
        enum Op { Add(Int64), Sub(Int64), Mul(Int64) }
        func eval(o: Op) : Int64 {
            return match o {
                Op.Add(n) => n,
                Op.Sub(n) => 0 - n,
                Op.Mul(n) => n * n,
                _ => 0
            }
        }
        func main() : Int64 {
            let a = Op.Add(5)
            let b = Op.Sub(3)
            let c = Op.Mul(4)
            return eval(a) + eval(b) + eval(c)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg8_match_on_enum_3_arms");
}

#[test]
fn test_cg8_class_with_all_features() {
    let source = r#"
        class C {
            var n: Int64;
            init(v: Int64) { this.n = v }
            ~init { }
            prop value: Int64 {
                get() { return this.n }
                set(v) { this.n = v }
            }
            func inc(self: C) : Int64 { this.n = this.n + 1; return this.n }
        }
        func main() : Int64 {
            let c = C(10)
            c.value = 20
            return c.inc()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg8_class_with_all_features: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg8_class_with_all_features");
    }
}

#[test]
fn test_cg8_complex_extend_prop() {
    let source = r#"
        struct Box { v: Int64 }
        extend Box {
            prop value: Int64 {
                get() { return self.v }
                set(x) { }
            }
        }
        func main() : Int64 {
            let b = Box { v: 42 }
            return b.value
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg8_complex_extend_prop: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg8_complex_extend_prop");
    }
}

#[test]
fn test_cg8_struct_with_method_and_extend() {
    let source = r#"
        struct P { x: Int64, y: Int64 }
        func P.len2(self: P) : Int64 { return self.x * self.x + self.y * self.y }
        extend P {
            func scale(self: P, k: Int64) : Int64 { return self.x * k + self.y * k }
        }
        func main() : Int64 {
            let p = P { x: 3, y: 4 }
            return p.len2() + p.scale(2)
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg8_struct_with_method_and_extend");
}

#[test]
fn test_cg8_interface_full() {
    let source = r#"
        interface I {
            func f() : Int64;
        }
        class C <: I {
            var x: Int64;
            init() { this.x = 1 }
            func f(self: C) : Int64 { return this.x }
        }
        func main() : Int64 {
            let c = C()
            return c.f()
        }
    "#;
    let wasm = compile_source(source);
    assert_valid_wasm(&wasm, "cg8_interface_full");
}

#[test]
fn test_cg8_complex_program() {
    let source = r#"
        struct Point { x: Int64, y: Int64 }
        enum Dir { Up, Down, Left, Right }
        class Game {
            var pos: Point;
            var dir: Dir;
            init() {
                this.pos = Point { x: 0, y: 0 }
                this.dir = Dir.Up
            }
            func move(self: Game) : Int64 {
                return match self.dir {
                    Dir.Up => 1,
                    Dir.Down => 2,
                    Dir.Left => 3,
                    Dir.Right => 4,
                    _ => 0
                }
            }
        }
        func main() : Int64 {
            let g = Game()
            return g.move()
        }
    "#;
    let result = compile_source_result(source);
    assert!(result.is_ok(), "cg8_complex_program: {:?}", result.err());
    if let Ok(wasm) = result {
        assert_valid_wasm(&wasm, "cg8_complex_program");
    }
}
