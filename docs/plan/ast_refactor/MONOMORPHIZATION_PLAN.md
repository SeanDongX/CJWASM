# CJWasm 全单态化支持实现计划

## 概述

当前 CJWasm 的单态化 (monomorphization) 实现不完整，导致标准库编译失败。主要缺失：
- 隐式类型推断
- 泛型类方法单态化  
- 标准库内建泛型类型支持
- 跨文件实例化

**预计工作量**: 7-10 天

## 当前状态

### 已实现 ✅
- 类型名修饰 (name mangling)
- TypeParam 类型替换
- 显式 type_args 收集 (Call, StructInit, ConstructorCall)
- 生成单态化后的 struct/enum/class 定义
- 表达式重写为修饰后名字

### 未实现 ❌
- 隐式类型推断 (无显式 type_args 时)
- 泛型类方法的单态化
- 从返回类型/字段类型推断类型实参
- 跨文件实例化
- 标准库泛型类型 (Array, Map, Option 等)

---

## Phase 1: 完善实例化收集

**预计时间**: 2-3 天

### 1.1 隐式类型推断

**问题**: 当调用 `Array<Int64>()` 或 `Array(value)` 时，如果没有显式 `type_args`，单态化无法收集实例化。

**当前代码问题** (`src/monomorph/mod.rs:675-682`):
```rust
Call { name, type_args, .. } => {
    if let Option::Some(tas) = type_args.as_ref() {  // 只处理显式 type_args
        // ...
    }
}
```

**实现方案**:

1. 添加类型推断辅助函数 `infer_type_from_context()`
2. 修改 `visit_expr` 函数处理更多表达式类型

```rust
// 1. 首先添加类型推断辅助函数
fn infer_type_from_context(expr: &Expr, program: &Program) -> Option<Vec<Type>> {
    // 从变量声明推断: let x: Array<Int64> = ...
    // 从赋值推断: x = Array<Int64>()  
    // 从参数类型推断: func foo(arr: Array<String>)
    None  // TODO: 实现
}

// 2. 增强 visit_expr 处理隐式类型
MethodCall { object, method, args, .. } => {
    // 检查是否是泛型方法调用
    // 尝试从方法返回类型推断
    if let Some(inferred) = infer_method_type_args(object, method, args) {
        // 添加到 func_insts
    }
}
```

**需要修改的文件**: `src/monomorph/mod.rs`

### 1.2 从返回类型推断

**问题**: `func getOrThrow<T>(): T` 被调用时，需要从调用上下文推断 `T`。

**实现方案**:

```rust
// 在 collect_instantiations 中添加返回类型分析
fn collect_from_return_types(program: &Program) -> HashSet<(String, Vec<Type>)> {
    let mut insts = HashSet::new();
    
    for func in &program.functions {
        if func.type_params.is_empty() { continue; }
        
        // 遍历函数体中的 return 语句
        for stmt in &func.body {
            collect_return_types_from_stmt(stmt, &func.name, &func.return_type, &mut insts);
        }
    }
    insts
}

fn collect_return_types_from_stmt(stmt: &Stmt, func_name: &str, ret_type: &Option<Type>, insts: &mut HashSet<(String, Vec<Type>)>) {
    match stmt {
        Stmt::Return(Some(expr)) => {
            // 分析 expr 的类型，尝试推断泛型参数
        }
        Stmt::Expr(expr) => {
            // 分析表达式类型
        }
        _ => {}
    }
}
```

### 1.3 从字段类型推断

**问题**: 看到 `Foo<String>` 时，需要同时实例化 `Foo` 内部使用的 `Array<String>`, `Map<String, ...>` 等。

**实现方案**:

```rust
// 修改 collect_instantiations，在处理类时同时处理字段中的泛型
fn collect_field_instantiations(class_def: &ClassDef) -> HashSet<(String, Vec<Type>)> {
    let mut insts = HashSet::new();
    
    for field in &class_def.fields {
        collect_types_from_field(&field.ty, &mut insts);
    }
    insts
}

fn collect_types_from_field(ty: &Type, insts: &mut HashSet<(String, Vec<Type>)>) {
    match ty {
        Type::Array(inner) => {
            insts.insert(("Array".to_string(), vec![*inner.clone()]));
            collect_types_from_field(inner, insts);
        }
        Type::Map(k, v) => {
            insts.insert(("Map".to_string(), vec![*k.clone(), *v.clone()]));
            collect_types_from_field(k, insts);
            collect_types_from_field(v, insts);
        }
        Type::Option(inner) => {
            insts.insert(("Option".to_string(), vec![*inner.clone()]));
            collect_types_from_field(inner, insts);
        }
        Type::Struct(name, args) if !args.is_empty() => {
            insts.insert((name.clone(), args.clone()));
        }
        _ => {}
    }
}
```

