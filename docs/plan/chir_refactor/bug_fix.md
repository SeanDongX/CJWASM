# CHIR Codegen Bug Fix 总结

## 背景

旧 AST→WASM 路径 34/37 通过，CHIR 路径初始仅 8/37 通过。根本原因是 CHIR 路径缺失内存分配、类构造、结构体初始化、for 循环等关键功能。

经过多轮修复，system_test 通过率从 **8/37 → 26/37**，cargo test 全部通过（1320+ tests）。

---

## Phase 1: `__alloc` 运行时函数

**文件**: `src/codegen/chir_codegen.rs`

- 在 `RT_NAMES` 中添加 `"__alloc"`
- 在 `emit_rt_functions` 中复用 `src/memory.rs` 的 `emit_alloc_func` 生成 bump allocator
- 签名: `__alloc(size: i32) -> i32`

这是所有堆分配（类、结构体、元组、字符串 trim）的基础。

## Phase 2: 类 init 函数生成

**文件**: `src/chir/lower.rs`, `src/codegen/chir_codegen.rs`

- 在 `lower_program` 的 `all_funcs` 构建阶段，对每个有 `init` 的类生成 `__ClassName_init` 函数
- 在 `emit_function` 中对 `__*_init` 函数添加 prologue（调用 `__alloc` 分配对象）和 epilogue（返回 `this` 指针）
- 在 `method_class_map` 中注册 init 函数名→类名映射

## Phase 3: ConstructorCall lowering

**文件**: `src/chir/lower_expr.rs`

- 修复 `ConstructorCall` 查找逻辑：优先查找 `__ClassName_init` 格式的函数索引
- 之前查找裸类名导致找不到 → Nop（i32.const 0）

## Phase 4: 类字段偏移修复

**文件**: `src/chir/lower_expr.rs`, `src/chir/lower.rs`

- `get_field_offset` 同时查找 `struct_field_offsets` 和 `class_field_offsets`
- vtable 偏移从 8 改为 4（匹配旧 codegen 的 i32 vtable_ptr）
- 修复 `this` 变量类型注册，使 init 函数内 `this.field` 能正确推断字段偏移

## Phase 5: StructNew emission

**文件**: `src/codegen/chir_codegen.rs`

- 实现 `CHIRExprKind::StructNew` 的 WASM 代码生成
- 调用 `__alloc` 分配内存，按字段偏移写入各字段值
- 在 `CHIRCodeGen` 中维护 `struct_field_offsets` 和 `class_field_offsets`

## Phase 6: For 循环 + Assert

**文件**: `src/chir/lower_stmt.rs`

- `Stmt::For` 脱糖为 `CHIRStmt::While`（`var = start; while var < end { body; var += step }`）
- `Stmt::Assert` 脱糖为 if + proc_exit（条件不满足时调用 unreachable）

---

## 后续修复

### Enum 无参数变体修复

- 修复 `has_payload` 枚举中无参数变体的构造和匹配
- 修复 Store/Load emit 中的类型匹配

### 字符串运行时函数（4 个）

**文件**: `src/codegen/chir_codegen.rs`, `src/chir/lower.rs`, `src/chir/lower_expr.rs`

| 函数 | 签名 | 说明 |
|------|------|------|
| `__str_contains` | `(i32, i32) → i32` | 内联子串搜索，返回 0/1 |
| `__str_starts_with` | `(i32, i32) → i32` | 前缀匹配 |
| `__str_ends_with` | `(i32, i32) → i32` | 后缀匹配 |
| `__str_trim` | `(i32) → i32` | 去除首尾空白，分配新字符串 |

- 在 `RT_NAMES`（codegen）和 `rt_names`（lowering）中同步注册
- 在 `try_lower_builtin_method` 中添加 `contains`、`startsWith`、`endsWith`、`trim`、`isBlank` 方法的 lowering
- `isBlank` 实现为 `trim().isEmpty()`

### 结构体模式匹配

**文件**: `src/chir/types.rs`, `src/chir/lower_expr.rs`, `src/codegen/chir_codegen.rs`

