# CJWasm

仓颉语言（Cangjie）到 WebAssembly 的编译器前端，使用 Rust 编写。

CJWasm 将仓颉源代码直接编译为 WASM 字节码，无需中间表示，生成的 `.wasm` 文件可在 wasmtime、浏览器等任何 WASM 运行时中执行。

## 特性

- **完整的类型系统** — Int8~Int64、UInt8~UInt64、Float32/64、Bool、Char、String、Array、Tuple、Option、Result
- **面向对象** — 结构体、类（含继承、abstract/sealed）、接口（含默认实现）
- **泛型** — 泛型函数、泛型结构体/枚举，支持类型约束和 `where` 子句
- **模式匹配** — `match` 表达式、枚举解构、`if-let`、guard 条件
- **错误处理** — `try-catch-finally`、`throws` 声明、`Result<T,E>` / `Option<T>`、`?` 运算符
- **内存管理** — Free List 分配器 + 引用计数 + Mark-Sweep GC
- **模块系统** — 多文件编译、`import` 自动依赖解析
- **Lambda** — 闭包表达式、函数类型
- **编译优化** — 常量折叠、死代码消除、单态化

## 快速开始

### 安装

```bash
# 克隆并构建
git clone https://gitcode.com/SeanXDO/CJWasm
cd cjwasm
cargo build --release
```

### 编译仓颉程序

```bash
# 编译单文件
./target/release/cjwasm examples/hello.cj

# 指定输出
./target/release/cjwasm examples/hello.cj -o hello.wasm

# 多文件编译
./target/release/cjwasm main.cj lib.cj -o app.wasm
```
### 运行单元测试

```bash
cargo test
```

以上命令会运行所有 Rust 层的单元测试和集成测试（源码与测试目录共计 165 个），可帮助快速验证 cjwasm 各模块的正确性。

如需查看具体测试覆盖率，可执行：

```bash
./scripts/coverage.sh
./scripts/coverage.sh --html   # 生成 HTML 报告到 target/llvm-cov/html/index.html
```


### 运行 WASM

```bash
# 使用 wasmtime 运行
wasmtime run --invoke main hello.wasm
```



## 语言示例

```cangjie
// 函数与递归
func fibonacci(n: Int64) -> Int64 {
    if n <= 1 { n } else { fibonacci(n - 1) + fibonacci(n - 2) }
}

// 泛型结构体
struct Pair<T, U> {
    first: T,
    second: U
}

// 类与方法
class Vector2D {
    var x: Int64;
    var y: Int64;
    func length(self: Vector2D) -> Int64 {
        return self.x * self.x + self.y * self.y
    }
}

// 枚举与模式匹配
enum Shape {
    Circle(Int64),
    Rectangle(Int64, Int64)
}

func area(s: Shape) -> Int64 {
    match s {
        Shape.Circle(r) => r * r * 3,
        Shape.Rectangle(w, h) => w * h
    }
}

// 错误处理
func safeDivide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 { return Err("division by zero") }
    return Ok(a / b)
}

func main() -> Int64 {
    let fib = fibonacci(10)
    let p = Pair<Int64, Int64> { first: 10, second: 20 }
    return fib + p.first + p.second
}
```

更多示例见 [`examples/`](examples/) 目录。

## 项目结构

```
cjwasm/
├── src/
│   ├── main.rs          # CLI 入口
│   ├── lib.rs           # 库入口
│   ├── lexer/           # 词法分析（基于 logos）
│   ├── parser/          # 递归下降语法分析器
│   ├── ast/             # 抽象语法树定义
│   ├── optimizer/       # 常量折叠 & 死代码消除
│   ├── monomorph/       # 泛型单态化
│   ├── codegen/         # WASM 代码生成（基于 wasm-encoder）
│   ├── memory.rs        # 内存管理（分配器 + RC + GC）
│   └── pipeline.rs      # 编译管线 & 多文件解析
├── examples/            # 仓颉示例程序（26 个）
├── benches/             # 性能基准测试
│   ├── compile_bench.rs # Criterion 微基准（各编译阶段）
│   ├── fixtures/        # 基准测试用仓颉源文件
│   └── report.html      # 性能对比报告（自动生成）
├── scripts/
│   ├── benchmark.sh     # CJWasm vs CJC 综合性能对比
│   └── coverage.sh      # 测试覆盖率
├── docs/
│   └── spec.md          # 编译器规格说明书
└── Cargo.toml
```