---

## Phase 2: 泛型方法单态化

**预计时间**: 2 天

### 2.1 类方法收集

**问题**: 泛型类 `Option<T>` 的方法 `getOrThrow()` 没有被实例化。

**当前代码** (`src/monomorph/mod.rs:1210-1240`):
```rust
// 泛型类单态化 - 只处理了类本身，没有处理方法
for (name, type_args) in &class_insts {
    let def = program.classes.iter().find(...);
    // 只生成了类，没有生成方法
}
```

**实现方案**:

```rust
// 在类单态化后，添加方法单态化
for (name, type_args) in &class_insts {
    let class_def = program.classes.iter()
        .find(|c| &c.name == name && c.type_params.len() == type_args.len())
        .unwrap();
    
    let subst: HashMap<_, _> = class_def
        .type_params
        .iter()
        .cloned()
        .zip(type_args.iter().cloned())
        .collect();
    
    // 为每个方法生成单态化版本
    for method in &class_def.methods {
        let mangled_class_name = mangle_name(name, type_args);
        let method_name = format!("{}.{}", mangled_class_name, method.func.name);
        
        // 替换参数和返回类型中的 TypeParam
        let new_params: Vec<Param> = method.func.params.iter().map(|p| {
            Param {
                name: p.name.clone(),
                ty: substitute_type(&p.ty, &subst),
                ..p.clone()
            }
        }).collect();
        
        let new_return_type = method.func.return_type.as_ref()
            .map(|t| substitute_type(t, &subst));
        
        // 添加到 program.functions
        program.functions.push(Function {
            name: method_name,
            type_params: vec![],  // 方法不再是泛型
            params: new_params,
            return_type: new_return_type,
            body: method.func.body.iter().map(|s| {
                substitute_stmt(s.clone(), &subst, &rewrites)
            }).collect(),
            ..method.func.clone()
        });
    }
}
```

### 2.2 方法调用重写

**问题**: `option.getOrThrow()` 被解析为 `getOrThrow`，但实际应该是 `Option$Int64.getOrThrow`。

**当前代码** (`src/codegen/expr.rs:5388-5400`):
```rust
let struct_ty = obj_type
    .and_then(|ty| match ty {
        Type::Struct(s, _) => Some(s),
        Type::Option(_) => Some("Option".to_string()),
        Type::Result(_, _) => Some("Result".to_string()),
        _ => None,
    });
// 没有处理带类型参数的 Option<T>
```

**实现方案**:

```rust
// 增强 get_object_type 处理带类型参数的泛型类
fn get_object_type(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<Type> {
    match expr {
        Expr::Var(name) => {
            // 现有逻辑...
            
            // 新增: 检查泛型类的已实例化版本
            for (key, _) in &self.classes {
                if key.starts_with(name) && key.contains("$") {
                    // 例如: Array$Int64 匹配 Array
                    return Some(Type::Struct(key.clone(), vec![]));
                }
            }
        }
        // ...
    }
}

// 增强方法调用 key 生成
let key = if is_static {
    format!("{}.{}", type_name_opt.unwrap(), method)
} else {
    let obj_type = self.get_object_type(object, locals);
    let struct_name = match obj_type {
        Some(Type::Struct(s, args)) if !args.is_empty() => {
            // 使用修饰后的名字: Array$Int64.get
            mangle_name(&s, &args)
        }
        Some(Type::Struct(s, _)) => s,
        Some(Type::Option(inner)) => {
            mangle_name("Option", &[*inner])
        }
        Some(Type::Result(ok, err)) => {
            mangle_name("Result", &[*ok, *err])
        }
        _ => method.to_string()
    };
    format!("{}.{}", struct_name, method)
};
```

---

## Phase 3: 标准库支持

**预计时间**: 2-3 天

### 3.1 注册内建泛型类型

**问题**: `Array`, `Map` 等标准库类型未在 codegen 中注册。

**当前代码** (`src/codegen/mod.rs:288-320`):
```rust
// 只注册了 Option 和 Result 枚举
if !self.enums.contains_key("Option") {
    self.enums.insert("Option".to_string(), ...);
}
```

**实现方案**:

