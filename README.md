English README: [README_EN.md](README_EN.md)

# CJWasm

仓颉语言（Cangjie）到 WebAssembly 的编译器前端，使用 Rust 编写。

CJWasm 将仓颉源代码编译为 WASM 字节码：源码解析为 AST 后降级为 CHIR 中间表示，在 CHIR 层执行优化（小函数内联、冗余局部变量消除），再生成 `.wasm` 文件，可在 wasmtime、浏览器等任何 WASM 运行时中执行。

## 项目状态

- ✅ **官方一致性套件**: 总体 **99.1%**（正测试 5,132/5,132 = **100%**，负测试 19,832/20,069 = **98.8%**）
- ✅ **std 标准库 API 套件**: **99.3%**（1,413/1,441；L1 **100%** / L2 99.2%，剔除 @Unittest 用例 99.8%；28 个非通过 = 10 个官方负例被正确拒绝 + 18 个结构跳过）
- ✅ **测试数量**: 2,188 个测试全部通过（1,543 单元测试 + 631 集成测试 + 14 标准库测试）
- ✅ **示例覆盖率**: 40/40 系统测试通过 (100%)，0 WASM 验证错误
- ✅ **代码量**: ~130,900 行 Rust 代码（含测试）
- ✅ **AST 节点**: 117 个（Expr: 58, Stmt: 17, Pattern: 11, Type: 31）
- ✅ **CHIR 优化器**: 小函数内联 + 冗余局部变量消除，重负载基准 wasmtime 1.44x 加速（2026-11-26 实测）

## 官方测试套件兼容性

以官方仓颉测试套件（`third_party/cangjie_test`，git submodule）为兼容性判据：

| 套件 | 规模 | 判定标准 | 当前结果 |
|------|------|----------|----------|
| **Conformance 编译器套件** | 29,705 个测试文件，覆盖 17 章语言规范（剔除 4,504 个多文件结构用例后执行 25,201 个） | 正测试必须编译成功；负测试（`@Negative: yes`）必须编译失败 | **99.1%**（正 **100%** / 负 98.8%） |
| **std 标准库 API 套件** | 1,441 个测试文件，覆盖 33 个 std 模块（L1 基础 + L2 进阶） | 源文件可被编译器接受 | **99.3%**（L1 **100%** / L2 99.2%） |

2026-11-24 启动的全兼容冲刺中，总体兼容率自基线 **49.1%**（正 60.1% / 负 46.3%）逐批提升：

```
49.1% ──▶ 65.1% ──▶ 79.3% ──▶ 91.2% ──▶ 93.7% ──▶ 95.3% ──▶ 96.0% ──▶ 97.0% ──▶ 98.1% ──▶ 99.1%
基线      批次4     批次7     批次12    批次14    批次15    批次16    批次19    批次23    批次35
```

每批合并的红线：正测试 100% 不回归、std 零回归、`cargo test` 2,188 全过、系统测试 40/40。冲刺采用 4-agent 并行 workflow（git worktree 隔离）批量实现语义检查，30 分钟心跳测量汇报。

剩余未通过的负测试（237/20,069 ≈ 1.2%）已全部定级：**X 类官方矛盾 160 项**（13 章 CPointer 转换组 144 项 + 各章元数据矛盾 16 项）、**L 类类型系统深度项 54 项**（重载决议 / 分支公共超类型 / 整型 match 穷尽 / `as` 相容性等）、**G 类文法/宏/名称解析 8 项**、**R 类运行时机制 4 项**、**W 类多文件包结构伪负例 11 项**。正测试零失败；逐项分类与证据见 `.compat-run/final-report/classification.md`。

std 套件的 1,441 个文件中，28 个非通过计数由两部分构成：10 个官方负例（macro_errors / invalid_syntax / foreignclass / strategy_wrong_target / random 整型越界等）被编译器**正确拒绝**，以及 18 个因缺少对应的跨文件/派生结构而结构性跳过（deriving / argOpt）。

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

