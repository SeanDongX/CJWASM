# CJWasm WasmGC 对象模型设计与实施计划（路线 C）

> 文档归属：`gc-wasmgc` 分支（基 `compat-full@7aabc42`）· gc1a 交付
> 依据：`.compat-run/gc-research.md`（§3 WasmGC 可用性、§5-6 路线 C）、`.compat-run/gc-bench/BASELINE.md` v5 节
> 本文件是后续所有 GC 阶段的**唯一依据**。凡与本文冲突的实现，以本文为准；本文未覆盖处先在 PR 中补文再实现。
>
> **红线**：严禁改 `third_party/`；GC 路径必须由 `--gc`（=环境变量 `CJWASM_GC=1`）启用；默认输出行为**逐字节不变**（compat 侧 conformance/std 不受影响）。

---

## 0. 现状与总策略

- 默认后端：CHIR（`src/main.rs:62,350`，`use_chir=true`）。对象全部在线性内存 bump 分配（`src/memory.rs::emit_alloc_func`），`MEMORY_PAGES_MIN=4 / MAX=256`（16MB 硬顶，`chir_codegen.rs:37-39`），越界即 trap。无回收。
- 路线 C = **WasmGC 增量实施**：类/结构体先映射为 **GC struct 类型**，String/ArrayList/HashMap 暂留线性内存，形成"混合堆"。宿主（wasmtime 51 / Node V8 ≥22）负责可达性与环回收。
- 目标：用 **GC bench 成绩**（`.compat-run/gc-bench/run_v5.py`，五负载 churn/survive/cycles/mixed/control）驱动每一阶段的取舍与验收。线性基线 ns/op：churn 9.10 / survive 19.19 / cycles 38.58 / mixed 10.47 / control 4.62；trap 点：churn~700K、survive~500K、cycles~150K、mixed~950K。

### 里程碑（主会话定义）

| 里程碑 | 达标条件 |
|---|---|
| **M1** | churn stress **4M** 完成（不再 trap）、宿主 GC 堆有界、ns/op ≤ 9.10×1.8 = **16.4** |
| **M2** | cycles stress **1M** 完成（环可回收） |
| **M3** | survive / mixed stress 完成 |
| **M4**（可选） | String/集合迁移到 WasmGC |

> 全部 benchmark 源码见 `.compat-run/gc-bench/work/*.cj`；本节 API 面推导自这些文件，阶段顺序即按 M1→M4 排列。

---

## 1. 对象模型映射表

### 1.1 设计原则

1. **简单可靠优先**：能用一个 struct 表达就不引入间接层；能用宿主 GC 语义就不自研。
2. **类型化 ref，不用 anyref 偷懒**：Cangjie 的 `class` 变量**不可为 null**（只有 `Option<T>` 可空），因此绝大多数场景用**非空** typed ref（`(ref $T)`），无 `ref.null` 需求；只有 `Option<类>` 用可空 ref。
3. **值与引用分离**：值类型（Int/Float/Bool/Rune/Unit 字段）直接作 struct/array 字段；引用类型（class/struct/tuple/array/String）作 ref 字段。
4. **混合边界显式化**：线性侧（String/集合/WASI buffer）只存 i32/i64/f32/f64；**ref 绝不写进线性内存**（见 §5 R2）。

### 1.2 类型映射总表

