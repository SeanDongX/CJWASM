# CHIR Codegen Bug Fix 总结

## 背景

旧 AST→WASM 路径 34/37 通过，CHIR 路径初始仅 8/37 通过。根本原因是 CHIR 路径缺失内存分配、类构造、结构体初始化、for 循环等关键功能。

经过多轮修复，system_test 通过率从 **8/37 → 26/37 → 37/37 (100%)**，cargo test 全部通过。

---

## 已完成修复

### Phase 1: `__alloc` 运行时函数

**文件**: `src/codegen/chir_codegen.rs`

- 在 `RT_NAMES` 中添加 `"__alloc"`
- 在 `emit_rt_functions` 中复用 `src/memory.rs` 的 `emit_alloc_func` 生成 bump allocator
- 签名: `__alloc(size: i32) -> i32`

这是所有堆分配（类、结构体、元组、字符串 trim）的基础。

### Phase 2: 类 init 函数生成

**文件**: `src/chir/lower.rs`, `src/codegen/chir_codegen.rs`

- 在 `lower_program` 的 `all_funcs` 构建阶段，对每个有 `init` 的类生成 `__ClassName_init` 函数
- 在 `emit_function` 中对 `__*_init` 函数添加 prologue（调用 `__alloc` 分配对象）和 epilogue（返回 `this` 指针）
- 在 `method_class_map` 中注册 init 函数名→类名映射

### Phase 3: ConstructorCall lowering

**文件**: `src/chir/lower_expr.rs`

- 修复 `ConstructorCall` 查找逻辑：优先查找 `__ClassName_init` 格式的函数索引
- 之前查找裸类名导致找不到 → Nop（i32.const 0）

### Phase 4: 类字段偏移修复

**文件**: `src/chir/lower_expr.rs`, `src/chir/lower.rs`

- `get_field_offset` 同时查找 `struct_field_offsets` 和 `class_field_offsets`
- vtable 偏移从 8 改为 4（匹配旧 codegen 的 i32 vtable_ptr）
- 修复 `this` 变量类型注册，使 init 函数内 `this.field` 能正确推断字段偏移

### Phase 5: StructNew emission

**文件**: `src/codegen/chir_codegen.rs`

- 实现 `CHIRExprKind::StructNew` 的 WASM 代码生成
- 调用 `__alloc` 分配内存，按字段偏移写入各字段值
- 在 `CHIRCodeGen` 中维护 `struct_field_offsets` 和 `class_field_offsets`

### Phase 6: For 循环 + Assert

**文件**: `src/chir/lower_stmt.rs`

- `Stmt::For` 脱糖为 `CHIRStmt::While`（`var = start; while var < end { body; var += step }`）
- `Stmt::Assert` 脱糖为 if + proc_exit（条件不满足时调用 unreachable）

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
- 支持 guard 条件（`case Point { x, y } where x == y => "diagonal"`）
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
- 修复 `pair[0]` 被错误解析为 `ArrayGet`（应为 `TupleGet`）
- 添加 `Expr::TupleIndex` 类型推断（从 Tuple 元素类型列表按索引取）

### 二元表达式类型提升

**文件**: `src/chir/lower_expr.rs`

- 当操作数实际为 I64/F64 但 `type_ctx.infer_expr` 返回 I32 时，自动提升结果类型
- 修复 `width * height` 在 struct pattern binding 中被错误编译为 `i32.mul`

### 轮次 1: Option 类型 + `??` + if-let + while-let ✅

**文件**: `src/chir/lower_expr.rs`, `src/chir/lower_stmt.rs`, `src/codegen/chir_codegen.rs`

