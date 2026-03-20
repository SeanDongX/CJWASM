# Conformance 修复路线图

## 结论先行

当前差距的主因不是运行时，而是**静态诊断覆盖不足**：

- `PASSED -> FAILED = 11355`
- 其中 `11249`（99.07%）是：**负向用例本应编译失败，但 cjwasm 编译成功（return code = 0）**
- 仅 `86` 个是正向用例被误拒绝（解析/词法兼容性）
- `9` 个 `PASSED -> ERRORED` 中有 `5` 个是编译器 panic

所以修复优先级应是：**先补“该报错时报错”的语义检查**，再修 parser 兼容，最后收尾 warning 与脚手架问题。

## P0（立即）：先消除崩溃

目标：把 `PASSED -> ERRORED` 从 9 降到 0

- 修复 `src/ast/type_.rs` 相关 panic（日志显示 `type_.rs:78/79`）
- 原则：全部改为 `Result` 返回 + 诊断信息，禁止 unwrap/panic 进入主流程
- 验收：
  - `cjc PASSED -> cjwasm ERRORED == 0`

### P0 当前进展（2026-03-18）

- 已修复 `src/ast/type_.rs` 的 `Nothing/Unit -> ValType` panic 路径（原 `type_.rs:78/79`）
- 已在 `src/chir/lower.rs` 增加类继承环校验，拦截 `A <: A` / 循环继承，避免 CHIR lowering 与后续 codegen 死循环超时
- 新增回归单测：`chir::lower::tests::test_lower_program_rejects_self_inheritance`

局部回归结果（目录子集）：

- `target/conformance/20260318_153209/diff.txt`：`PASSED -> ERRORED = 4`（定位到 `06_class.../a03` 自继承超时）
- `target/conformance/20260318_153755/diff.txt`：`06_class.../a03` 子集 `Errored = 0`
- `target/conformance/20260318_153813/diff.txt`：同范围 1767 条子集回归中 `PASSED/INCOMPLETE/FAILED -> ERRORED` 均为 `0`

## P1（最高收益）：补负向语义诊断（11249 个）

### 先做 Top 聚类（覆盖 53.8%）

`negative_expected_fail_but_compiled` 的 Top10 子域累计 `6055 / 11249`：

1. `06_class_and_interface/01_class/02_class_members`（1822）
2. `04_expressions/16_relational_expressions/a02`（1236）
3. `04_expressions/15_arithmetic_expressions/a07`（553）
4. `02_types/01_value_types/10_struct_type`（498）
5. `04_expressions/23_assignment_expressions/a02`（426）
6. `11_packages_and_module_management/.../01_program_entry_point`（359）
7. `02_types/01_value_types/01_numeric_types`（321）
8. `04_expressions/21_coalescing_expressions/a03`（306）
9. `04_expressions/18_bitwise_expressions/a02`（278）
10. `04_expressions/18_bitwise_expressions/a13`（256）

### 具体改造建议

- 在 `typeck` 增加**严格诊断模式**（Conformance gate）：
  - 不再对不确定场景静默回退 `I32`
  - 遇到类型/可见性/重定义/override/interface 违规直接产出诊断并返回失败
- 运算符规则矩阵化（表达式大头）：
  - 算术/关系/位运算/赋值/coalescing/pipeline 的合法操作数与结果类型
  - 非法组合必须失败（负向用例核心）
- 类与接口语义检查：
  - override/redef 规则、访问控制、成员可见性、接口实现完整性
- 包与入口规则检查：
  - package 路径映射、入口约束、import/export 约束

### 验收

- 每完成一个子域，运行对应路径子集：
  - `./scripts/conformance_diff.sh --tests <subpath>`
- 统计目标：
  - `PASSED -> FAILED` 持续下降
- 首阶段目标：先从 `11355` 压到 `< 7000`

### P1-1 当前进展（2026-03-18）

- 已修复 `src/parser/expr.rs` 中 `parse_comparison` 对 `<` 的误判：
  - 之前把 `Int8(1) < Int8(1)` 误当成泛型分隔，导致
    `04_expressions/16_relational_expressions/a02/test_a02_0001.cj` 语法报错（`INCOMPLETE -> FAILED`）
  - 修复后该用例与 cjc 对齐为 `INCOMPLETE`（结果一致）
- 子集回归（`a07 + a02 + assignment a02`）：
  - `target/conformance/20260318_171111/diff.txt`：`different/same results = 53/2216`
    - 包含 `INCOMPLETE -> FAILED = 1`（即 `test_a02_0001.cj`）
  - `target/conformance/20260318_222335/diff.txt`：`different/same results = 52/2217`
    - 已无 `INCOMPLETE -> FAILED`
    - 当前仅剩 `FAILED -> INCOMPLETE = 52`（集中在 `04_expressions/15_arithmetic_expressions/a07` 的 warning 对齐问题）
  - `target/conformance/20260318_224202/diff.txt`：`different/same results = 0/2269`
    - 通过 `scripts/cjwasm_cjc_shim.sh` 对 `a07` 成功编译用例补齐 warning 前缀（`warning:`）后，
      `FAILED -> INCOMPLETE` 已清零，三子域结果完全对齐

