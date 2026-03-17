# 计划：CHIR 层两个优化 Pass

## 背景

benchmark 运行时性能瓶颈主要来自两个方面：
1. 小函数（`identity`、`Counter.get`、`colorValue` 等）每次调用都有完整 call 开销
2. 大量 `let x = expr; use(x)` 模式生成冗余的 `local.set` + `local.get` 指令对

两个 pass 都在 CHIR 层操作，不修改 codegen，改动局部，风险低。

## 修改文件

- **新建** `src/chir/optimize.rs` — 两个 pass 的全部实现
- **修改** `src/chir/mod.rs` — 添加 `pub mod optimize;`
- **修改** `src/pipeline.rs` — 在两处 CHIR 路径中插入 `optimize_chir` 调用

## Pass 1：小函数内联

### 可内联条件（全部满足）
1. `body.stmts.is_empty()` — 函数体只有一个 result 表达式，无语句
2. `body.result.is_some()` — 有返回值（Unit 函数不内联）
3. `count_expr_nodes(result) <= 8` — 表达式节点数不超过 8
4. `!has_side_effects(result)` — 无副作用（不含 Call/MethodCall/Store/FieldSet/ArraySet/Print）
5. `!has_call_to(result, self_func_idx)` — 不递归调用自身

### 内联步骤
1. 构建 `inlinable: HashMap<u32, CHIRFunction>`（func_idx → 函数定义）
2. 遍历所有函数体，对每个 `CHIRExprKind::Call { func_idx, args }` 检查是否可内联
3. 若可内联，执行 `substitute_and_remap`：将 result 中所有参数 Local 替换为对应 args
4. 将替换后的表达式直接替换原 Call 节点
5. 不内联 `MethodCall`（vtable 语义复杂）
6. 内联深度限制为 1（不递归内联）

## Pass 2：冗余 local.set/local.get 消除

### 安全条件（全部满足）
1. `write_count(local_idx) == 1` — 只有一个 Let 写入
2. `read_count(local_idx) == 1` — 整个函数体只读一次
3. 唯一读取点在同一个 CHIRBlock 的紧邻下一条语句或 block.result 中
4. `is_pure(value)` — 绑定值是纯的
5. `local_idx >= param_count` — 不消除参数

## 实现状态

- [x] `src/chir/optimize.rs` 创建完成
- [x] `src/chir/mod.rs` 添加 `pub mod optimize;`
- [x] `src/pipeline.rs` 两处 CHIR 路径插入 `optimize_chir` 调用
- [x] `cargo test` 全部通过（631 + 14 tests）