- **Option 内存布局**: `[tag: i32][value: i64]`，tag=0 为 None，tag=1 为 Some
- **Some(x) 构造**: alloc 12 字节 → store tag=1 → store value=x → 返回指针
- **None 构造**: alloc 12 字节 → store tag=0 → 返回指针
- **`??` 空合并**: 脱糖为 `if (opt.tag == 1) { opt.value } else { default }`
- **if-let**: `if (let Some(x) <- opt) { A } else { B }` → tag 检查 + 值绑定
- **while-let**: `while (let Some(n) <- current) { body }` → While + tag 检查

**解锁测试**: `phase2_types.cj`, `patterns.cj`, `p3_option_tuple.cj` (部分)

### 轮次 2: try-catch-finally + Result 类型 ✅

**文件**: `src/chir/lower_stmt.rs`, `src/chir/lower_expr.rs`, `src/codegen/chir_codegen.rs`

- **Result 类型**: 复用 Option 布局 `[tag: i32][value: i64]`，tag=0 为 Ok，tag=1 为 Err
- **try-catch 模拟**: 使用局部变量 `__err_flag` / `__err_val` 模拟异常
- **throw**: 设置 `__err_flag = 1`, `__err_val = expr`
- **catch**: 检查 `__err_flag`，绑定 catch 变量
- **finally**: 无条件执行块

**解锁测试**: `error_handling.cj`, `phase6_error_module.cj`, `p6_new_features.cj` (部分)

### 轮次 3: Lambda / 闭包 ✅

**文件**: `src/chir/lower_expr.rs`, `src/chir/lower.rs`, `src/codegen/chir_codegen.rs`

- **闭包表示**: 将 lambda body 提取为顶层函数 `__lambda_N`
- **Lambda 调用**: `call_indirect` 通过函数表间接调用
- **尾随闭包**: `apply(10) { x => x * 3 }` → parser 已处理为普通参数
- **动态数组 lambda init**: `Array<T>(n, { i => expr })` — 遍历 0..n 调用 lambda 填充

**解锁测试**: `p3_option_tuple.cj` (Lambda), `p6_new_features.cj` (trailing closure)

### 轮次 4: 动态数组 + 数组方法 ✅

**文件**: `src/chir/lower_expr.rs`, `src/codegen/chir_codegen.rs`

- **`Array<T>(n, init)`**: 分配 `4 + n * 8` 字节，前 4 字节存长度
- **`clone()`**: 分配同等大小内存，`memory.copy`
- **`slice(start, end)`**: 分配子数组，复制元素
- **`isEmpty()`**: 读取长度字段，`len == 0`

**解锁测试**: `p2_features.cj` (testDynArray + testArrayMethods)

### 轮次 5: 集合类型 — ArrayList / HashMap / HashSet + extend ✅

**文件**: `src/codegen/chir_codegen.rs`, `src/chir/lower_expr.rs`, `src/chir/lower.rs`

- **ArrayList**: 布局 `[len: i32][cap: i32][data_ptr: i32]`，支持 append/get/set/remove/size + 自动扩容
- **HashMap**: 开放寻址哈希表，支持 put/get/containsKey/remove/size
- **HashSet**: HashMap 的 key-only 包装，支持 add/contains/size
- **extend 方法**: parser 已处理 `this` 参数和名称修饰，lower.rs 直接收集

**关键修复**:
- `lower.rs` 中去除了 extend 方法的冗余 `format!("{}.{}", ...)` 和 `this` 参数插入（parser 已处理），防止双重前缀 (`Vec2.Vec2.length`)
- 方法调用返回类型从 `func_return_types` 精确查找，而非依赖 `infer_expr` 的 fallback

**解锁测试**: `p3_collections.cj`, `p4_collections.cj`

### 轮次 6: 可选链 + 类型转换 + std 补全 ✅

**文件**: `src/chir/lower_expr.rs`, `src/chir/type_inference.rs`, `src/codegen/chir_codegen.rs`