| Cangjie 构造 | 目标表示 | 决策与理由 | 阶段 |
|---|---|---|---|
| `class C { a: T; ... }`（叶子类/无继承无子类） | GC `struct $C`（字段按声明序，各字段 StorageType 同 §1.2 的元素规则）；变量/参数/返回/`this` = `(ref $C)` | 最直接映射；字段访问 O(1) | **G1** |
| `class` 继承（`B <: A`） | **flattening**：子类 struct 含父类全部字段（父在前），并声明 `supertype_idx=Some(A)` 做 **wasm struct subtyping** | 复用现有 `build_type_layout_info` 的"父类字段在前"布局（`chir_codegen.rs:280-301`）；subtyping 让 `A` 位置可放 `B` 实例，宿主 verifier 校验；避免 boxing | G5 |
| 虚方法派发 | 有继承的类在 offset 0 保留 `i32` vtable-base 字段，沿用现有全局表 `call_indirect`（`chir_codegen.rs:4797-4801` 现占位 0） | 改动最小；bench 无虚调用；`funcref` 字段方案留作后续优化 | G5 |
| 接口类型变量 | `anyref`（可空）或接口壳 struct；**暂缓**，接口类型值在混合期仍用 i32 线性指针 | 接口非 struct，需 common supertype；bench 不涉及 | G6 |
| 泛型 `Pair<T,U>` | 依赖 monomorph 后为具体类型；未单态化 `TypeParam` → `anyref` | `src/monomorph` 已单态化（bench 全部具体类型） | G5+ |
| `struct S { ... }` | GC `struct $S`，同叶子类（无 vtable 字段） | 与 class 同构，去掉继承头 | **G1**（值字段）/G2 |
| tuple `(A,B,...)` | GC `struct $tuple`，字段 t0..tn | 替代现有"8 字节槽线性 tuple"（`chir_codegen.rs:5388-5420`） | G4 |
| `enum E { A, B(T) }` | GC `struct { tag: i32, payload: <typed> }`；有 payload 用 anyref 或按最大 payload | 判别式 + 载荷；bench 不直接用 enum | G4/G6 |
| `Option<Class>` / `Option<Struct>` | **可空 typed ref**：`(ref null $T)`；`None = ref.null`，`Some(x) = x`（不装箱） | cycles 负载核心；语义最自然；`Cangjie class 非空`，Option 才是唯一可空点 | **G3（M2）** |
| `Option<值类型>`（如 `Option<Int64>`） | GC `struct { value: T }` 的**可空 ref**（null=None），或保留线性 `[tag][value]` | 值类型不能直接 null 表示；bench 不用，暂缓 | G3/G4 |
| `Result<T,E>` | 同 enum/tag 方案 | 暂缓 | G4 |
| 闭包 / lambda | 非捕获 lambda：沿用现有 `__lambda_N` + 表索引 `i32`（`CallIndirect`）；**捕获引用**的闭包 → GC `struct { funcref, env: anyref }` | bench lambda 均非捕获（`{idx => LNode(0)}`）；捕获闭包暂缓 | G7 |
| `Array<值类型>` | wasm `array (mut <val>)`（val 存储，不打包 i8/i16） | 长度 + 连续元素，宿主管理 | **G4（M3）** |
| `Array<引用类型>`（如 `Array<LNode>`） | wasm `array (mut (ref null $T))`（可空元素以支持 `array.new_default` 占位） | survive/mixed 核心；ref 不能进线性内存，必须走 wasm array | **G4（M3）** |
| `VArray<T,$N>` | 同 `Array<T>`（现 codegen 已等价处理） | 一致 | G4 |
| `Range` | 线性 20B 值结构或 GC struct（低优先级） | 非引用，bench 不用 | 保持线性 |
| `String` | **线性内存 i32 指针**（`[len:i32][utf8]`） | WASI `fd_write` 需要线性内存；字符串工具齐全 | 不迁移 |
| `ArrayList` / `HashMap` / `HashSet` | **线性内存 i32 指针** | 现有运行时助手（`emit_arraylist_*` / `emit_hashmap_*` / `emit_hashset_*`） | 不迁移（G8 可选） |
| `AtomicInt64/Bool`、`Mutex/ReentrantMutex` | 线性（值包装/占位） | 非 GC 对象 | 不迁移 |
| 值类型 Int8..UInt64/Float16..64/Bool/Rune | struct 字段用原生 ValType；array 元素用 val 存储 | 直接映射 | G1 |

### 1.3 混合堆的"边界规则"（强制）

- **ref ⇒ wasm GC 世界**；**i32 指针 ⇒ 线性世界**。二者**不得互相直接存储**。
- 线性助手（String/集合）**只接受/返回标量**（i32/i64/f64）。当某 API 需要"元素/值为类引用"时，只有两条合法路径：
  1. **该容器整体迁移为 wasm array/struct**（如 `Array<Class>` → §1.2）；
  2. **通过 `anyref` 表间接**（`table` + `ref.cast`）：线性容器存 i32 表索引，表槽存 ref。**仅在无法迁移容器时使用**，作为兜底。
- **五个 bench 负载全部不触及路径 2**：`survive/mixed` 用定长 `Array<T=class>`（迁移为 wasm array）；`churn/cycles` 只用类实例与标量；`println` 仅格式化 i64（String 线性、无 ref 交叉）。故 **M1–M3 完全不需要 String/集合承载引用**。

---

## 2. CHIR 变更

### 2.1 核心决策：复用现有 CHIR 节点 + 以 `wasm_ty` 作判别，**不新增 Gc\* 变体**