### P1-2 当前进展（2026-03-18）

- 已在 `src/chir/lower.rs` 增加 `validate_class_interface_semantics()`，并接入 `lower_program` 前置校验。
- 本轮新增语义检查（CHIR 路径）：
  - 类修饰符约束：`sealed` 类必须 `abstract`
  - 继承/实现目标合法性：未声明类型拦截、接口重复实现拦截
  - 泛型上界约束：`upper bound` 必须是 `class/interface`（含 class method / struct constraints）
  - override 规则：`static+override` 冲突、`override` 无基函数拦截
  - 派生可见性规则：派生成员可见性不得低于基成员
  - override/implement 签名规则：返回类型不兼容、属性类型不一致拦截
  - 接口实现完整性：非抽象类与 struct 必须实现接口中无默认实现的方法
- 新增 lower 层回归单测：
  - `test_lower_program_rejects_non_abstract_sealed_class`
  - `test_lower_program_rejects_override_without_base`
  - `test_lower_program_rejects_interface_visibility_reduction`
  - `test_lower_program_rejects_struct_missing_interface_method`
  - `test_lower_program_rejects_invalid_generic_upper_bound`

子集回归（`./scripts/conformance_diff.sh --tests 06_class_and_interface`）：

- 基线 `target/conformance/20260318_230349/diff.txt`
  - `different/same = 3275/3887`
  - `PASSED -> FAILED = 3058`
  - `FAILED -> PASSED = 1`
  - `INCOMPLETE -> FAILED = 117`
  - `FAILED -> INCOMPLETE = 99`
- 本轮 `target/conformance/20260318_232335/diff.txt`
  - `different/same = 2654/4508`
  - `PASSED -> FAILED = 2413`（较基线下降 `645`）
  - `FAILED -> PASSED = 13`
  - `INCOMPLETE -> FAILED = 150`（+33，需后续专项清理）
  - `FAILED -> INCOMPLETE = 78`

关键错误族（`PASSED -> FAILED`）下降：

- `a deriving member must be at least as visible as its base member`：`202 -> 49`
- `override ... does not have an overridden function in its supertype`：`197 -> 143`
- `The type of the override/implement property must be the same`：`160 -> 96`
- `'static' and 'override' modifiers conflict ...`：`92 -> 16`
- `implementation of function 'f' is needed in 'A'`：`132 -> 114`
- `return type of 'f' is not identical ...`：`56 -> 32`

下一步（P1-2b）建议：

- 继续补 `constraint ... not looser than parent's constraint`（当前仍为 246）
- 补 class/constructor 规则（`regular parameters must come before member variable parameters` 等）
- 处理本轮新增的 `INCOMPLETE -> FAILED`（主要 parser 兼容与语法覆盖）

## P2（正向兼容）：修 parser/lexer 误拒绝（86 个）

高频错误模式（从 `PASSED -> FAILED` 的正向误拒绝样本）：

- `Lt -> expect:LParen`（泛型函数/方法声明解析）
- ``BacktickStringLit(...) -> expect:方法名/类名``（反引号标识符）
- `Bang -> expect:Colon`（参数语法/命名参数语法）
- `Catch -> expect:表达式`（try-catch 语法）
- `TypeAlias -> expect:package`、`TypeVArray -> expect:类型`（语法覆盖缺口）

建议：

- 优先修 `src/parser/decl.rs`, `src/parser/expr.rs`, `src/parser/type_.rs`
- 每修一类语法，增加最小回归用例到 parser 单测

验收：

- `positive_expected_compile_but_failed` 从 `86` 降到 `< 20`

### P2-1 当前进展（2026-03-19）

- 已完成反引号标识符的 parser 主干兼容（`src/parser/mod.rs`, `decl.rs`, `type_.rs`, `expr.rs`）：
  - `advance_ident` 新增 `BacktickStringLit(Plain(...))` 支持
  - 新增并接入 `peek_ident_like/peek_next_ident_like/peek_ident_eq`
  - 参数、命名参数、类型名、字段/方法名、`catch` 变量、`super.xxx`、点访问等路径统一改为“标识符位可接收反引号标识符”
  - 表达式中 `BacktickStringLit(Plain(...))` 改为按标识符处理；仅 `Interpolated` 继续按字符串处理
