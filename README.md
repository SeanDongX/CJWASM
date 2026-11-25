English README: [README_EN.md](README_EN.md)

# CJWasm

仓颉语言（Cangjie）到 WebAssembly 的编译器前端，使用 Rust 编写。

CJWasm 将仓颉源代码编译为 WASM 字节码：源码解析为 AST 后降级为 CHIR 中间表示，在 CHIR 层执行优化（小函数内联、冗余局部变量消除），再生成 `.wasm` 文件，可在 wasmtime、浏览器等任何 WASM 运行时中执行。

## 项目状态

- ✅ **官方一致性套件**: 总体 **93.7%**（正测试 5,132/5,132 = **100%**，负测试 18,477/20,069 = **92.1%**）
- ✅ **std 标准库 API 套件**: **99.4%**（1,414/1,441；L1 **100%** / L2 99.3%，剔除 @Unittest 用例 99.8%）
- ✅ **测试数量**: 1,867 个测试全部通过（1,222 单元测试 + 631 集成测试 + 14 标准库测试）
- ✅ **示例覆盖率**: 40/40 系统测试通过 (100%)，0 WASM 验证错误
- ✅ **代码量**: ~87,100 行 Rust 代码（含测试）
- ✅ **AST 节点**: 144 个（Expr: 68, Stmt: 19, Pattern: 15, Type: 42）
- ✅ **CHIR 优化器**: 小函数内联 + 冗余局部变量消除，重负载基准 wasmtime 1.22x / 解释器 ~1.7x 加速

## 官方测试套件兼容性

以官方仓颉测试套件（`third_party/cangjie_test`，git submodule）为兼容性判据：

| 套件 | 规模 | 判定标准 | 当前结果 |
|------|------|----------|----------|
| **Conformance 编译器套件** | 29,705 个测试文件，覆盖 17 章语言规范（剔除 4,504 个多文件结构用例后执行 25,201 个） | 正测试必须编译成功；负测试（`@Negative: yes`）必须编译失败 | **93.7%**（正 **100%** / 负 92.1%） |
| **std 标准库 API 套件** | 1,441 个测试文件，覆盖 33 个 std 模块（L1 基础 + L2 进阶） | 源文件可被编译器接受 | **99.4%**（L1 **100%** / L2 99.3%） |

2026-11-24 启动的全兼容冲刺中，总体兼容率自基线 **49.1%**（正 60.1% / 负 46.3%）逐批提升：

```
49.1% ──▶ 65.1% ──▶ 79.3% ──▶ 91.2% ──▶ 92.6% ──▶ 93.7%
基线      批次4     批次7     批次12    批次13    批次14
```

每批合并的红线：正测试 100% 不回归、std 零回归、`cargo test` 1,867 全过、系统测试 40/40。冲刺采用 4-agent 并行 workflow（git worktree 隔离）批量实现语义检查，30 分钟心跳测量汇报。

剩余未通过的负测试（1,592/20,069 ≈ 7.9%）分布：官方套件自身的正负矛盾组（如 13 章指针组 144 个正负同源、`String <<|` 返回 `Rune` 却标注 `@Negative: no` 等）、需运行期 C 链接的互操作用例（13 章）、需宏展开的元编程/注解用例（14/17 章）、需跨文件包结构的用例（11 章）、以及需更强插值内类型推断的字符串插值诊断（01 章）。

复现测量：`git submodule update --init third_party/cangjie_test` 后，用 `target/release/cjwasm` 逐文件编译套件源码，正测试要求编译成功、负测试要求编译失败（exit ≠ 0）。

## 特性

### 核心语言特性
- **完整的类型系统** — Int8~Int64、UInt8~UInt64、Float16/32/64、Bool、Rune、String、Array、Tuple、Option、Result
- **字面量精度** — 整数类型后缀（`i8`/`i16`/`i32`/`i64`/`u8`/`u16`/`u32`/`u64`）、十六进制浮点、嵌套块注释、UTF-8 转义白名单
- **面向对象** — 结构体、类（含继承、abstract/sealed/open、主构造函数、多 init、静态初始化器）、接口（含默认实现、泛型方法）、属性（prop/mut prop）、扩展方法（extend）
- **泛型** — 泛型函数、泛型结构体/类/枚举，类型约束、多重约束、`where` 子句
- **模式匹配** — `match` 表达式、枚举解构、结构体解构、元组解构、`if-let`、`while-let`、guard 条件
- **错误处理** — `try-catch-finally`、`throws` 声明、`Result<T,E>` / `Option<T>`、`?` 运算符、空值合并 `??`