### 冲刺新增的编译期诊断与细则（批次 15–19）
- **原始标识符** — 反引号标识符（如 `` `type` ``）的词法识别与保留字冲突处理
- **unsafe 上下文检查** — `@C` / `foreign` / `CFunc` 调用必须在 `unsafe` 上下文中，非 unsafe 处调用报错
- **@C/CFunc/CType 编译期细则** — 结构体布局与跨语言参数/返回类型的编译期可判定约束
- **extend 泛型实参校验** — `extend` 声明上的泛型实参个数与约束校验
- **构造器成员形参可见性与可变性** — `let` / `private` 成员形参、命名形参一致性、可变性规则
- **可变闭包捕获不作一等值** — 捕获可变变量的闭包不可作为一等值逃逸
- **字符串插值内类型诊断** — 插值表达式内部的类型错误与未声明调用检查
- **并发内置类型编译期约束** — Future / spawn / ThreadLocal / Atomic 系列的类型约束
- **枚举语义** — 枚举不可被继承、同名构造器冲突检测
- **字面量细则** — 整数字面量的进制前缀不可用于初始化 `Float` 注解；浮点字面量指数部分仅允许负号

### 冲刺新增的根因修复与检查族（批次 20–35）
- **泛型约束实例化求解** — 类型别名继承原类型约束、实例方法调用点显式类型实参的约束求解、接口作类型实参须实现其无默认体静态函数（2.4.1(9) / 5.3.2(5) / 9.5.2(5)）
- **重载决议歧义判定** — 多候选同等匹配歧义：默认值形参不参与特异性比较（5.3.2(9)）、命名实参字面量适配歧义（10.1.3.2）、变长实参糖歧义（5.3.4(11)）、泛型形参名归一后的同名签名
- **`--checked` 运行时检查模式** — 按需插入整数溢出 / 数组越界 / 断言 `trap` 检查，用于 `@Mode:run` 负例的运行期判定
- **AST 根因变体根治** — `Type::VArray(elem, len)`、`Expr::FloatSuffixed`、`Expr::IntegerOverflow`、`Expr::GenericFunctionRef`，一次性使长度一致性/浮点后缀/i64 溢出/泛型函数作值四类此前不可判语义变得可判
- **方法元数与穷尽性检查族** — 调用缺参元数、无选择器枚举 match 穷尽性、match 标量/枚举公共超类型、`inout` 实参等检查

### 内存与运行时
- **内存管理** — 堆分配器（bump + free-list）：默认 CHIR 后端为只进不出的 bump 分配，RC / Mark-Sweep 为已实现但未接线的库存代码，按「每个 WASM 实例一次运行」设计；长驻/大内存程序需先解决回收（详见「已知限制」与 `.compat-run/gc-research.md`）
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
cargo test                     # 2,188 个单元/集成测试（全部通过）
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

// 类与继承（覆写需父方法为 public open）
open class Animal {
    var name: Int64;
    init(name: Int64) { this.name = name }
    public open func speak(): Int64 { return 0 }
}

class Dog <: Animal {
    init(name: Int64) { super(name) }
    public override func speak(): Int64 { return 1 }
}

// 枚举与模式匹配
enum Shape {
    | Circle(Int64)
    | Square(Int64)
}

func area(s: Shape): Int64 {
    match (s) {
        case Shape.Circle(r) => r * r * 3
        case Shape.Square(s) => s * s
    }
}

// 错误处理
func safeDivide(a: Int64, b: Int64): Result<Int64, String> {
    if (b == 0) { return Err("division by zero") }
    return Ok(a / b)
}

func unwrapOr(r: Result<Int64, String>, fallback: Int64): Int64 {
    match (r) {
        case Ok(v) => v
        case Err(_) => fallback
    }
}

func maybe(b: Bool): Option<Int64> {
    if (b) { return Some(7) }
    return None
}

