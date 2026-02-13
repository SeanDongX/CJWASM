# CJWasm 编译器规格说明书

仓颉语言到 WebAssembly 编译器的完整功能规格。

## 目录

1. [类型系统](#1-类型系统)
2. [字面量](#2-字面量)
3. [表达式](#3-表达式)
4. [语句](#4-语句)
5. [函数](#5-函数)
6. [结构体与类](#6-结构体与类)
7. [枚举与模式匹配](#7-枚举与模式匹配)
8. [泛型](#8-泛型)
9. [接口/Trait](#9-接口trait)
10. [模块系统](#10-模块系统)
11. [错误处理](#11-错误处理)
12. [内存管理](#12-内存管理)
13. [WASM 互操作](#13-wasm-互操作)
14. [标准库](#14-标准库)
15. [实现状态](#15-实现状态)

---

## 1. 类型系统

### 1.1 基础类型

| 类型 | 描述 | WASM 映射 | 状态 |
|------|------|-----------|------|
| `Int8` | 8位有符号整数 | i32 | [x] |
| `Int16` | 16位有符号整数 | i32 | [x] |
| `Int32` | 32位有符号整数 | i32 | [x] |
| `Int64` | 64位有符号整数 | i64 | [x] |
| `UInt8` | 8位无符号整数 | i32 | [x] |
| `UInt16` | 16位无符号整数 | i32 | [x] |
| `UInt32` | 32位无符号整数 | i32 | [x] |
| `UInt64` | 64位无符号整数 | i64 | [x] |
| `Float32` | 32位浮点数 | f32 | [x] |
| `Float64` | 64位浮点数 | f64 | [x] |
| `Bool` | 布尔值 | i32 (0/1) | [x] |
| `Char` | Unicode 字符 | i32 | [x] |
| `Unit` | 空类型 | (无返回值) | [x] |

### 1.2 复合类型

| 类型 | 语法 | 描述 | 状态 |
|------|------|------|------|
| `String` | `String` | UTF-8 字符串 | [x] |
| `Array<T>` | `Array<Int64>` | 固定长度数组 | [x] |
| `Slice<T>` | `Slice<Int64>` | 动态切片 | [x] |
| `Map<K, V>` | `Map<String, Int64>` | 键值映射 | [x] |
| `Tuple` | `(Int64, String)` | 元组类型 | [x] |
| `Option<T>` | `Option<Int64>` | 可选值 | [x] |
| `Result<T, E>` | `Result<Int64, Error>` | 结果类型 | [x] |
| `Struct` | `struct Point {...}` | 结构体 | [x] |
| `Enum` | `enum Color {...}` | 枚举类型（含关联值如 Ok(T)/Err(E)） | [x] |
| `Class` | `class Person {...}` | 类 | [x] |
| `Interface` | `interface Drawable {...}` | 接口 | [x] |
| `Function` | `(Int64) -> Int64` | 函数类型 | [x] |

### 1.3 类型修饰符

| 修饰符 | 描述 | 状态 |
|--------|------|------|
| `mut` | 可变引用 | [x] |
| `ref` | 引用类型 | [x] |
| `?` | 可空类型（`T?` → `Option<T>` 语法糖） | [x] |
| `!` | 非空断言 | [x] |

---

## 2. 字面量

### 2.1 数值字面量

```cangjie
// 整数
let a = 42           // Int64 (默认)
let b: Int32 = 42    // Int32
let c = 0xFF         // 十六进制
let d = 0o77         // 八进制
let e = 0b1010       // 二进制
let f = 1_000_000    // 数字分隔符

// 浮点数
let g = 3.14         // Float64 (默认)
let h = 3.14f        // Float32
let i = 1.0e10       // 科学计数法
```

| 功能 | 状态 |
|------|------|
| 十进制整数 | [x] |
| 十六进制 | [x] |
| 八进制 | [x] |
| 二进制 | [x] |
| 数字分隔符 | [x] |
| 浮点数 | [x] |
| 科学计数法 | [x] |
| 类型后缀 | [x] (Float32 后缀 `f`) |

### 2.2 字符串字面量

```cangjie
let s1 = "hello"              // 普通字符串
let s2 = "line1\nline2"       // 转义字符
let s3 = """                  // 多行字符串
    multi
    line
    """
let s4 = "Hello, ${name}!"    // 字符串插值
let s5 = r"raw\nstring"       // 原始字符串
```

| 功能 | 状态 |
|------|------|
| 基本字符串 | [x] |
| 转义字符 (\n \t \" \\) | [x] |
| 多行字符串 | [x] |
| 字符串插值 | [x] |
| 原始字符串 | [x] |

### 2.3 其他字面量

```cangjie
let arr = [1, 2, 3]           // 数组
let tuple = (1, "hello")      // 元组
let map = {"a": 1, "b": 2}    // Map (语法糖)
```

| 功能 | 状态 |
|------|------|
| 数组字面量 | [x] |
| 元组字面量 | [x] |
| Map 字面量 | [x] |

---

## 3. 表达式

### 3.1 算术运算

| 运算符 | 描述 | 状态 |
|--------|------|------|
| `+` | 加法 | [x] |
| `-` | 减法 | [x] |
| `*` | 乘法 | [x] |
| `/` | 除法 | [x] |
| `%` | 取模 | [x] |
| `**` | 幂运算 | [x] |
| `-x` | 负号 | [x] |

### 3.2 比较运算

| 运算符 | 描述 | 状态 |
|--------|------|------|
| `==` | 等于 | [x] |
| `!=` | 不等于 | [x] |
| `<` | 小于 | [x] |
| `>` | 大于 | [x] |
| `<=` | 小于等于 | [x] |
| `>=` | 大于等于 | [x] |

### 3.3 逻辑运算

| 运算符 | 描述 | 状态 |
|--------|------|------|
| `&&` | 逻辑与 | [x] |
| `\|\|` | 逻辑或 | [x] |
| `!` | 逻辑非 | [x] |

### 3.4 位运算

| 运算符 | 描述 | 状态 |
|--------|------|------|
| `&` | 按位与 | [x] |
| `\|` | 按位或 | [x] |
| `^` | 按位异或 | [x] |
| `~` | 按位取反 | [x] |
| `<<` | 左移 | [x] |
| `>>` | 右移 | [x] |
| `>>>` | 无符号右移 | [x] |

### 3.5 赋值运算

| 运算符 | 描述 | 状态 |
|--------|------|------|
| `=` | 赋值 | [x] |
| `+=` | 加法赋值 | [x] |
| `-=` | 减法赋值 | [x] |
| `*=` | 乘法赋值 | [x] |
| `/=` | 除法赋值 | [x] |
| `%=` | 取模赋值 | [x] |

### 3.6 其他表达式

```cangjie
// 条件表达式
let x = if a > b { a } else { b }

// 块表达式
let y = {
    let temp = compute()
    temp * 2
}

// 范围表达式
let range = 0..10      // [0, 10)
let range2 = 0..=10    // [0, 10]

// 类型转换
let n = value as Int32

// 空值合并
let v = maybeNull ?? defaultValue
```

| 功能 | 状态 |
|------|------|
| if 表达式 | [x] |
| 块表达式 | [x] |
| 方法调用 (obj.method()) | [x] |
| 枚举变体构造 (Type.Variant) | [x] |
| 范围表达式（作为值） | [x] |
| 类型转换 (as) | [x] |
| 空值合并 (??) | [x] |
| 三元运算符 | [ ] |

---

## 4. 语句

### 4.1 变量声明

```cangjie
let x = 10              // 不可变绑定
var y = 20              // 可变绑定
let z: Int64 = 30       // 显式类型注解
```

| 功能 | 状态 |
|------|------|
| let 声明 | [x] |
| var 声明 | [x] |
| 类型注解 | [x] |
| 解构绑定 | [x] |

### 4.2 控制流

```cangjie
// if-else
if condition {
    // ...
} else if other {
    // ...
} else {
    // ...
}

// while 循环
while condition {
    // ...
}

// for 循环
for i in 0..10 {
    // ...
}

for item in collection {
    // ...
}

// loop 无限循环
loop {
    if done { break }
}

// match 模式匹配
match value {
    0 => "zero"
    1..10 => "small"
    _ => "large"
}
```

| 功能 | 状态 |
|------|------|
| if-else | [x] |
| while | [x] |
| for-in（范围 0..n / 0..=n） | [x] |
| for-in（数组迭代） | [x] |
| loop | [x] |
| match | [x] |
| break | [x] |
| continue | [x] |
| return | [x] |

---

## 5. 函数

### 5.1 函数定义

```cangjie
// 基本函数
func add(a: Int64, b: Int64) -> Int64 {
    return a + b
}

// 无返回值
func greet(name: String) {
    print("Hello, ${name}!")
}

// 默认参数
func power(base: Int64, exp: Int64 = 2) -> Int64 {
    // ...
}

// 可变参数
func sum(numbers: Int64...) -> Int64 {
    // ...
}

// 泛型函数
func identity<T>(value: T) -> T {
    return value
}

// Lambda 表达式
let double = (x: Int64) -> Int64 { x * 2 }
let triple = { x: Int64 => x * 3 }
```

| 功能 | 状态 |
|------|------|
| 基本函数 | [x] |
| 参数 | [x] |
| 返回类型 | [x] |
| 默认参数 | [x] |
| 可变参数 | [x] |
| 泛型函数 | [x] |
| Lambda | [x] |
| 闭包 | [x] |
| 递归 | [x] |
| 尾递归优化 | [x] |

### 5.2 函数重载

```cangjie
func process(x: Int64) -> Int64 { ... }
func process(x: String) -> String { ... }
```

| 功能 | 状态 |
|------|------|
| 函数重载 | [x] |

---

## 6. 结构体与类

### 6.1 结构体

```cangjie
struct Point {
    x: Int64
    y: Int64
}

// 带方法的结构体
struct Rectangle {
    width: Int64
    height: Int64

    func area() -> Int64 {
        return this.width * this.height
    }

    static func square(size: Int64) -> Rectangle {
        return Rectangle { width: size, height: size }
    }
}

// 结构体初始化
let p = Point { x: 10, y: 20 }
let r = Rectangle { width: 5, height: 10 }
```

| 功能 | 状态 |
|------|------|
| 结构体定义 | [x] |
| 字段访问 | [x] |
| 结构体初始化 | [x] |
| 实例方法（func Type.method + obj.method()） | [x] |
| 静态方法 | [x] |
| 构造函数 | [x] |
| this 关键字 | [x] |

### 6.2 类

```cangjie
class Person {
    private var name: String
    private var age: Int64

    // 构造函数
    init(name: String, age: Int64) {
        this.name = name
        this.age = age
    }

    // 析构函数
    deinit {
        // 清理资源
    }

    // 实例方法
    func greet() -> String {
        return "Hello, I'm ${this.name}"
    }

    // Getter/Setter
    var displayName: String {
        get { return this.name }
        set { this.name = value }
    }
}

// 继承
class Student extends Person {
    private var grade: Int64

    override func greet() -> String {
        return "${super.greet()}, I'm a student"
    }
}
```

| 功能 | 状态 |
|------|------|
| 类定义 | [x] |
| 构造函数 (init) | [x] |
| 析构函数 (deinit) | [x] |
| 实例方法 | [x] |
| 静态方法 | [x] |
| Getter/Setter | [x] |
| 继承 (extends) | [x] |
| 方法重写 (override) | [x] |
| super 调用 | [x] |
| 访问修饰符 | [x] |
| abstract 类 | [x] |
| sealed 类 | [x] |

---

## 7. 枚举与模式匹配

### 7.1 枚举定义

```cangjie
// 简单枚举
enum Color {
    Red
    Green
    Blue
}

// 带关联值的枚举
enum Result<T, E> {
    Ok(T)
    Err(E)
}

enum Option<T> {
    Some(T)
    None
}

// 带方法的枚举
enum Direction {
    North
    South
    East
    West

    func opposite() -> Direction {
        match this {
            North => South
            South => North
            East => West
            West => East
        }
    }
}
```

| 功能 | 状态 |
|------|------|
| 简单枚举 | [x] |
| 关联值枚举 | [x] |
| 泛型枚举 | [x] |
| 枚举方法 | [x] |

### 7.2 模式匹配

```cangjie
match value {
    // 字面量模式
    0 => "zero"
    1 | 2 | 3 => "small"

    // 范围模式
    4..10 => "medium"

    // 解构模式
    Point { x, y } => "point at (${x}, ${y})"

    // 守卫条件
    n if n > 100 => "large"

    // 通配符
    _ => "other"
}

// if-let 模式
if let Some(value) = maybeValue {
    // 使用 value
}

// while-let 模式
while let Some(item) = iterator.next() {
    // 处理 item
}
```

| 功能 | 状态 |
|------|------|
| 字面量匹配 | [x] |
| 多模式 (\|) | [x] |
| 范围匹配 | [x] |
| 枚举变体匹配 (Type.Variant) | [x] |
| 解构匹配 | [x] |
| 守卫条件 (if) | [x] |
| 通配符 (_) | [x] |
| if-let | [x] |
| while-let | [x] |

---

## 8. 泛型

```cangjie
// 泛型函数
func swap<T>(a: mut T, b: mut T) {
    let temp = a
    a = b
    b = temp
}

// 泛型结构体
struct Pair<T, U> {
    first: T
    second: U
}

// 泛型约束
func print<T: ToString>(value: T) {
    println(value.toString())
}

// 多重约束
func compare<T: Comparable & Hashable>(a: T, b: T) -> Int64 {
    // ...
}

// where 子句
func process<T, U>(a: T, b: U) -> Bool
    where T: Iterable, U: Collection {
    // ...
}
```

| 功能 | 状态 |
|------|------|
| 泛型函数 | [x] |
| 泛型结构体 | [x] |
| 泛型类 | [x] |
| 泛型枚举 | [x] |
| 类型约束 | [x] |
| 多重约束 | [x] |
| where 子句 | [x] |
| 泛型特化 | [x] |

---

## 9. 接口/Trait

```cangjie
// 接口定义
interface Drawable {
    func draw()
    func boundingBox() -> Rectangle
}

// 带默认实现
interface Printable {
    func toString() -> String

    func print() {
        println(this.toString())
    }
}

// 实现接口
struct Circle implements Drawable {
    radius: Float64

    func draw() {
        // ...
    }

    func boundingBox() -> Rectangle {
        // ...
    }
}

// 扩展实现
extend Int64: Printable {
    func toString() -> String {
        // ...
    }
}
```

| 功能 | 状态 |
|------|------|
| 接口定义 | [x] |
| 默认实现 | [x] |
| implements | [x] |
| 扩展 (extend) | [x] |
| 接口继承 | [x] |
| 关联类型 | [x] |

---

## 10. 模块系统

```cangjie
// 模块声明 (文件顶部)
module math

// 导入
import std.io
import std.collections.{HashMap, HashSet}
import math.geometry as geo
import math.* // 通配符导入

// 可见性
public func publicFunc() { }
internal func internalFunc() { }
private func privateFunc() { }

// 包结构
// src/
//   main.cj
//   math/
//     mod.cj
//     geometry.cj
//     algebra.cj
```

| 功能 | 状态 |
|------|------|
| module 声明 | [x] |
| import | [x] |
| 别名 (as) | [x] |
| 通配符导入 | [x] |
| public | [x] |
| internal | [x] |
| private | [x] |
| 包管理 | [ ] |

---

## 11. 错误处理

```cangjie
// 定义错误类型
class FileError extends Error {
    var path: String

    init(path: String, message: String) {
        super(message)
        this.path = path
    }
}

// 抛出异常
func readFile(path: String) -> String throws FileError {
    if !exists(path) {
        throw FileError(path, "File not found")
    }
    // ...
}

// 捕获异常
try {
    let content = readFile("config.json")
} catch e: FileError {
    println("File error: ${e.message}")
} catch e: Error {
    println("Unknown error: ${e.message}")
} finally {
    // 清理代码
}

// Result 类型 (函数式错误处理)
func divide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 {
        return Err("Division by zero")
    }
    return Ok(a / b)
}

// 错误传播
func process() -> Result<Int64, String> {
    let x = divide(10, 2)?  // 自动传播错误
    return Ok(x * 2)
}
```

| 功能 | 状态 |
|------|------|
| try-catch | [x] |
| throw | [x] |
| throws 声明 | [x] |
| finally | [x] |
| Result 类型 | [x] |
| ? 操作符 | [x] |
| Error 类 | [x] |

---

## 12. 内存管理

### 12.1 WASM 线性内存布局

```
┌─────────────────────────────────────────────────────────┐
│ 0x0000 - 0x03FF │ 数据段 (字符串常量)                    │
├─────────────────────────────────────────────────────────┤
│ 0x0400 (HEAP_BASE + data_offset) - ...                  │
│                 │ 堆空间 (动态分配, Free List Allocator)  │
│                 │                                       │
│  对象头部 (8 字节，用户指针前):                            │
│  [block_size:i32][refcount:i32][user_data...]            │
│                                ^-- __alloc 返回的指针     │
│                                                         │
│  各类型布局 (user_data 部分):                              │
│   - 数组:    [length:i32][elem0:i64][elem1:i64]...      │
│   - 字符串:  [length:i32][UTF-8字节...]                  │
│   - 结构体:  [vtable_ptr?:i32][字段1][字段2]...          │
│   - 枚举:    [discriminant:i32][payload...]              │
│   - 元组:    [field0:i64][field1:i64]...                 │
│   - Range:   [start:i64][end:i64][inclusive:i32]         │
└─────────────────────────────────────────────────────────┘

Globals:
  Global 0: heap_ptr (bump allocator 指针)
  Global 1: free_list_head (空闲链表头指针)
```

### 12.2 内存管理策略

| 策略 | 描述 | 状态 |
|------|------|------|
| Free List Allocator | __alloc/__free，替代 bump allocator，支持内存复用 | [x] |
| 引用计数 | __rc_inc/__rc_dec，对象头 refcount，赋值/离开作用域自动 inc/dec | [x] |
| 垃圾回收 | __gc_collect，堆扫描回收 refcount==0 的对象 | [x] |

---

## 13. WASM 互操作

### 13.1 导入外部函数

```cangjie
// 从 WASM 宿主导入
@import("env", "print")
extern func hostPrint(ptr: Int32, len: Int32)

@import("wasi_snapshot_preview1", "fd_write")
extern func fdWrite(fd: Int32, iovs: Int32, iovsLen: Int32, nwritten: Int32) -> Int32
```

### 13.2 导出函数

```cangjie
// 导出为 WASM 函数
@export("add")
func add(a: Int64, b: Int64) -> Int64 {
    return a + b
}
```

### 13.3 WASI 支持

| 功能 | 描述 | 状态 |
|------|------|------|
| args_get | 命令行参数 | [ ] |
| fd_read | 文件读取 | [ ] |
| fd_write | 文件写入 | [ ] |
| fd_close | 文件关闭 | [ ] |
| clock_time_get | 获取时间 | [ ] |
| random_get | 随机数 | [ ] |

| 功能 | 状态 |
|------|------|
| @import | [x] |
| @export | [x] (自动) |
| extern func | [x] |
| WASI 基础 | [ ] (需运行时提供 fd_write 等) |

---

## 14. 标准库

### 14.1 核心模块

| 模块 | 功能 | 状态 |
|------|------|------|
| `std.core` | 基础类型和函数 | [ ] |
| `std.string` | 字符串操作 | [ ] |
| `std.array` | 数组操作 | [ ] |
| `std.math` | 数学函数 | [ ] |
| `std.io` | 输入输出 | [ ] |
| `std.collections` | 集合类型 | [ ] |
| `std.time` | 时间处理 | [ ] |
| `std.json` | JSON 解析 | [ ] |
| `std.fmt` | 格式化 | [ ] |

### 14.2 内置函数

```cangjie
// 输出
print(value)
println(value)

// 类型转换
Int64(value)
String(value)
Float64(value)

// 数组操作
len(array)
push(array, value)
pop(array)

// 字符串操作
len(string)
concat(a, b)
substring(s, start, end)

// 数学
abs(x)
min(a, b)
max(a, b)
sqrt(x)
pow(base, exp)
```

| 功能 | 状态 |
|------|------|
| print/println | [ ] |
| 类型转换函数 | [ ] |
| 数组函数 | [ ] |
| 字符串函数 | [ ] |
| 数学函数 | [x]（min/max/abs，Int64；pow 已通过 ** 与 __pow_i64） |

---

## 15. 实现状态

*未完成特性的完整实施计划见 [docs/next_steps.md](next_steps.md)。*

### 15.1 当前版本: v0.7.0

正文第 1–15 节各表「状态」列已与本节一致，已实现项均已标为 [x]。Phase 1-6, 8-9 全部完成，下一步计划详见 [next_steps.md](next_steps.md)。

#### 已完成功能

- [x] 基础类型 (Int32, Int64, Float32, Float64, Bool)
- [x] 字符串类型 (String)
- [x] 数组类型 (Array<T>)
- [x] 结构体定义和初始化
- [x] 函数定义和调用
- [x] 算术运算符
- [x] 比较运算符
- [x] let/var 变量声明
- [x] if-else 表达式
- [x] while 循环
- [x] return 语句
- [x] 递归函数
- [x] 字段访问（按结构体类型计算各字段偏移与 load/store 类型）
- [x] 数组索引访问
- [x] WASM 内存管理 (Free List Allocator + RC + GC)
- [x] 函数自动导出
- [x] for 循环：范围迭代 (0..n / 0..=n)
- [x] for 循环：数组迭代 (for item in arr)
- [x] match 表达式
- [x] 模式匹配：字面量、通配符 (_)、范围、多模式 (|)、守卫 (if)、分支绑定 (x if x < 0)
- [x] 逻辑运算符 (&&, \|\|, !)，短路求值
- [x] 复合赋值 (+=, -=, *=, /=, %=)
- [x] 一元负号 (-expr)
- [x] 块表达式 ({ stmt; expr? })
- [x] break / continue（单层循环）
- [x] 字符串转义 (\n \t \" \\)
- [x] 类型推断完善（函数返回类型用于 Call 推断）
- [x] 方法定义（func Type.method + obj.method()）
- [x] 简单枚举（enum 定义、Type.Variant 构造、match 枚举变体）
- [x] 解析错误位置信息（字节偏移与行/列）
- [x] loop 语句（无限循环，与 break/continue 复用）
- [x] 十六进制/八进制/二进制字面量 (0x / 0o / 0b)
- [x] 数字分隔符 (1_000_000)
- [x] 幂运算 (`**`，右结合，i64 通过运行时 __pow_i64)
- [x] 类型转换 (`expr as Type`，Int32/Int64/Float32/Float64/Bool 间转换)
- [x] 位运算 (`&` `|` `^` `~` `<<` `>>`，i32/i64)
- [x] Float32 类型与字面量后缀 `f`
- [x] 科学计数法 (`1.0e10`, `1e-5`)
- [x] 静态方法（func Type.staticMethod()，调用 Type.staticMethod()）
- [x] this 关键字（方法体内解析为第一参数）
- [x] 枚举方法（func Enum.method(self: Enum)，与结构体方法一致）
- [x] 关联值枚举（变体可带单关联类型如 Ok(Int64)，堆布局 [判别式][payload]，match 解构绑定）
- [x] 多行字符串 (`"""..."""`，自动 strip 公共缩进)
- [x] 原始字符串 (`r"raw\nstring"`，不处理转义)
- [x] 范围作为值 (`let r = 0..10`，类型为 Range)
- [x] 构造函数语法糖 (`Point(1, 2)` 转换为 `Point { x: 1, y: 2 }`)
- [x] Lambda 表达式（`(x: Int64) -> Int64 { body }` 和 `{ x: Int64 => body }`；codegen 通过 WASM Table + call_indirect 实现）
- [x] 默认参数 (`func power(base: Int64, exp: Int64 = 2)`)
- [x] 可变参数 (`func sum(args: Int64...)`)
- [x] 函数重载（按参数类型区分，名称修饰）
- [x] 解构绑定 (`let Point { x, y } = p`)
- [x] 解构匹配 (`Point { x, y } =>` 等 match 分支)
- [x] if-let 模式 (`if let x = expr { ... }`)
- [x] while-let 模式 (`while let x = expr { ... }`)
- [x] Option<T> 类型 (`Some(value)` / `None`，堆布局 [tag:i32][value])
- [x] Result<T, E> 类型 (`Ok(value)` / `Err(error)`，堆布局 [tag:i32][value])
- [x] ? 运算符（错误传播，早期返回 None/Err）
- [x] try-catch 表达式 (`try { ... } catch e { ... }`)
- [x] throw 表达式（创建 Err 并早期返回）
- [x] 模块声明 (`module math`)
- [x] import 语句 (`import std.io`、`import math.{sin, cos}`、`import math as m`、`import math.*`)
- [x] 可见性修饰符 (`public` / `private`)
- [x] 字符串插值 (`"Hello, ${name}!"`，支持 `${expr}` 嵌入表达式)
- [x] 优化器（常量折叠：整数/浮点二元运算、一元 Neg/Not，编译前 AST 优化）
- [x] extern func 与 @import（`@import("module","name") extern func ...`，生成 WASM 导入段）
- [x] 内置数学函数（`min(a,b)`、`max(a,b)`、`abs(x)`，Int64）
- [x] 泛型单态化（泛型函数、泛型结构体；显式类型实参如 `identity<Int64>(42)`、`Pair<Int64,Int64>{...}`）
- [x] 接口与类（解析：interface、implements、class、init、deinit、extends、override、super；无继承类展平为结构体编译；super codegen、init 中 this、继承类 vtable 待完善）

#### v0.2.0 已完成

v0.2.0 全部功能已实现，包括 Lambda codegen（WASM Table + call_indirect）。

#### v0.3.0 ~ v0.6.0 新增完成功能

- [x] 基础类型补全（Int8, Int16, UInt8, UInt16, UInt32, UInt64, Char）
- [x] 元组类型与字面量 (`(1, 2, 3)`，堆布局 `[field0:i64][field1:i64]...`）
- [x] 无符号右移 (`>>>`，i32/i64)
- [x] 空值合并 (`??`，Option 空值默认)
- [x] 类 codegen 完整实现（init/deinit/继承/vtable/override/super/Getter/Setter/abstract/sealed）
- [x] 泛型完善（泛型类/枚举、类型约束 `<T:Bound>`、多重约束 `<T:A&B>`、where 子句、泛型特化、约束检查）
- [x] 接口多态（接口定义、默认实现、implements、extend、接口继承、关联类型）
- [x] 闭包编译（Lambda 捕获变量）
- [x] throws 声明 (`func f() -> T throws E`)
- [x] finally 块 (`try { } catch { } finally { }`)
- [x] Error 基类与自定义错误继承 (`class MyError extends Error`)
- [x] internal 可见性修饰符
- [x] 多文件编译与 import 自动解析
- [x] Free List Allocator（__alloc/__free，替代 bump allocator，支持内存复用）
- [x] 引用计数 RC（__rc_inc/__rc_dec，对象头 refcount，赋值/作用域退出自动管理）
- [x] Mark-Sweep GC（__gc_collect，堆扫描回收 refcount==0 对象）

#### v0.7.0 新增完成功能

- [x] Slice<T> 切片类型（`arr[start..end]`，堆布局 `[ptr:i32][len:i32]`）
- [x] Map<K, V> 类型与 Map 字面量（`Map { key => val, ... }`，堆布局线性键值对）
- [x] 类型修饰符（`mut T`、`ref T`、`T?` → `Option<T>` 语法糖、`T!` 非空断言）
- [x] 尾递归优化（检测尾调用位置，将递归转为 loop + 参数重赋值）
- [x] 死代码消除（移除 return/break/continue 后不可达语句）
- [x] 优化器扩展（函数内联基础）

#### v0.3.0 ~ v0.6.0 已完成

- [x] Phase 2: 基础类型补全（Int8/16, UInt8/16/32/64, Char, Tuple, >>>, ??）
- [x] Phase 3: 类与继承（类定义, init/deinit, 继承, vtable, override, super, Getter/Setter, abstract/sealed）
- [x] Phase 4: 泛型完善（泛型函数/结构体/类/枚举, 类型约束, 多重约束, where 子句, 单态化, 特化, 约束检查）
- [x] Phase 5: 接口多态（接口定义, 默认实现, implements, extend, 接口继承, 关联类型, 闭包/Lambda 编译）
- [x] Phase 6: 错误处理 + 模块（throws 声明, finally, Error 基类, 自定义错误继承, 多文件编译, import 自动解析）
- [x] Phase 8: 内存管理升级（Free List Allocator, 引用计数 RC, Mark-Sweep GC, __alloc/__free/__rc_inc/__rc_dec/__gc_collect）

#### v0.7.0 已完成

- [x] Phase 9: 补充特性（Slice<T>, Map<K,V>, 类型修饰符 mut/ref/?/!, 尾递归优化, 死代码消除, 函数内联）

#### 未来版本计划

- [ ] Phase 7: WASI + 标准库（fd_write/fd_read/fd_close/args_get/clock_time_get/random_get, std.core/io/collections）
- [x] Phase 9: 补充特性（Slice<T>, Map 字面量, 类型修饰符, 尾递归优化, 死代码消除, 函数内联）
- [ ] 包管理

---

## 附录

### A. 保留关键字

```
as       break    catch    class    const    continue
do       else     enum     extends  extern   false
finally  for      func     if       implements import
in       init     interface internal let     loop
match    module   mut      override private  public
ref      return   sealed   static   struct   super
this     throw    throws   true     try      type
var      where    while
```

### B. 运算符优先级 (从高到低)

1. `()` `[]` `.` `::`
2. `!` `~` `-` (一元)
3. `**` (幂运算，右结合)
4. `*` `/` `%`
5. `+` `-`
6. `<<` `>>` `>>>`
7. `<` `<=` `>` `>=`
8. `==` `!=`
9. `&`
10. `^`
11. `|`
12. `&&`
13. `||`
14. `?:`
15. `=` `+=` `-=` 等赋值运算符

### C. WASM 指令映射参考

| 仓颉操作 | WASM 指令 |
|----------|-----------|
| Int64 加法 | i64.add |
| Int64 减法 | i64.sub |
| Int64 乘法 | i64.mul |
| Int64 除法 | i64.div_s |
| Int64 取模 | i64.rem_s |
| Float64 加法 | f64.add |
| 函数调用 | call |
| 方法调用 | call（首参为 receiver） |
| 局部变量读取 | local.get |
| 局部变量设置 | local.set |
| 内存加载 | i64.load / i32.load |
| 内存存储 | i64.store / i32.store |
| 条件分支 | if / else / end |
| 循环 / break / continue | block + loop + br / br_if |

---

*文档版本: 2.1.0*
*最后更新: 2026-02-13（v0.7.0 完成，Phase 1-6, 8-9 全部完成；新增 Slice/Map/类型修饰符/尾递归优化/DCE）*
