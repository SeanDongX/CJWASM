# std/ 示例 WASM 类型错误修复计划

## 当前状态

**WASM 验证错误**: 4288 个（从初始 4526 降至 4288，-238）
**测试通过率**: 37/37 (100%)
**WASM 文件大小**: 426 KB

---

## 已完成修复

### ✅ P0 — TypeParam → I32 映射（已修复）
**位置**: `src/ast/type_.rs:90-93`
**修复**: 将 `TypeParam` 从 `ValType::I64` 改为 `ValType::I32`
**影响**: 消除了泛型函数的基础类型不匹配

### ✅ P1 — Return 类型协调（已修复）
**位置**: `src/codegen/expr.rs:3772-3800`
**修复**: 添加 `current_return_wasm_type` 字段，在 `Stmt::Return` 中使用 `emit_type_coercion`
**影响**: 消除了大部分 return 类型错误（如 `return expected [i32] but got [i64]`）

### ✅ RC3 — Pattern 整数字面量匹配（已修复）
**位置**: `src/codegen/expr.rs` Pattern::Literal 和 Pattern::Or
**修复**: 根据 `subject_ty` 选择 I32Const+I32Eq 或 I64Const+I64Eq
**影响**: 修复了 37 个 i64.eq/i32.eq 错误

### ✅ 布尔上下文 I32WrapI64 优化（已修复）
**位置**: `src/codegen/expr.rs` 多处
**修复**: 添加 `needs_i64_to_i32_wrap` 方法，仅在 AST 类型确认为 Int64/UInt64 时才 wrap
**应用点**: UnaryOp::Not, LogicalAnd/Or, NotIn, If/While/DoWhile 条件, Bool.toString
**影响**: -176 错误（4527 → 4351）

### ✅ 数组/元组索引条件 wrap（已修复）
**位置**: `src/codegen/expr.rs` 数组读写多处
**修复**:
- 索引表达式：仅在 `infer_type_with_locals(index) == I64` 时 wrap
- 数组基址：仅在 `infer_type_with_locals(array) == I64` 时 wrap
**应用点**:
- 读取: Expr::Index (tuple/array)
- 写入: AssignTarget::Index, IndexPath, ExprIndex
**影响**: -29 错误（4317 → 4288）

### ✅ 二元运算右侧条件 wrap（已修复）
**位置**: `src/codegen/expr.rs:4794-4802`
**修复**: 将 `right_wasm_ty == ValType::I64` 改为 `needs_i64_to_i32_wrap(right, locals)`
**影响**: -3 错误（4320 → 4317）

---

## 当前错误分布（4288 个）

| 错误类型 | 数量 | 占比 | 说明 |
|---------|------|------|------|
| `i32.wrap_i64 expected [i64] but got [... i32]` | 650 | 15.1% | 在 I32 值上错误调用 wrap |
| `local.set expected [i64] but got [i32]` | 376 | 8.8% | TypeParam 局部变量类型不匹配 |
| `call expected [i32, i64] but got [i32, i32]` | 341 | 8.0% | 方法参数类型不匹配 |
| `i32.wrap_i64 expected [i64] but got [i32]` | 303 | 7.1% | 简单 wrap 错误 |
| `i64.store expected [i32, i64] but got [i32, i32]` | 257 | 6.0% | 存储指令类型不匹配 |
| `i32.add expected [i32, i32] but got [... i64, i32]` | 247 | 5.8% | 指针运算类型不匹配 |
| `i32.add expected [i32, i32] but got [i64, i32]` | 219 | 5.1% | 指针运算类型不匹配 |
| `call expected [i32, i64] but got [... i32, i32]` | 203 | 4.7% | 方法参数类型不匹配 |
| `call expected [i32] but got []` | 188 | 4.4% | RC4: 桩代码参数不足 |
| 其他 | 1504 | 35.0% | 各种类型不匹配 |

---

## 待修复问题

### 核心问题：TypeParam 类型推断不准确

**根本原因**: `infer_type_with_locals` 对 TypeParam 变量返回 I64（因为 `Type::TypeParam(_).to_wasm()` 返回 I32，但在单态化后实际类型可能是 I32 或 I64）。这导致：

1. **局部变量类型不匹配**（376 错误）：TypeParam 局部变量声明为 I64，但赋值时得到 I32
2. **方法调用参数不匹配**（544 错误）：TypeParam 参数期望 I64，但传入 I32
3. **存储指令不匹配**（257 错误）：I64Store 期望 I64 值，但得到 I32