- `CHIRExpr` 已携带 `wasm_ty: ValType`（`src/chir/types.rs:18`）。WasmGC 的 ref 是合法 `ValType::Ref(RefType)`，故**无需改数据类型即可承载 typed ref**。
- 判别方式：**某节点的 `wasm_ty` 是 `ValType::Ref(_)` ⇔ 该值走 GC 路径**；是 `I32` ⇔ 走原线性路径。混合堆下同模块内两类节点并存，per-node 判别正确。
- `StructNew`/`FieldGet`/`FieldSet`/`TupleNew`/`ArrayGet`/`ArraySet`/`ArrayLiteral`/`ArrayNew` **保持变体不变**：
  - `StructNew { struct_name, fields }` 的 `wasm_ty` 为 ref 时，codegen 走 `struct.new_default` + 逐字段 `struct.set`（或 `struct.new`）。
  - `FieldGet { object, field_offset, field_ty }` 的 `field_offset` 仍按**字节偏移**携带；codegen 在 GC 布局表里「按字节偏移 → 字段下标」反查（flattening 保持声明序，一一对应），再发 `struct.get`。
  - 理由：`optimize.rs`（1393 行）、`type_inference.rs`（1439 行）、`chir_codegen.rs` 对 `CHIRExprKind` 有**大量穷尽 match**；新增并行 `Gc*` 变体需改所有这些 match，**显著增加默认路径掉红线的风险**（红线：compat 不受影响）。用 `wasm_ty` 判别 + 程序级 `gc` 开关，默认路径代码与 match 结构**逐字节不变**。
- 新增程序级标记：`CHIRProgram { ..., gc: bool }`（`src/chir/types.rs:316`）。`gc=true` 时 codegen 才发射 GC 类型段并允许 ref；否则完全按现状。

### 2.2 类型映射函数（单一真源）

新增 `src/chir/gc.rs`（新文件，约 150-250 行）：

```rust
/// GC 布局：类/结构体 → GC struct 类型索引 + 字段(名, 字节偏移, Type) + 父类
pub struct GcStructLayout {
    pub type_index: u32,
    pub fields: Vec<(String, u32 /*byte offset*/, Type)>,
    pub super_type: Option<String>,
    pub has_vtable: bool, // offset 0 i32 vtable
}
pub struct GcTypeMap { pub structs: HashMap<String, GcStructLayout>, pub arrays: ..., }
```

- 预处理（`lower_program` 开头，类似现有 `build_type_layout_info` 在 codegen 侧做的事，但需在**降低期**即可知）：为「会被迁移的类/结构体」分配 GC 类型索引。
- **wasm 类型映射 helper**（替代散落的 `t.to_wasm()`）：
  ```rust
  fn gc_wasm_ty(&self, ty: &Type) -> ValType {
      if let Some(idx) = self.gc.type_of(ty) { ValType::Ref(RefType{nullable:false, heap_type:HeapType::Concrete(idx)}) }
      else { /* 原逻辑：Unit/Nothing→I32，其余 to_wasm() */ }
  }
  ```
- **迁移范围受控**：只有登记进 `GcTypeMap` 的类型才返回 ref；其余仍 `to_wasm()`。这保证「未迁移类型」（Atomic/Mutex/String/集合/未覆盖的类）行为不变。

### 2.3 需要改造的 lowering 点位（M1 最小集）

`wasm_ty` 的来源集中在以下点，G1 只需改与「迁移类/结构体」相关的分支（其余分支透传 `gc_wasm_ty` 后行为不变）：

- `src/chir/lower.rs:54,77,113,456-461`（函数参数/返回 wasm 类型）。
- `src/chir/lower_stmt.rs:18,26,68,76`（Let/Var 声明局部 wasm 类型）——`let c = Cell(...)` 的局部 `c` 由此定为 ref。
- `src/chir/lower_expr.rs` 约 23 处 `to_wasm()`（`161,204,330,409,954,986,1058,1100,1112,1228,1292,1325,1416,1431,1456,1499,1549,1724,1822,1939,2131,3885,3916`）——逐点替换为 `gc_wasm_ty`；G1 仅覆盖类/结构体构造、字段读、局部/参数/返回。
- 构造：`lower_expr.rs:1891-1930`（`__X_init` 解析）→ 结果 `wasm_ty` 改为 ref。
- `Call`/`MethodCall` 的实参与返回做 ref↔ref 一致（`insert_cast_if_needed` 对 ref 间不插 cast，对 i32↔ref 应报错——见 §5 R5）。

### 2.4 局部/参数/返回/ABI