### 高级特性
- **集合类型** — ArrayList、HashMap、HashSet（完整支持 get/put/remove/contains/size）
- **Lambda 与闭包** — 闭包表达式、函数类型、尾随闭包语法
- **运算符重载** — 自定义类型的运算符重载（op_add, op_sub 等）
- **类型转换** — `as` 类型转换、`is` 类型检查、自动类型协调（Bool ↔ Int64, i32 ↔ i64）
- **范围操作** — `start..end`、`start..=end`、开放式范围 `arr[..end]`、`arr[start..]`
- **字符串插值** — `"Hello, ${name}!"` 语法
- **可选链** — `obj?.method()?.field` 安全访问
- **条件编译** — `@When[os == "Windows"]` 平台条件编译
- **@C 互操作** — `@C` struct/foreign func、`CFunc`/`CType` 跨语言类型规则（13 章规范细则，编译期可判定部分）

### 内存与运行时
- **内存管理** — Free List 分配器 + 引用计数 + Mark-Sweep GC
- **模块系统** — 多文件编译、`import` 自动依赖解析、cjpm 工程支持
- **标准库** — 数学函数、字符串操作、格式化、排序、时间/随机数（WASI）
- **测试断言** — `@Assert(a, b)` / `@Expect(a, b)` 编译器内建断言

### 编译优化
- **类型推断** — 局部变量类型推断、全局变量类型推断、方法返回类型推断
- **类型协调** — 自动插入类型转换指令（i32 ↔ i64, Bool ↔ Int64）
- **常量折叠** — 编译时常量计算
- **死代码消除** — 未使用代码移除
- **CHIR 小函数内联** — 纯表达式函数（无副作用、非递归、≤8 个表达式节点）的调用点直接替换为函数体表达式，消除 call 指令开销
- **CHIR 冗余局部变量消除** — 单写单读且紧邻使用的纯值绑定消除 `local.set` + `local.get` 指令对

> CHIR 优化默认开启，设置环境变量 `NO_CHIR_OPT=1` 可关闭；`NO_CHIR=1` 可整体回退到 AST 直出 WASM 的旧路径。

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
cjwasm tests/examples/hello.cj

# 指定输出
cjwasm tests/examples/hello.cj -o hello.wasm

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
cargo test                     # 1,867 个单元/集成测试（全部通过）
./scripts/system_test.sh       # 40 个系统测试（全部通过）
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

更多示例见 [`tests/examples/`](tests/examples/) 目录（36 个顶层示例 + multifile/project/std 子目录，全部通过）。

## 测试覆盖

### 系统测试 (40/40 通过)

| 类别 | 示例 | 状态 |
|------|------|------|
| **基础语法** | hello.cj, println.cj, literals.cj, advanced.cj | ✅ |
| **控制流** | control_flow.cj, loop_control.cj, for_in_and_guards.cj | ✅ |
| **函数与方法** | functions.cj, methods.cj | ✅ |
| **面向对象** | class.cj, inheritance.cj, interface.cj, phase5_interface.cj | ✅ |
| **数据结构与模式** | enum.cj, patterns.cj | ✅ |
| **泛型** | generic.cj, generic_advanced.cj | ✅ |
| **错误处理** | error_handling.cj, phase6_error_module.cj | ✅ |
| **集合类型** | p3_collections.cj, p4_collections.cj, p3_option_tuple.cj (ArrayList/HashMap/HashSet) | ✅ |
| **运算符与字符串** | operators.cj, strings.cj, str_methods_test.cj | ✅ |
| **并发** | p5_concurrent.cj (spawn/synchronized/Atomic) | ✅ |
| **标准库** | std_math.cj, std_features.cj, p5_stdlib.cj, type_methods.cj | ✅ |
| **P2/P6 特性** | p2_features.cj, phase2_types.cj, p6_new_features.cj (可选链、尾随闭包、范围操作) | ✅ |
| **内存与模块** | memory_management.cj, modules.cj, math.cj | ✅ |
| **多文件/工程** | multifile/ (跨文件编译), project/ (cjpm 工程), std/ (L1 标准库) | ✅ |
| **std 兼容性** | std_test.sh — `third_party/cangjie_test` L1 模块 100% | ✅ |

> std 兼容性测试需要先初始化子模块：`git submodule update --init third_party/cangjie_test`。

### 代码覆盖率

> 以下数据测于 **2026-11-23（兼容冲刺前）**，当时代码量 64.7k 行、测试 1,320 个；冲刺新增约 2.25 万行语义检查后尚未重测（要求 `cargo-llvm-cov` 工具链）。

总行覆盖率 **80.15%**，区域覆盖率 **89.93%**。