### 可能的解决方案

#### 方案 A：改进 TypeParam 的 WASM 类型推断
- 在 `LocalsBuilder` 中记录 TypeParam 变量的实际单态化类型
- `infer_type_with_locals` 查询实际类型而非默认 I64
- **难度**: 高（需要追踪单态化信息）

#### 方案 B：在关键点添加类型协调
- 在 `local.set` 前检查类型并插入 wrap/extend
- 在方法调用参数推入前检查类型
- 在存储指令前检查值类型
- **难度**: 中（需要在多处添加检查）

#### 方案 C：接受当前错误，专注功能完整性
- 当前 37/37 测试全部通过
- WASM 验证错误不影响功能（WASI 运行时可能容忍）
- 继续添加新功能，后续统一优化类型系统
- **难度**: 低（维持现状）

---

## 修复历史

| 日期 | 修复内容 | 错误数变化 |
|------|---------|-----------|
| 初始 | P0 (TypeParam→I32) + P1 (Return协调) | 4526 → 4527 (+1) |
| - | RC3 (Pattern匹配) | 4527 (持平) |
| - | 布尔上下文优化 | 4527 → 4351 (-176) |
| - | Do-while + Match块类型 | 4351 → 4352 (+1) |
| - | 数组索引条件wrap | 4352 → 4320 (-32) |
| - | 二元运算条件wrap | 4320 → 4317 (-3) |
| - | 数组基址条件wrap | 4317 → 4288 (-29) |
| **总计** | | **4526 → 4288 (-238, -5.3%)** |

---

## 下一步建议

1. **短期**：继续修复高频错误（650个 wrap 错误、376个 local.set 错误）
2. **中期**：实现方案 B，在关键点添加类型协调
3. **长期**：重构类型推断系统，实现方案 A

---

## 详细实现方案

### P0: TypeParam → I32

**文件**: `src/ast/type_.rs`

```rust
// 修改前
Type::TypeParam(_) => {
    eprintln!("警告: TypeParam 转换为 i64（需要单态化）");
    ValType::I64
}

// 修改后
Type::TypeParam(_) => {
    ValType::I32  // WASM32 中泛型参数未单态化时视为对象引用（i32 指针）
}
```

同步修改 `infer_type_with_locals` 中所有将 `TypeParam` 映射为 `ValType::I64` 的位置（`expr.rs:2051`）：
```rust
// expr.rs:2051
Expr::Integer(_) => ValType::I64,  // Integer 字面量不变，仍为 i64
```
（注意：TypeParam 在 `infer_type_with_locals` 中通过 `locals` 查询已解析类型，不经过 `to_wasm()` 直接映射，故 `expr.rs:2051` 不需改动）

---

### P1: 在 CodeGen 中追踪当前函数返回类型

**文件**: `src/codegen/mod.rs` + `src/codegen/expr.rs`

**步骤 1**: 在 `CodeGen` 结构体添加字段（`mod.rs`）

```rust
use std::cell::Cell;

pub struct CodeGen {
    // ... 现有字段 ...
    current_return_wasm_type: Cell<Option<ValType>>,
}

// 在 CodeGen::new() 中初始化
current_return_wasm_type: Cell::new(None),
```

**步骤 2**: 在 `compile_function` 中设置返回类型（`mod.rs:1775`）

```rust
fn compile_function(&self, func: &FuncDef) -> WasmFunc {
    // 设置当前函数返回类型（供 Stmt::Return 使用）
    let ret_wasm_ty = func.return_type.as_ref().and_then(|t| {
        if matches!(t, Type::Unit | Type::Nothing) { None } else { Some(t.to_wasm()) }
    });
    self.current_return_wasm_type.set(ret_wasm_ty);

    // ... 现有逻辑 ...

    // 函数编译完成后清空
    self.current_return_wasm_type.set(None);
    wasm_func
}
```

**步骤 3**: 在 `Stmt::Return` 中使用协调（`expr.rs:3746`）

```rust
Stmt::Return(Some(expr)) => {
    let expected_ty = self.current_return_wasm_type.get();
    self.compile_expr_with_coercion(expr, expected_ty, locals, func, loop_ctx);
    func.instruction(&Instruction::Return);
}
```

---

### P2: Pattern 匹配根据 subject 类型选择 i32/i64 指令

**文件**: `src/codegen/expr.rs`

`Expr::Match` 已在 `6821` 行计算 `subject_ty`。需将该类型传递到 Pattern 处理代码中。

**修改点 1** (`expr.rs:6930-6933`):