- `CHIRLocal/CHIRParam.wasm_ty` 可为 ref（结构已支持，无改）。
- 函数签名：`wasm_result_tys`（`chir_codegen.rs:6575`）已按 `return_wasm_ty` 生成 `vec![ref]`，**wasm 允许 ref 作为 func 参数/结果**，直接可用。
- `call_indirect` 的 ref 参数：wasm 规范允许 typed ref 出现在 funcref 签名中；`find_or_create_func_type_idx` 以 `Vec<ValType>` 为 key，ref 已可作为 key 元素——**可行性成立**（G4 用 lambda/call_indirect 时验证）。
- **禁止**：任何把 ref 当 i32 存储/运算的路径（`I32WrapI64`、`I32Load/Store`、`i32` 算术）——见 §5 R2。

---

## 3. codegen 变更（`src/codegen/chir_codegen.rs`）

### 3.1 类型段（核心新增）

- 当前类型段只有函数类型（`generate` 605-638）。GC 模式追加：
  - **一个 rec group** 容纳全部 GC struct/array 类型（允许自引用 / 互引用，如 cycles 的 `CycleNode.peer: Option<CycleNode>`）。
  - 用 `wasm_encoder` API：`types.ty().rec(vec![SubType{...}, ...])`；struct = `CompositeInnerType::Struct(StructType{fields})`，array = `CompositeInnerType::Array(ArrayType(FieldType{...}))`；继承用 `SubType.supertype_idx=Some(parent_idx)`。
  - **类型索引分配**：函数类型索引已占用 0..N；GC 类型须在**全部函数类型之后**统一分配并记录 `GcTypeMap.type_index`（避免与 `user_type_indices`/`rt_type_indices` 冲突）。注意 `types.ty().rec(..)` 只把 rec group 计为 **1 个类型项**，但其内部 N 个子类型各占一个索引，需精确算基址。
- **spike 已验证**该 API 路径可被 wasmtime 51 与 Node V8 执行（§7）。

### 3.2 分配/访问改写（按节点 `wasm_ty` 分支）

| CHIR 节点 | 线性现状 | GC 路径 |
|---|---|---|
| `StructNew`（`chir_codegen.rs:5625-5672`） | `__alloc` + 逐字段 `emit_store_by_type` | `struct.new_default $T` → 暂存 local → 逐字段 `struct.set $T $idx` → 推 ref |
| `__X_init` prologue（`emit_function` 4788-4803） | `__alloc(obj_size)` + 写 vtable | `struct.new_default $T` → `LocalSet(this)`；vtable 字段 `struct.set` |
| `FieldGet`（`5021+` 的 Load 分支 `5590-5615`） | `i32.add offset` + `emit_load` | `struct.get $T $idx` |
| `FieldSet`（5731-5746） | `i32.add offset` + `emit_store` | `struct.set $T $idx` |
| `TupleNew`（5388-5420） | `__alloc` + 8B 槽 | `struct.new_default $tuple` + `struct.set` |
| `ArrayLiteral`（5678-5729） | `__alloc` + 长度头 + 槽 | `array.new_fixed $A n`（或 new_default + 逐 `array.set`） |
| `ArrayNew`（5802-5828） | `__alloc` + 头 | `array.new_default $A`（+ 填充循环）/ `array.new` |
| `ArrayGet`（5360-5386） | `i32.add 4 + idx*size` + load | `array.get $A`（值元素用 `array.get_s/u` 若打包，本期不打包） |
| `ArraySet`（5748-5800） | `i32.add 4 + idx*size` + store | `array.set $A` |

- **元素步长一致性**：线性路径 `wasm_ty_bytes`（`chir_codegen.rs:6645`）对 ref 返回 4，但 GC 路径**不使用字节步长**（`array.get/set` 自带元素类型），故此函数不变，仅 GC 分支绕过它。

### 3.3 Option<T=class> ↔ nullable ref（G3）

- `Expr::Some/None` lowering（`lower_expr.rs:2210-2268`）G3 改为：
  - `Option<RefT>`：`Some(x)` → 直接 `x`（ref）；`None` → `ref.null $T`；`wasm_ty = ValType::Ref(nullable:true)`。
  - 匹配 `case Some(x) => ... case None => ...`（`emit_match`，`chir_codegen.rs:6124+`）→ `ref.is_null` 分支。
- `??`（NullCoalesce，`lower_expr.rs:2270+`）→ `ref.is_null` + 选值。
- cycles 负载正是 `var peer: Option<CycleNode>`，G3 使其成为 `struct.set` 一个可空 ref 字段，环由宿主回收。

