CJWasm2 是面向 WebAssembly 的 Cangjie 语言编译器，支持从基础语法到高级特性的完整语言特性集。本文档通过代码示例系统展示各项特性，帮助开发者快速掌握语言能力。

## 基础特性

### 函数与变量

CJWasm2 支持标准函数定义和变量声明，函数通过 `func` 关键字定义，变量通过 `let` 或 `var` 声明。

```cangjie
// 简单的加法函数
func add(a: Int64, b: Int64): Int64 {
    return a + b
}

// 阶乘函数
func factorial(n: Int64): Int64 {
    if (n <= 1) {
        1
    } else {
        n * factorial(n - 1)
    }
}

main(): Int64 {
    let result = factorial(5)
    @Assert(result, 120)
    return factorial(5)
}
```

[来源: tests/examples/hello.cj](tests/examples/hello.cj#L1-L29)

### 运算符与表达式

支持完整的算术运算符、位运算符、逻辑运算符和复合赋值运算符。

```cangjie
// 位运算
func bitOperations(a: Int64, b: Int64): Int64 {
    let and = a & b      // 按位与
    let or = a | b       // 按位或
    let xor = a ^ b      // 按位异或
    let shl = a << 2     // 左移
    let shr = b >> 1     // 右移
    return and + or + xor + shl + shr
}

// 复合赋值
func compoundAssignment(): Int64 {
    var x: Int64 = 10
    x += 5      // 15
    x -= 3      // 12
    x *= 2      // 24
    x /= 4      // 6
    x %= 4      // 2
    return x
}
```

[来源: tests/examples/operators.cj](tests/examples/operators.cj#L1-L40)

### 数值字面量

支持多种进制的整数、浮点数以及数字分隔符。

```cangjie
func integerLiterals(): Int64 {
    let dec = 42              // 十进制
    let hex = 0xFF            // 十六进制 = 255
    let oct = 0o77            // 八进制 = 63
    let bin = 0b1010          // 二进制 = 10
    let sep = 1_000_000       // 数字分隔符 = 1000000
    return dec + hex + oct + bin
}

func floatLiterals(): Float64 {
    let f64 = 3.14159         // Float64 (默认)
    let sci = 1.5e10          // 科学计数法
    return f64
}
```

[来源: tests/examples/literals.cj](tests/examples/literals.cj#L1-L20)

## 控制流

### 条件语句与循环

支持 `if-else` 条件语句、`for` 循环（含步长）和 `while` 循环。

```cangjie
// for-in 循环
func sumRange(): Int64 {
    var total: Int64 = 0
    for (i in 0..10) {
        total = total + i
    }
    return total  // 0+1+2+...+9 = 45
}

// for-in 带步长 (P2.6)
func testForStep(): Int64 {
    var sum: Int64 = 0
    for (i in 0..=10 : 2) {    // 步长为2: 0,2,4,6,8,10
        sum = sum + i
    }
    return sum  // 30
}

// while 循环
func findFirst(): Int64 {
    let arr = [5, 12, 8, 20, 3, 15]
    var result: Int64 = 0
    var i: Int64 = 0
    while (i < 6) {
        if (arr[i] > 10) {
            result = arr[i]
            break
        }
        i = i + 1
    }
    return result  // 12
}
```

[来源: tests/examples/control_flow.cj](tests/examples/control_flow.cj#L1-L66)
[来源: tests/examples/p2_features.cj](tests/examples/p2_features.cj#L44-L58)
[来源: tests/examples/loop_control.cj](tests/examples/loop_control.cj#L1-L123)

### Match 表达式

强大的模式匹配能力，支持字面量匹配、范围匹配、枚举匹配和守卫条件。

```cangjie
// 多模式匹配
func matchMultiple(n: Int64): Int64 {
    match (n) {
        case 1 | 2 | 3 => 10
        case 4 | 5 => 20
        case _ => 0
    }
}

// guard 条件 (cjc: case x where condition =>)
func classifyNumber(n: Int64): Int64 {
    match (n) {
        case x where x < 0 => 1
        case 0 => 2
        case x where x < 10 => 3
        case x where x < 100 => 4
        case _ => 5
    }
}
```

[来源: tests/examples/control_flow.cj](tests/examples/control_flow.cj#L25-L50)
[来源: tests/examples/for_in_and_guards.cj](tests/examples/for_in_and_guards.cj#L1-L38)

## 数据结构

### 结构体与类

支持 `struct` 值类型和 `class` 引用类型，均可定义方法和构造函数。

```cangjie
// 结构体
struct Point {
    var x: Int64;
    var y: Int64;

    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }

    func distance(): Int64 {
        return this.x * this.x + this.y * this.y
    }
}

// 类 (无继承，展平为结构体编译)
class Counter {
    var count: Int64;

    init(start: Int64) {
        this.count = start
    }

    static func create(): Counter {  // 静态方法 (P2.4)
        return Counter(0)
    }

    func inc(): Counter {
        this.count = this.count + 1
        return this
    }
}
```

[来源: tests/examples/methods.cj](tests/examples/methods.cj#L1-L73)
[来源: tests/examples/p2_features.cj](tests/examples/p2_features.cj#L9-L32)
[来源: tests/examples/inheritance.cj](tests/examples/inheritance.cj#L1-L158)

### 继承与多态

支持 `open class` 开放类继承、`override` 方法重写和 `super` 调用父类方法。

```cangjie
open class Animal {
    var kind: Int64;
    var age: Int64;

    init(kind: Int64, age: Int64) {
        this.kind = kind
        this.age = age
    }

    func speak(): Int64 {
        return 0
    }
}

class Dog <: Animal {
    var breed: Int64;

    init(kind: Int64, age: Int64, breed: Int64) {
        super(kind, age)
        this.breed = breed
    }

    override func speak(): Int64 {
        return 1  // "Woof"
    }
}
```

[来源: tests/examples/inheritance.cj](tests/examples/inheritance.cj#L23-L56)

### 枚举

支持简单枚举和带关联值的枚举类型。

```cangjie
// 简单枚举
enum Color {
    | Red
    | Green
    | Blue
}

// 带关联值的枚举
enum Message {
    | Quit
    | Move(Int64)
    | Write(String)
}

// 使用 match 处理枚举
func handleMessage(m: Message): Int64 {
    match (m) {
        case Message.Quit => 0
        case Message.Move(distance) => distance
        case Message.Write(s) => 100
    }
}
```

[来源: tests/examples/enum.cj](tests/examples/enum.cj#L1-L42)

### 接口

支持接口定义、默认实现和结构体/类实现接口。

```cangjie
// 接口定义
interface Drawable {
    func draw(): Int64;
    func area(): Int64;
}

// 结构体实现接口
struct Rect {
    var width: Int64;
    var height: Int64;

    init(width: Int64, height: Int64) {
        this.width = width
        this.height = height
    }

    func draw(): Int64 {
        return 0
    }

    func area(): Int64 {
        return this.width * this.height
    }
}

// extend 扩展 (P3)
extend Vec2 {
    func length(): Int64 {
        return this.x * this.x + this.y * this.y
    }
}
```

[来源: tests/examples/interface.cj](tests/examples/interface.cj#L1-L30)
[来源: tests/examples/p3_collections.cj](tests/examples/p3_collections.cj#L38-L54)

## 泛型系统

### 泛型函数与结构体

支持类型参数 `<T>` 的泛型函数和结构体。

```cangjie
struct Pair<T, U> {
    var first: T;
    var second: U;
    init(first: T, second: U) {
        this.first = first
        this.second = second
    }
}

func identity<T>(value: T): T {
    return value
}

main(): Int64 {
    let p = Pair<Int64, Int64>(1, 2)
    let x = identity<Int64>(42)
    return p.first + x  // 43
}
```

[来源: tests/examples/generic.cj](tests/examples/generic.cj#L1-L23)

### 类型约束

支持 `where` 子句和多重约束 (`&`)。

```cangjie
// 单类型约束
func maximum<T>(a: T, b: T): T where T <: Comparable<T> {
    return a
}

// 多重约束
func process<T>(value: T): T where T <: Comparable<T> & Hashable {
    return value
}

// 泛型结构体约束
struct SortedPair<T> where T <: Comparable<T> {
    var first: T;
    var second: T;
}
```

[来源: tests/examples/generic_advanced.cj](tests/examples/generic_advanced.cj#L1-L200)

### 泛型枚举

支持泛型枚举类型如 `Option<T>`、`Result<T, E>`。

```cangjie
enum MyOption<T> {
    | MySome(T)
    | MyNone
}

enum Tree<T> {
    | Leaf(T)
    | Empty
}
```

[来源: tests/examples/generic_advanced.cj](tests/examples/generic_advanced.cj#L72-L79)

## 集合类型

### Array 与 ArrayList

```cangjie
// 静态数组
let arr = [10, 20, 30, 40, 50]

// 动态数组构造 (P2.7)
let arr1 = Array<Int64>(5, 7)           // 5个元素，全为7
let arr2 = Array<Int64>(5, { i => i * i })  // [0,1,4,9,16]

// ArrayList
let list = ArrayList<Int64>()
list.append(10)
list.append(20)
let v = list.get(0)  // 10
```

[来源: tests/examples/p2_features.cj](tests/examples/p2_features.cj#L60-L78)
[来源: tests/examples/p3_collections.cj](tests/examples/p3_collections.cj#L1-L30)

### HashMap 与 HashSet

```cangjie
// HashMap
let map = HashMap<Int64, Int64>()
map.put(1, 100)
map.put(2, 200)
let val = map.get(1)  // 100
if (map.containsKey(2) != 0) {
    result = result + 1
}

// HashSet (P4)
let set = HashSet<Int64>()
set.add(1)
set.add(2)
if (set.contains(1) != 0) {
    result = result + 10
}
```

[来源: tests/examples/p3_collections.cj](tests/examples/p3_collections.cj#L33-L56)
[来源: tests/examples/p4_collections.cj](tests/examples/p4_collections.cj#L48-L70)

## 错误处理

### Option 与 Result

```cangjie
// Option 类型
func divide(a: Int64, b: Int64): Option<Int64> {
    if (b == 0) {
        return None
    }
    return Some(a / b)
}

// Result 类型
func compute(x: Int64, y: Int64): Result<Int64, String> {
    match (divide(x, y)) {
        case Ok(result) => Ok(result * 2)
        case Err(e) => Err(e)
    }
}
```

[来源: tests/examples/error_handling.cj](tests/examples/error_handling.cj#L1-L30)

### try-catch-finally

```cangjie
// try-catch-finally (P6)
func safeCompute(a: Int64, b: Int64): Int64 {
    var result: Int64 = 0
    var log: Int64 = 0

    try {
        if (b == 0) {
            throw 0
        }
        result = a / b
    } catch (e: Exception) {
        result = -1
    } finally {
        log = 1  // 无论成功或失败都执行
    }

    return result + log
}
```

[来源: tests/examples/error_handling.cj](tests/examples/error_handling.cj#L40-L60)
[来源: tests/examples/phase6_error_module.cj](tests/examples/phase6_error_module.cj#L39-L60)

## 模式匹配高级特性

### 解构绑定

```cangjie
// if-let 模式
func processOption(opt: Option<Int64>): Int64 {
    if (let Some(value) <- opt) {
        return value * 2
    }
    return 0
}

// 结构体解构
func deconstructPoint(): Int64 {
    let p = Point { x: 10, y: 20 }
    let Point { x, y } = p
    return x + y  // 30
}

// match 中的解构
func matchPoint(p: Point): Int64 {
    match (p) {
        case Point { x: 0, y: 0 } => 0
        case Point { x: 0, y } => y
        case Point { x, y: 0 } => x
        case Point { x, y } => x + y
    }
}
```

[来源: tests/examples/patterns.cj](tests/examples/patterns.cj#L1-L60)

### while-let 循环

```cangjie
func countDown(start: Int64): Int64 {
    var current = Some(start)
    var sum: Int64 = 0

    while (let Some(n) <- current) {
        sum = sum + n
        if (n > 0) {
            current = Some(n - 1)
        } else {
            current = None
        }
    }
    return sum  // 5+4+3+2+1+0 = 15
}
```

[来源: tests/examples/patterns.cj](tests/examples/patterns.cj#L27-L42)

## 字符串特性

### 字符串插值与操作

```cangjie
// 字符串插值
func greet(name: String): String {
    return "Hello, ${name}!"
}

// 表达式插值
func formatResult(a: Int64, b: Int64): String {
    return "The sum of ${a} and ${b} is ${a + b}"
}

// 多行字符串
func multilineString(): String {
    return """
    This is a
    multi-line
    string literal
    """
}

// 字符串方法 (P2.10)
func testStringMethods(): Int64 {
    var result: Int64 = 0
    let greeting = "hello world"
    
    if (greeting.startsWith("hello")) { result = result + 2 }
    if (greeting.endsWith("world")) { result = result + 4 }
    if (greeting.contains("lo wo")) { result = result + 16 }
    if (greeting.isBlank() == false) { result = result + 128 }
    
    return result
}
```

[来源: tests/examples/strings.cj](tests/examples/strings.cj#L1-L56)
[来源: tests/examples/p2_features.cj](tests/examples/p2_features.cj#L150-L200)

### 类型方法 (#52-#54)

```cangjie
// String 方法
let numStr = "12345"
let val = numStr.toInt64()  // 12345

// Int64 方法
let x: Int64 = -42
x.abs()        // 42
x.toString()   // "-42"

// Float64 方法
let f: Float64 = 3.14
f.toString()   // "3.14"
f.toInt64()    // 3
```

[来源: tests/examples/type_methods.cj](tests/examples/type_methods.cj#L1-L78)

## 闭包与高阶函数

### 尾随闭包

```cangjie
func map(arr: Array<Int64>, f: (Int64) -> Int64): Array<Int64> {
    let result = Array<Int64>()
    for (i in 0..arr.size()) {
        result.append(f(arr[i]))
    }
    return result
}

// 尾随闭包语法
let arr = [1, 2, 3, 4, 5]
let doubled = map(arr) { x => x * 2 }
```

[来源: tests/fixtures/trailing_closure_test.cj](tests/fixtures/trailing_closure_test.cj#L1-L49)

## 并发特性 (P5)

```cangjie
// spawn: 单线程桩
spawn {
    result = result + 10
}

// synchronized 同步块
let lock = Mutex()
synchronized(lock) {
    result = result + 100
}

// AtomicInt64 原子操作
let counter = AtomicInt64(0)
counter.store(42)
let old = counter.fetchAdd(8)  // 返回42
counter.compareAndSwap(50, 100)  // 成功返回1

// AtomicBool
let flag = AtomicBool()  // 默认 false
flag.store(1)
flag.compareAndSwap(1, 0)  // 成功返回1
```

[来源: tests/examples/p5_concurrent.cj](tests/examples/p5_concurrent.cj#L1-L147)

## 其他特性

### 类型别名

```cangjie
// 简单类型别名
type MyInt = Int64
type MyString = String

// 函数类型别名
type Transformer = (Int64) -> Int64
type BinaryOp = (Int64, Int64) -> Int64

main(): Int64 {
    let f: Transformer = double
    let result = f(10)  // 20
    return result
}
```

[来源: tests/fixtures/type_alias_test.cj](tests/fixtures/type_alias_test.cj#L1-L33)

### 可选链

```cangjie
struct Point {
    var x: Int64
    var y: Int64
}

main(): Int64 {
    let p1: ?Point = Some(Point(10, 20))
    let p2: ?Point = None

    let x1 = p1?.x   // 返回 10
    let x2 = p2?.x   // None 时返回 0
    return x1 + x2   // 15
}
```

[来源: tests/fixtures/optional_chain_test.cj](tests/fixtures/optional_chain_test.cj#L1-L30)

### 命名参数

```cangjie
func createValue(m: Int64, n: Int64, 
                 offset!: Int64 = 0, 
                 scale!: Int64 = 1): Int64 {
    return (m + n + offset) * scale
}

main(): Int64 {
    let r1 = createValue(3, 4)                        // (3+4+0)*1 = 7
    let r2 = createValue(3, 4, offset!: 10)          // (3+4+10)*1 = 17
    let r3 = createValue(3, 4, offset!: 10, scale!: 2) // (3+4+10)*2 = 34
    return r1 + r2 + r3  // 58
}
```

[来源: tests/examples/p2_features.cj](tests/examples/p2_features.cj#L110-L130)

## 模块系统

### 多文件编译

```cangjie
// module_lib.cj
package examples.lib

func add(a: Int64, b: Int64): Int64 {
    return a + b
}

func multiply(a: Int64, b: Int64): Int64 {
    return a * b
}

// module_main.cj
package examples.main
import examples.lib

func main(): Int64 {
    let a = add(10, 20)       // 30
    let b = multiply(3, 4)    // 12
    return a + b              // 42
}
```

编译命令：`cjwasm module_main.cj module_lib.cj -o module_test.wasm`

[来源: tests/examples/multifile/module_lib.cj](tests/examples/multifile/module_lib.cj#L1-L15)
[来源: tests/examples/multifile/module_main.cj](tests/examples/multifile/module_main.cj#L1-L13)

## 语言特性对照表

| 特性分类 | 具体特性 | 示例语法 | 文档位置 |
|---------|---------|---------|---------|
| 基础语法 | 函数定义 | `func add(a: Int64, b: Int64): Int64` | [hello.cj](tests/examples/hello.cj) |
| | 变量声明 | `let x = 10` / `var x: Int64 = 10` | [hello.cj](tests/examples/hello.cj) |
| | 运算符 | `+`, `-`, `*`, `/`, `%`, `&`, `|`, `^` | [operators.cj](tests/examples/operators.cj) |
| | 位移运算 | `<<`, `>>` | [operators.cj](tests/examples/operators.cj) |
| | 复合赋值 | `x += 5`, `x *= 2` | [operators.cj](tests/examples/operators.cj) |
| 控制流 | if-else | `if (cond) { } else { }` | [hello.cj](tests/examples/hello.cj) |
| | for-in | `for (i in 0..10)` | [control_flow.cj](tests/examples/control_flow.cj) |
| | for-in 步长 | `for (i in 0..=10 : 2)` | [p2_features.cj](tests/examples/p2_features.cj) |
| | while | `while (cond) { }` | [loop_control.cj](tests/examples/loop_control.cj) |
| | match | `match (x) { case 1 => ... }` | [control_flow.cj](tests/examples/control_flow.cj) |
| | match guard | `case x where x > 0 =>` | [patterns.cj](tests/examples/patterns.cj) |
| 数据结构 | struct | `struct Point { var x: Int64; }` | [methods.cj](tests/examples/methods.cj) |
| | class | `class Counter { var count: Int64; }` | [p2_features.cj](tests/examples/p2_features.cj) |
| | 继承 | `class Dog <: Animal` | [inheritance.cj](tests/examples/inheritance.cj) |
| | 枚举 | `enum Color { | Red | Green }` | [enum.cj](tests/examples/enum.cj) |
| | 接口 | `interface Drawable { func draw(); }` | [interface.cj](tests/examples/interface.cj) |
| | extend | `extend Vec2 { func length(); }` | [p3_collections.cj](tests/examples/p3_collections.cj) |
| 泛型 | 泛型函数 | `func identity<T>(v: T): T` | [generic.cj](tests/examples/generic.cj) |
| | 泛型结构体 | `struct Pair<T, U> { }` | [generic.cj](tests/examples/generic.cj) |
| | 类型约束 | `where T <: Comparable<T>` | [generic_advanced.cj](tests/examples/generic_advanced.cj) |
| 集合 | Array | `[1, 2, 3]`, `Array<Int64>(5, 7)` | [p2_features.cj](tests/examples/p2_features.cj) |
| | ArrayList | `ArrayList<Int64>()` | [p3_collections.cj](tests/examples/p3_collections.cj) |
| | HashMap | `HashMap<Int64, Int64>()` | [p3_collections.cj](tests/examples/p3_collections.cj) |
| | HashSet | `HashSet<Int64>()` | [p4_collections.cj](tests/examples/p4_collections.cj) |
| 错误处理 | Option | `Some(x)`, `None` | [enum.cj](tests/examples/enum.cj) |
| | Result | `Ok(x)`, `Err(e)` | [error_handling.cj](tests/examples/error_handling.cj) |
| | try-catch | `try { } catch (e: Exception) { }` | [error_handling.cj](tests/examples/error_handling.cj) |
| | throw | `throw 0` | [error_handling.cj](tests/examples/error_handling.cj) |
| 字符串 | 插值 | `"Hello, ${name}!"` | [strings.cj](tests/examples/strings.cj) |
| | 多行字符串 | `"""..."""` | [strings.cj](tests/examples/strings.cj) |
| | 字符串方法 | `s.trim()`, `s.startsWith()` | [p2_features.cj](tests/examples/p2_features.cj) |
| 模式匹配 | if-let | `if (let Some(v) <- opt)` | [patterns.cj](tests/examples/patterns.cj) |
| | while-let | `while (let Some(n) <- opt)` | [patterns.cj](tests/examples/patterns.cj) |
| | 解构 | `let Point { x, y } = p` | [patterns.cj](tests/examples/patterns.cj) |
| 并发 | spawn | `spawn { ... }` | [p5_concurrent.cj](tests/examples/p5_concurrent.cj) |
| | Mutex | `Mutex()`, `synchronized(lock)` | [p5_concurrent.cj](tests/examples/p5_concurrent.cj) |
| | AtomicInt64 | `AtomicInt64(0)` | [p5_concurrent.cj](tests/examples/p5_concurrent.cj) |
| 其他 | 类型别名 | `type MyInt = Int64` | [type_alias_test.cj](tests/fixtures/type_alias_test.cj) |
| | 可选链 | `p1?.x` | [optional_chain_test.cj](tests/fixtures/optional_chain_test.cj) |
| | 尾随闭包 | `map(arr) { x => x * 2 }` | [trailing_closure_test.cj](tests/fixtures/trailing_closure_test.cj) |
| | 模块导入 | `import examples.lib` | [multifile](tests/examples/multifile) |

## 下一步

- 深入了解编译流程：参考 [编译流水线](8-bian-yi-liu-shui-xian)
- 学习代码生成：参考 [WebAssembly 代码生成器](12-webassembly-dai-ma-sheng-cheng-qi)
- 查看测试用例：参考 [测试框架](17-ce-shi-kuang-jia)