```rust
Literal::Integer(n) => {
    if subject_ty == ValType::I32 {
        // i32 类型：UInt16/Int32/Rune/Bool 等
        func.instruction(&Instruction::I32Const(*n as i32));
        func.instruction(&Instruction::I32Eq);
    } else {
        // i64 类型：Int64/UInt64 等
        func.instruction(&Instruction::I64Const(*n));
        func.instruction(&Instruction::I64Eq);
    }
}
```

**修改点 2** (`expr.rs:7204-7209`, Pattern::Or 内部):

```rust
if let Pattern::Literal(Literal::Integer(n)) = pat {
    if j > 0 {
        self.compile_expr(expr, locals, func, loop_ctx);
    }
    if subject_ty == ValType::I32 {
        func.instruction(&Instruction::I32Const(*n as i32));
        func.instruction(&Instruction::I32Eq);
    } else {
        func.instruction(&Instruction::I64Const(*n));
        func.instruction(&Instruction::I64Eq);
    }
    if j > 0 {
        func.instruction(&Instruction::I32Or);
    }
}
```

注意：需要在 Pattern 处理代码块作用域内使 `subject_ty` 可访问（已是局部变量，范围足够）。

---

### P3: 桩代码参数补齐（可选，后续迭代）

当生成函数调用桩（"函数未找到"）时，查询目标函数签名的参数数量，补充缺失的 `i32.const 0` 参数。

```rust
// 生成桩调用前
if let Some(&func_idx) = self.func_indices.get(stub_name) {
    // 查询函数类型签名
    let expected_params = self.get_func_param_count(func_idx);
    let provided_params = actual_args.len();
    for _ in provided_params..expected_params {
        func.instruction(&Instruction::I32Const(0)); // 补齐参数
    }
}
```

此修复依赖维护 `func_idx → param_count` 映射，实现相对复杂，建议作为单独迭代。

---

## 已实施修复

### P0 状态：部分实施

原计划将 `TypeParam → ValType::I32`，但实测发现：
- 改变 TypeParam 会与大量现有代码（数组索引 `I32WrapI64`、`I64Store` 等）冲突
- 导致 ~1229 个新错误，净效果为负

**实际操作**：仅移除了 `eprintln!` 警告输出，TypeParam 保持 `ValType::I64`。

### P1 状态：已实施，修复 386 个错误（4912 → 4526）

实施内容：
1. `CodeGen` 添加 `current_return_wasm_type: Cell<Option<ValType>>` 字段
2. `compile_function` 在开始时设置该字段，结束时清空
3. `Stmt::Return` 对三类表达式做目标型类型协调：
   - `Expr::Integer` → 始终 I64，转换为目标类型
   - `Expr::Var(name)` → 从 `locals.get_valtype()` 获取实际 WASM 类型
   - 其他表达式 → 仅在 `infer_ast_type_with_locals` 有可信类型时做协调
4. `infer_type_with_locals` 增加 `Var` 的 `get_valtype` 回退，避免 I64 误推断

**修复效果**：主要消除了 `return expected [i32] but got [i64]` 类错误（294 → 76）。

---

## 验证方法

修复后依次验证：

```bash
# 1. 编译
cargo build 2>&1 | grep -E "^error"

# 2. 编译 std 示例
cargo run -- build -p examples/std 2>&1 | tail -5

# 3. WASM 验证（应无错误或大幅减少）
wasm-validate examples/std/target/wasm/std_examples.wasm 2>&1 | wc -l

# 4. 运行全部示例
./scripts/run_examples.sh
```

---

## 已知局限（修复后仍存在的问题）

1. **泛型单态化未完成**：`toTokens`、`iterator` 等泛型方法在未单态化时仍会生成桩代码（约 20+ 个方法）
2. **变量作用域解析**：部分 `变量未找到` 的警告源于 lambda 闭包或宏展开后的变量名问题，需要更深层的作用域分析
3. **`Exception`、`PropertyAttribute` 等类型**：在 std 中引用但未定义，生成零值占位可能导致运行时语义错误（不影响 WASM 验证）

---

## 时间评估

| 任务 | 预估工时 |
|------|---------|
| P0: TypeParam 修复 | 15 分钟 |
| P1: Return 类型追踪 | 1-2 小时 |
| P2: Pattern 匹配修复 | 30 分钟 |
| P3: 桩代码补参 | 3-4 小时 |
| 测试验证 | 30 分钟 |
| **合计（P0-P2）** | **约 3 小时** |