### 3.4 运行时助手边界

- `RT_NAMES`（`chir_codegen.rs:127-156`）与 `RT_PARAM_TYPES`（523-596）**全部保持 i32/i64/f64 标量签名**，不接收/返回 ref。
- String 打印（`build_rt_print_str` 等）继续读线性内存；GC 对象**不进入** `fd_write`。
- `println(obj)` 目前对 class 无格式化路径（会走零值/占位）——GC 模式下对 class 的 print 保持现有行为（不新增），避免引入 ref→String 的跨界。

### 3.5 `--gc` 开关（与默认路径并存）

- `src/main.rs`：`cmd_compile` / `cmd_build` 解析 `--gc` → `unsafe { env::set_var("CJWASM_GC","1") }`（沿用 `--checked`→`CJWASM_CHECKED`、`--no-chir`→`NO_CHIR` 的既有模式，`main.rs:98-102,218-220`）。
- `lower_program` / `CHIRCodeGen::generate` 读 `std::env::var("CJWASM_GC").is_ok()` → 决定 `gc` 标记。
- **默认（未设 `CJWASM_GC`）路径必须与 7aabc42 输出逐字节一致**——每个阶段以 `git diff <wasm>` 回归验证。

---

## 4. 分阶段计划（C1–C8 → G1..G8）

> 每阶段的验收门统一含：`cargo test` 全绿（0 failed）+ 默认路径（无 `--gc`）wasm 逐字节不变 + `scripts/system_test.sh` 示例段不回归。额外门见各阶段。

### G1 —— `--gc` 开关 + 类→GC struct（叶子类、值字段）→ **M1**

- **改动文件/函数**：
  - 新增 `src/chir/gc.rs`（`GcTypeMap`/`GcStructLayout`）；`src/chir/mod.rs` 导出。
  - `src/chir/types.rs`：`CHIRProgram.gc`。
  - `src/chir/lower.rs`：`lower_program` 建 `GcTypeMap`（叶子类：无 `extends`、无子类、无接口）；`gc_wasm_ty` 接入参数/返回（54/77/113/456-461）。
  - `src/chir/lower_stmt.rs`：18/26/68/76 用 `gc_wasm_ty`。
  - `src/chir/lower_expr.rs`：构造（1891-1930）、字段读（1425-1442）、字段写、`FieldGet/FieldSet`、`StructNew` 相关 `wasm_ty`；`get_field_offset` 在 GC 模式返回**字段下标**语义（或保持不变、由 codegen 反查）。
  - `src/codegen/chir_codegen.rs`：`generate` 发 GC 类型段（3.1）；`StructNew`/`FieldGet`/`FieldSet`/`__X_init` prologue 的 GC 分支（3.2）；局部声明已由 `local_wasm_types` 驱动（无需改）。
  - `src/main.rs`：`--gc` 解析。
- **预估行数**：~600-900 行（含 gc.rs、分支、测试）。
- **验收门**：
  - `cargo run -- --gc .compat-run/gc-bench/work/churn_stress.cj -o churn_gc.wasm`（N→4M）由 `wasmtime run --invoke`（或 Node）**跑完不 trap**，stdout 与 cjc/线性版 checksum 一致。
  - Node 观测：4M 后 `v8.getHeapStatistics().used_heap_size` 增量有界（≪ 4M×对象大小），rss 平稳。
  - churn ns/op ≤ **16.4**（run_v5 口径）。
  - 默认路径 `cargo test` + 至少 3 个 class 示例 wasm 逐字节不变。
- **风险**：typed ref 穿过 `optimize_chir`（见 §5 R1）；`insert_cast_if_needed` 对 ref 误插 cast（R5）。

### G2 —— struct / 值字段扩展到所有 struct（含嵌套、泛型单态化）

- 迁移 `Type::Struct` 字面量构造、嵌套 struct；`StructDef` 泛型经 monomorph 后具体化。
- 改动：`gc.rs` 登记 struct；`StructNew` 通用；`build_type_layout_info` 的 struct 部分 GC 化。
- 预估：~250-400 行。验收：struct 相关示例 + `cargo test`；bench mixed（含 MNode，但用 Array，仍受限见 G4）。

### G3 —— `Option<Class>` → nullable ref → **M2**

