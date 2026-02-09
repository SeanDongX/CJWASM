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
| `Int8` | 8位有符号整数 | i32 | [ ] |
| `Int16` | 16位有符号整数 | i32 | [ ] |
| `Int32` | 32位有符号整数 | i32 | [x] |
| `Int64` | 64位有符号整数 | i64 | [x] |
| `UInt8` | 8位无符号整数 | i32 | [ ] |
| `UInt16` | 16位无符号整数 | i32 | [ ] |
| `UInt32` | 32位无符号整数 | i32 | [ ] |
| `UInt64` | 64位无符号整数 | i64 | [ ] |
| `Float32` | 32位浮点数 | f32 | [ ] |
| `Float64` | 64位浮点数 | f64 | [x] |
| `Bool` | 布尔值 | i32 (0/1) | [x] |
| `Char` | Unicode 字符 | i32 | [ ] |
| `Unit` | 空类型 | (无返回值) | [x] |

### 1.2 复合类型

| 类型 | 语法 | 描述 | 状态 |
|------|------|------|------|
| `String` | `String` | UTF-8 字符串 | [x] |
| `Array<T>` | `Array<Int64>` | 固定长度数组 | [x] |
| `Slice<T>` | `Slice<Int64>` | 动态切片 | [ ] |
| `Tuple` | `(Int64, String)` | 元组类型 | [ ] |
| `Option<T>` | `Option<Int64>` | 可选值 | [ ] |
| `Result<T, E>` | `Result<Int64, Error>` | 结果类型 | [ ] |
| `Struct` | `struct Point {...}` | 结构体 | [x] |
| `Enum` | `enum Color {...}` | 枚举类型（简单枚举，无关联值） | [x] |
| `Class` | `class Person {...}` | 类 | [ ] |
| `Interface` | `interface Drawable {...}` | 接口 | [ ] |
| `Function` | `(Int64) -> Int64` | 函数类型 | [ ] |

### 1.3 类型修饰符

| 修饰符 | 描述 | 状态 |
|--------|------|------|
| `mut` | 可变引用 | [ ] |
| `ref` | 引用类型 | [ ] |
| `?` | 可空类型 | [ ] |
| `!` | 非空断言 | [ ] |

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
| 十六进制 | [ ] |
| 八进制 | [ ] |
| 二进制 | [ ] |
| 数字分隔符 | [ ] |
| 浮点数 | [x] |
| 科学计数法 | [ ] |
| 类型后缀 | [ ] |

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
| 多行字符串 | [ ] |
| 字符串插值 | [ ] |
| 原始字符串 | [ ] |

### 2.3 其他字面量

```cangjie
let arr = [1, 2, 3]           // 数组
let tuple = (1, "hello")      // 元组
let map = {"a": 1, "b": 2}    // Map (语法糖)
```

| 功能 | 状态 |
|------|------|
| 数组字面量 | [x] |
| 元组字面量 | [ ] |
| Map 字面量 | [ ] |

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
| `**` | 幂运算 | [ ] |
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
| `&` | 按位与 | [ ] |
| `\|` | 按位或 | [ ] |
| `^` | 按位异或 | [ ] |
| `~` | 按位取反 | [ ] |
| `<<` | 左移 | [ ] |
| `>>` | 右移 | [ ] |
| `>>>` | 无符号右移 | [ ] |

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
| 范围表达式（作为值） | [ ] |
| 类型转换 (as) | [ ] |
| 空值合并 (??) | [ ] |
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
| 解构绑定 | [ ] |

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
| loop | [ ] |
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
| 默认参数 | [ ] |
| 可变参数 | [ ] |
| 泛型函数 | [ ] |
| Lambda | [ ] |
| 闭包 | [ ] |
| 递归 | [x] |
| 尾递归优化 | [ ] |

### 5.2 函数重载

```cangjie
func process(x: Int64) -> Int64 { ... }
func process(x: String) -> String { ... }
```