- **可选链 `?.`**: `obj?.field` 脱糖为 `if (obj != None) { obj.field } else { None }`
- **Range 类型**: `start..end` → alloc 16 字节存储 `[start: i64][end: i64]`，注册 Range 虚拟结构体字段
- **`toFloat64()`**: `f64.convert_i64_s` / `toInt64()`: `i64.trunc_f64_s`
- **`toString()`**: 委托到 `__i64_to_str` / `__f64_to_str` / `__bool_to_str`
- **`sort(arr)`**: 内联冒泡排序 WASM 实现
- **`@Assert` / `@Expect` 宏**: 条件不满足时触发 `unreachable`
- **ArrayStack / LinkedList**: 基于 ArrayList 运行时函数实现
- **字符串方法**: `toArray()` → `__str_to_array`，`indexOf()` → `__str_index_of`，`replace()` → `__str_replace`
- **Int/Float `format()` 方法**: fallback 到 `toString()`
- **字符串插值 `Expr::Interpolate`**: 各部分依次转字符串后用 `__str_concat` 拼接；i32 整数自动 cast 到 i64 后调用 `__i64_to_str`

**关键修复**:
- `lower.rs` 运行时函数名列表补全 `__str_to_array`/`__str_index_of`/`__str_replace`，与 `chir_codegen.rs` 的 `RT_NAMES` 保持同步（之前缺失导致 collections 函数索引偏移 3 位）
- Range 的 `__alloc` 调用参数类型修正为 `Type::Int32 / ValType::I32`（之前用 I64 导致类型不匹配）
- `FieldGet` 的 `wasm_ty` 从字段实际类型派生（而非外层 infer_expr 的可能错误结果）

**解锁测试**: `p6_new_features.cj`, `std_features.cj`, `std_math.cj`

### 轮次 7: 并发原语模拟 ✅

**文件**: `src/chir/lower_expr.rs`, `src/chir/lower_stmt.rs`

WASM 单线程模型下的退化实现：
- **spawn**: 同步执行 body（忽略线程语义）
- **synchronized**: 直接执行 body（无竞争）
- **AtomicInt64/AtomicBool**: 退化为内存 load/store（alloc 12 字节 `[value]`）
- **Mutex / ReentrantMutex**: lock/unlock 为 no-op，`tryLock()` 返回 true

**关键修复**:
- Atomic/Mutex 方法处理从 `_ =>` 兜底分支移到 `Type::Struct(_, _)` 分支内，确保正确匹配
- 构造器的 `__alloc` 参数类型修正为 `I32`

**解锁测试**: `p5_concurrent.cj`

---

## 当前状态

| 指标 | 值 |
|------|-----|
| system_test 通过 | **37/37 (100%)** + 1 SKIP |
| cargo test | 全部通过 |
| 起始通过率 | 8/37 (22%) |
| 提升 | **+29 个测试** |

---

## 原始 11 个失败测试 — 全部已修复 ✅

| # | 测试文件 | 阻塞特性 | 修复轮次 | 状态 |
|---|---------|---------|---------|------|
| 1 | `error_handling.cj` | Result + try-catch-finally | 轮次 2 | ✅ PASS |
| 2 | `p2_features.cj` | `Array<T>(n, init)` + clone/slice | 轮次 3-4 | ✅ PASS |
| 3 | `p3_collections.cj` | ArrayList / HashMap / extend | 轮次 5 | ✅ PASS |
| 4 | `p3_option_tuple.cj` | Option + if-let + Lambda | 轮次 1, 3 | ✅ PASS |
| 5 | `p4_collections.cj` | HashMap/HashSet/ArrayList/Range | 轮次 5-6 | ✅ PASS |
| 6 | `p5_concurrent.cj` | spawn/synchronized/Atomic/Mutex | 轮次 7 | ✅ PASS |
| 7 | `p6_new_features.cj` | try-catch + 可选链 + 尾随闭包 + HashSet | 轮次 2-3, 5-6 | ✅ PASS |
| 8 | `patterns.cj` | while-let | 轮次 1 | ✅ PASS |
| 9 | `phase2_types.cj` | Option + `??` 空合并运算符 | 轮次 1 | ✅ PASS |
| 10 | `phase6_error_module.cj` | try-catch-finally + Result | 轮次 2 | ✅ PASS |
| 11 | `std_features.cj` | toFloat64/toInt64 + 集合 + 数学 + 插值 | 轮次 5-6 | ✅ PASS |

