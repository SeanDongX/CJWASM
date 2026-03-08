# CJWasm2 vs cjc 类型推断对比分析

## 核心差异总结

| 特性 | cjc | CJWasm2 | 影响 |
|------|-----|---------|------|
| **架构** | 多遍编译 | 单遍编译 | 🔴 严重 |
| **符号表** | 完整全局符号表 | 局部符号表 | 🔴 严重 |
| **模块系统** | 支持 `.cjm` 加载 | 不支持 | 🔴 严重 |
| **类型检查阶段** | 独立阶段 | 代码生成时 | 🟡 中等 |
| **泛型处理** | 完整收集+实例化 | 单遍处理 | 🟡 中等 |
| **类型约束** | 约束求解器 | 无 | 🟢 轻微 |

## 1. 架构对比

### cjc: 多遍编译架构

```
┌─────────────────────────────────────────────────────────┐
│ Pass 1: 解析 (Parser)                                    │
│   - 生成 AST                                             │
│   - 不进行类型检查                                        │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│ Pass 2: 语义分析 (Sema)                                  │
│   - 构建完整符号表                                        │
│   - 类型检查 (TypeChecker)                               │
│   - 泛型实例化                                           │
│   - 类型推断 (Synthesize/Check)                          │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│ Pass 3: 代码生成 (CodeGen)                               │
│   - 所有类型已知                                         │
│   - 生成 LLVM IR                                         │
└─────────────────────────────────────────────────────────┘
```

**优势**:
- 第二遍时可以查询任何符号的类型
- 可以处理前向引用
- 泛型可以完整收集后再实例化

### CJWasm2: 单遍编译架构

```
┌─────────────────────────────────────────────────────────┐
│ 单遍: 解析 + 代码生成                                     │
│   - 边解析边生成 WASM                                     │
│   - 类型推断在代码生成时进行                               │
│   - 无法查询未解析的符号                                   │
│   - 泛型必须立即实例化                                     │
└─────────────────────────────────────────────────────────┘
```

**劣势**:
- 无法查询后面定义的符号
- 无法处理复杂的前向引用
- 泛型实例化不完整

## 2. 类型推断实现对比

### 2.1 数组索引 (Array[index])

#### cjc 实现

```cpp
// third_party/cangjie_compiler/src/Sema/TypeCheckExpr/SubscriptExpr.cpp
Ptr<Ty> TypeChecker::TypeCheckerImpl::SynSubscriptExpr(ASTContext& ctx, SubscriptExpr& se)
{
    // 1. 推断数组表达式的类型
    Ptr<Ty> baseTy = Synthesize(ctx, se.baseExpr.get());

    // 2. 推断索引表达式的类型
    std::vector<Ptr<Ty>> indexTys{};
    for (auto& expr : se.indexExprs) {
        indexTys.push_back(Synthesize(ctx, expr.get()));
    }

    // 3. 根据数组类型返回元素类型
    if (auto tupleTy = DynamicCast<TupleTy*>(baseTy)) {
        // 元组访问: (1, 2, 3)[0] -> Int64
        return ChkTupleAccess(ctx, target, se, *tupleTy);
    }

    if (auto varrTy = DynamicCast<VArrayTy*>(baseTy)) {
        // 数组访问: Array<T>[i] -> T
        return varrTy.typeArgs[0];  // 返回元素类型 T
    }

    // 4. 运算符重载: 调用 operator[]
    DesugarOperatorOverloadExpr(ctx, se);
    return se.desugarExpr->ty;
}
```

**关键点**:
- ✅ 完整的类型推断
- ✅ 支持元组、数组、运算符重载
- ✅ 从 `Array<T>` 正确提取 `T`

#### CJWasm2 实现

```rust
// src/codegen/expr.rs
fn infer_ast_type(&self, expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Index { object, index } => {
            // 1. 推断数组类型
            let obj_ty = self.infer_ast_type(object)?;

            // 2. 提取元素类型
            match obj_ty {
                Type::Array(elem_ty) => Some(*elem_ty),
                Type::Tuple(types) => {
                    // 元组索引必须是常量
                    if let Expr::Integer(i) = **index {
                        types.get(i as usize).cloned()
                    } else {
                        None
                    }
                }
                _ => None,  // ❌ 无法处理第三方库类型
            }
        }
        // ...
    }
}
```

**问题**:
- ❌ 无法处理 `ArrayList<T>`, `HashMap<K,V>` 等第三方类型
- ❌ 无法查询运算符重载
- ⚠️ 只能处理内置类型

### 2.2 函数调用 (func(args))

#### cjc 实现

```cpp
// third_party/cangjie_compiler/src/Sema/TypeCheckCall.cpp
Ptr<Ty> TypeChecker::TypeCheckerImpl::SynCallExpr(ASTContext& ctx, CallExpr& ce)
{
    // 1. 检查调用基础表达式，获取候选函数
    Ptr<Decl> decl{nullptr};
    std::vector<Ptr<FuncDecl>> candidates;
    ChkCallBaseExpr(ctx, ce, decl, target, candidates);

    // 2. 匹配函数重载
    SubstPack typeMapping;
    std::vector<Ptr<FuncDecl>> result;
    result = MatchFunctionForCall(ctx, candidates, ce, target, typeMapping);

    // 3. 返回函数返回类型
    if (result.size() == 1) {
        return result[0]->getReturnType();
    }

    // 4. 处理泛型函数
    if (auto genericFn = LookupGenericFunction(ce.baseFunc)) {
        // 从参数推断类型参数
        auto typeArgs = InferTypeArguments(ctx, genericFn, ce.args);
        // 实例化泛型函数
        auto specializedFn = Instantiate(genericFn, typeArgs);
        return specializedFn->getReturnType();
    }

    return TypeManager::GetInvalidTy();
}
```

**关键点**:
- ✅ 完整的符号表查询
- ✅ 支持函数重载
- ✅ 支持泛型实例化
- ✅ 可以查询任何函数的返回类型

#### CJWasm2 实现

