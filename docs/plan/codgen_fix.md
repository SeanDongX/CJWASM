# CJWasm Codegen 问题修复计划

参考文档：`docs/codegen_compare.md`

---

## 问题概览

| # | 问题 | 位置 | 影响 | 状态 |
|---|------|------|------|------|
| Bug1 | 泛型 Struct 方法调用丢弃类型参数 | `expr.rs:5782` | 泛型类方法调用找不到 | 待修复 |
| Bug2 | ConstructorCall 内置分发使用未 mangle 的名称 | `expr.rs:6344` | 内置分发逻辑正确（已分析） | 已确认安全 |
| Bug3 | 两套 mangle 方案无统一查找策略 | 全局 | 泛型方法解析混乱 | 架构问题 |

---

## Bug1：泛型 Struct 方法调用键名错误

### 问题位置

`src/codegen/expr.rs:5780-5792`

```rust
let struct_ty = obj_type.and_then(|ty| match ty {
    Type::Struct(s, _) => Some(s),  // ← 丢弃了 type_args!
    Type::Option(_) => Some("Option".to_string()),
    Type::Result(_, _) => Some("Result".to_string()),
    Type::Map(_, _) => Some("Map".to_string()),
    _ => None,
});
let key = struct_ty
    .as_ref()
    .map(|s| format!("{}.{}", s, method))  // ← 生成 "Box.init"
    .unwrap_or_else(|| method.clone());
```

### 根本原因

对象类型为 `Type::Struct("Box", [Type::Int64])` 时，`type_args` 被 `_` 丢弃。最终查找键为 `"Box.init"`，而 monomorphization 注册的方法名为 `"Box$Int64.init"`（通过 `mangle_name`），导致 `resolve_method_index` 找不到方法，回退为桩代码。

### 修复方案

将 `Type::Struct(s, _)` 改为 `Type::Struct(s, type_args)`，并在 type_args 非空时使用 `mangle_name` 构造带类型参数的结构体名：

```rust
let struct_ty = obj_type.and_then(|ty| match ty {
    Type::Struct(s, type_args) => {
        if type_args.is_empty() {
            Some(s)
        } else {
            // 使用 monomorph::mangle_name 生成 "Box$Int64" 形式的名字
            Some(crate::monomorph::mangle_name(&s, &type_args))
        }
    }
    Type::Option(_) => Some("Option".to_string()),
    Type::Result(_, _) => Some("Result".to_string()),
    Type::Map(_, _) => Some("Map".to_string()),
    _ => None,
});
// key 仍为 format!("{}.{}", struct_name, method)
// 例如: "Box$Int64.init"
```

### 修改文件

`src/codegen/expr.rs:5782`（单行修改）

---

## Bug2：ConstructorCall 内置分发（已确认安全）

### 分析

`expr.rs:6344` 的 `match name.as_str()` 使用未 mangle 的名称，这对于内置类型是**正确的**：
- `ArrayList<Int64>()` → `name = "ArrayList"`，正确匹配内置分支
- 如果改为 `mangled_name`（`"ArrayList$Int64"`），反而匹配不到

内置分发完成后（`_ => {}` 分支），代码在 6542 行起已正确使用 `mangled_name` 查找用户定义类：

```rust
let init_func_name = format!("__{}_init", mangled_name);  // 6548: ✓
if self.func_indices.contains_key(&init_func_name) { ... }
// 后续 classes.get(&mangled_name) 等也均使用 mangled_name ✓
```

`docs/codegen_compare.md` 中记录的"语法错误"指的是较早版本的问题，**当前代码已正确处理**。

**结论：Bug2 无需修复。**

---

## Bug3：两套 mangle 方案的架构分析

### 两套方案对比

| 方案 | 函数 | 格式 | 用途 |
|------|------|------|------|
| `mangle_key` | `src/codegen/type_.rs:86` | `name$ParamTy1$ParamTy2` | 函数重载解析（按参数类型） |
| `mangle_name` | `src/monomorph/mod.rs:85` | `name$TypeArg1$TypeArg2` | 泛型单态化（按类型实参） |

字符串格式相同，但语义不同：`mangle_key` 处理参数列表，`mangle_name` 处理泛型参数。

### 当前查找流程（方法调用）

1. `get_object_type(object, locals)` → 获取对象的 `Type`
2. `Type::Struct(s, _)` 提取结构体名（**Bug1 在此处**）
3. `format!("{}.{}", s, method)` 构造查找键
4. `resolve_method_index(&key, method)` 查找 → 先精确匹配，再继承链向上

### 问题所在

单态化时函数被注册为 `"Box$Int64.init"` 形式，但查找时键为 `"Box.init"`。修复 Bug1 后，两套 mangle 方案的实际格式在这个路径上是兼容的（`mangle_name("Box", [Int64])` = `"Box$Int64"`，查找键变为 `"Box$Int64.init"`，与注册名一致）。

### 潜在的深层问题（低优先级）

当同一泛型类型有多个实例化（`Box<Int64>` 和 `Box<String>`）时，`resolve_method_index` 需要能区分两者。当前实现在修复 Bug1 后应该能正确区分，因为查找键包含了具体类型参数。

---

## 修复优先级与实施方案

### 唯一需要实施的修复：Bug1

**文件**: `src/codegen/expr.rs`
**位置**: 第 5782 行
**改动量**: ~5 行

**修改前**:
```rust
let struct_ty = obj_type.and_then(|ty| match ty {
    Type::Struct(s, _) => Some(s),
```

**修改后**:
```rust
let struct_ty = obj_type.and_then(|ty| match ty {
    Type::Struct(s, type_args) => {
        if type_args.is_empty() {
            Some(s)
        } else {
            Some(crate::monomorph::mangle_name(&s, &type_args))
        }
    }
```

### 预期效果

修复后，对 `Box<Int64>` 等泛型类型的方法调用（如 `.init()`、`.getValue()` 等）将能正确找到单态化生成的函数，而不是回退到桩代码（`i32.const 0`）。

---

## 验证步骤

```bash
# 1. 编译
cargo build 2>&1 | grep "^error"

# 2. 编译 std 示例
cargo run -- build -p tests/examples/std 2>&1 | tail -5

# 3. 验证 WASM 错误数变化
wasm-validate tests/examples/std/target/wasm/std_examples.wasm 2>&1 | wc -l

# 4. 运行全部系统测试（38/38 必须全部通过）
./scripts/system_test.sh
```

---

## 与 CJC 的架构差异（参考）

CJC 在**语义分析阶段**完成方法查找，codegen 直接使用已解析的方法索引（vtable 偏移）。CJWasm 在 codegen 阶段动态解析方法名，依赖名称 mangling 的一致性。这是 CJWasm 方法解析脆弱性的根本原因，长期需要更完整的类型推断基础设施（类似 CHIR）。