- 改动：`lower_expr.rs` 2210-2268（Some/None）、2270+（??）；`chir_codegen.rs` `emit_match`/`Option` 分支（`ref.is_null`/`ref.cast`）。
- 预估：~200-350 行。验收：**cycles stress 1M 跑完**（环回收），checksum 一致；cycles ns/op 记录。
- 风险：`Option` 在混合期同时存在两种表示（ref 可空 / 值 tag），需按 payload 类型分派（R6）。

### G4 —— `Array<T>`（值 & 引用元素）→ **M3**

- 改动：`gc.rs` 登记 array 类型；`ArrayLiteral`/`ArrayNew`/`ArrayGet`/`ArraySet` GC 分支；`Array<T>(n, lambda)` 初始化循环（`lower_expr.rs:1709-1820`）改 `array.new_default`+填充；`call_indirect` 传 ref（lambda 返回 ref）验证。
- 预估：~350-500 行。验收：**survive / mixed stress 跑完**；checksum 一致；记录 ns/op 与堆。
- 风险：survive 活集本身就大（450K×对象），需确认 wasmtime GC 堆能容纳且不 OOM；array 元素可空 vs 非空策略（R7）。

### G5 —— 继承 + 虚方法（struct subtyping + vtable）

- 改动：`gc.rs` flattening + `supertype_idx`；`build_type_layout_info` 继承链 GC 化；vtable 字段与 `call_indirect`。
- 预估：~300-500 行。验收：继承/多态示例运行；bench 不涉及，主要保证不强回归。

### G6 —— enum / Result / 接口 / 泛型残留

- tag struct；接口用 anyref/壳；未单态化泛型 anyref。预估：~400-700 行。验收：对应 conformance 子集在 `--gc` 下可运行（非必需全量）。

### G7 —— 闭包捕获 + `is`/`as` downcast

- 捕获引用闭包 → `struct { funcref, env }`；`is`/`as` → `ref.test`/`ref.cast`（现状 `Expr::IsType` 无专门 lowering，降级为占位——见 G7 补全）。
- 预估：~300-500 行。

### G8（可选，M4）—— String / ArrayList / HashMap 迁移

- 视需求；WASI 边界需 `array.new_data`/`memory.init` 或边界拷贝。低优先。

---

## 5. 风险清单

| ID | 风险 | 影响 | 缓解 |
|---|---|---|---|
| **R1** | typed ref 穿过 `optimize_chir`（`eliminate_redundant_locals` 等遍历） | ref 被误当 i32 / 丢弃 | optimize 的表达式克隆保持 `wasm_ty` 字段不变；为「GC 模式」在 optimize 前后加 wasm-validate 冒烟；必要时 GC 模式跳过危险 pass |
| **R2** | ref 当 i32 存储/运算（`i32.add/store/load`） | 类型错误 / verifier 失败 / 语义崩溃 | 硬规则：`wasm_ty==Ref` 的节点禁止任何 i32 线性指令；codegen 分支以 `ValType::Ref` 为唯一判别 |
| **R3** | `call_indirect` 含 ref 参数 | 可行性 | wasm 规范允许 typed ref 入签名；`find_or_create_func_type_idx` 以 ValType Vec 为 key 天然支持；G4 实测验证 |
| **R4** | wasmtime / Node 宿主兼容 | 运行失败 | **已实测**：wasmtime 51.0.0 / Node 25.9（V8 14.1）均执行 spike（§7）；目标宿主只有 wasmtime CLI |
| **R5** | `insert_cast_if_needed` 对 ref↔ref 或 i32↔ref 误插转换 | verifier 失败 | ref 间不插 cast；i32↔ref 直接 `bail!`（编译期错误而非生成非法 wasm） |
| **R6** | Option 双表示（ref/值）混淆 | 语义错 | 按 `Type::Option(inner)` 的 inner 是否 ref 分派；混合期明确"哪些 Option 迁移"白名单 |
| **R7** | Array 元素可空策略（`array.new_default` 需 defaultable） | 无法 default | 元素一律 `(ref null $T)`；非空性由不变量保证（初始填充后无 null） |
| **R8** | `--gc` 与默认路径并存导致默认输出漂移 | **红线** | GC 全部逻辑以 `gc` 标记/`wasm_ty==Ref` 为门；每阶段 `git diff` 默认 wasm 逐字节比对 |
| **R9** | GC 模块无 finalizer（`~init`） | 语义缺失 | 明确不支持或宿主侧模拟（wasmtime 无 JS FinalizationRegistry，需自建弱引用队列）；文档化 |
| **R10** | 类型索引与 rec group 索引计算错误 | 非法模块 | `rec()` 计 1 个类型项但含 N 子类型；集中算基址 + wasm-validate 冒烟（spike 已演示正确布局） |
| **R11** | GC 堆内存上限 / survive 活集过大 | OOM | 宿主默认堆足够；survive 450K 活集 ~15MB 线性，GC 后更小 |