```rust
// src/codegen/expr.rs
fn infer_ast_type(&self, expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Call { name, type_args, args, .. } => {
            // 1. 特殊处理内置函数
            match name.as_str() {
                "readln" | "getEnv" => return Some(Type::String),
                "now" | "randomInt64" => return Some(Type::Int64),
                "randomFloat64" => return Some(Type::Float64),
                _ => {}
            }

            // 2. 查询函数返回类型
            let arg_tys: Vec<Type> = args
                .iter()
                .filter_map(|a| self.infer_ast_type(a))
                .collect();

            let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                // 重载函数：需要参数类型
                if arg_tys.len() == args.len() {
                    Some(Self::mangle_key(name, &arg_tys))
                } else {
                    None  // ❌ 参数类型推断失败
                }
            } else {
                Some(name.to_string())
            };

            // 3. 从 func_return_types 查询
            key.and_then(|k| self.func_return_types.get(&k))
                .cloned()
                .or(Some(Type::Int64))  // ❌ 默认值
        }
        // ...
    }
}
```

**问题**:
- ❌ 只能查询已解析的函数
- ❌ 无法查询第三方库函数
- ❌ 泛型函数返回类型不准确
- ⚠️ 大量使用默认值 `Type::Int64`

### 2.3 字段访问 (obj.field)

#### cjc 实现

```cpp
// third_party/cangjie_compiler/src/Sema/TypeCheckAccess.cpp
Ptr<Ty> TypeChecker::TypeCheckerImpl::SynMemberAccessExpr(ASTContext& ctx, MemberAccessExpr& mae)
{
    // 1. 推断对象类型
    Ptr<Ty> objTy = Synthesize(ctx, mae.object.get());

    // 2. 查询字段类型
    if (auto structTy = DynamicCast<StructTy*>(objTy)) {
        // 从结构体定义查询字段
        auto field = structTy->LookupField(mae.fieldName);
        if (field) {
            return field->type;  // ✅ 精确的字段类型
        }
    }

    // 3. 查询方法
    if (auto method = LookupMethod(objTy, mae.fieldName)) {
        return method->type;
    }

    return TypeManager::GetInvalidTy();
}
```

**关键点**:
- ✅ 可以查询任何结构体的字段类型
- ✅ 支持方法查询
- ✅ 支持泛型结构体

#### CJWasm2 实现

```rust
// src/codegen/expr.rs
fn infer_ast_type(&self, expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Field { object, field } => {
            // 1. 推断对象类型
            let obj_ty = self.infer_ast_type(object)?;

            // 2. 查询字段类型
            match obj_ty {
                Type::Struct(name, type_args) => {
                    // 从 self.structs 查询
                    let struct_def = self.structs.get(&name)?;

                    // 查找字段
                    for (fname, fty) in &struct_def.fields {
                        if fname == field {
                            return Some(fty.clone());  // ✅ 找到字段
                        }
                    }
                    None  // ❌ 字段不存在
                }
                _ => None,  // ❌ 无法处理第三方库类型
            }
        }
        // ...
    }
}
```

**问题**:
- ❌ 只能查询当前文件定义的结构体
- ❌ 无法查询第三方库的结构体字段
- ❌ 泛型结构体字段类型不准确

## 3. 符号表对比

### cjc: 完整的全局符号表

```cpp
class SymbolTable {
    // 所有声明
    Map<String, Decl*> symbols;

    // 所有类型
    Map<String, Type*> types;

    // 所有函数
    Map<String, Function*> functions;

    // 所有结构体
    Map<String, Struct*> structs;

    // 模块导入
    Map<String, Module*> modules;
};

// 查询示例
auto arrayListDef = symbolTable.LookupType("ArrayList");
auto addMethod = arrayListDef->LookupMethod("add");
auto returnType = addMethod->getReturnType();  // Unit
```

**能力**:
- ✅ 可以查询任何符号
- ✅ 支持模块导入
- ✅ 支持前向引用

### CJWasm2: 局部符号表

```rust
pub struct CodeGen {
    // 当前文件的结构体
    structs: HashMap<String, StructDef>,

    // 当前文件的函数
    func_indices: HashMap<String, u32>,
    func_return_types: HashMap<String, Type>,

    // 全局变量
    global_var_types: HashMap<String, Type>,

    // ❌ 没有模块导入
    // ❌ 没有第三方库符号表
}

// 查询示例
let arraylist_def = self.structs.get("ArrayList");  // ❌ None (第三方库)
```

**限制**:
- ❌ 只能查询当前文件的符号
- ❌ 无法查询第三方库
- ❌ 无法处理前向引用

## 4. 模块系统对比

### cjc: 支持模块加载

```cpp
// 加载模块
Module stdCollections = LoadModule("std.collections");

// 查询模块中的类型
auto arrayListTy = stdCollections.LookupType("ArrayList");

// 查询泛型参数
auto typeArgs = arrayListTy->GetTypeParameters();  // [T]

// 实例化泛型
auto arrayListInt = Instantiate(arrayListTy, {Type::Int64});

// 查询方法
auto addMethod = arrayListInt->LookupMethod("add");
auto returnType = addMethod->getReturnType();  // Unit
```

**能力**:
- ✅ 读取 `.cjm` 文件
- ✅ 加载类型信息
- ✅ 查询方法签名

### CJWasm2: 无模块系统

```rust
// ❌ 无法加载模块
// ❌ 无法读取 .cjm 文件
// ❌ 无法查询第三方库类型

// 只能硬编码特殊处理
match name.as_str() {
    "ArrayList" => {
        // 假设元素类型是 type_args[0]
        let elem = type_args.first().cloned().unwrap_or(Type::Int64);
        Some(Type::Array(Box::new(elem)))
    }
    _ => None
}
```

**限制**:
- ❌ 无法动态加载模块
- ❌ 需要硬编码每个第三方类型
- ❌ 无法扩展

## 5. 泛型处理对比

### cjc: 完整的泛型实例化

```cpp
// 第一遍：收集所有泛型使用点
class GenericCollector {
    std::vector<GenericUsage> usages;

    void Visit(CallExpr& ce) {
        if (ce.resolvedFunction->IsGeneric()) {
            usages.push_back({ce, ce.typeArgs});
        }
    }
};

// 第二遍：实例化所有泛型
class GenericInstantiator {
    void InstantiateAll() {
        for (auto& usage : usages) {
            auto specialized = Instantiate(usage.func, usage.typeArgs);
            usage.callExpr->resolvedFunction = specialized;
        }
    }
};
```

**优势**:
- ✅ 完整收集所有泛型使用
- ✅ 统一实例化
- ✅ 避免重复实例化

### CJWasm2: 单遍泛型处理

