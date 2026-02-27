# 高级特性实现计划

## 1. 接口虚表问题

### 问题描述
标准库中的接口方法虚表构建失败，错误信息：
```
vtable 方法 Scope.lookup 未找到函数索引
位置: src/codegen/decl.rs:216
```

### 根本原因分析
- 接口方法在 `func_indices` 中未正确注册
- 虚表构建时无法找到接口方法的函数索引
- 可能是接口方法的名称映射问题（如 `Scope.lookup` vs `Scope::lookup`）

### 实现步骤

#### 1.1 接口方法注册机制 (src/codegen/decl.rs)
```rust
// 在 build_vtables() 之前，确保所有接口方法已注册
fn register_interface_methods(&mut self, interface: &InterfaceDecl) {
    for method in &interface.methods {
        let mangled_name = format!("{}::{}", interface.name, method.name);
        // 注册到 func_indices
        self.func_indices.insert(mangled_name, next_func_idx);
    }
}
```

#### 1.2 虚表查找增强
```rust
// 在 build_vtables() 中改进查找逻辑
fn find_method_index(&self, interface_name: &str, method_name: &str) -> Option<u32> {
    // 尝试多种命名格式
    let candidates = vec![
        format!("{}.{}", interface_name, method_name),
        format!("{}::{}", interface_name, method_name),
        format!("{}_{}", interface_name, method_name),
    ];

    for candidate in candidates {
        if let Some(&idx) = self.func_indices.get(&candidate) {
            return Some(idx);
        }
    }
    None
}
```

#### 1.3 调试信息输出
在虚表构建失败时输出详细信息：
- 当前所有已注册的函数名称
- 正在查找的接口方法名称
- 可能的命名格式

### 测试验证
- 使用 `/tmp/test_rparen.cj` 中的简单接口测试
- 逐步启用标准库模块（从 std.overflow 开始）
- 确认 `examples/std/` 能够编译通过

---

## 2. 复杂的泛型系统

### 问题描述
标准库大量使用泛型，当前实现的单态化不完整：
- `None<GenericParam>` 等显式泛型实例化
- 泛型类型参数在不同上下文中的传播
- 嵌套泛型类型（如 `Option<Array<T>>`）

### 当前实现状态
- `src/monomorph/mod.rs` 有基础单态化框架
- 仅处理函数调用时的类型实参
- 缺少类型推断和隐式实例化

### 实现步骤

#### 2.1 类型推断系统 (新建 src/typeinfer/mod.rs)
```rust
pub struct TypeInferencer {
    // 类型变量到具体类型的映射
    type_vars: HashMap<String, Type>,
    // 类型约束集合
    constraints: Vec<TypeConstraint>,
}

impl TypeInferencer {
    // 从表达式推断类型
    pub fn infer_expr(&mut self, expr: &Expr) -> Result<Type, String> {
        match expr {
            Expr::None => {
                // 推断 None 的类型参数
                self.fresh_type_var("T")
            }
            Expr::Call { func, args, .. } => {
                // 从参数推断泛型类型
                self.infer_from_args(func, args)
            }
            _ => { /* ... */ }
        }
    }
}
```

#### 2.2 增强单态化收集 (src/monomorph/mod.rs)
```rust
// 扩展 collect_instantiations 以处理更多场景
fn collect_from_expr(&mut self, expr: &Expr) {
    match expr {
        // 显式泛型实例化: None<T>, Some<T>
        Expr::None | Expr::Some(_) if has_explicit_type_arg => {
            self.instantiations.insert((func_name, type_args));
        }

        // 字段访问中的泛型
        Expr::Field { object, .. } => {
            if let Some(generic_type) = self.get_field_type(object) {
                self.collect_from_type(&generic_type);
            }
        }

        // Match 表达式中的泛型
        Expr::Match { subject, arms } => {
            self.collect_from_expr(subject);
            for arm in arms {
                self.collect_from_pattern(&arm.pattern);
            }
        }

        _ => { /* ... */ }
    }
}
```

#### 2.3 泛型类型替换增强
```rust
// 处理嵌套泛型类型
fn substitute_type(&self, ty: &Type, type_map: &HashMap<String, Type>) -> Type {
    match ty {
        Type::TypeParam(name) => {
            type_map.get(name).cloned().unwrap_or(ty.clone())
        }
        Type::Array(elem) => {
            Type::Array(Box::new(self.substitute_type(elem, type_map)))
        }
        Type::Option(inner) => {
            Type::Option(Box::new(self.substitute_type(inner, type_map)))
        }
        Type::Struct(name, type_args) => {
            let new_args = type_args.iter()
                .map(|t| self.substitute_type(t, type_map))
                .collect();
            Type::Struct(name.clone(), new_args)
        }
        _ => ty.clone()
    }
}
```

