# CJWasm

仓颉语言（Cangjie）到 WebAssembly 的编译器前端，使用 Rust 编写。

CJWasm 将仓颉源代码直接编译为 WASM 字节码，无需中间表示，生成的 `.wasm` 文件可在 wasmtime、浏览器等任何 WASM 运行时中执行。

## 特性

- **完整的类型系统** — Int8~Int64、UInt8~UInt64、Float32/64、Bool、Rune、String、Array（含动态构造）、Tuple、Option、Result
- **面向对象** — 结构体、类（含继承、abstract/sealed）、接口（含默认实现）、属性（prop）
- **泛型** — 泛型函数、泛型结构体/类/枚举，类型约束、多重约束、`where` 子句、泛型特化
- **模式匹配** — `match` 表达式、枚举解构、结构体解构、`if-let`、`while-let`、guard 条件
- **错误处理** — `try-catch-finally`、`throws` 声明、`Result<T,E>` / `Option<T>`、`?` 运算符、空值合并 `??`
- **内存管理** — Free List 分配器 + 引用计数 + Mark-Sweep GC
- **模块系统** — 多文件编译、`import` 自动依赖解析、cjpm 工程支持
- **Lambda** — 闭包表达式、函数类型
- **标准库** — 数学函数、字符串操作（trim/startsWith/endsWith/contains/indexOf/split/replace）、格式化、排序、时间/随机数（WASI）
- **测试断言** — `@Assert(a, b)` / `@Expect(a, b)` 编译器内建断言（兼容仓颉 `std.unittest` 语义）
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
# 交互式测试菜单（推荐）
./scripts/run_test.sh

# 或直接选择测试级别
./scripts/run_test.sh 1    # 单元测试 (cargo test)
./scripts/run_test.sh 2    # 系统测试 (编译运行 .cj 示例)
./scripts/run_test.sh 3    # 性能测试
./scripts/run_test.sh 4    # 单元 + 系统测试
./scripts/run_test.sh 5    # 全部测试

# 也可以单独运行
cargo test                     # 230 个单元/集成测试（229 passed, 1 ignored, 100%）
./scripts/system_test.sh       # 30 个系统测试（编译运行示例并验证返回值）
./scripts/coverage.sh          # 测试覆盖率
./scripts/coverage.sh --html   # HTML 报告 → target/llvm-cov/html/
```

## 语言示例

```cangjie
// 函数与递归
func fibonacci(n: Int64): Int64 {
    if (n <= 1) { return n }
    return fibonacci(n - 1) + fibonacci(n - 2)
}

// 泛型结构体
struct Pair<T, U> {
    var first: T;
    var second: U;
    init(first: T, second: U) {
        this.first = first
        this.second = second
    }
}

// 类与继承
open class Animal {
    var name: Int64;
    init(name: Int64) { this.name = name }
    func speak(): Int64 { return 0 }
}

class Dog <: Animal {
    init(name: Int64) { super(name) }
    override func speak(): Int64 { return 1 }
}

// 枚举与模式匹配
enum Shape {
    | Circle(Int64)
    | Rectangle(Int64, Int64)
}

func area(s: Shape): Int64 {
    match (s) {
        case Shape.Circle(r) => r * r * 3,
        case Shape.Rectangle(w, h) => w * h
    }
}

// 错误处理
func safeDivide(a: Int64, b: Int64): Result<Int64, String> {
    if (b == 0) { return Err("division by zero") }
    return Ok(a / b)
}