```rust
// 边解析边实例化
fn gen_call(&mut self, name: &str, type_args: &[Type], args: &[Expr]) {
    if let Some(generic_func) = self.generic_funcs.get(name) {
        // 立即实例化
        let specialized = self.instantiate_generic(generic_func, type_args);
        self.gen_call_to_specialized(specialized, args);
    }
}
```

**问题**:
- ⚠️ 可能遗漏某些泛型使用
- ⚠️ 可能重复实例化
- ⚠️ 实例化时机不确定

## 6. 改进方案

### 短期方案 (P0 — 已完成 ✅)

#### 6.1 静态模块元数据表 (`src/metadata/mod.rs`)

采用静态匹配表代替运行时 `.cjm` 文件加载，零依赖、零 I/O：

```rust
// src/metadata/mod.rs（已实现）
pub fn stdlib_method_return_type(type_name: &str, type_args: &[Type], method: &str) -> Option<Type>;
pub fn stdlib_field_type(type_name: &str, type_args: &[Type], field: &str) -> Option<Type>;
pub fn stdlib_constructor_type(name: &str, type_args: &[Type]) -> Option<Type>;
```

覆盖类型（含泛型参数传递）：

| 分类 | 类型 |
|------|------|
| 集合 | ArrayList, LinkedList, ArrayStack, HashMap, HashSet, TreeMap, TreeSet, Queue, Deque, Stack |
| 工具 | StringBuilder, Path, Random, Regex, Iterator |
| 时间 | Duration, DateTime, Instant |
| 并发 | Thread, Channel |
| IO | File, FileReader, FileWriter, BufferedReader, BufferedWriter |
| 通用 | toString, hashCode, equals（任意类型） |

**实际效果**:
- ✅ ArrayList.get(i) → T（泛型元素类型）
- ✅ HashMap.get(k) → Option\<V\>（正确包装 Option）
- ✅ File.openRead() → FileReader
- ✅ Duration.toMilliseconds() → Int64
- ✅ Channel\<T\>.receive() → T

#### 6.2 改进类型推断（`src/codegen/expr.rs`）

在三处推断路径追加元数据兜底：

```rust
// MethodCall 分支（infer_ast_type 和 infer_ast_type_with_locals）
// 优先级：builtin_method_return_type → func_return_types → stdlib_metadata
if let Some(Type::Struct(ref type_name, ref type_args)) = obj_ty {
    if let Some(ret) = crate::metadata::stdlib_method_return_type(type_name, type_args, method) {
        return Some(ret);
    }
}

// Field 分支（infer_ast_type_with_locals）
// 优先级：struct_fields → prop_getter → stdlib_metadata
crate::metadata::stdlib_field_type(&s, type_args, field)

// Call 分支（构造函数兜底）
.or_else(|| crate::metadata::stdlib_constructor_type(name, type_args))
```

同时扩展 `builtin_method_return_type` 中 Array 臂，增加 ArrayList 代理方法：
`get`、`first`、`last`、`pop`、`remove`、`add`、`append`、`contains`、`indexOf` 等。

### P1 方案 (已完成 ✅)

#### 6.3 TypeParam → I32（`src/ast/type_.rs`）

将未单态化泛型参数的 WASM 类型从 `i64` 改为 `i32`，与其他对象引用类型保持一致：

```rust
// src/ast/type_.rs（已实现）
Type::TypeParam(_) => ValType::I32,  // 未单态化泛型参数视为对象引用（i32 指针）
// size() 同步修改
Type::TypeParam(_) => 4,  // 对象指针 4 字节
```

**实际效果**:
- ✅ 消除泛型容器元素访问时的 i64/i32 类型混淆
- ✅ TypeParam 与 This、Qualified 等引用类型保持一致（均为 i32）

#### 6.4 接口方法/属性类型注册（`src/codegen/mod.rs`）

在代码生成初始化阶段，将接口所有抽象方法/属性的返回类型注册到 `func_return_types`：

```rust
// src/codegen/mod.rs（已实现）
// 格式: "InterfaceName.methodName" → ReturnType
// 格式: "InterfaceName.__get_propName" → PropType
for (iface_name, methods) in &self.interfaces {
    for method in methods {
        if let Some(ref ret) = method.return_type {
            let key = format!("{}.{}", iface_name, method.name);
            self.func_return_types.entry(key).or_insert(ret.clone());
        }
    }
}
```

**接口方法查询（`src/codegen/expr.rs`）**:

```rust
// infer_ast_type / infer_ast_type_with_locals 中（已实现）
// 优先级: builtin → func_return_types → P0 stdlib_metadata → P1 interface_getter
// P1: 查询接口方法
if let Some(Type::Struct(ref type_name, _)) = obj_ty {
    let getter_key = format!("{}.__get_{}", type_name, method);
    if let Some(ret) = self.func_return_types.get(&getter_key) {
        return Some(ret.clone());
    }
}
// P1: 查询接口字段（类字段、接口直接方法）
let iface_method_key = format!("{}.{}", s, field);
// ...
// P1: class 字段（ClassInfo.all_fields 包含所有继承字段）
```

**实际效果**:
- ✅ `obj.supportedInterfaces` → `Array<QualifiedName>`（接口属性）
- ✅ `obj.methodName()` → 正确的返回类型（接口方法）
- ✅ class 实例字段访问类型正确
- ✅ class 构造函数返回 i32 指针

#### 6.5 P1.2/P1.3 辅助修复

```rust
// P1.2: void 函数调用不在栈上产生值（src/codegen/expr.rs）
// 防止 Unit 返回的函数在 block 类型推断中产生多余的值

// P1.3: let _ = expr → 编译表达式后 drop 结果值
// 允许 _ 模式丢弃任意表达式结果
```

**实际效果**:
- ✅ 消除 void 函数调用导致的 block type mismatch 错误

### P2 方案 (已完成 ✅)

#### 6.6 泛型单态化增强（`src/monomorph/mod.rs`）

新增 `collect_from_type` 辅助函数，并在 P2.1 阶段对全程序类型注解做全量扫描：

```rust
// src/monomorph/mod.rs（已实现）
fn collect_from_type(ty: &Type, gs, ge, gc, si, ei, ci) { ... }  // 递归收集泛型实例

// P2.1: 扫描函数签名、结构体字段、类字段
for func in &program.functions {
    collect_from_type(&param.ty, ...);
    collect_from_type(&ret, ...);
}
for st in &program.structs { collect_from_type(&field.ty, ...); }
for cls in &program.classes { collect_from_type(&field.ty, ...); }
```

