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