// 测试断言
main(): Int64 {
    let fib = fibonacci(10)
    @Assert(fib, 55)

    let result = safeDivide(10, 2) ?? 0
    @Assert(result, 5)

    return fib + result
}
```

更多示例见 [`examples/`](examples/) 目录（28 个示例文件）。

## 项目结构

```
cjwasm/
├── src/
│   ├── main.rs            # CLI 入口（build/init/compile 子命令）
│   ├── lib.rs             # 库入口
│   ├── cjpm.rs            # cjpm.toml 解析 & build 命令
│   ├── lexer/             # 词法分析（基于 logos）
│   │   └── mod.rs         # Token 定义 & 词法规则
│   ├── parser/            # 递归下降语法分析器
│   │   ├── mod.rs         # Parser 主逻辑
│   │   ├── expr.rs        # 表达式解析
│   │   ├── stmt.rs        # 语句解析
│   │   ├── decl.rs        # 声明解析
│   │   ├── type_.rs       # 类型解析
│   │   ├── pattern.rs     # 模式解析
│   │   ├── macro.rs       # 宏系统解析（@Assert/@Expect）
│   │   └── error.rs       # 错误处理
│   ├── ast/               # 抽象语法树定义
│   │   ├── mod.rs         # Expr, Stmt, Pattern 等枚举（91 个节点）
│   │   └── type_.rs       # Type 枚举（19 个类型）
│   ├── optimizer/         # 编译优化
│   │   └── mod.rs         # 常量折叠 & 死代码消除
│   ├── monomorph/         # 泛型单态化
│   │   └── mod.rs         # 类型参数替换 & 实例生成
│   ├── codegen/           # WASM 代码生成（基于 wasm-encoder）
│   │   ├── mod.rs         # CodeGen 主逻辑
│   │   ├── expr.rs        # 表达式代码生成
│   │   ├── decl.rs        # 声明代码生成
│   │   ├── type_.rs       # 类型代码生成
│   │   └── macro.rs       # 宏代码生成（@Assert/@Expect）
│   ├── memory.rs          # 内存管理（分配器 + RC + GC）
│   └── pipeline.rs        # 编译管线 & 多文件解析
├── examples/              # 仓颉示例程序（41 个）
│   ├── hello.cj           # Hello World
│   ├── functions.cj       # 函数定义与递归
│   ├── class.cj           # 类与属性
│   ├── inheritance.cj     # 类继承与多态
│   ├── interface.cj       # 接口与默认实现
│   ├── generic.cj         # 泛型基础
│   ├── generic_advanced.cj # 泛型约束、where 子句、泛型类
│   ├── enum.cj            # 枚举与关联值
│   ├── patterns.cj        # 模式匹配（if-let, while-let, 解）
│   ├── error_handling.cj  # try-catch-finally, Result/Option
│   ├── control_flow.cj    # 控制流（if/while/for/match）
│   ├── operators.cj       # 运算符（算术/比较/逻辑/位运算）
│   ├── strings.cj         # 字符串插值与操作
│   ├── math.cj            # 数学函数
│   ├── std_math.cj        # 标准数学库（sin/cos/exp/log/...）
│   ├── std_features.cj    # 格式化、WASI扩展、字符串操作、排序
│   ├── type_methods.cj    # 内建类型方法（toString/toInt64/abs/...）
│   ├── memory_management.cj # 内存管理与引用计数
│   ├── multifile/         # 多文件编译示例
│   ├── project/           # cjpm 工程示例
│   ├── phase5_interface.cj # 接口高级特性
│   └── modules.cj        # 模块系统示例
├── tests/fixtures/        # 测试夹具（20+ 个）
│   ├── macro_test.cj      # 宏系统测试
│   ├── if_let_test.cj     # if-let 模式匹配测试
│   ├── optional_chain_test.cj # 可选链测试
│   ├── trailing_closure_test.cj # 尾随闭包测试
│   ├── type_alias_test.cj  # 类型别名测试
│   └── ...
├── benches/               # 性能基准测试
│   ├── compile_bench.rs   # Criterion 微基准（各编译阶段）
│   ├── fixtures/          # 基准测试用仓颉源文件
│   └── report.html        # 性能对比报告（自动生成）
├── scripts/
│   ├── run_test.sh        # 测试运行器（交互式菜单，选择测试级别）
│   ├── system_test.sh     # 系统测试（编译运行示例 & 验证结果）
│   ├── benchmark.sh       # CJWasm vs CJC 综合性能对比
│   ├── coverage.sh        # 测试覆盖率
│   └── run_examples.sh    # 运行所有示例
├── docs/
│   ├── spec.md            # 编译器规格说明书
│   ├── next_steps.md      # 开发路线图与进度追踪
│   ├── coverage.md        # 测试覆盖率报告
│   └── plan/              # 设计方案文档
│       └── ast_refactor/ # AST 重构文档
│           ├── README.md              # 文档索引
│           ├── ast_mapping.md         # AST 节点映射表
│           ├── MIGRATION_SUMMARY.md  # 迁移总结
│           ├── QUICK_REFERENCE.md     # 快速参考
│           ├── CJC_MIGRATION_GUIDE.md # 迁移指南
│           ├── ARCHITECTURE_COMPARISON.md # 架构对比
│           ├── macro_implementation_summary.md # 宏系统实现
│           └── macro_research.md     # 宏系统研究
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
  Parser                递归下降 → AST（含 @Assert/@Expect 解析）
    │
    ▼
  Optimizer             常量折叠 / 死代码消除
    │
    ▼
  Monomorphizer         泛型单态化
    │
    ▼
  CodeGen (wasm-encoder) AST → WASM 字节码（含 WASI 运行时函数）
    │
    ▼
  .wasm 文件            可直接运行于 wasmtime / 浏览器