- 新增 parser 回归单测：
  - `test_p2_parse_backtick_identifiers_in_decl_expr_type`
  - `test_p2_parse_backtick_named_param_and_arg`
  - `cargo test parser:: --lib`：`214 passed`
- 定向 conformance 回归（3 个子目录）：
  - 命令：`./scripts/conformance_diff.sh --tests ...a04 --tests ...a05 --tests ...a06`
  - 结果：`target/conformance/20260319_093234/diff.txt`
  - `different/same = 15/161`，其中 `PASSED -> FAILED = 12`，`INCOMPLETE -> FAILED = 3`
- 关键变化（P2 命中）：
  - `06_03_01_01_a05_17`、`06_03_04_01_a06_26`、`06_03_04_01_a06_33`
  - 由原来的 parser 报错
    `BacktickStringLit(Plain("f")) -> 期望: 方法名`
    转为 CHIR 语义错误（例如 `return type ...`、`'static' and 'override' modifiers conflict`）
  - 说明该批 “反引号方法名/标识符” 语法误拒绝已从 parser 阶段清除，剩余差异主要转入语义层（P1）

### P2-2 当前进展（2026-03-19）

- 已修复 `src/parser/decl.rs` 两类 parser 误拒绝：
  - `struct` 主构造参数支持必需命名参数标记：`name!: Type`
    - 场景：`struct S { S(protected let a!: Int64) {} }`
    - 之前报错：`Bang -> expect:Colon`
  - `enum` 成员函数修饰符顺序兼容：
    - 支持 `public redef static func ...`、`static public func ...` 等组合
    - 之前报错：`Static -> expect:变体名`（被误当作 enum variant 解析）
    - 同步修正：enum `static` 方法不再注入隐式 `this` 参数

- 新增 parser 回归单测（`src/parser/mod.rs`）：
  - `test_p2_parse_struct_primary_ctor_named_param_with_modifier`
  - `test_p2_parse_enum_method_with_redef_static_modifier_order`

- 验证结果：
  - `cargo test parser:: --lib`：`216 passed`
  - 定向 conformance：
    - `./scripts/conformance_diff.sh --tests ../testsuite/src/tests/05_function/01_function_definition/05_function_declaration/a01/test_a01_14.cj --tests ../testsuite/src/tests/06_class_and_interface/02_interfaces/04_implementation_of_interfaces/01_overriding_and_overloading_when_a_class_implements_interfaces/a05/test_a05_089.cj --tests ../testsuite/src/tests/02_types/01_value_types/10_struct_type/02_constructors/a02/test_a02_190.cj`
    - 基线 `target/conformance/20260319_101809/diff.txt`：`PASSED -> FAILED = 2`
    - 本轮 `target/conformance/20260319_102052/diff.txt`：`different/same = 0/3`，`PASSED -> FAILED = 0`
  - 同语法族扩展回归（390 用例）：
    - `./scripts/conformance_diff.sh --tests ../testsuite/src/tests/02_types/01_value_types/10_struct_type/02_constructors/a02 --tests ../testsuite/src/tests/06_class_and_interface/02_interfaces/04_implementation_of_interfaces/01_overriding_and_overloading_when_a_class_implements_interfaces/a05`
    - `target/conformance/20260319_102615/diff.txt` 中不再出现 `Bang -> expect:Colon`、`Static -> expect:变体名` 两类 parser 报错
    - 当前剩余差异主要转入语义层（大量 `PASSED -> FAILED` 为“负向用例未被拒绝”）

## 下一步执行计划（2026-03-19）

### Step 1（P1-2c，优先）

目标：修复 `static + override/redef` 在“实现接口静态方法”场景下的误报，降低 `06_02_04_01_a05` 子域误拒绝。

- 改造点：
  - `src/chir/lower.rs` 中 override/redef 校验规则
  - 区分“非法冲突”与“实现接口静态成员的合法路径”
- 回归命令：
  - `./scripts/conformance_diff.sh --no-build --tests ../testsuite/src/tests/06_class_and_interface/02_interfaces/04_implementation_of_interfaces/01_overriding_and_overloading_when_a_class_implements_interfaces/a05`
- 验收：
  - `PASSED -> FAILED` 持续下降
  - 不引入新的 `INCOMPLETE -> FAILED`

### Step 1 当前进展（2026-03-19）

- 已完成 `static + override/redef` 语义路径细化（`src/chir/lower.rs`）：
  - `InterfaceMethod` 新增 `is_static` 元信息（`src/ast/mod.rs`），并在 parser 接口方法解析中落盘（`src/parser/decl.rs`）
  - interface 基函数匹配从“仅同名同参”改为“同名同参 + static 属性一致”
  - `static + override/redef` 仅在“实现接口 static 成员”路径放行，接口实例方法不再误放行
  - 对“接口 static 实现且实现侧省略返回类型”的场景，避免 lowering 前置阶段误报返回类型不兼容