- 新增 `CHIRPattern::Struct` 变体，含 `Vec<StructPatternField>`
- 新增 `StructPatternField` 枚举：`Literal`、`Binding`、`NestedLiteral`、`NestedBinding`
- 支持嵌套结构体模式（如 `Rectangle { topLeft: Point { x: 0, y: 0 }, width, height }`）
- 支持 guard 条件（`case Point { x, y } where x == y => ...`）
- 修复 `emit_match` 中 `End` 指令的闭合逻辑

### 结构体解构赋值

**文件**: `src/chir/lower_stmt.rs`

- 支持 `let Point { x, y } = p` 语法
- 在 `lower_stmts_to_block` 中识别 `Pattern::Struct`，展开为多条 `Let` 语句
- 每条 `Let` 通过 `FieldGet` 从结构体指针按偏移加载字段

### `!in` 运算符

**文件**: `src/chir/lower_expr.rs`

- 将 `a !in b` 脱糖为 `!(b.contains(a))`
- 构造 `MethodCall { object: b, method: "contains", args: [a] }` 后取反

### 元组支持

**文件**: `src/codegen/chir_codegen.rs`, `src/chir/lower_expr.rs`, `src/chir/type_inference.rs`

- 实现 `TupleNew` codegen：调用 `__alloc` 分配 `n * 8` 字节，按 8 字节对齐写入元素
- 修复 `pair[0]` 被错误解析为 `ArrayGet`（应为 `TupleGet`）：在 `Expr::Index` lowering 中检查 `local_ast_types` 判断是否为 Tuple 类型
- 添加 `Expr::TupleIndex` 类型推断（从 Tuple 元素类型列表按索引取）

### 二元表达式类型提升

**文件**: `src/chir/lower_expr.rs`

- 当操作数实际为 I64/F64 但 `type_ctx.infer_expr` 返回 I32 时（如 match 模式绑定变量），自动提升结果类型
- 修复 `width * height` 在 struct pattern binding 中被错误编译为 `i32.mul`

### 模式绑定类型注册

**文件**: `src/chir/lower_expr.rs`

- 在 `lower_pattern` 处理 `Pattern::Struct` 时，将绑定变量插入 `local_ast_types`
- 确保后续表达式（如 `width * height`）能正确推断操作数类型

---

## 当前状态

| 指标 | 值 |
|------|-----|
| system_test 通过 | 26/37 (70%) |
| cargo test | 1320+ 全部通过 |
| 起始通过率 | 8/37 (22%) |
| 提升 | +18 个测试 |

## 剩余 11 个失败测试分析

| 缺失特性 | 影响测试 |
|-----------|----------|
| Option/Result + try-catch | error_handling, phase6_error_module |
| ArrayList/HashMap/HashSet | p3_collections, p4_collections |
| Option/Tuple/Lambda | p3_option_tuple |
| Array(n, fill)/clone/slice | p2_features |
| spawn/synchronized/atomics | p5_concurrent |
| try-with-resources + catch, 可选链, 尾随闭包 | p6_new_features |
| Option + while-let | patterns |
| ?? 运算符 | phase2_types |
| 多种 std 特性 | std_features |

这些测试需要 **新功能开发**（非 bug 修复），包括：
1. 异常处理（try-catch-finally, throw）
2. 集合类型（ArrayList, HashMap, HashSet）
3. Option/Result 类型系统
4. Lambda/闭包
5. 并发原语（spawn, synchronized, Atomic*）

## 修改文件清单

| 文件 | 修改类型 |
|------|----------|
| `src/codegen/chir_codegen.rs` | 运行时函数、StructNew/TupleNew emit、Match struct pattern |
| `src/chir/lower_expr.rs` | 字符串方法、!in、TupleIndex、struct pattern、类型提升 |
| `src/chir/lower_stmt.rs` | For 循环、Assert、结构体解构 |
| `src/chir/lower.rs` | init 函数生成、字段偏移、RT 名称注册 |
| `src/chir/types.rs` | CHIRPattern::Struct、StructPatternField |
| `src/chir/type_inference.rs` | TupleIndex 类型推断 |