```

## 已支持的仓颉语法

| 分类 | 特性 | 状态 |
|------|------|------|
| **基础类型** | Int8~64, UInt8~64, Float16/32/64, Bool, Rune, String, const 常量 | ✅ |
| **复合类型** | Array, Tuple, Option, Result, Struct, Enum, Class, Range, Slice, Map | ✅ |
| **函数** | 函数定义, 递归, 默认参数, 可变参数, 命名参数(name!:), Lambda, 尾随闭包, inout 参数, 函数类型 | ✅ |
| **泛型** | 泛型函数/结构体/类/枚举, 类型约束, 多重约束, where 子句, 泛型特化 | ✅ |
| **OOP** | 类, 继承(open/<:), abstract/sealed, 主构造函数, 接口, 默认实现, 属性(prop), operator func, extend, static init | ✅ |
| **控制流** | if/else, while, do-while, for-in（含步长）, loop, break/continue, match, while-let, spawn, synchronized | ✅ |
| **模式匹配** | 枚举解构, 结构体解构, if-let, while-let, guard(where), 嵌套解构, match type pattern, is 表达式 | ✅ |
| **错误处理** | try-catch-finally, try-with-resources, throws, Result/Option, ? 运算符, 空值合并 ?? | ✅ |
| **模块** | import, 多文件编译, 可见性修饰符(public/internal/private) | ✅ |
| **内存** | 堆分配, 引用计数, Mark-Sweep GC, Free List 分配器 | ✅ |
| **运算符** | 算术, 比较, 逻辑, 位运算, 幂运算(**), !in, 字符串插值, 类型转换(as), 可选链(?.), 方法重载 | ✅ |
| **集合** | HashMap(put/get/remove/containsKey/size), HashSet(add/contains/size), ArrayList, LinkedList, ArrayStack | ✅ |
| **并发桩** | spawn(同步执行), synchronized(直通), AtomicInt64/AtomicBool, Mutex/ReentrantMutex | ✅ |
| **标准库** | 数学函数(sin/cos/exp/log/...), 字符串操作(trim/startsWith/endsWith/contains/indexOf/replace), 数组方法(clone/isEmpty/slice), now() 时间戳 | ✅ |
| **格式化** | Int64.format("x"/"b"/"o"), Float64.format("2f"), toString() | ✅ |
| **WASI** | println/print, 时间(now), 随机数, 排序(sort), 进程退出(exit) | ✅ |
| **测试** | @Assert(a, b), @Expect(a, b), 单参数布尔形式, 类型自动协调 | ✅ |
| **外部接口** | extern func + @import("module", "name") | ✅ |

完整语法规格见 [`docs/spec.md`](docs/spec.md)。

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

## 脚本说明

| 脚本 | 用途 | 用法 |
|------|------|------|
| `run_test.sh` | 测试运行器，交互式选择测试级别 | `./scripts/run_test.sh [1-5]` |
| `system_test.sh` | 编译运行 28 个 .cj 示例并验证返回值 | `./scripts/system_test.sh [--verbose]` |
| `benchmark.sh` | CJWasm vs CJC 性能对比（编译/运行/大小） | `./scripts/benchmark.sh [--quick]` |
| `coverage.sh` | 生成测试覆盖率报告 | `./scripts/coverage.sh [--html]` |
| `run_examples.sh` | 运行所有示例程序 | `./scripts/run_examples.sh` |

## 开发

### 构建 & 测试

```bash
# 构建
cargo build --release

# 运行全部测试（388 个）
cargo test

# 交互式测试菜单
./scripts/run_test.sh

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

## License

MIT