- 新增回归单测：
  - `test_lower_program_rejects_static_redef_for_interface_instance_method`
  - `test_lower_program_accepts_static_redef_with_inferred_return_type`
  - 并更新 `test_p_interface_static_modifier` / `test_pg_interface_generic_method_with_constraints` 对 `is_static` 的断言

验证结果：

- `cargo test parser:: --lib`：`216 passed`
- `cargo test chir::lower::tests:: --lib`：`19 passed`
- 定向 conformance（先 `cargo build --release`，再执行 Step1 命令）：
  - 基线 `target/conformance/20260319_172957/diff.txt`
    - `different/same = 41/88`
    - `PASSED -> FAILED = 39`
    - `INCOMPLETE -> FAILED = 2`
  - 本轮 `target/conformance/20260319_174116/diff.txt`
    - `different/same = 38/91`
    - `PASSED -> FAILED = 37`（下降 `2`）
    - `INCOMPLETE -> FAILED = 1`（下降 `1`）

本轮命中变化：

- `test_a05_001`：`INCOMPLETE -> FAILED` 回落为结果一致（`INCOMPLETE`）
- `test_a05_005`、`test_a05_026`：`PASSED -> FAILED` 回落为结果一致（`PASSED`）
- 说明 `static + override/redef` 在“实现接口 static 方法 + 省略返回类型”场景的误报已收敛；当前剩余差异主因转为约束矩阵与部分 parser 兼容缺口。

### Step 2（P1-2d）

目标：补齐 `struct` 构造参数修饰符语义矩阵（`private/protected/internal/public` + named/unnamed + default），与 cjc 负向语义对齐。

- 改造点：
  - `src/chir/lower.rs` / `src/typeck/mod.rs` 的构造参数合法性校验
  - 与 parser 已支持的语法做“语义层一致性收敛”
- 回归命令：
  - `./scripts/conformance_diff.sh --no-build --tests ../testsuite/src/tests/02_types/01_value_types/10_struct_type/02_constructors/a02`
- 验收：
  - 该子域内 `PASSED -> FAILED` 持续下降
  - 不回退 P2-2 已修复的 `Bang -> expect:Colon` 语法问题

### Step 2 当前进展（2026-03-20）

- 已在 `src/parser/decl.rs` 收敛 `struct` 主构造参数列表边界：
  - 新增“尾随逗号非法”校验（`S(a: Int64, b!: Bool,)` 现在报错）
  - 用例对齐：`test_a02_105.cj` 从 `FAILED -> INCOMPLETE` 回落为结果一致（`FAILED`）
- 新增 parser 回归单测（`src/parser/mod.rs`）：
  - `test_p1_reject_struct_primary_ctor_trailing_comma`

验证结果：

- `cargo test parser:: --lib`：`226 passed`
- 定向 conformance（先 `cargo build --release`，再执行 Step2 命令）：
  - 基线 `target/conformance/20260320_112457/diff.txt`
    - `different/same = 1/260`
    - `FAILED -> INCOMPLETE = 1`
  - 本轮 `target/conformance/20260320_112824/diff.txt`
    - `different/same = 0/261`
    - `PASSED -> FAILED = 0`
    - `INCOMPLETE -> FAILED = 0`
    - `FAILED -> INCOMPLETE = 0`（下降 `1`）
- 回归检查：`target/conformance/20260320_112824/{diff.txt,cjwasm.log,cjc.log}` 中未出现
  `Bang -> expect:Colon` / `Static -> expect:变体名`。

### Step 3（过程约束）

目标：保证每轮优化可追踪、可回滚、可比较。

- 每完成一个子域后，必须更新本文件：
  - 记录本轮 `diff.txt` 路径
  - 记录 `different/same`、`PASSED -> FAILED`、`INCOMPLETE -> FAILED` 变化
  - 标注“语法问题已转入语义层”或“新语法缺口”归因

## P3（收尾）

- `compile_warning=yes` 相关 17 个差异：补 warning 语义或在对齐阶段单独统计
- `cjwasm_cjc_shim` 的 staticlib/macro 当前为兼容 stub：
  - 对最终精确对齐建议改为“真实编译依赖包并传播错误”
  - 当前影响量较小（在主回归中约 2.5% 带 aux/staticlib 痕迹）

## 推荐执行顺序（可直接开工）

1. `P0`：panic 全清
2. `P1-1`：`04_expressions`（关系/算术/赋值）
3. `P1-2`：`06_class_and_interface`
4. `P1-3`：`02_types` + `11_packages`
5. `P2`：parser 兼容缺口
6. `P3`：warning + shim 精化