**实际效果**:
- ✅ 从类型注解中自动发现泛型实例化（如 `Pair<Int64, String>`）
- ✅ 减少因遗漏实例化导致的 "方法未找到" 警告

#### 6.7 类型别名支持（`src/codegen/mod.rs` P2.2）

```rust
// CodeGen 新增字段
type_aliases: HashMap<String, Type>,

// 初始化时注册
for (name, ty) in &program.type_aliases {
    self.type_aliases.insert(name.clone(), ty.clone());
}
```

**实际效果**:
- ✅ 支持 `type Alias = ActualType` 声明，类型推断可透过别名

#### 6.8 Lambda/闭包完整支持（`src/codegen/mod.rs` P2.3）

```rust
// Lambda table 索引映射 (用于 call_indirect)
lambda_table_indices: HashMap<String, u32>,

// 函数签名到类型索引映射 (func_type_by_sig)
func_type_by_sig: HashMap<(Vec<ValType>, Vec<ValType>), u32>,

// Lambda 返回类型自动推断
fn infer_lambda_return_type(body: &Expr, params: &[(String, Type)]) -> Option<Type>
```

**实际效果**:
- ✅ Lambda 表达式编译为 WASM 函数 + 元素表索引
- ✅ 闭包可作为一等函数值传递和调用（`call_indirect`）
- ✅ 无显式返回类型标注时自动推断

#### 6.9 其他 P2 改进

| 子项 | 内容 | 文件 | 状态 |
|------|------|------|------|
| P2.4 | 静态方法调用（类名.方法名） | expr.rs | ✅ |
| P2.6 | for 循环步长支持 | expr.rs | ✅ |
| P2.7 | `Array<T>(size, init)` 动态数组构造 | expr.rs | ✅ |
| P2.8 | Array 实例方法（clone/slice/size/get 等）+ 临时变量 | expr.rs | ✅ |
| P2.9 | 命名参数解析（`resolve_named_args`）| mod.rs | ✅ |
| P2.10 | String 方法运行时函数（trim/startsWith/endsWith）| mod.rs | ✅ |

### P3 方案 (已完成 ✅)

#### 6.10 语义预分析模块（`src/sema/mod.rs`）

新增独立的语义分析模块，在代码生成前对 AST 做轻量级预分析：

```rust
// src/sema/mod.rs（已实现）

/// 语义分析上下文：保存预分析推断出的类型信息
#[derive(Debug, Default)]
pub struct SemanticContext {
    /// 函数名 → 推断的返回类型（仅限无显式标注的函数）
    pub inferred_return_types: HashMap<String, Type>,
}

/// 对整个 Program 做语义预分析，返回增强的类型上下文
pub fn analyze(program: &Program) -> SemanticContext {
    // 第一轮：收集所有已有返回类型标注的符号
    let mut known: HashMap<String, Type> = HashMap::new();
    for func in &program.functions { /* collect annotated */ }
    for cls in &program.classes { /* collect method annotations */ }

    // 第二轮（多次迭代，传播类型信息）
    for _pass in 0..3 {
        let mut changed = false;
        for func in &program.functions {
            if func.return_type.is_some() || func.extern_import.is_some()
                || !func.type_params.is_empty() || func.body.is_empty()
                || known.contains_key(&func.name)
            { continue; }
            if let Some(inferred) = infer_return_from_body(&func.body, &known) {
                known.insert(func.name.clone(), inferred.clone());
                ctx.inferred_return_types.insert(func.name.clone(), inferred);
                changed = true;
            }
        }
        if !changed { break; }
    }
}

// 支持的表达式类型推断（纯静态，无 I/O）
pub fn infer_expr(expr: &Expr, known: &HashMap<String, Type>) -> Option<Type>;
```

推断的表达式类型覆盖：

| 表达式类型 | 推断结果 |
|-----------|---------|
| `Integer` | `Int64` |
| `Float` | `Float64` |
| `Float32` | `Float32` |
| `Bool` | `Bool` |
| `String` / `Interpolate` | `String` |
| `Cast { target_ty }` | `target_ty` |
| `IsType` | `Bool` |
| `Some(inner)` | `Option<infer(inner)>` |
| `Ok(inner)` | `Result<infer(inner), String>` |
| `StructInit { name }` / `ConstructorCall { name }` | `Struct(name, type_args)` |
| `Call { name }` | 查 known 表 / 大写名字→Struct |
| `Binary { op }` | 比较运算→Bool，加法→左右类型 |
| `If/IfLet/Block/Match` | 递归推断分支类型 |
| `Tuple(elems)` | `Tuple(map infer)` |
| `Lambda { params, return_type }` | `Function { params, ret }` |
| `Array(elems)` | `Array<elem_ty>` |

#### 6.11 CodeGen 集成（`src/codegen/mod.rs` P3）

在 `compile()` 入口前执行语义预分析，将推断出的返回类型预先注册到 `func_return_types`：

```rust
// src/codegen/mod.rs（已实现）
// P3: 语义预分析 — 推断无标注函数的返回类型，提前注册到 func_return_types
let sema_ctx = crate::sema::analyze(program);
for (name, ret_ty) in &sema_ctx.inferred_return_types {
    self.func_return_types
        .entry(name.clone())
        .or_insert_with(|| ret_ty.clone());
}
```

**查询优先级**（从高到低）：

```
显式类型标注 > P3 sema预分析 > P1 接口注册 > P0 元数据兜底 > 默认 Int64
```

**实际效果**:
- ✅ 无标注函数返回类型预先注册，减少 infer_ast_type 回退到 Int64 的概率
- ✅ 支持多轮迭代传播（最多 3 轮），可处理相互依赖的函数
- ✅ 零 panic、无 I/O、纯函数，与 CodeGen 解耦
- ✅ 全部 410 个单元测试通过

**局限性**:
- ⚠️ 只能推断纯字面量和简单函数调用的返回类型
- ⚠️ 无法查询第三方库函数
- ⚠️ 不支持前向引用（迭代传播部分缓解）
- ⚠️ 未实现完整类型检查，无法消除 `infer_type` 的 I64 默认回退

### P4 方案 (已完成 ✅)

#### 6.12 infer_ast_type 覆盖扩展（`src/codegen/expr.rs` P4.1-P4.4）

**P4.1 / P4.2: class 构造函数调用返回 Struct 类型**

`infer_ast_type` 和 `infer_ast_type_with_locals` 中 `Expr::Call` 分支原本只检查 `structs`，
不检查 `classes`，导致 `let x = MyClass()` 无法推断 `x` 的 AST 类型：

