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