### 测试验证
- 测试 `Option<T>`, `Result<T, E>` 的各种组合
- 测试嵌套泛型 `Array<Option<Int64>>`
- 验证标准库中的泛型使用场景

---

## 3. 高级类型系统

### 问题描述
标准库使用了高级类型特性：
- 类型约束（trait bounds）
- 关联类型（associated types）
- Where 子句
- 类型别名和类型投影

### 实现步骤

#### 3.1 类型约束系统 (扩展 src/ast/mod.rs)
```rust
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    pub type_param: String,
    pub bounds: Vec<InterfaceBound>,
}

#[derive(Debug, Clone)]
pub struct InterfaceBound {
    pub interface_name: String,
    pub associated_types: HashMap<String, Type>,
}

// 在函数和类型定义中添加约束
pub struct FuncDecl {
    // ... 现有字段
    pub type_constraints: Vec<TypeConstraint>,
}
```

#### 3.2 约束检查 (新建 src/typecheck/constraints.rs)
```rust
pub struct ConstraintChecker {
    interfaces: HashMap<String, InterfaceDecl>,
}

impl ConstraintChecker {
    // 检查类型是否满足约束
    pub fn check_constraint(
        &self,
        ty: &Type,
        constraint: &TypeConstraint
    ) -> Result<(), String> {
        for bound in &constraint.bounds {
            if !self.implements_interface(ty, &bound.interface_name) {
                return Err(format!(
                    "类型 {:?} 未实现接口 {}",
                    ty, bound.interface_name
                ));
            }
        }
        Ok(())
    }

    // 检查类型是否实现了接口
    fn implements_interface(&self, ty: &Type, interface: &str) -> bool {
        // 查找类型的 impl 声明
        // 递归检查父类和组合
        todo!()
    }
}
```

#### 3.3 关联类型解析
```rust
// 在接口定义中支持关联类型
pub struct InterfaceDecl {
    pub name: String,
    pub methods: Vec<FuncDecl>,
    pub associated_types: Vec<String>,  // 新增
}

// 解析关联类型投影 (如 T::Item)
pub enum Type {
    // ... 现有变体
    Associated {
        base: Box<Type>,
        name: String,
    },
}
```

#### 3.4 Where 子句支持
```rust
// 在解析器中添加 where 子句解析
fn parse_where_clause(&mut self) -> Result<Vec<TypeConstraint>, String> {
    if !self.check(&Token::Where) {
        return Ok(vec![]);
    }
    self.advance();

    let mut constraints = vec![];
    loop {
        let type_param = self.expect_ident()?;
        self.expect(Token::Colon)?;

        let bounds = self.parse_interface_bounds()?;
        constraints.push(TypeConstraint {
            type_param,
            bounds,
        });

        if !self.check(&Token::Comma) {
            break;
        }
        self.advance();
    }
    Ok(constraints)
}
```

### 测试验证
- 创建带约束的泛型函数测试
- 测试关联类型的使用
- 验证 where 子句的解析和检查

---

## 实施优先级

### 第一阶段：接口虚表修复（1-2天）
- 最紧急，阻塞标准库编译
- 影响范围明确，修复点集中
- 完成后可立即验证 examples/std/

### 第二阶段：泛型系统增强（3-5天）
- 基础设施已存在，需要扩展
- 对标准库支持至关重要
- 可以增量实现和测试

### 第三阶段：高级类型系统（5-7天）
- 最复杂，需要新的基础设施
- 可以分步实现（先约束，后关联类型）
- 部分特性可以延后实现

## 风险和依赖

### 技术风险
- 接口虚表可能涉及更深层的设计问题
- 类型推断可能需要完整的类型检查器
- 关联类型可能需要重构现有类型系统

### 依赖关系
- 泛型系统依赖基本的类型推断
- 高级类型系统依赖完整的泛型支持
- 所有特性都依赖接口虚表的正确实现

## 成功标准

1. **接口虚表**：examples/std/ 编译通过，无 vtable 错误
2. **泛型系统**：标准库中所有泛型使用场景正确单态化
3. **高级类型**：支持带约束的泛型函数，关联类型基本可用