| 模块 | 行覆盖率 | 区域覆盖率 |
|------|----------|------------|
| **ast/** | 98.3% | 100% |
| **lexer/** | 90.6% | 98.7% |
| **parser/** | 73.5% | 93.5% |
| **optimizer/** | 96.0% | 96.1% |
| **monomorph/** | 86.3% | 80.2% |
| **codegen/** | 76.5% | 78.5% |
| **chir/** | 91.4% | 97.5% |
| **typeck/** | 95.2% | 100% |
| **sema/** | 94.1% | 97.9% |
| **metadata/** | 93.8% | 97.5% |
| **pipeline/** | 90.2% | 95.2% |
| **memory/** | 100% | 100% |

```bash
./scripts/coverage.sh          # 终端覆盖率报告
./scripts/coverage.sh --html   # HTML 报告 → target/llvm-cov/html/
```

### 单元 & 集成测试 (1,867 个全部通过)

```bash
cargo test
# 1,222 passed (单元测试) + 631 passed (集成测试) + 14 passed (标准库测试)
```

| 类别 | 测试数 | 说明 |
|------|--------|------|
| **语义分析** | 384 | 类/接口/成员/构造器规则（230）+ 表达式类型与运算符细则（154）—— 兼容冲刺主力 |
| **语法解析** | 356 | 声明/语句/表达式/模式解析（含属性语法、修饰符、@C 保留） |
| **CHIR 降级** | 161 | AST → CHIR 表达式/语句降级、CHIR 层类型推断 |
| **代码生成** | 91 | WASM 生成、CHIR CodeGen、宏编译 |
| **词法分析** | 74 | Token 识别、整数后缀、十六进制浮点、嵌套块注释 |
| **编译管线** | 57 | 端到端编译流程、cjpm 工程解析 |
| **优化器/单态化** | 41 | 常量折叠、死代码消除、泛型实例化 |
| **元数据** | 31 | 标准库类型信息 |
| **类型检查** | 12 | 变量解析、类型推断 |
| **集成测试** | 631 | 完整编译管线（源码 → WASM） |
| **标准库测试** | 14 | std 模块编译验证 |

### 官方套件兼容测试

`third_party/cangjie_test`（git submodule）包含两类官方测试：

- **Conformance 编译器套件**（29,705 文件 / 17 章规范）：正测试（5,132）必须编译成功，负测试（20,069）必须编译失败 —— 当前 **93.7%**（正 **100%**）
- **std 标准库 API 套件**（1,441 文件 / 33 个 std 模块）：L1+L2 源文件可编译验证 —— 当前 **99.4%**（L1 **100%**）

详细口径与冲刺进展见前文「官方测试套件兼容性」一节。

### 已知限制

1. **官方套件矛盾与边界类别** — 剩余 1,592 个负测试中约 340 个属于官方套件自身正负同源的矛盾组（13 章指针组 144 个、`String <<|` 标注矛盾等），或需运行期 C 链接（13 章 R 类）、需宏展开（14/17 章）、需跨文件包结构（11 章）的类别——单文件编译前端无法判定
2. **字符串插值深层诊断** — 插值表达式内部的类型错误需要更强的插值内类型推断（01 章 ~122 个负测试）
3. **std/ 包 WASM 验证** - 包含 97 个标准库文件的大型包可以编译，但生成的 WASM 在验证时有类型不匹配（涉及复杂嵌套泛型类型 `Map<K, Tuple<Array<...>>>`）
4. **泛型单态化** - 部分泛型方法使用桩代码而非完整单态化
5. **宏系统** - 宏展开功能有限

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
│   ├── sema/              # 语义分析（validate_program 类/接口/成员/构造器规则 + expr_check 表达式类型与运算符）
│   ├── typeck/            # 类型检查（变量解析、类型推断）
│   ├── metadata/          # 标准库类型元数据
│   ├── chir/              # CHIR 中间表示（默认代码生成路径）
│   │   ├── types.rs       # CHIR 数据结构定义
│   │   ├── builder.rs     # CHIR 构建器
│   │   ├── lower.rs       # AST → CHIR 降级入口
│   │   ├── lower_expr.rs  # 表达式降级
│   │   ├── lower_stmt.rs  # 语句降级
│   │   ├── type_inference.rs # CHIR 层类型推断
│   │   └── optimize.rs    # CHIR 优化 pass（小函数内联 + 冗余 local 消除）
│   ├── optimizer/         # AST 层编译优化
│   │   └── mod.rs         # 常量折叠 & 死代码消除
│   ├── monomorph/         # 泛型单态化
│   │   └── mod.rs         # 类型参数替换 & 实例生成
│   ├── codegen/           # WASM 代码生成（基于 wasm-encoder）
│   │   ├── mod.rs         # CodeGen 主逻辑（旧路径：AST → WASM）
│   │   ├── chir_codegen.rs # CHIR CodeGen（默认路径：CHIR → WASM）
│   │   ├── expr.rs        # 表达式代码生成
│   │   ├── decl.rs        # 声明代码生成
│   │   ├── type_.rs       # 类型代码生成
│   │   └── macro.rs       # 宏代码生成（@Assert/@Expect）
│   ├── memory.rs          # 内存管理（分配器 + RC + GC）
│   └── pipeline.rs        # 编译管线 & 多文件解析
├── tests/examples/        # 仓颉示例程序（53 个：36 顶层 + multifile/project/std 子目录）
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
│   ├── fixtures/          # 基准测试用仓颉源文件（含 bench_heavy.cj 重负载基准）
│   └── report.html        # 性能对比报告（自动生成）
├── scripts/
│   ├── run_test.sh        # 测试运行器（交互式菜单，选择测试级别）
│   ├── system_test.sh     # 系统测试（编译运行示例 & 验证结果）
│   ├── benchmark.sh       # CJWasm vs CJC 综合性能对比
│   ├── coverage.sh        # 测试覆盖率
│   └── system_test.sh     # 系统测试（编译+WASM验证+运行+预期值校验）
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
  CHIR Lowering         AST → CHIR 中间表示
    │
    ▼
  CHIR Optimizer        小函数内联 / 冗余局部变量消除（默认开启）
    │
    ▼
  CHIR CodeGen (wasm-encoder) CHIR → WASM 字节码（含 WASI 运行时函数）
    │
    ▼
  .wasm 文件            可直接运行于 wasmtime / 浏览器
```

> 默认走 CHIR 路径；`NO_CHIR=1` 回退到 AST 直接生成 WASM 的旧路径，`NO_CHIR_OPT=1` 仅关闭 CHIR 优化 pass。

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
| **C 互操作** | @C struct/class、foreign func、CFunc/CType 类型规则、@C 构造器细则（13 章编译期部分） | ✅ |

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

### CHIR 优化效果

在 `bench_heavy.cj`（100,000 次密集小函数调用：identity / double / square / addOne / Counter.get 等）上对比：

| 版本 | wasmtime 耗时 | WASM 大小 |
|------|---------------|-----------|
| CHIR + 优化（默认） | 28.17 ms | 9012 B |
| CHIR 无优化（`NO_CHIR_OPT=1`） | 34.49 ms | 9028 B |

- **wasmtime（JIT）加速 1.22x** — JIT 会对两个版本都做本地优化，差异被部分抹平
- **解释器模式（wasm-interp，无 JIT）约 1.7x 加速** — 更能反映指令级优化效果

### 运行时性能（wasmtime vs cjc native）

| 规模 | wasmtime | cjc native | 比率 |
|------|----------|------------|------|
| 小规模（27 行） | 24.19 ms | 37.02 ms | wasm 1.5x 快 |
| 中规模（100 行） | 23.61 ms | 37.30 ms | wasm 1.6x 快 |
| 大规模（311 行） | 24.20 ms | 37.00 ms | wasm 1.5x 快 |
| 高负载（89 行） | 28.13 ms | 83.35 ms | wasm 3.0x 快 |

> 端到端进程耗时（含启动）；wasmtime 时间包含 JIT 编译开销，native 包含进程启动开销。

运行完整基准测试：

```bash
# 综合对比（编译速度 + 输出大小 + 运行时 + CHIR 优化对比）
./scripts/benchmark.sh

# 快速模式
./scripts/benchmark.sh --quick

# 仅对比某一项
./scripts/benchmark.sh --compile    # 仅编译速度
./scripts/benchmark.sh --runtime    # 仅运行时性能
./scripts/benchmark.sh --size       # 仅输出大小

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
| `system_test.sh` | 系统测试：编译所有 .cj 示例 + WASM 验证 + 运行 + 预期值校验 | `./scripts/system_test.sh [--verbose] [file.cj]` |
| `benchmark.sh` | CJWasm vs CJC 性能对比（编译/运行/大小/CHIR 优化） | `./scripts/benchmark.sh [--quick\|--compile\|--runtime\|--size\|--criterion]` |
| `coverage.sh` | 生成测试覆盖率报告 | `./scripts/coverage.sh [--html]` |

## 开发

### 构建 & 测试

```bash
# 构建
cargo build --release

# 运行全部测试（1,867 个）
cargo test

# 交互式测试菜单
./scripts/run_test.sh

# 测试覆盖率
./scripts/coverage.sh
./scripts/coverage.sh --html   # HTML 报告 → target/llvm-cov/html/

# 官方套件兼容性（需先初始化 submodule）
git submodule update --init third_party/cangjie_test
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

Apache 2.0