额外修复: `std_math.cj` — 字符串插值中 i32 整数未转字符串导致 OOB 内存访问（轮次 6）

---

## 修复计划（全部 7 轮已完成 ✅）

### 特性依赖关系

```
                    ┌─────────────┐
                    │  Option<T>  │ ← 轮次 1 ✅
                    │  Some/None  │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │   ??     │ │  if-let  │ │ while-let│
        │空合并运算│ │ 条件解构 │ │ 循环解构 │
        └──────────┘ └──────────┘ └──────────┘
                                              
        ┌──────────────┐
        │  try-catch   │ ← 轮次 2 ✅
        │  -finally    │
        └──────────────┘
                       
        ┌──────────────┐
        │   Lambda     │ ← 轮次 3 ✅
        │   闭包捕获   │
        └──────────────┘
                       
        ┌──────────────┐
        │  动态数组    │ ← 轮次 4 ✅
        │  Array<T>()  │
        └──────────────┘
                       
        ┌──────────────┐
        │  集合类型    │ ← 轮次 5 ✅
        │  ArrayList   │
        │  HashMap/Set │
        └──────────────┘
                       
        ┌──────────────┐
        │  可选链/std  │ ← 轮次 6 ✅
        │  ?./ 插值    │
        └──────────────┘
                       
        ┌──────────────┐
        │  并发原语    │ ← 轮次 7 ✅
        │  Atomic/Mutex│
        └──────────────┘
```

---

### 修复轮次实际结果

| 轮次 | 特性 | 实际新增通过 | 累计通过 | 复杂度 |
|------|------|-------------|---------|--------|
| 1 | Option + ?? + if-let + while-let | +4 | 30/37 | ★★☆ |
| 2 | try-catch-finally + Result | +3 | 33/37 | ★★★ |
| 3 | Lambda / 闭包 | +1 | 34/37 | ★★★ |
| 4 | 动态数组 + 数组方法 | +1 | 35/37 | ★★☆ |
| 5 | 集合 (ArrayList/HashMap/HashSet) + extend | +1 | 36/37 | ★★★ |
| 6 | 可选链 + 类型转换 + std + 插值 | +1 | 37/37 | ★★☆ |
| 7 | 并发原语 (模拟) | (已含在轮次 5-6) | 37/37 | ★☆☆ |

**最终结果**: **37/37 (100%)** 通过，+1 SKIP (`str_methods_test.cj` 无预期值)。

### 修改文件清单

| 文件 | 修改内容 |
|------|---------|
| `src/codegen/chir_codegen.rs` | 运行时函数 (40+)、StructNew/TupleNew、Match struct pattern、Option/Result 布局、字符串运行时 (toArray/indexOf/replace)、集合运行时 (ArrayList/HashMap/HashSet) |
| `src/chir/lower_expr.rs` | Option/Result 构造、?? 脱糖、Lambda、Array<T>(n,init)、集合方法分派、extend 方法、Range 构造/字段、可选链、Atomic/Mutex 模拟、@Assert 宏、sort 内联、format fallback、字符串插值 |
| `src/chir/lower_stmt.rs` | For 循环、Assert、结构体解构、if-let/while-let、try-catch-finally、spawn/synchronized 模拟 |
| `src/chir/lower.rs` | init 函数生成、字段偏移、RT 名称注册同步、extend 方法收集（去除冗余修饰） |
| `src/chir/types.rs` | CHIRPattern::Struct、StructPatternField |
| `src/chir/type_inference.rs` | TupleIndex 类型推断、Range 虚拟结构体字段、extend 方法签名注册 |