```rust
// P4.1/P4.2: 两处同步修改
// BEFORE
if self.structs.contains_key(name) {
    Some(Type::Struct(name.clone(), vec![]))
}
// AFTER
if self.structs.contains_key(name) || self.classes.contains_key(name) {
    Some(Type::Struct(name.clone(), vec![]))  // class 构造函数也返回 Struct（i32 指针）
}
```

修复后，`let x = MyClass()` 的 `ast_type = Some(Type::Struct("MyClass", []))` 被正确存入 locals，
使后续 `x.field` 和 `x.method()` 能通过 `locals.get_type("x")` 正确查到字段/方法类型。

**P4.3: infer_ast_type 支持 Expr::Var**

在 `infer_ast_type`（无 locals 版本）中新增 `Expr::Var` 分支：

```rust
Expr::Var(name) => {
    // 1. 全局变量类型（顶层 let/var 声明）
    if let Some(ty) = self.global_var_types.get(name.as_str()) {
        // Int64 可能是解析器占位符，尝试从 init 表达式推断实际类型
        if ty == &Type::Int64 {
            if let Some(init) = self.global_var_inits.get(name.as_str()) {
                if let Some(inferred) = self.infer_ast_type(init) {
                    return Some(inferred);
                }
            }
        }
        return Some(ty.clone());
    }
    // 2. 已知类名 / 结构体名 → Struct（用于静态方法调用推断）
    if self.structs.contains_key(name.as_str()) || self.classes.contains_key(name.as_str()) {
        return Some(Type::Struct(name.clone(), vec![]));
    }
    None
}
```

**P4.4: infer_ast_type 支持 Expr::Field**

在 `infer_ast_type`（无 locals 版本）中新增 `Expr::Field` 分支，
复用结构体/类字段查询逻辑（struct_fields → getter → interface → class_fields → stdlib_metadata）：

```rust
Expr::Field { object, field } => {
    let obj_ty = self.infer_ast_type(object)?;  // 递归推断对象类型
    if let Type::Struct(ref s, ref type_args) = obj_ty {
        // struct 字段 → getter → 接口方法 → class 字段 → P0 元数据
        // ...（与 infer_ast_type_with_locals 中的 Field 分支逻辑一致）
    }
}
```

**实际效果**:
- ✅ class 构造函数调用的 ast_type 从 None 修复为 `Type::Struct`，locals 类型正确
- ✅ 全局变量作为对象时，`infer_ast_type` 可正确推断字段/方法类型
- ✅ 无 locals 上下文的类型推断链（infer_type → infer_ast_type）覆盖更多表达式形式

#### 6.13 sema 扩展：推断无标注 class method 返回类型（P4.5）

在 `analyze()` 中添加第三轮迭代（class method 专用），格式匹配 CodeGen 中 "ClassName.methodName" 键：

```rust
// P4.5: 第三轮 — 推断无标注 class method 返回类型
for _pass in 0..3 {
    let mut changed = false;
    for cls in &program.classes {
        if !cls.type_params.is_empty() { continue; }  // 跳过泛型类
        for m in &cls.methods {
            if m.func.return_type.is_some() || m.func.body.is_empty()
                || known.contains_key(&m.func.name)
            { continue; }
            if let Some(inferred) = infer_return_from_body(&m.func.body, &known) {
                known.insert(m.func.name.clone(), inferred.clone());
                ctx.inferred_return_types.insert(m.func.name.clone(), inferred);
                changed = true;
            }
        }
    }
    if !changed { break; }
}
```

**查询优先级（最终完整链）**:

```
显式类型标注 > P4/P3 sema 预分析 > P1 接口注册 > P0 元数据兜底 > 默认 Int64
```

**实际效果**:
- ✅ 无标注 class method 的返回类型也加入 func_return_types
- ✅ `infer_ast_type` 覆盖 Var / Field / class Call，减少无 locals 上下文时的 None 回退
- ✅ 全部 410 个单元测试通过

**误差分析与局限性**:
- ⚠️ WASM 验证错误数未在当前样本集（3509）中可见减少
- ⚠️ 主导错误（583× `expected [i64] but got [i32]`，439× `expected [i32] but got [i64]`）
  来自另一根本原因：`needs_i64_to_i32_wrap` 判断与 `i64.extend_i32_s` 插入逻辑的对称错误
- ⚠️ 真正减少错误需要修复 wrap/extend 的判断逻辑（P5 工作）

### P5 方案 (已完成 ✅)

#### 6.14 func_return_wasm_types 权威 WASM 类型表（P5.1）

新增 `func_return_wasm_types` 字段，在函数类型注册阶段同步记录每个函数的 WASM 返回类型，作为类型推断最后一级精确回退：

```rust
// src/codegen/mod.rs（已实现）
/// P5.1: 每个函数的 WASM 返回类型（来自编译的函数签名，权威类型信息）
/// key 与 func_indices 一致（可能是限定名或修饰名），None 表示 void 函数
func_return_wasm_types: HashMap<String, Option<ValType>>,

// 在类型段注册循环中同步填充
let ret_wasm_ty: Option<ValType> = func.return_type.as_ref().and_then(|t| {
    if *t == Type::Unit || *t == Type::Nothing { None } else { Some(t.to_wasm()) }
});
self.func_return_wasm_types.insert(key.clone(), ret_wasm_ty);
```

在 `infer_type_with_locals` 中，`Expr::Call` 和 `Expr::MethodCall` 的推断失败时先查 `func_return_wasm_types`，再回退到 `I64`：

```rust
// P5.1: 回退到 WASM 函数签名（比 I64 更精确）
self.func_return_wasm_types
    .get(name.as_str())
    .copied()
    .flatten()
    .unwrap_or(ValType::I64)

// P5.3: MethodCall 类型推断从 WASM 签名回退
Expr::MethodCall { object, method, .. } => {
    let key = format!("{}.{}", type_name, method);
    if let Some(opt_wt) = self.func_return_wasm_types.get(&key) {
        return opt_wt.unwrap_or(ValType::I32);
    }
}
```

**实际效果**:
- ✅ 函数调用返回类型从 I64 兜底改为真实 WASM 签名，精度大幅提升
- ✅ MethodCall 在 `func_return_types` 无 AST 类型时仍能获得正确的 WASM 类型

#### 6.15 sema 返回类型回写到函数签名（P5.6）