## 编译管线

```
仓颉源码 (.cj)
    │
    ▼
  Lexer (logos)         词法分析 → Token 流
    │
    ▼
  Parser                递归下降 → AST
    │
    ▼
  Optimizer             常量折叠 / 死代码消除
    │
    ▼
  Monomorphizer         泛型单态化
    │
    ▼
  CodeGen (wasm-encoder) AST → WASM 字节码
    │
    ▼
  .wasm 文件            可直接运行于 wasmtime / 浏览器
```

## 性能基准

CJWasm 编译速度和输出大小与仓颉原生编译器 (cjc) 的对比：

| 指标 | cjwasm | cjc | 倍率 |
|------|--------|-----|------|
| 编译速度（小规模） | ~1ms | ~700ms | **~36x 快** |
| 编译速度（中规模） | ~2ms | ~900ms | **~7x 快** |
| 输出大小（小规模） | 1.2 KB | 1020 KB | **845x 小** |
| 输出大小（中规模） | 1.9 KB | 1021 KB | **543x 小** |

> cjwasm 是轻量级前端编译器，仅生成 WASM 字节码；cjc 是完整原生编译器，生成含标准库的可执行文件。两者定位不同。

运行完整基准测试：

```bash
# 综合对比（编译速度 + 输出大小 + 运行时）
./scripts/benchmark.sh

# 快速模式
./scripts/benchmark.sh --quick

# 仅运行时性能（需要 wasmtime）
./scripts/benchmark.sh --runtime

# Criterion 微基准（各编译阶段耗时）
cargo bench
```

报告输出：
- 对比报告：`benches/report.html`
- Criterion 报告：`target/criterion/report/index.html`

## 开发

### 构建 & 测试

```bash
# 构建
cargo build

# 运行全部测试（165 个）
cargo test

# 测试覆盖率
./scripts/coverage.sh
./scripts/coverage.sh --html   # HTML 报告 → target/llvm-cov/html/
```

### 依赖

| 依赖 | 用途 |
|------|------|
| [logos](https://crates.io/crates/logos) | 词法分析器生成 |
| [wasm-encoder](https://crates.io/crates/wasm-encoder) | WASM 字节码编码 |
| [thiserror](https://crates.io/crates/thiserror) | 错误类型派生 |
| [ariadne](https://crates.io/crates/ariadne) | 编译错误诊断输出 |
| [criterion](https://crates.io/crates/criterion) | 性能基准测试（dev） |

### 运行时工具（可选）

```bash
# WASM 运行时
brew install wasmtime

# CLI 基准测试工具
brew install hyperfine

# WASM 文本格式查看
brew install wabt        # 提供 wasm2wat
```

## 已支持的仓颉语法

| 分类 | 特性 | 状态 |
|------|------|------|
| **类型** | Int8~64, UInt8~64, Float32/64, Bool, Char, String | ✅ |
| **复合** | Array, Tuple, Option, Result, Struct, Enum, Class | ✅ |
| **函数** | 函数定义, 递归, 默认参数, 可变参数, Lambda | ✅ |
| **泛型** | 泛型函数/结构体/枚举, 类型约束, where 子句 | ✅ |
| **OOP** | 类, 继承, abstract/sealed, 接口, 默认实现 | ✅ |
| **控制流** | if/else, while, for-in, loop, break/continue, match | ✅ |
| **错误处理** | try-catch-finally, throws, Result/Option, ? 运算符 | ✅ |
| **模块** | import, 多文件编译, 可见性修饰符 | ✅ |
| **内存** | 堆分配, 引用计数, GC | ✅ |
| **运算符** | 算术, 比较, 逻辑, 位运算, 幂运算, 字符串插值 | ✅ |
| **其他** | extern 函数导入 (@import), 类型转换 (as) | ✅ |

完整语法规格见 [`docs/spec.md`](docs/spec.md)。

## License

MIT