```rust
// 在 CodeGen::new() 中添加
pub fn new() -> Self {
    let mut gen = Self { ... };
    
    // 注册内建 Array 类型
    gen.structs.insert("Array".to_string(), StructDef {
        visibility: Visibility::Public,
        name: "Array".to_string(),
        type_params: vec!["T".to_string()],
        constraints: vec![],
        fields: vec![
            FieldDef { name: "ptr".to_string(), ty: Type::Int64, default: None },
            FieldDef { name: "length".to_string(), ty: Type::Int64, default: None },
            FieldDef { name: "capacity".to_string(), ty: Type::Int64, default: None },
        ],
    });
    
    // 注册常用 Array 实例
    gen.classes.insert("Array$Int64".to_string(), ClassInfo {
        name: "Array$Int64".to_string(),
        // ...
    });
    
    // 类似注册 Map, HashMap 等
}
```

### 3.2 预填充标准库实例

**问题**: vendor/std 中使用了大量泛型，需要预生成常用实例。

**实现方案**:

```rust
// 创建标准库实例预填充模块
// src/monomorph/stdlib_instances.rs

pub fn register_stdlib_instances(codegen: &mut CodeGen) {
    // Array 实例
    codegen.classes.insert("Array$Int64".to_string(), ...);
    codegen.classes.insert("Array$String".to_string(), ...);
    codegen.classes.insert("Array$UInt8".to_string(), ...);
    
    // Map 实例  
    codegen.classes.insert("Map$String$String".to_string(), ...);
    codegen.classes.insert("Map$String$Int64".to_string(), ...);
    
    // Option 实例
    codegen.enums.insert("Option$Int64".to_string(), ...);
    
    // Result 实例
    codegen.enums.insert("Result$Int64$String".to_string(), ...);
}
```

---

## Phase 4: 优化与测试

**预计时间**: 1-2 天

### 4.1 增量编译支持

**实现方案**:

```rust
// 添加实例化缓存
use std::sync::Mutex;

lazy_static! {
    static ref INSTANTIATION_CACHE: Mutex<HashSet<(String, Vec<Type>)>> = 
        Mutex::new(HashSet::new());
}

pub fn get_or_create_instantiation(name: &str, type_args: &[Type]) -> String {
    let key = (name.to_string(), type_args.to_vec());
    let mut cache = INSTANTIATION_CACHE.lock().unwrap();
    
    if let Some(existing) = cache.get(&key) {
        return existing.clone();
    }
    
    let mangled = mangle_name(name, type_args);
    cache.insert(key);
    mangled
}
```

### 4.2 测试用例

**需要添加的测试** (`tests/monomorph/`):

```rust
// test_generic_function_call
fn test_generic_function_call() {
    let src = r#"
        func identity<T>(x: T): T { x }
        main() {
            let x = identity<Int64>(42)
            let y = identity<String>("hello")
        }
    "#;
    // 验证生成了 identity$Int64 和 identity$String
}

// test_generic_class_method
fn test_generic_class_method() {
    let src = r#"
        class Box<T> {
            var value: T
            init(v: T) { this.value = v }
            func get(): T { this.value }
        }
        main() {
            let box = Box<Int64>(42)
            let x = box.get()
        }
    "#;
    // 验证生成了 Box$Int64.get 方法
}

// test_nested_generics
fn test_nested_generics() {
    let src = r#"
        class Wrapper<T> {
            var inner: Array<T>
        }
        main() {
            let w = Wrapper<String>(Array<String>(1, "test"))
        }
    "#;
    // 验证嵌套泛型正确实例化
}
```

---

## 详细修改清单

| 文件 | 修改内容 | 行号范围 |
|------|----------|----------|
| `src/monomorph/mod.rs` | 添加 `infer_type_from_context` | ~630 |
| `src/monomorph/mod.rs` | 修改 `visit_expr` 处理更多类型 | ~672-731 |
| `src/monomorph/mod.rs` | 增强类方法单态化 | ~1210-1250 |
| `src/codegen/expr.rs` | 增强 `get_object_type` | ~1616-1660 |
| `src/codegen/expr.rs` | 增强方法调用 key 生成 | ~5388-5420 |
| `src/codegen/mod.rs` | 注册内建泛型类型 | ~80-150 |
| `src/monomorph/stdlib_instances.rs` | 新建文件，预填充实例 | 新建 |

---

## 风险与注意事项

1. **类型推断复杂度**: 完全的隐式类型推断需要类型推导算法，建议先用"白名单"方式预填充常用实例
2. **递归实例化**: 需要防止无限递归 (A<T> 包含 B<T>, B<T> 包含 A<T>)
3. **性能**: 大量实例化可能导致编译时间增长，需要缓存机制
4. **测试覆盖**: 需要大量测试用例覆盖各种泛型使用场景