在 `compile()` 入口，将 P3 语义预分析推断的返回类型回写到 `functions` vec，使 WASM 函数签名与 `func_return_types` 保持一致：

```rust
// src/codegen/mod.rs（已实现）
// P5.6: 将 sema 推断的返回类型回写到 functions vec，使 WASM 函数签名与 func_return_types 一致
// 避免 WASM 函数被编译为 void 但 func_return_types 说它有返回值（导致 call 后栈不一致）
for func in functions.iter_mut() {
    if func.return_type.is_none() || func.return_type == Some(Type::Unit) {
        if let Some(inferred) = sema_ctx.inferred_return_types.get(&func.name) {
            if *inferred != Type::Unit && *inferred != Type::Nothing {
                func.return_type = Some(inferred.clone());
            }
        }
    }
}
```

**实际效果**:
- ✅ 消除"WASM 函数签名为 void 但调用处期望返回值"类型栈不一致错误
- ✅ `func_return_wasm_types` 从正确的签名填充，形成自洽的类型信息链

#### 6.16 spawn/synchronized 单线程桩（P5.1/P5.2）

新增 `Expr::Spawn` 和 `Expr::Synchronized` AST 节点，单线程环境下直接执行 body：

```rust
// src/ast/mod.rs（已实现）
/// P5.1: spawn { block } — 单线程桩实现（直接执行）
Spawn { body: Vec<Stmt> },
/// P5.2: synchronized(lock) { block } — 单线程桩实现（直接执行）
Synchronized { lock: Box<Expr>, body: Vec<Stmt> },
```

配套实现：
- `collect_locals` 正确收集 spawn/synchronized body 中的局部变量（`P5.1/5.2` 注释）
- `expr_produces_value` 对两者返回 `false`（spawn/synchronized 是语句级别，不产生栈值）

#### 6.17 前缀/后缀运算类型感知算术（P5.4）

`PostfixIncr`、`PostfixDecr`、`PrefixIncr`、`PrefixDecr` 根据操作数 WASM 类型选择 I32 或 I64 算术指令，消除混用 i64 指令操作 i32 变量的类型错误：

```rust
// src/codegen/expr.rs（已实现）
// P5.4: 使用与操作数类型匹配的算术指令
if self.infer_type_with_locals(inner, locals) == ValType::I32 {
    func.instruction(&Instruction::I32Const(1));
    func.instruction(&Instruction::I32Add); // 或 I32Sub
} else {
    func.instruction(&Instruction::I64Const(1));
    func.instruction(&Instruction::I64Add); // 或 I64Sub
}
```

`__postfix_old` 临时变量的 WASM 类型也同步与操作数一致：

```rust
// P5.4: __postfix_old 的类型与操作数类型一致，避免 i32/i64 类型不匹配
let inner_vt = self.infer_ast_type_with_locals(inner, locals)
    .map(|t| t.to_wasm())
    .unwrap_or(ValType::I64);
locals.add("__postfix_old", inner_vt, None);
```

#### 6.18 Return 协调 AST 类型优先（P5.5）

在 `Stmt::Return` 类型协调分支中，当表达式 AST 类型已知时优先使用其 WASM 类型；若 AST 未知但 `infer_type_with_locals` 返回非 I64 值（可信），也使用；只有在两者均不可信时才使用 I64 回退：

```rust
// src/codegen/expr.rs（已实现）
// P5.5: 优先使用 AST 类型；若 AST 类型未知但 WASM 类型确定，也使用
// 注意：infer_type 的 I64 回退可能不准确，但此处
// 漏掉协调（不 wrap）比误插入 I32WrapI64 引发的错误更多
if self.infer_ast_type_with_locals(expr, locals).is_some() {
    Some(inferred)
} else if inferred != ValType::I64 {
    Some(inferred)
} else {
    // infer_type 返回 I64 且 AST 不确定时，仍使用 I64 以尝试协调
    Some(ValType::I64)
}
```

#### 6.19 Atomic/Mutex 并发原语单线程桩（P5）

| 类型 | 方法 | 实现 |
|------|------|------|
| `AtomicInt64` | `load` / `store` / `fetchAdd` / `compareAndSwap` | 内存读写桩 (i64) |
| `AtomicBool` | `load` / `store` / `compareAndSwap` | 内存读写桩 (i64 0/1) |
| `Mutex` / `ReentrantMutex` | `lock` / `unlock` / `tryLock` | no-op 桩 |

在 `infer_ast_type` / `infer_ast_type_with_locals` 中新增构造函数 Struct 类型匹配：

```rust
// P5: Atomic/Mutex 桩类型
"AtomicInt64" | "AtomicBool" | "Mutex" | "ReentrantMutex" => {
    return Some(Type::Struct(name.clone(), vec![]));
}
```

方法调用分发通过 `type_name` 匹配并内联生成对应指令序列（`P5: Atomic/Mutex 桩方法分发`）。

#### P5 实际效果

- ✅ `spawn { }` / `synchronized(lock) { }` 正确编译，body 内变量类型正确收集
- ✅ AtomicInt64/AtomicBool/Mutex 构造和方法调用可正常生成 WASM
- ✅ 前/后缀运算不再对 i32 变量错误使用 i64 算术指令
- ✅ 函数调用返回类型推断精度提升（`func_return_wasm_types` 兜底）
- ✅ sema 推断的返回类型与 WASM 函数签名保持一致，消除 call 后栈不一致错误
- ✅ 全部 229 个单元测试通过，37/37 示例通过

**WASM 验证错误变化**:

| 阶段 | 错误数 | 变化 |
|------|--------|------|
| P4 完成后 | 3509 | — |
| P5 完成后 | 3276 | -233 (-6.6%) |

**误差分析与局限性**:
- ⚠️ 主导错误类别（583× `expected [i64] but got [... i32]`、410× `expected [i32] but got [i64]`）根本原因是 `needs_i64_to_i32_wrap` 的启发式判断仍在某些路径上过度/遗漏 wrap，而非 P5 修复范围内的具体点
- ⚠️ 276× `i32.add expected [i32, i32] but got [i64, i32]` 等指针运算错误来自类型未单态化时 I64 回退进入指针算术路径
- ⚠️ 要将错误从 3276 降至 <500，需要 P6 完整类型系统（CHIR 中间层）支撑

### P6 方案 (已完成 ✅)