---

## 6. 分配点清单（迁移 checklist）

> 全项目 `Call(alloc_idx)`（线性 `__alloc`）共 **40 处**。分三类：①CHIR 表达式级（codegen）5 处；②lowering 内联分配 12 处；③运行时助手内部分配 23 处。GC 迁移只动 ①②（且仅其中"类/结构体/数组/Option"类），③ 全部保留线性。

### 6.1 CHIR 表达式级（`chir_codegen.rs`）—— 迁移目标

| 行 | 所属函数 | 用途 | 阶段 |
|---|---|---|---|
| 4795 | `emit_function` | 类 `__X_init` prologue 分配 `this` | **G1** |
| 5396 | `emit_expr` TupleNew | 元组对象 | G4 |
| 5641 | `emit_expr` StructNew | 结构体/类字面量对象 | **G1** |
| 5702 | `emit_expr` ArrayLiteral | 数组字面量 | **G4** |
| 5815 | `emit_expr` ArrayNew | `Array<T>(n, init)` | **G4** |

### 6.2 lowering 内联分配（`lower_expr.rs`，均发 `Call(__alloc)`）

| 行 | 构造 | 用途 | 阶段 / 处置 |
|---|---|---|---|
| 1620 | `AtomicInt64` | 8B 值包装 | 保持线性 |
| 1651 | `AtomicBool` | 8B 值包装 | 保持线性 |
| 1679 | `Mutex/ReentrantMutex` | 4B 占位 | 保持线性 |
| 1711 | `Array<T>(n,init)` | 数组 | **G4** → array.new |
| 2144 | 枚举变体构造（`VariantConst`） | enum | G6 |
| 2212 | `Option::Some` | Option | **G3** → nullable ref（ref payload） |
| 2247 | `Option::None` | Option | **G3** → ref.null |
| 2335 | `Result::Ok` | Result | G6 |
| 2369 | `Result::Err` | Result | G6 |
| 2473 | `Range` | 20B 值结构 | 保持线性（或 G6） |
| 3127 | `Array.clone()` | 数组 | **G4** |
| 3229 | `Array.slice()` | 数组 | **G4** |

> 注：类实例的"分配"实际发生在 `__X_init` prologue（6.1 行 4795）+ 构造调用（lower_expr 1891-1930 解析 `__X_init`），非此处内联。

### 6.3 运行时助手内部分配（保留线性内存）

| 所属 builder（定义行） | 内部 alloc 行 | 用途 |
|---|---|---|
| `build_rt_i64_to_str` (1931) | 2038 | i64→String |
| `build_rt_bool_to_str` (2090) | 2101, 2114 | bool→String |
| `build_rt_str_concat` (2243) | 2264 | String 拼接 |
| `build_rt_str_repeat` (2301) | 2326 | String 重复 |
| `build_rt_f64_to_str` (2378) | 2444, 2458, 2469 | f64→String |
| `build_rt_str_trim` (2789) | 2860 | trim |
| `build_rt_str_to_array` (2904) | 2918 | String→Array |
| `build_rt_str_index_of` (2975) | 2995, 3019, 3037, 3104, 3126 | indexOf |
| `build_rt_str_index_of_byte` (3142) | 3157, 3197, 3219 | indexOf(byte) |
| `build_rt_str_last_index_of` (3234) | 3253, 3277, 3295, 3362, 3384 | lastIndexOf |
| `build_rt_str_last_index_of_byte` (3398) | 3413, 3455, 3477 | lastIndexOf(byte) |
| `build_rt_str_replace` (3501) | 3571, 3792 | replace |
| `emit_arraylist_new` (3941) | 3946, 3949 | ArrayList |
| `emit_arraylist_append` (3965) | 3993 | ArrayList |
| `emit_hashmap_new` (4152) | 4157, 4160 | HashMap |
| `emit_hashset_new` (4532) | 4537, 4540 | HashSet |

`__alloc` 本体：`src/memory.rs::emit_alloc_func`（bump + `memory.grow` 兜底）。GC 模式下线性侧仍用它（String/集合）。

---

## 7. 工具链 spike 实测（`tests/gc_spike.rs`，本阶段交付）