// 测试断言
main(): Int64 {
    let fib = fibonacci(10)
    @Assert(fib, 55)

    let result = unwrapOr(safeDivide(10, 2), 0)
    @Assert(result, 5)

    let opt = maybe(false) ?? 42
    @Assert(opt, 42)

    let dog = Dog(1)
    @Assert(dog.speak(), 1)

    let square = area(Shape.Square(4))
    @Assert(square, 16)

    let pair = Pair<Int64, Int64>(1, 2)
    @Assert(pair.first + pair.second, 3)

    return fib + result
}
```

> 以上示例已用 `target/release/cjwasm` 编译并经 wasmtime 运行验证（`main` 返回 60，全部 `@Assert` 通过）。

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

> 以下数据测于 **2026-11-23（兼容冲刺前）**，当时代码量 64.7k 行、测试 1,320 个；冲刺新增约 5.5 万行语义检查后尚未重测（要求 `cargo-llvm-cov` 工具链）。

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

### 单元 & 集成测试 (2,188 个全部通过)

```bash
cargo test
# 1,543 passed (单元测试) + 631 passed (集成测试) + 14 passed (标准库测试)
```

| 类别 | 测试数 | 说明 |
|------|--------|------|
| **语义分析** | 642 | 类/接口/成员/构造器规则 + 表达式类型、运算符、重载决议与泛型约束细则 —— 兼容冲刺主力 |
| **语法解析** | 416 | 声明/语句/表达式/模式解析（含属性语法、修饰符、@C 保留） |
| **CHIR 降级** | 161 | AST → CHIR 表达式/语句降级、CHIR 层类型推断 |
| **代码生成** | 91 | WASM 生成、CHIR CodeGen、宏编译 |
| **词法分析** | 77 | Token 识别、整数后缀、十六进制浮点、嵌套块注释、原始标识符 |
| **编译管线** | 57 | 端到端编译流程、cjpm 工程解析 |
| **优化器/单态化** | 41 | 常量折叠、死代码消除、泛型实例化 |
| **元数据** | 31 | 标准库类型信息 |
| **类型检查** | 12 | 变量解析、类型推断 |
| **AST/内存** | 16 | 语法树构造、内存分配器 |
| **集成测试** | 631 | 完整编译管线（源码 → WASM） |
| **标准库测试** | 14 | std 模块编译验证 |

### 官方套件兼容测试

`third_party/cangjie_test`（git submodule）包含两类官方测试：

- **Conformance 编译器套件**（29,705 文件 / 17 章规范）：正测试（5,132）必须编译成功，负测试（20,069）必须编译失败 —— 当前 **99.1%**（正 **100%** / 负 98.8%）
- **std 标准库 API 套件**（1,441 文件 / 33 个 std 模块）：L1+L2 源文件可编译验证 —— 当前 **99.3%**（L1 **100%** / L2 99.2%）

详细口径与冲刺进展见前文「官方测试套件兼容性」一节。

### 已知限制

1. **剩余负测试构成** — 237 个未通过负例（占 20,069 的 1.2%）全部为负测试，正测试 5,132/5,132 零失败；按类别为 **X 类官方矛盾 160 + L 类类型系统深度 54 + G 类文法/宏 8 + R 类运行时 4 + W 类多文件伪负例 11**。std 套件 1,413/1,441（99.3%），28 个非通过中 10 个是官方负例被正确拒绝、18 个为结构跳过（deriving / argOpt），并非编译器缺陷。逐项分类与证据见 `.compat-run/final-report/classification.md`
2. **官方矛盾（X 类 160 项，不可修）** — 13 章 CPointer 转换组 144 项：`a04/gen/cvt.cj.tpl` 正例与 `cvt_neg.cj.tpl` 负例在同样的非 `unsafe` 上下文生成逐字相同的转换语句（132 项存在逐字对应的官方正例 + 12 项 `Src==Dst` 恒等转换），前端无法区分；另有 16 项跨章元数据矛盾（无符号/Bool 一元取负、`!Bool`、range 正例误标负例、setter 尾值、接口扩展等）。均已冻结、不再尝试
3. **类型系统深度项（L 类 54 项）** — 需重载决议引擎（候选集 / 最精确匹配 / 命名实参 / 默认值 / 变长糖 / 泛型实例化的统一求解）、分支最小公共超类型（用户类型与接口集合求交）、整型 match 值域穷尽、`as` 类型相容性、lambda 形参注解标记与上下文推断、接口默认方法 this-type、枚举裸名/构造器一等值歧义等——多需重基建，单规则收益低且易误伤正例
4. **文法/宏/运行时/多文件（G 8 + R 4 + W 11 = 23 项）** — 宏声明表、注解名解析、import 场景下的未定义符号检查（G）；`throw` 须在全局/字段/static 初始化期执行并 trap 的 `@Mode:run` 语义（R）；以及错误源自 `custom.py`/目录布局驱动的多文件包构建、单文件 harness 无法复现的伪负例（W）
5. **WASM 垃圾回收尚未接线** — 默认 CHIR 后端为只进不出的 bump 分配器，`src/memory.rs` 的 free-list / 引用计数 / Mark-Sweep 均为已实现但从未被发射调用的库存代码，且全项目无 `memory.grow`，长运行约 256KB 分配即 OOB trap。由于 5132 正测试与 1413 std 红线均为「只编译不运行」判定、示例每进程新建实例跑完即弃，GC 对红线零影响；中期若需长驻模块，建议直接评估 WasmGC（wasmtime 51 已完整支持、wasm-encoder 0.220 可发射），而非自研线性内存 GC —— 详见 `.compat-run/gc-research.md`
6. **其他工程边界** — `std/` 包（97 个标准库文件）可编译，但复杂嵌套泛型（如 `Map<K, Tuple<Array<...>>>`）生成的 WASM 在验证时仍有类型不匹配；泛型单态化部分仍用桩代码；宏展开能力有限

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
│   │   └── type_.rs       # Type 枚举（31 个类型）
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
| **内存** | 堆分配（bump + free-list），按单次运行设计；RC/Mark-Sweep 已实现未接线 | ✅ |
| **运算符** | 算术, 比较, 逻辑, 位运算, 幂运算(**), !in, 字符串插值, 类型转换(as), 可选链(?.), 方法重载 | ✅ |
| **集合** | HashMap(put/get/remove/containsKey/size), HashSet(add/contains/size), ArrayList, LinkedList, ArrayStack | ✅ |
| **并发桩** | spawn(同步执行), synchronized(直通), AtomicInt64/AtomicBool, Mutex/ReentrantMutex | ✅ |
| **标准库** | 数学函数(sin/cos/exp/log/...), 字符串操作(trim/startsWith/endsWith/contains/indexOf/replace), 数组方法(clone/isEmpty/slice), now() 时间戳 | ✅ |
| **格式化** | Int64.format("x"/"b"/"o"), Float64.format("2f"), toString() | ✅ |
| **WASI** | println/print, 时间(now), 随机数, 排序(sort), 进程退出(exit) | ✅ |
| **测试** | @Assert(a, b), @Expect(a, b), 单参数布尔形式, 类型自动协调 | ✅ |
| **外部接口** | extern func + @import("module", "name") | ✅ |
| **C 互操作** | @C struct/class、foreign func、CFunc/CType 类型规则、@C 构造器细则（13 章编译期部分） | ✅ |
| **原始标识符** | 反引号标识符（如 `` `type` ``），与保留字冲突处理 | ✅ |
| **unsafe 上下文** | `@C` / `foreign` / `CFunc` 调用须处于 `unsafe` 上下文，否则编译期报错 | ✅ |
| **extend 校验** | `extend` 声明的泛型实参个数与约束校验 | ✅ |
| **构造器形参** | 成员形参可见性/可变性（`let` / `private`）、命名形参一致性 | ✅ |
| **闭包捕获** | 捕获可变变量的闭包不可作为一等值逃逸 | ✅ |
| **插值诊断** | 字符串插值内部类型错误与未声明调用检查 | ✅ |
| **并发类型约束** | Future / spawn / ThreadLocal / Atomic 系列的编译期类型约束 | ✅ |
| **枚举语义** | 枚举不可继承、同名构造器冲突检测 | ✅ |
| **字面量细则** | 进制前缀不可初始化 `Float` 注解；浮点指数仅允许负号 | ✅ |
| **泛型约束求解** | 类型别名继承约束、实例方法显式类型实参约束、接口作类型实参的静态函数实现检查 | ✅ |
| **重载决议** | 多候选同等匹配歧义判定（默认值/命名实参/变长糖/泛型形参归一） | ✅ |
| **运行时检查** | `--checked` 整数溢出 / 数组越界 / 断言 trap | ✅ |
| **AST 根因变体** | VArray 长度 / 浮点后缀 / i64 溢出 / 泛型函数作值 | ✅ |
| **穷尽性与元数** | 无选择器枚举 match 穷尽、标量公共超类型、调用缺参元数 | ✅ |

完整语法规格见 [`docs/spec.md`](docs/spec.md)。

## 性能基准

CJWasm 编译速度和输出大小与仓颉原生编译器 (cjc) 的对比：

| 指标 | cjwasm | cjc | 倍率 |
|------|--------|-----|------|
| 编译速度（小规模） | ~5.4ms | ~184ms | **~34x 快** |
| 编译速度（中规模） | ~6.4ms | ~189ms | **~29x 快** |
| 输出大小（小规模） | 8.2 KB | 1034 KB | **~126x 小** |
| 输出大小（中规模） | 9.2 KB | 1036 KB | **~113x 小** |

> cjwasm 是轻量级前端编译器，仅生成 WASM 字节码；cjc 是完整原生编译器，生成含标准库的可执行文件。两者定位不同。
> 测于 2026-11-26（Apple Silicon / Darwin 27，cjc 1.0.5，macOS 14.4 SDK sysroot）；端到端进程耗时口径。

### CHIR 优化效果

在 `bench_heavy.cj`（100,000 次密集小函数调用：identity / double / square / addOne / Counter.get 等）上对比：

| 版本 | wasmtime 耗时 | WASM 大小 |
|------|---------------|-----------|
| CHIR + 优化（默认） | 13.84 ms | 9116 B |
| CHIR 无优化（`NO_CHIR_OPT=1`） | 19.92 ms | 9132 B |

- **wasmtime（JIT）加速 1.44x** — JIT 会对两个版本都做本地优化，差异被部分抹平
- 解释器模式（wasm-interp，无 JIT）更能反映指令级差异，但当前本机 wasm-interp 构建未含 WASI 支持，未能复测

### 运行时性能（wasmtime vs cjc native）

| 规模 | wasmtime | cjc native | 比率 |
|------|----------|------------|------|
| 小规模（27 行） | 8.04 ms | 22.43 ms | wasm 2.8x 快 |
| 中规模（109 行） | 8.23 ms | 22.21 ms | wasm 2.7x 快 |
| 大规模（124 行） | 8.37 ms | 22.46 ms | wasm 2.7x 快 |
| 高负载（89 行） | 13.76 ms | 73.95 ms | wasm 5.4x 快 |

> 测于 2026-11-26（Apple Silicon / Darwin 27，cjc 1.0.5，macOS 14.4 SDK sysroot）；端到端进程耗时（含启动）：wasmtime 时间包含 JIT 编译开销，native 包含进程启动开销。

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

# 运行全部测试（2,188 个）
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