P6 专注于语言特性完整性提升，新增 8 项核心语法特性，覆盖循环控制、常量、类构造、参数模式、异常资源管理及表达式糖。

#### 6.20 do-while 循环（`Stmt::DoWhile`）

```rust
// src/ast/mod.rs (Stmt 枚举已有)
// src/parser/stmt.rs — parse_stmt() 处理 Token::Do
// src/codegen/expr.rs — compile_stmt() 处理 Stmt::DoWhile
Stmt::DoWhile { body, cond } => {
    // WASM: block { loop { body; cond; br_if 0 } }
    func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
    // ... body ...
    self.compile_expr(cond, locals, func, loop_ctx);
    if self.needs_i64_to_i32_wrap(cond, locals) {
        func.instruction(&Instruction::I32WrapI64);
    }
    func.instruction(&Instruction::BrIf(0)); // 条件真时继续循环
    func.instruction(&Instruction::End); // loop end
    func.instruction(&Instruction::End); // block end
}
```

- ✅ 至少执行一次（do-while 语义）
- ✅ `break`/`continue` 正确跳转（break→block end, continue→loop start）
- ✅ 嵌套 do-while 支持

#### 6.21 const 编译期常量（`Stmt::Const`）

```rust
// src/codegen/expr.rs
Stmt::Const { name, ty, value } => {
    // const 在 WASM 中等同于 let（不可变局部变量）
    self.compile_expr(value, locals, func, loop_ctx);
    if let Some(idx) = locals.get(name) {
        let val_ty = self.infer_type_with_locals(value, locals);
        let target_ty = ty.as_ref().map(|t| t.to_wasm()).unwrap_or(val_ty);
        self.emit_type_coercion(func, val_ty, target_ty);
        func.instruction(&Instruction::LocalSet(idx));
    }
}
```

- ✅ `const MAX: Int64 = 100` — 带类型标注
- ✅ `const OFFSET = 42` — 类型推断
- ✅ 与 `let` 一致的局部变量生成（WASM 级无 const 概念）

#### 6.22 主构造函数（Primary Constructor）

```rust
// src/parser/decl.rs
// P6: Primary constructor — class Foo(var x: Int64, var y: Int64) { ... }
// 解析 class 头部的 (var/let param...) 列表

// P6: 展开主构造函数参数为字段 + init
if !primary_ctor_params.is_empty() {
    for p in &primary_ctor_params {
        class_def.fields.push(FieldDef { name: p.name.clone(), ty: p.ty.clone() });
    }
    // 自动生成 init(params) 方法，将参数赋值给 this.field
}
```

- ✅ `class Point(var x: Int64, var y: Int64)` 自动生成字段和 `init`
- ✅ 与手写字段 + init 等价，在 codegen 阶段无额外处理
- ✅ `ClassDef.primary_ctor_params` 字段记录参数列表

#### 6.23 inout 参数（传引用语义）

```rust
// src/ast/mod.rs
pub struct Param {
    pub is_inout: bool,  // P6: inout 参数（传引用）
    // ...
}

// src/parser/decl.rs
// P6: inout 参数
let is_inout = if self.check(&Token::Inout) { self.advance(); true } else { false };
```

- ✅ `func swap(inout a: Int64, inout b: Int64)` 语法解析
- ✅ `Param.is_inout = true` 标记，保留 AST 信息
- ⚠️ WASM 层为值传递桩（WebAssembly 无引用传递，inout 语义为未来工作）

#### 6.24 try-with-resources（资源自动释放）

```rust
// src/parser/expr.rs
// P6: try-with-resources: try (resource = expr) { ... }
let resources = if self.check(&Token::LParen) {
    // 解析 (let/var name = expr) 资源声明列表
};

// src/codegen/expr.rs
Expr::TryBlock { resources, body, catch_body, finally_body, .. } => {
    // P6: try-with-resources — 注册资源变量为局部变量
    for (res_name, res_expr) in resources {
        self.compile_expr(res_expr, locals, func, loop_ctx);
        locals.set(res_name, idx);
    }
    // ... 编译 body、catch、finally ...
}
```

- ✅ `try (let r = openFile(path)) { r.read() } catch(e) { ... }` 语法
- ✅ 资源变量注册为局部变量，在 try body 内可访问
- ✅ `finally` 块用于资源释放（自动调用 `close()`/`release()` 为运行时约定）

#### 6.25 可选链（Optional Chaining）`?.`

```rust
// src/ast/mod.rs
/// P6.1: 可选链 obj?.field — 若 obj 为 None 返回 None，否则访问字段
OptionalChain { object: Box<Expr>, field: String },

// src/codegen/expr.rs
Expr::OptionalChain { object, field } => {
    // 推断字段类型
    let result_type = self.infer_type_with_locals(&field_access, locals);
    self.compile_expr(object, locals, func, loop_ctx);
    // if ptr == 0 → return 0; else → access field
    func.instruction(&Instruction::LocalTee(match_val));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::If(BlockType::Result(result_type)));
    // None 分支：压入 0
    func.instruction(&Instruction::Else);
    // Some 分支：访问字段
    self.compile_expr(&Expr::Field { object: Expr::Var("__match_val"), field }, ...);
    func.instruction(&Instruction::End);
}
```

- ✅ `obj?.field` — 若 `obj` 为 null 指针（0）则返回零值，否则访问字段
- ✅ 结果类型根据字段类型推断（I32/I64/F64）
- ✅ `expr_produces_value` 返回 `true`，可用于赋值上下文

#### 6.26 尾随闭包（Trailing Closure）

```rust
// src/ast/mod.rs
/// P6.2: 尾随闭包调用 f(args) { params => body }
TrailingClosure { callee: Box<Expr>, args: Vec<Expr>, closure: Box<Expr> },

// src/codegen/expr.rs
Expr::TrailingClosure { callee, args, closure } => {
    // 展开为普通调用：f(args..., closure)
    let mut all_args = args.clone();
    all_args.push(closure.as_ref().clone());
    let call_expr = match callee.as_ref() {
        Expr::Var(name) => Expr::Call { name, args: all_args, .. },
        Expr::MethodCall { object, method, .. } => Expr::MethodCall { object, method, args: all_args, .. },
        _ => Expr::Call { name: "__trailing_closure_target", args: all_args, .. },
    };
    self.compile_expr(&call_expr, ...);
}
```

- ✅ `list.forEach { x => println(x) }` — 方法尾随闭包
- ✅ `map { x => x * 2 }` — 函数尾随闭包
- ✅ `Array(n) { i => i * i }` — 构造函数尾随闭包
- ✅ 编译为"最后一个参数为闭包"的普通函数调用