**模块**（111 字节，wasm-encoder 0.220.1 手写）：rec group 含
- type 1 `struct Node { value: i64 mut, next: (ref null 1) mut }`（自引用 ref 字段）
- type 2 `array (mut (ref null 1))`
- `main: () -> i64`：循环 **1,000,000** 次 `struct.new_default 1`（覆盖同一 local 制造垃圾）+ `array.new_default 2`（垃圾数组）；随后构造 `Node{value=42}` 并 `struct.get 1 0` 返回。

| 通道 | 结果 |
|---|---|
| **wasm-encoder 0.220.1 编码** | 成功（rec group / SubType / StructType / ArrayType / RefType(Concrete) / struct.new_default / array.new_default / struct.get） |
| **wasmtime 51.0.0** `wasmtime run --invoke main` | **成功**，退出码 0，stdout=`42`，墙钟 **25.4 ms**（含进程启动；1M 次 struct+array 分配 + GC） |
| **Node v25.9.0（V8 14.1）** 直接 `WebAssembly.instantiate` + `main()` | **成功**，返回 `42n`；guest 纯执行 **0.296 ms**（热身后再测）；`used_heap_size` 增量 **+24,408 B**、`rss` 增量 **0**、`total_heap_size` 不变 → 1M 垃圾对象（约 50MB 若泄漏）**被 V8 GC 回收**，堆统计信号明确 |
| **wat2wasm 1.0.42** | 不支持 GC（已知，未使用） |

> 结论：`wasm-encoder 0.220 → wasmtime 51 → Node V8` 三段路**全部可运行 GC 模块**且宿主 GC 有界。artifact 已复制到 `.compat-run/gc-bench/work/gc_spike.wasm` 供 gc1b 的 Node 堆观测通道复用。

---

## 8. 阶段 2（G1）首个可交付定义（供主会话直接派发）

**目标**：`--gc` 下把「叶子类（无继承、无子类、无接口）+ 值类型字段」映射为 GC struct，令 **churn stress 4M 跑完且 ns/op ≤ 16.4**（M1）。

**范围（只做这些，超出即拒）**：
1. `src/main.rs`：`--gc` → `CJWASM_GC=1`。
2. `src/chir/gc.rs`（新）：`GcTypeMap`，只登记叶子类；`gc_wasm_ty()` 单点映射。
3. `src/chir/types.rs`：`CHIRProgram.gc: bool`。
4. `src/chir/lower.rs`：建 `GcTypeMap`；参数/返回 wasm 类型走 `gc_wasm_ty`。
5. `src/chir/lower_stmt.rs`：Let/Var 局部 wasm 类型走 `gc_wasm_ty`。
6. `src/chir/lower_expr.rs`：类构造（`__X_init` 调用）、字段读/写、`StructNew` 的 `wasm_ty` 走 ref。
7. `src/codegen/chir_codegen.rs`：发射 rec group GC 类型段；`StructNew`/`FieldGet`/`FieldSet`/`__X_init` prologue 的 GC 分支（依据 `wasm_ty==Ref`）。

**不做**：Option、Array、继承、接口、虚方法、String/集合迁移、finalizer。

**验收命令**：
```bash
cargo test                                          # 全绿（0 failed）

# 1) 4M churn stress 完成性（--gc，不再 trap）
cargo build --release
cargo run --release -- .compat-run/gc-bench/work/churn_n4000000.cj --gc -o /tmp/churn_4m.wasm
wasmtime run --invoke main /tmp/churn_4m.wasm        # 退出 0，stdout 含 BENCH churn n=4000000（checksum 与 cjc/线性版一致）

# 2) ns/op 门（run_v5 同口径；churn 标准 N = wasm_n 650K / cjc_n 20M）
#    run_v5.py 的 compile_cjwasm 继承 os.environ，故用 CJWASM_GC=1 让其以 GC 模式编译；
#    并将 release 二进制放入 run_v5 固定路径（或设 CJWASM_V5=/path/to/cjwasm）。
cp target/release/cjwasm .compat-run/gc-bench/work/cjwasm_v5.bin
CJWASM_GC=1 python3 .compat-run/gc-bench/run_v5.py --only churn --repeats 7
#    读 work/results_v5.json：churn 的 cjwasm ns/op ≤ 16.4（=9.10×1.8）

# 3) 默认路径回归：无 --gc / 无 CJWASM_GC 编译等价源，wasm 逐字节不变
```

**预估**：~600-900 行；风险 R1/R5/R8 为主。