| 功能 | 状态 |
|------|------|
| 函数重载 | [ ] |

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
| 静态方法 | [ ] |
| 构造函数 | [ ] |
| this 关键字 | [ ] |

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
| 类定义 | [ ] |
| 构造函数 (init) | [ ] |
| 析构函数 (deinit) | [ ] |
| 实例方法 | [ ] |
| 静态方法 | [ ] |
| Getter/Setter | [ ] |
| 继承 (extends) | [ ] |
| 方法重写 (override) | [ ] |
| super 调用 | [ ] |
| 访问修饰符 | [ ] |
| abstract 类 | [ ] |
| sealed 类 | [ ] |

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
| 关联值枚举 | [ ] |
| 泛型枚举 | [ ] |
| 枚举方法 | [ ] |

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
| 解构匹配 | [ ] |
| 守卫条件 (if) | [x] |
| 通配符 (_) | [x] |
| if-let | [ ] |
| while-let | [ ] |

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
| 泛型函数 | [ ] |
| 泛型结构体 | [ ] |
| 泛型类 | [ ] |
| 泛型枚举 | [ ] |
| 类型约束 | [ ] |
| 多重约束 | [ ] |
| where 子句 | [ ] |
| 泛型特化 | [ ] |

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
| 接口定义 | [ ] |
| 默认实现 | [ ] |
| implements | [ ] |
| 扩展 (extend) | [ ] |
| 接口继承 | [ ] |
| 关联类型 | [ ] |

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
| module 声明 | [ ] |
| import | [ ] |
| 别名 (as) | [ ] |
| 通配符导入 | [ ] |
| public | [ ] |
| internal | [ ] |
| private | [ ] |
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
| try-catch | [ ] |
| throw | [ ] |
| throws 声明 | [ ] |
| finally | [ ] |
| Result 类型 | [ ] |
| ? 操作符 | [ ] |
| Error 类 | [ ] |

---

## 12. 内存管理

### 12.1 WASM 线性内存布局

```
┌─────────────────────────────────────────────────────────┐
│ 0x0000 - 0x03FF │ 数据段 (字符串常量)                    │
├─────────────────────────────────────────────────────────┤
│ 0x0400 - 0x0FFF │ 栈空间 (函数调用栈)                    │
├─────────────────────────────────────────────────────────┤
│ 0x1000 - ...    │ 堆空间 (动态分配)                      │
│                 │   - 数组: [length:i32][元素...]        │
│                 │   - 字符串: [length:i32][UTF-8字节...] │
│                 │   - 结构体: [字段1][字段2]...          │
└─────────────────────────────────────────────────────────┘
```

### 12.2 内存管理策略

| 策略 | 描述 | 状态 |
|------|------|------|
| 简单分配器 | bump allocator | [x] |
| 引用计数 | RC/ARC | [ ] |
| 垃圾回收 | Mark-Sweep GC | [ ] |
| 手动管理 | malloc/free | [ ] |

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
| @import | [ ] |
| @export | [x] (自动) |
| extern func | [ ] |
| WASI 基础 | [ ] |

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
| 数学函数 | [ ] |

---

## 15. 实现状态

### 15.1 当前版本: v0.1.0

#### 已完成功能

- [x] 基础类型 (Int32, Int64, Float64, Bool)
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
- [x] WASM 内存管理 (简单分配器)
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

#### 进行中

- （暂无）

#### 下一版本 (v0.2.0) 计划

- [ ] Lambda 表达式（需 table / call_indirect）
- [ ] 关联值枚举
- [ ] 枚举方法

#### 未来版本计划

- [ ] 类和继承
- [ ] 泛型
- [ ] 接口/Trait
- [ ] 模块系统
- [ ] 错误处理
- [ ] WASI 支持
- [ ] 标准库
- [ ] 优化器

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
3. `*` `/` `%`
4. `+` `-`
5. `<<` `>>` `>>>`
6. `<` `<=` `>` `>=`
7. `==` `!=`
8. `&`
9. `^`
10. `|`
11. `&&`
12. `||`
13. `?:`
14. `=` `+=` `-=` 等赋值运算符

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

*文档版本: 1.2.0*
*最后更新: 2025-02（与当前实现同步：break/continue、字符串转义、类型推断、方法、简单枚举、错误位置）*