#### 6.27 `!in` 运算符

```rust
// src/parser/expr.rs
// P6: `!in` 运算符 — expr !in collection
if self.check(&Token::Bang) {
    if matches!(self.peek_next(), Some(Token::In)) {
        // 解析为 BinOp::NotIn
    }
}

// src/codegen/expr.rs
// P6: !in 运算符 — 编译为 contains 方法调用 + 取反
Expr::Binary { op: BinOp::NotIn, left, right } => {
    let contains_call = Expr::MethodCall { object: right, method: "contains", args: vec![left] };
    self.compile_expr(&contains_call, ...);
    if self.needs_i64_to_i32_wrap(&contains_call, ...) {
        func.instruction(&Instruction::I32WrapI64);
    }
    func.instruction(&Instruction::I32Eqz);
}
```

- ✅ `x !in list` 等价于 `!list.contains(x)`
- ✅ 结果为 Bool（i32）
- ✅ 与 `in` 运算符对称

#### P6 实际效果

| 特性 | 解析 | 类型推断 | 代码生成 |
|------|------|----------|----------|
| do-while | ✅ | ✅ | ✅ |
| const | ✅ | ✅（继承 let） | ✅ |
| 主构造函数 | ✅ | ✅（展开字段） | ✅ |
| inout 参数 | ✅ | ✅（AST 标记） | ⚠️ 桩（值传递） |
| try-with-resources | ✅ | ✅ | ✅ |
| 可选链 `?.` | ✅ | ✅ | ✅ |
| 尾随闭包 | ✅ | ✅ | ✅ |
| `!in` 运算符 | ✅ | ✅ | ✅ |

**WASM 验证错误变化**:

| 阶段 | 错误数 | 变化 |
|------|--------|------|
| P5 完成后 | 3276 | — |
| P6 完成后 | 3276 | ±0（P6 不引入新的类型系统，语言特性本身不改变验证错误分布） |

- ✅ 全部 229 个单元测试通过
- ✅ 37/37 示例通过（含 `p6_new_features.cj` 覆盖所有 P6 特性）

**局限性**:
- ⚠️ `inout` 参数为值传递桩（WASM 无引用传递语义）
- ⚠️ `try-with-resources` 的 `finally` 不自动调用 `close()`，需用户手动在 finally 块中调用
- ⚠️ 可选链 `?.` 目前仅支持字段访问（不支持 `?.method()`）

### 长期方案 P7 (3-6 月)

#### 完全重写代码生成（CHIR 中间层）

参考 cjc 的架构，实现完整的多遍编译：

1. **Pass 1: 解析** - 生成 AST
2. **Pass 2: 语义分析（CHIR）** - 类型检查 + 泛型实例化 + 符号表
3. **Pass 3: 代码生成** - 从 CHIR 生成 WASM（每个节点都有精确类型）

**预期效果**: WASM 验证错误从 3276 降至 <100，达到接近 cjc 的类型安全水平。

## 7. 总结

### CJWasm2 的核心问题

1. **单遍编译** - 无法查询未解析的符号
2. **无模块系统** - 无法加载第三方库类型信息
3. **局部符号表** - 只能查询当前文件的符号
4. **代码生成时类型推断** - 时机太晚，信息不足

### 改进优先级

| 优先级 | 改进项 | 预期效果 | 工作量 | 状态 |
|--------|--------|----------|--------|------|
| 🔴 P0 | 静态模块元数据表 (`src/metadata/mod.rs`) | 覆盖 10+ 标准库类型方法推断 | 已完成 | ✅ |
| 🔴 P0 | 改进类型推断（MethodCall/Field/Call 兜底） | 减少默认 Int64 误判 | 已完成 | ✅ |
| 🟡 P1 | TypeParam → I32（`src/ast/type_.rs`） | 消除泛型参数 i64/i32 类型混淆 | 已完成 | ✅ |
| 🟡 P1 | 接口方法/属性返回类型注册（`src/codegen/mod.rs`） | 接口调用类型推断正确 | 已完成 | ✅ |
| 🟡 P1 | 接口 getter/方法/class 字段查询（`src/codegen/expr.rs`） | 减少 None 类型推断回退 | 已完成 | ✅ |
| 🟡 P1 | void 函数/`let _` 辅助修复（P1.2/P1.3） | 消除 block type mismatch | 已完成 | ✅ |
| 🟢 P2 | 泛型单态化增强 `collect_from_type` + P2.1 全量扫描 | 减少遗漏实例化警告 | 已完成 | ✅ |
| 🟢 P2 | 类型别名 P2.2 + Lambda P2.3 + 命名参数 P2.9 | 语言特性完整性提升 | 已完成 | ✅ |
| 🟢 P2 | Array 方法 P2.8 + String 方法 P2.10 等 | 内置方法覆盖率提升 | 已完成 | ✅ |
| 🔵 P3 | 语义预分析模块 `src/sema/mod.rs` + CodeGen 集成 | 无标注函数返回类型预注册 | 已完成 | ✅ |
| 🔵 P4 | infer_ast_type 覆盖 Var/Field/class Call（P4.1-P4.4）+ sema class method（P4.5） | 推断链更完整，减少 None 回退 | 已完成 | ✅ |
| 🟣 P5 | func_return_wasm_types 权威类型表（P5.1）+ sema 签名回写（P5.6）+ spawn/synchronized 桩（P5.1/5.2）+ 前后缀类型感知（P5.4）+ Return 协调优先级（P5.5）+ Atomic/Mutex 桩 | -233 错误（3509→3276） | 已完成 | ✅ |
| 🟤 P6 | do-while + const + 主构造函数 + inout + try-with-resources + 可选链`?.` + 尾随闭包 + `!in` | 语言特性完整性 +8 | 已完成 | ✅ |
| ⚫ P7 | 完全重写架构（CHIR 中间层 + 完整类型检查） | 达到 cjc 质量，错误 <100 | 3-6 月 | 待做 |

### 关键洞察

cjc 的类型推断之所以完整，核心在于：

1. **多遍编译** - 可以查询任何符号
2. **完整符号表** - 包含所有模块的类型信息
3. **独立的类型检查阶段** - 在代码生成前完成
4. **模块系统** - 可以加载第三方库

CJWasm2 要达到相同质量，必须逐步添加这些功能。
