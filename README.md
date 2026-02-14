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

### 使用 cjpm 工程（推荐）

CJWasm 兼容仓颉包管理器 (cjpm) 的项目结构，可直接读取 `cjpm.toml` 进行编译：

```bash
# 初始化新项目
cjwasm init myproject
cd myproject

# 编译工程（读取 cjpm.toml，自动发现 src/ 下的 .cj 文件）
cjwasm build

# 指定输出文件
cjwasm build -o app.wasm

# 显示详细编译信息
cjwasm build -v

# 运行编译结果
wasmtime run --invoke main target/wasm/myproject.wasm
```

生成的项目结构：
```
myproject/
├── cjpm.toml          # 项目配置（兼容 cjpm 格式）
└── src/
    └── main.cj        # 入口源文件
```

### 直接编译（无需 cjpm.toml）

```bash
# 编译单文件
cjwasm examples/hello.cj

# 指定输出
cjwasm examples/hello.cj -o hello.wasm

# 多文件编译
cjwasm main.cj lib.cj -o app.wasm
```

### 运行 WASM

```bash
# 使用 wasmtime 运行
wasmtime run --invoke main hello.wasm
```

### 测试

```bash
# 运行单元测试（178 个）
cargo test

# 运行系统测试（编译运行所有示例并验证返回值）
./scripts/system_test.sh

# 测试覆盖率
./scripts/coverage.sh
./scripts/coverage.sh --html   # 生成 HTML 报告到 target/llvm-cov/html/index.html
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
│   ├── main.rs          # CLI 入口（build/init/compile 子命令）
│   ├── lib.rs           # 库入口
│   ├── cjpm.rs          # cjpm.toml 解析 & build 命令
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
│   ├── system_test.sh   # 系统测试（编译运行示例 & 验证结果）
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
| [toml](https://crates.io/crates/toml) + [serde](https://crates.io/crates/serde) | cjpm.toml 配置解析 |
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
