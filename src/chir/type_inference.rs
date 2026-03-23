//! 类型推断器 - 遍历 AST 推断表达式类型

use crate::ast::{BinOp, Expr, Function, Pattern, Program, Stmt, Type, UnaryOp};
use std::collections::{HashMap, HashSet};

/// 从函数体推断返回类型：找第一个 Stmt::Return(Some(expr)) 并推断 expr 类型
/// 用于处理没有显式返回类型注解的函数（如 Cangjie 隐式类型推断函数）
pub fn infer_return_type_from_body(body: &[Stmt], ctx: &TypeInferenceContext) -> Option<Type> {
    for stmt in body {
        if let Some(ty) = infer_return_type_from_stmt(stmt, ctx) {
            return Some(ty);
        }
    }
    None
}

fn infer_return_type_from_stmt(stmt: &Stmt, ctx: &TypeInferenceContext) -> Option<Type> {
    match stmt {
        Stmt::Return(Some(expr)) => ctx
            .infer_expr(expr)
            .ok()
            .filter(|t| !matches!(t, Type::Unit | Type::Nothing)),
        Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::For { body, .. } => {
            for s in body {
                if let Some(ty) = infer_return_type_from_stmt(s, ctx) {
                    return Some(ty);
                }
            }
            None
        }
        _ => None,
    }
}

/// 函数签名
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub name: String,
    pub params: Vec<Type>,
    pub return_ty: Type,
}

/// 类型推断上下文
#[derive(Clone)]
pub struct TypeInferenceContext {
    /// 局部变量类型表
    pub locals: HashMap<String, Type>,
    /// 局部变量可变性（true=可变 var/inout，false=不可变 let/普通参数）
    pub local_mutability: HashMap<String, bool>,

    /// 名义类型继承关系：type_name -> direct supertypes
    /// 包含 class extends / class implements / interface parents
    pub nominal_supertypes: HashMap<String, Vec<String>>,

    /// 函数签名表（单态化后）
    pub functions: HashMap<String, FunctionSignature>,

    /// 结构体字段类型
    pub struct_fields: HashMap<String, HashMap<String, Type>>,

    /// 类字段类型
    pub class_fields: HashMap<String, HashMap<String, Type>>,

    /// 类静态字段名：class_name -> {field_name}
    /// 用于表达式校验（static 成员只能用类型名访问）
    pub class_static_fields: HashMap<String, HashSet<String>>,

    /// 类方法返回类型：class_name → method_name → return_type
    pub class_method_returns: HashMap<String, HashMap<String, Type>>,

    /// 裸枚举变体名到枚举类型的映射（如 `TR` -> `CasingOption`）
    pub enum_variant_types: HashMap<String, Type>,

    /// 当前函数返回类型
    pub current_return_ty: Option<Type>,

    /// 全局变量类型
    pub globals: HashMap<String, Type>,
}

impl TypeInferenceContext {
    /// 创建新的类型推断上下文
    pub fn new() -> Self {
        TypeInferenceContext {
            locals: HashMap::new(),
            local_mutability: HashMap::new(),
            nominal_supertypes: HashMap::new(),
            functions: HashMap::new(),
            struct_fields: HashMap::new(),
            class_fields: HashMap::new(),
            class_static_fields: HashMap::new(),
            class_method_returns: HashMap::new(),
            enum_variant_types: HashMap::new(),
            current_return_ty: None,
            globals: HashMap::new(),
        }
    }

    /// 从程序构建上下文
    pub fn from_program(program: &Program) -> Self {
        let mut ctx = Self::new();

        // 收集函数签名（支持重载：用 name$arity 修饰名额外注册一份）
        for func in &program.functions {
            let sig = FunctionSignature {
                name: func.name.clone(),
                params: func.params.iter().map(|p| p.ty.clone()).collect(),
                return_ty: func.return_type.clone().unwrap_or(Type::Unit),
            };
            // 修饰名（精确匹配重载版本）
            let mangled = format!("{}${}", func.name, func.params.len());
            ctx.functions.insert(mangled, sig.clone());
            // 原名（保留，供非重载场景使用）
            ctx.functions.entry(func.name.clone()).or_insert(sig);

            // 如果函数名包含 "."，说明是 struct/class 方法（如 "Point.distance"）
            // 注册到 class_method_returns 供 infer_method_return 使用
            if let Some(dot_pos) = func.name.find('.') {
                let type_name = &func.name[..dot_pos];
                let method_name = &func.name[dot_pos + 1..];
                let ret_ty = func.return_type.clone().unwrap_or(Type::Unit);
                ctx.class_method_returns
                    .entry(type_name.to_string())
                    .or_default()
                    .entry(method_name.to_string())
                    .or_insert(ret_ty);
            }
        }

        // 收集结构体字段
        for struct_def in &program.structs {
            let mut fields = HashMap::new();
            for field in &struct_def.fields {
                fields.insert(field.name.clone(), field.ty.clone());
            }
            ctx.struct_fields.insert(struct_def.name.clone(), fields);
        }

        // Range 虚拟结构体字段
        {
            let mut range_fields = HashMap::new();
            range_fields.insert("start".to_string(), Type::Int64);
            range_fields.insert("end".to_string(), Type::Int64);
            ctx.struct_fields.insert("Range".to_string(), range_fields);
        }

        // Validate extend declarations for duplicate interface implementations
        {
            use std::collections::HashSet;

            // Known standard library interface implementations
            // Based on Cangjie standard library
            let mut known_implementations: HashMap<String, HashSet<String>> = HashMap::new();

            // Numeric types implement ToString, Equatable, Hashable
            for ty in &["Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
                        "Float32", "Float64", "IntNative", "UIntNative"] {
                let mut interfaces = HashSet::new();
                interfaces.insert("ToString".to_string());
                interfaces.insert("Equatable".to_string());
                interfaces.insert("Hashable".to_string());
                known_implementations.insert(ty.to_string(), interfaces);
            }

            // String implements ToString, Equatable, Hashable
            {
                let mut interfaces = HashSet::new();
                interfaces.insert("ToString".to_string());
                interfaces.insert("Equatable".to_string());
                interfaces.insert("Hashable".to_string());
                known_implementations.insert("String".to_string(), interfaces);
            }

            // Bool implements ToString, Equatable, Hashable
            {
                let mut interfaces = HashSet::new();
                interfaces.insert("ToString".to_string());
                interfaces.insert("Equatable".to_string());
                interfaces.insert("Hashable".to_string());
                known_implementations.insert("Bool".to_string(), interfaces);
            }

            // Rune implements ToString, Equatable, Hashable
            {
                let mut interfaces = HashSet::new();
                interfaces.insert("ToString".to_string());
                interfaces.insert("Equatable".to_string());
                interfaces.insert("Hashable".to_string());
                known_implementations.insert("Rune".to_string(), interfaces);
            }

            let mut type_interfaces: HashMap<String, HashSet<String>> = HashMap::new();

            for ext in &program.extends {
                if let Some(ref interface) = ext.interface {
                    let type_name = &ext.target_type;

                    // Check if this type already implements this interface in standard library
                    if let Some(known_ifaces) = known_implementations.get(type_name) {
                        if known_ifaces.contains(interface) {
                            eprintln!(
                                "错误: 类型 '{}' 已经实现了接口 '{}' (标准库)",
                                type_name, interface
                            );
                            // For now, we just warn. In a full implementation, this should be an error.
                        }
                    }

                    // Check for duplicate extends in the same program
                    let interfaces = type_interfaces.entry(type_name.clone()).or_insert_with(HashSet::new);

                    if interfaces.contains(interface) {
                        eprintln!(
                            "错误: 类型 '{}' 在程序中重复实现接口 '{}'",
                            type_name, interface
                        );
                    }
                    interfaces.insert(interface.clone());
                }
            }
        }

        // 注册 extend 方法签名
        for ext in &program.extends {
            for method in &ext.methods {
                let short_name = method
                    .name
                    .rsplit_once('.')
                    .map(|(_, short)| short.to_string())
                    .unwrap_or_else(|| method.name.clone());
                let sig = FunctionSignature {
                    name: short_name.clone(),
                    params: method.params.iter().map(|p| p.ty.clone()).collect(),
                    return_ty: method.return_type.clone().unwrap_or(Type::Unit),
                };
                let mangled = format!("{}${}", short_name, method.params.len());
                ctx.functions.insert(mangled, sig.clone());
                ctx.functions.entry(short_name.clone()).or_insert(sig);

                let ret_ty = method.return_type.clone().unwrap_or(Type::Unit);
                ctx.class_method_returns
                    .entry(ext.target_type.clone())
                    .or_default()
                    .entry(short_name)
                    .or_insert(ret_ty);
            }
        }

        // 收集类字段 + 类方法签名
        for class_def in &program.classes {
            let mut fields = HashMap::new();
            for field in &class_def.fields {
                fields.insert(field.name.clone(), field.ty.clone());
            }
            ctx.class_fields.insert(class_def.name.clone(), fields);
            // 由于 AST 里 FieldDef 尚未携带 is_static 元信息，这里只做“保守”静态字段识别：
            // 仅当类没有 init / 主构造参数 / 方法 / 继承关系时，才将类内字段视为静态字段。
            let can_infer_static_fields = class_def.init.is_none()
                && class_def.primary_ctor_params.is_empty()
                && class_def.methods.is_empty()
                && class_def.extends.is_none();
            let static_fields: HashSet<String> = if can_infer_static_fields {
                class_def.fields.iter().map(|f| f.name.clone()).collect()
            } else {
                HashSet::new()
            };
            ctx.class_static_fields
                .insert(class_def.name.clone(), static_fields);

            let mut method_returns = HashMap::new();
            for method in &class_def.methods {
                let full_name = &method.func.name; // "ClassName.methodName"
                let short_name = full_name
                    .strip_prefix(&format!("{}.", class_def.name))
                    .unwrap_or(full_name);
                let ret_ty = method.func.return_type.clone().unwrap_or(Type::Unit);
                method_returns.insert(short_name.to_string(), ret_ty);

                // 注册完整签名到 functions（含 this 参数）
                let sig = FunctionSignature {
                    name: full_name.clone(),
                    params: method.func.params.iter().map(|p| p.ty.clone()).collect(),
                    return_ty: method.func.return_type.clone().unwrap_or(Type::Unit),
                };
                let mangled = format!("{}${}", full_name, method.func.params.len());
                ctx.functions.insert(mangled, sig.clone());
                ctx.functions.entry(full_name.clone()).or_insert(sig);
            }
            ctx.class_method_returns
                .insert(class_def.name.clone(), method_returns);
        }

        for enum_def in &program.enums {
            let enum_ty = Type::Struct(enum_def.name.clone(), vec![]);
            for variant in &enum_def.variants {
                ctx.enum_variant_types
                    .entry(variant.name.clone())
                    .or_insert_with(|| enum_ty.clone());
            }
        }

        // 收集名义子类型关系（class/interface）
        for iface in &program.interfaces {
            for parent in &iface.parents {
                ctx.add_nominal_supertype(iface.name.clone(), parent.clone());
            }
        }
        for class_def in &program.classes {
            if let Some(parent) = &class_def.extends {
                ctx.add_nominal_supertype(class_def.name.clone(), parent.clone());
            }
            for iface in &class_def.implements {
                ctx.add_nominal_supertype(class_def.name.clone(), iface.clone());
            }
        }

        // 注册顶层常量/变量类型。显式类型保持声明类型；隐式类型按初始化表达式顺序推断。
        for constant in &program.constants {
            let ty = if constant.explicit_ty {
                constant.ty.clone()
            } else {
                ctx.infer_expr(&constant.init).unwrap_or_else(|_| constant.ty.clone())
            };
            ctx.globals.insert(constant.name.clone(), ty);
        }

        // 继承合并：将父类字段 + 方法签名传播到子类（多轮直到稳定）
        let class_extends: HashMap<String, Option<String>> = program
            .classes
            .iter()
            .map(|c| (c.name.clone(), c.extends.clone()))
            .collect();
        for _ in 0..10 {
            let mut changed = false;
            for class_def in &program.classes {
                let mut parent = class_def.extends.clone();
                while let Some(ref parent_name) = parent {
                    if let Some(parent_fields) = ctx.class_fields.get(parent_name).cloned() {
                        let child_fields = ctx.class_fields.get_mut(&class_def.name).unwrap();
                        for (name, ty) in parent_fields {
                            if !child_fields.contains_key(&name) {
                                child_fields.insert(name, ty);
                                changed = true;
                            }
                        }
                    }
                    if let Some(parent_methods) = ctx.class_method_returns.get(parent_name).cloned()
                    {
                        let child_methods =
                            ctx.class_method_returns.get_mut(&class_def.name).unwrap();
                        for (name, ret_ty) in parent_methods {
                            child_methods.entry(name).or_insert(ret_ty);
                        }
                    }
                    parent = class_extends.get(parent_name).and_then(|p| p.clone());
                }
            }
            if !changed {
                break;
            }
        }

        ctx
    }

    /// 添加局部变量
    pub fn add_local(&mut self, name: String, ty: Type) {
        self.add_local_with_mutability(name, ty, true);
    }

    /// 添加局部变量并显式设置可变性
    pub fn add_local_with_mutability(&mut self, name: String, ty: Type, mutable: bool) {
        self.locals.insert(name.clone(), ty);
        self.local_mutability.insert(name, mutable);
    }

    /// 赋值兼容性：`value` 是否可赋给 `target`
    pub fn is_assignable_type(&self, target: &Type, value: &Type) -> bool {
        self.is_subtype(value, target)
    }

    fn is_subtype(&self, sub: &Type, sup: &Type) -> bool {
        if sub == sup {
            return true;
        }
        if Self::is_integral(sub) && Self::is_integral(sup) {
            // 数值上下文下允许整型宽度互通（如 Int64 字面量赋给 Int32）
            return true;
        }
        if matches!(sup, Type::TypeParam(_) | Type::This | Type::Qualified(_)) {
            // lowering 阶段对未单态化类型保持保守放行
            return true;
        }
        if matches!(sub, Type::Nothing) {
            // Nothing 是底类型
            return true;
        }
        if let Type::Struct(sup_name, _) = sup {
            if sup_name == "Object" && Self::is_reference_like(sub) {
                return true;
            }
        }

        match (sub, sup) {
            (Type::Array(sub_t), Type::Array(sup_t)) => self.is_subtype(sub_t, sup_t),
            (Type::Slice(sub_t), Type::Slice(sup_t)) => self.is_subtype(sub_t, sup_t),
            (Type::Map(sub_k, sub_v), Type::Map(sup_k, sup_v)) => {
                self.is_subtype(sub_k, sup_k) && self.is_subtype(sub_v, sup_v)
            }
            (value, Type::Option(inner))
                if !matches!(value, Type::Option(_))
                    && !matches!(value, Type::Struct(name, _) if name == "Option") =>
            {
                self.is_subtype(value, inner)
            }
            (value, Type::Struct(name, args))
                if name == "Option" && args.len() == 1 && !matches!(value, Type::Option(_)) =>
            {
                self.is_subtype(value, &args[0])
            }
            (Type::Option(sub_t), Type::Struct(name, args))
                if name == "Option" && args.len() == 1 =>
            {
                self.is_subtype(sub_t, &args[0])
            }
            (Type::Struct(name, args), Type::Option(sup_t))
                if name == "Option" && args.len() == 1 =>
            {
                self.is_subtype(&args[0], sup_t)
            }
            (Type::Result(sub_ok, sub_err), Type::Struct(name, args))
                if name == "Result" && args.len() == 2 =>
            {
                self.is_subtype(sub_ok, &args[0]) && self.is_subtype(sub_err, &args[1])
            }
            (Type::Struct(name, args), Type::Result(sup_ok, sup_err))
                if name == "Result" && args.len() == 2 =>
            {
                self.is_subtype(&args[0], sup_ok) && self.is_subtype(&args[1], sup_err)
            }
            (Type::Struct(sub_name, _), Type::Struct(sup_name, _)) => {
                self.nominal_is_subtype(sub_name, sup_name)
            }
            (Type::Tuple(sub_items), Type::Tuple(sup_items)) => {
                sub_items.len() == sup_items.len()
                    && sub_items
                        .iter()
                        .zip(sup_items.iter())
                        .all(|(s, t)| self.is_subtype(s, t))
            }
            (
                Type::Function {
                    params: sub_params,
                    ret: sub_ret,
                },
                Type::Function {
                    params: sup_params,
                    ret: sup_ret,
                },
            ) => {
                sub_params.len() == sup_params.len()
                    // 函数参数逆变：sup_param <: sub_param
                    && sub_params
                        .iter()
                        .zip(sup_params.iter())
                        .all(|(s, t)| self.is_subtype(t, s))
                    // 返回值协变：sub_ret <: sup_ret
                    && self.is_function_ret_subtype(sub_ret.as_ref(), sup_ret.as_ref())
            }
            (Type::Option(sub_t), Type::Option(sup_t)) => self.is_subtype(sub_t, sup_t),
            (Type::Result(sub_ok, sub_err), Type::Result(sup_ok, sup_err)) => {
                self.is_subtype(sub_ok, sup_ok) && self.is_subtype(sub_err, sup_err)
            }
            _ => false,
        }
    }

    fn is_function_ret_subtype(&self, sub: &Option<Type>, sup: &Option<Type>) -> bool {
        match (sub, sup) {
            (None, None) => true,
            (Some(s), Some(t)) => self.is_subtype(s, t),
            _ => false,
        }
    }

    fn is_reference_like(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Struct(..)
                | Type::String
                | Type::Array(_)
                | Type::Tuple(_)
                | Type::Function { .. }
                | Type::Option(_)
                | Type::Result(_, _)
                | Type::Slice(_)
                | Type::Map(_, _)
                | Type::This
                | Type::Qualified(_)
        )
    }

    fn add_nominal_supertype(&mut self, ty: String, parent: String) {
        let entry = self.nominal_supertypes.entry(ty).or_default();
        if !entry.iter().any(|p| p == &parent) {
            entry.push(parent);
        }
    }

    fn nominal_is_subtype(&self, sub_name: &str, sup_name: &str) -> bool {
        if sub_name == sup_name {
            return true;
        }
        let mut stack = vec![sub_name.to_string()];
        let mut visited = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            if let Some(parents) = self.nominal_supertypes.get(&cur) {
                for parent in parents {
                    if parent == sup_name {
                        return true;
                    }
                    stack.push(parent.clone());
                }
            }
        }
        false
    }

    /// 获取局部变量类型
    pub fn get_local(&self, name: &str) -> Option<&Type> {
        self.locals.get(name)
    }

    /// 推断表达式类型
    pub fn infer_expr(&self, expr: &Expr) -> Result<Type, String> {
        match expr {
            // 字面量
            Expr::Integer(_) => Ok(Type::Int64),
            Expr::Float(_) => Ok(Type::Float64),
            Expr::Float32(_) => Ok(Type::Float32),
            Expr::Bool(_) => Ok(Type::Bool),
            Expr::Rune(_) => Ok(Type::Rune),
            Expr::String(_) => Ok(Type::String),
            Expr::Interpolate(_) => Ok(Type::String),
            Expr::VariantConst { enum_name, .. } => {
                Ok(Type::Struct(enum_name.clone(), vec![]))
            }
            Expr::Some(inner) => Ok(Type::Option(Box::new(self.infer_expr(inner)?))),
            Expr::None => Ok(Type::Option(Box::new(Type::Nothing))),
            Expr::Ok(inner) => Ok(Type::Result(
                Box::new(self.infer_expr(inner)?),
                Box::new(Type::Nothing),
            )),
            Expr::Err(inner) => Ok(Type::Result(
                Box::new(Type::Nothing),
                Box::new(self.infer_expr(inner)?),
            )),
            Expr::NullCoalesce { option, default } => {
                let opt_ty = self.infer_expr(option)?;
                let default_ty = self.infer_expr(default)?;
                match opt_ty {
                    Type::Option(inner) => {
                        if self.is_assignable_type(inner.as_ref(), &default_ty) {
                            Ok(*inner)
                        } else {
                            Ok(default_ty)
                        }
                    }
                    Type::Struct(name, args) if name == "Option" => {
                        let inner = args.first().cloned().unwrap_or(Type::Int64);
                        if self.is_assignable_type(&inner, &default_ty) {
                            Ok(inner)
                        } else {
                            Ok(default_ty)
                        }
                    }
                    _ => Ok(default_ty),
                }
            }

            // 变量
            Expr::Var(name) => {
                // 先查局部变量
                if let Some(ty) = self.locals.get(name) {
                    return Ok(ty.clone());
                }
                // 再查全局变量
                if let Some(ty) = self.globals.get(name) {
                    return Ok(ty.clone());
                }
                // 数学常量
                if matches!(
                    name.as_str(),
                    "PI" | "E" | "TAU" | "INF" | "INFINITY" | "NEG_INF" | "NEG_INFINITY" | "NAN"
                ) {
                    return Ok(Type::Float64);
                }
                if let Some(enum_ty) = self.enum_variant_types.get(name) {
                    return Ok(enum_ty.clone());
                }
                // 类/结构体名称 → 类型引用
                if self.struct_fields.contains_key(name.as_str())
                    || self.class_fields.contains_key(name.as_str())
                {
                    return Ok(Type::Struct(name.clone(), vec![]));
                }
                // 未知变量保守推断为对象引用 (I32)，避免 lowering 失败
                Ok(Type::Int32)
            }

            // 二元运算
            Expr::Binary { op, left, right } => {
                let left_ty = self.infer_expr(left)?;
                let right_ty = self.infer_expr(right)?;
                self.infer_binary_result(op, &left_ty, &right_ty)
            }

            // 一元运算
            Expr::Unary { op, expr } => {
                let expr_ty = self.infer_expr(expr)?;
                self.infer_unary_result(op, &expr_ty)
            }

            // 函数调用
            Expr::Call { name, args, .. } => {
                // 优先按 name$arity 修饰名查找（支持重载函数精确匹配）
                let mangled = format!("{}${}", name, args.len());
                if let Some(sig) = self
                    .functions
                    .get(mangled.as_str())
                    .or_else(|| self.functions.get(name.as_str()))
                {
                    return Ok(sig.return_ty.clone());
                }
                // 内置函数
                match name.as_str() {
                    "println" | "print" | "eprintln" | "eprint" => Ok(Type::Unit),
                    "readln" => Ok(Type::String),
                    "exit" => Ok(Type::Nothing),
                    "abs" | "min" | "max" if !args.is_empty() => self.infer_expr(&args[0]),
                    "sqrt" | "floor" | "ceil" | "trunc" | "nearest" | "sin" | "cos" | "exp"
                    | "log" | "pow" => Ok(Type::Float64),
                    // 整数类型转换构造函数 → 对应整数类型
                    "Int8" | "Int16" | "Int32" | "UInt8" | "UInt16" | "UInt32" => Ok(Type::Int32),
                    "Int64" | "UInt64" | "IntNative" | "UIntNative" => Ok(Type::Int64),
                    // 浮点类型转换构造函数
                    "Float16" | "Float32" => Ok(Type::Float32),
                    "Float64" => Ok(Type::Float64),
                    // 字符串相关
                    "toString" | "format" => Ok(Type::String),
                    // WASI 运行时函数
                    "now" | "randomInt64" => Ok(Type::Int64),
                    "randomFloat64" => Ok(Type::Float64),
                    _ => {
                        // 检查是否为结构体/类构造函数 → 对象引用 (I32)
                        if self.struct_fields.contains_key(name.as_str())
                            || self.class_fields.contains_key(name.as_str())
                        {
                            Ok(Type::Struct(name.clone(), vec![]))
                        } else {
                            // 未知函数保守推断为 I32（对象引用比裸 I64 更常见）
                            Ok(Type::Int32)
                        }
                    }
                }
            }

            // 方法调用
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let obj_ty = self.infer_expr(object)?;
                self.infer_method_return(&obj_ty, method, args)
            }

            // 字段访问
            Expr::Field { object, field } => {
                let obj_ty = self.infer_expr(object)?;
                if matches!(obj_ty, Type::Array(_)) && matches!(field.as_str(), "size" | "length") {
                    return Ok(Type::Int64);
                }
                if matches!(obj_ty, Type::Range) && matches!(field.as_str(), "start" | "end" | "step")
                {
                    return Ok(Type::Int64);
                }
                if let Type::Struct(class_name, _) = &obj_ty {
                    if self.is_static_class_field(class_name, field) && !self.is_type_name_expr(object)
                    {
                        return Err(format!(
                            "semantic error: static member '{}' cannot be accessed via instance",
                            field
                        ));
                    }
                }
                if let Some(field_ty) = self.lookup_field_type(&obj_ty, field) {
                    return Ok(field_ty);
                }
                if let Some(method_ret) = self.lookup_method_return(&obj_ty, field) {
                    return Ok(Type::Function {
                        params: vec![],
                        ret: Box::new(Some(method_ret)),
                    });
                }
                Ok(Type::Int32)
            }

            // 前后缀自增/自减：仅允许可赋值表达式
            Expr::PostfixIncr(expr)
            | Expr::PostfixDecr(expr)
            | Expr::PrefixIncr(expr)
            | Expr::PrefixDecr(expr) => {
                self.ensure_update_target_assignable(expr)?;
                self.infer_expr(expr)
            }

            // 数组
            Expr::Array(elems) => {
                if elems.is_empty() {
                    Ok(Type::Array(Box::new(Type::Int64)))
                } else {
                    let elem_ty = self.infer_expr(&elems[0])?;
                    Ok(Type::Array(Box::new(elem_ty)))
                }
            }

            // 数组索引
            Expr::Index { array, index } => {
                let arr_ty = self.infer_expr(array)?;
                match arr_ty {
                    Type::Array(elem_ty) => {
                        if matches!(index.as_ref(), Expr::Range { .. }) {
                            Ok(Type::Array(elem_ty))
                        } else {
                            Ok(*elem_ty)
                        }
                    }
                    Type::Tuple(types) => {
                        // 元组索引，返回第一个元素类型（简化）
                        Ok(types.first().cloned().unwrap_or(Type::Int64))
                    }
                    _ => Ok(Type::Int64),
                }
            }

            Expr::Range { .. } => Ok(Type::Range),

            // 元组索引 pair[0]
            Expr::TupleIndex { object, index } => {
                let obj_ty = self.infer_expr(object)?;
                match obj_ty {
                    Type::Tuple(types) => {
                        Ok(types.get(*index as usize).cloned().unwrap_or(Type::Int64))
                    }
                    _ => Ok(Type::Int64),
                }
            }

            // 元组
            Expr::Tuple(elems) => {
                let types: Result<Vec<_>, _> = elems.iter().map(|e| self.infer_expr(e)).collect();
                Ok(Type::Tuple(types?))
            }

            // 结构体初始化
            Expr::StructInit { name, .. } => Ok(Type::Struct(name.clone(), vec![])),

            // 构造函数调用
            Expr::ConstructorCall {
                name, type_args, ..
            } => {
                match name.as_str() {
                    // 类型转换构造函数
                    "Float32" => return Ok(Type::Float32),
                    "Float64" => return Ok(Type::Float64),
                    "Int32" | "UInt32" => return Ok(Type::Int32),
                    "Int64" | "UInt64" => return Ok(Type::Int64),
                    "Array" => {
                        let elem_ty = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        Ok(Type::Array(Box::new(elem_ty)))
                    }
                    "ArrayList" | "LinkedList" => {
                        let elem_ty = type_args
                            .as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        Ok(Type::Array(Box::new(elem_ty)))
                    }
                    _ => Ok(Type::Struct(
                        name.clone(),
                        type_args.clone().unwrap_or_default(),
                    )),
                }
            }

            // If 表达式
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                let then_ty = self.infer_expr(then_branch)?;
                if let Some(else_expr) = else_branch {
                    let _else_ty = self.infer_expr(else_expr)?;
                    // 简化：返回 then 分支类型
                    Ok(then_ty)
                } else {
                    Ok(Type::Unit)
                }
            }

            Expr::IfLet {
                pattern,
                expr,
                then_branch,
                else_branch,
            } => {
                let subject_ty = self.infer_expr(expr)?;
                let mut then_ctx = self.clone();
                then_ctx.bind_pattern_types(pattern, &subject_ty, false);
                let then_ty = then_ctx.infer_expr(then_branch)?;
                if let Some(else_expr) = else_branch {
                    let _else_ty = self.infer_expr(else_expr)?;
                    Ok(then_ty)
                } else {
                    Ok(Type::Unit)
                }
            }

            // Match 表达式
            Expr::Match { expr, arms } => {
                if arms.is_empty() {
                    Ok(Type::Unit)
                } else {
                    let subject_ty = self.infer_expr(expr)?;
                    let mut arm_ctx = self.clone();
                    arm_ctx.bind_pattern_types(&arms[0].pattern, &subject_ty, false);
                    arm_ctx.infer_expr(&arms[0].body)
                }
            }

            // 其他表达式：保守推断为对象引用 (I32)
            _ => Ok(Type::Int32),
        }
    }

    fn is_type_name_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Var(name) => {
                !self.locals.contains_key(name)
                    && !self.globals.contains_key(name)
                    && (self.struct_fields.contains_key(name) || self.class_fields.contains_key(name))
            }
            _ => false,
        }
    }

    fn is_static_class_field(&self, class_name: &str, field: &str) -> bool {
        self.class_static_fields
            .get(class_name)
            .map(|fields| fields.contains(field))
            .unwrap_or(false)
    }

    fn ensure_update_target_assignable(&self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Var(name) => {
                if !self.local_mutability.get(name).copied().unwrap_or(true) {
                    return Err("semantic error: cannot assign to immutable value".to_string());
                }
                Ok(())
            }
            Expr::Field { object, field } => {
                let obj_ty = self.infer_expr(object)?;
                if let Type::Struct(class_name, _) = &obj_ty {
                    if self.is_static_class_field(class_name, field) && !self.is_type_name_expr(object)
                    {
                        return Err(format!(
                            "semantic error: static member '{}' cannot be accessed via instance",
                            field
                        ));
                    }
                }
                Ok(())
            }
            Expr::Index { array, .. } => {
                if matches!(self.infer_expr(array)?, Type::Tuple(_)) {
                    Err("semantic error: expression is not assignable".to_string())
                } else {
                    Ok(())
                }
            }
            Expr::TupleIndex { .. } => Err("semantic error: expression is not assignable".to_string()),
            _ => Err("semantic error: expression is not assignable".to_string()),
        }
    }

    /// 推断二元运算结果类型
    fn infer_binary_result(&self, op: &BinOp, left: &Type, right: &Type) -> Result<Type, String> {
        let invalid = || {
            Err(format!(
                "semantic error: invalid binary operator '{}' on type '{}' and '{}'",
                Self::binop_symbol(op),
                Self::type_name(left),
                Self::type_name(right)
            ))
        };
        match op {
            // 比较运算符返回 Bool
            BinOp::Eq | BinOp::NotEq => {
                if left == right
                    || (Self::is_integral(left) && Self::is_integral(right))
                    || (matches!(left, Type::Bool) && Self::is_integral(right))
                    || (matches!(right, Type::Bool) && Self::is_integral(left))
                    || self.is_assignable_type(left, right)
                    || self.is_assignable_type(right, left)
                {
                    Ok(Type::Bool)
                } else {
                    invalid()
                }
            }
            BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                if (left == right
                    && (Self::is_numeric(left) || matches!(left, Type::Rune | Type::String)))
                    || (Self::is_integral(left) && Self::is_integral(right))
                {
                    Ok(Type::Bool)
                } else {
                    invalid()
                }
            }
            // 逻辑运算符返回 Bool
            BinOp::LogicalAnd | BinOp::LogicalOr => {
                if matches!((left, right), (Type::Bool, Type::Bool)) {
                    Ok(Type::Bool)
                } else {
                    invalid()
                }
            }
            // 算术运算符返回操作数类型
            BinOp::Mod => {
                if Self::is_integral(left) && Self::is_integral(right) {
                    Ok(Self::promote_integral_binary_type(left, right))
                } else {
                    invalid()
                }
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
                if matches!((left, right), (Type::String, Type::String)) && matches!(op, BinOp::Add)
                {
                    return Ok(Type::String);
                }
                if left == right && Self::is_numeric(left) {
                    Ok(left.clone())
                } else if Self::is_integral(left) && Self::is_integral(right) {
                    Ok(Self::promote_integral_binary_type(left, right))
                } else {
                    invalid()
                }
            }
            // 位运算返回整数类型
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                if Self::is_integral(left) && Self::is_integral(right) {
                    Ok(Self::promote_integral_binary_type(left, right))
                } else {
                    invalid()
                }
            }
            // 移位结果保持左操作数类型；右操作数只需是整数
            BinOp::Shl | BinOp::Shr => {
                if Self::is_integral(left) && Self::is_integral(right) {
                    Ok(left.clone())
                } else {
                    invalid()
                }
            }
            // 集合不包含：当前先保守返回 Bool（后续在 P1-2 细化）
            BinOp::NotIn => Ok(Type::Bool),
            // 管道表达式由 lowering 特化处理，这里返回右值类型
            BinOp::Pipeline => Ok(right.clone()),
        }
    }

    /// 推断一元运算结果类型
    fn infer_unary_result(&self, op: &UnaryOp, expr_ty: &Type) -> Result<Type, String> {
        match op {
            UnaryOp::Not => {
                if matches!(expr_ty, Type::Bool) {
                    Ok(Type::Bool)
                } else if Self::is_integral(expr_ty) {
                    Ok(expr_ty.clone())
                } else {
                    Ok(Type::Bool)
                }
            }
            UnaryOp::Neg | UnaryOp::BitNot => Ok(expr_ty.clone()),
        }
    }

    fn is_numeric(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Int8
                | Type::Int16
                | Type::Int32
                | Type::Int64
                | Type::IntNative
                | Type::UInt8
                | Type::UInt16
                | Type::UInt32
                | Type::UInt64
                | Type::UIntNative
                | Type::Float16
                | Type::Float32
                | Type::Float64
                | Type::Rune
        )
    }

    fn is_integral(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Int8
                | Type::Int16
                | Type::Int32
                | Type::Int64
                | Type::IntNative
                | Type::UInt8
                | Type::UInt16
                | Type::UInt32
                | Type::UInt64
                | Type::UIntNative
                | Type::Rune
        )
    }

    fn binop_symbol(op: &BinOp) -> &'static str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
            BinOp::LogicalAnd => "&&",
            BinOp::LogicalOr => "||",
            BinOp::Pow => "**",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::NotIn => "!in",
            BinOp::Pipeline => "|>",
        }
    }

    fn promote_integral_binary_type(left: &Type, right: &Type) -> Type {
        if left == right {
            return left.clone();
        }
        if matches!(
            left,
            Type::Int64 | Type::UInt64 | Type::IntNative | Type::UIntNative
        ) || matches!(
            right,
            Type::Int64 | Type::UInt64 | Type::IntNative | Type::UIntNative
        ) {
            Type::Int64
        } else {
            Type::Int32
        }
    }

    fn type_name(ty: &Type) -> String {
        match ty {
            Type::Int8 => "Int8".to_string(),
            Type::Int16 => "Int16".to_string(),
            Type::Int32 => "Int32".to_string(),
            Type::Int64 => "Int64".to_string(),
            Type::IntNative => "IntNative".to_string(),
            Type::UInt8 => "UInt8".to_string(),
            Type::UInt16 => "UInt16".to_string(),
            Type::UInt32 => "UInt32".to_string(),
            Type::UInt64 => "UInt64".to_string(),
            Type::UIntNative => "UIntNative".to_string(),
            Type::Float16 => "Float16".to_string(),
            Type::Float32 => "Float32".to_string(),
            Type::Float64 => "Float64".to_string(),
            Type::Rune => "Rune".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Nothing => "Nothing".to_string(),
            Type::Unit => "Unit".to_string(),
            Type::String => "String".to_string(),
            Type::Array(elem) => format!("Array<{}>", Self::type_name(elem)),
            Type::Struct(name, args) => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let params = args
                        .iter()
                        .map(Self::type_name)
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{}<{}>", name, params)
                }
            }
            Type::Tuple(types) => {
                let items = types
                    .iter()
                    .map(Self::type_name)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("({})", items)
            }
            Type::Range => "Range".to_string(),
            Type::Function { .. } => "Function".to_string(),
            Type::Option(t) => format!("Option<{}>", Self::type_name(t)),
            Type::Result(ok, err) => {
                format!("Result<{},{}>", Self::type_name(ok), Self::type_name(err))
            }
            Type::Slice(t) => format!("Slice<{}>", Self::type_name(t)),
            Type::Map(k, v) => format!("Map<{},{}>", Self::type_name(k), Self::type_name(v)),
            Type::TypeParam(name) => name.clone(),
            Type::This => "This".to_string(),
            Type::Qualified(parts) => parts.join("."),
        }
    }

    /// 推断方法返回类型
    fn infer_method_return(
        &self,
        obj_ty: &Type,
        method: &str,
        _args: &[Expr],
    ) -> Result<Type, String> {
        // 优先按对象类型分派
        let obj_type_name = match obj_ty {
            Type::Struct(n, _) => Some(n.as_str()),
            Type::Array(_) => Some("Array"),
            Type::String => Some("String"),
            Type::Rune => Some("Rune"),
            Type::Bool => Some("Bool"),
            Type::Int8 => Some("Int8"),
            Type::Int16 => Some("Int16"),
            Type::Int32 => Some("Int32"),
            Type::Int64 => Some("Int64"),
            Type::IntNative => Some("IntNative"),
            Type::UInt8 => Some("UInt8"),
            Type::UInt16 => Some("UInt16"),
            Type::UInt32 => Some("UInt32"),
            Type::UInt64 => Some("UInt64"),
            Type::UIntNative => Some("UIntNative"),
            Type::Float16 => Some("Float16"),
            Type::Float32 => Some("Float32"),
            Type::Float64 => Some("Float64"),
            Type::Option(_) => Some("Option"),
            Type::Result(_, _) => Some("Result"),
            Type::Slice(_) => Some("Slice"),
            Type::Map(_, _) => Some("Map"),
            _ => None,
        };

        // 优先查找用户定义的类方法真实返回类型
        if let Some(obj_type_name) = obj_type_name {
            if let Some(methods) = self.class_method_returns.get(obj_type_name) {
                if let Some(ret_ty) = methods.get(method) {
                    return Ok(ret_ty.clone());
                }
            }
            match (obj_type_name, method) {
                // ArrayList
                ("ArrayList", "append" | "set" | "clear") => return Ok(Type::Unit),
                ("ArrayList", "get" | "remove" | "size") => return Ok(Type::Int64),
                ("ArrayList", "isEmpty") => return Ok(Type::Bool),
                // HashMap
                ("HashMap", "put" | "clear") => return Ok(Type::Unit),
                ("HashMap", "get" | "remove" | "size") => return Ok(Type::Int64),
                ("HashMap", "containsKey") => return Ok(Type::Int64),
                // HashSet
                ("HashSet", "add" | "clear") => return Ok(Type::Unit),
                ("HashSet", "size") => return Ok(Type::Int64),
                ("HashSet", "contains") => return Ok(Type::Int64),
                // Array
                ("Array", "push" | "append" | "set" | "clear") => return Ok(Type::Unit),
                ("Array", "get" | "size" | "length") => return Ok(Type::Int64),
                ("Array", "isEmpty") => return Ok(Type::Bool),
                _ => {}
            }
            if let Some(ret_ty) = self.lookup_method_return(obj_ty, method) {
                return Ok(ret_ty);
            }
        }
        // 通用方法名推断（fallback）
        // 原则：宁可返回 Int64（emit 一个零值），也不能错误地返回 Unit（导致 empty-stack）
        // 只有确定对任何对象类型都是 void 的方法才返回 Unit
        match method {
            "toString" => Ok(Type::String),
            // 默认返回 Int64（保守推断，避免 empty-stack 错误）
            _ => Ok(Type::Int64),
        }
    }

    fn lookup_method_return(&self, obj_ty: &Type, method: &str) -> Option<Type> {
        match obj_ty {
            Type::Struct(name, type_args) => crate::metadata::stdlib_method_return_type(
                name,
                type_args,
                method,
            ),
            Type::Int8 => crate::metadata::stdlib_method_return_type("Int8", &[], method),
            Type::Int16 => crate::metadata::stdlib_method_return_type("Int16", &[], method),
            Type::Int32 => crate::metadata::stdlib_method_return_type("Int32", &[], method),
            Type::Int64 => crate::metadata::stdlib_method_return_type("Int64", &[], method),
            Type::IntNative => crate::metadata::stdlib_method_return_type("IntNative", &[], method),
            Type::UInt8 => crate::metadata::stdlib_method_return_type("UInt8", &[], method),
            Type::UInt16 => crate::metadata::stdlib_method_return_type("UInt16", &[], method),
            Type::UInt32 => crate::metadata::stdlib_method_return_type("UInt32", &[], method),
            Type::UInt64 => crate::metadata::stdlib_method_return_type("UInt64", &[], method),
            Type::UIntNative => crate::metadata::stdlib_method_return_type("UIntNative", &[], method),
            Type::Float16 => crate::metadata::stdlib_method_return_type("Float16", &[], method),
            Type::Float32 => crate::metadata::stdlib_method_return_type("Float32", &[], method),
            Type::Float64 => crate::metadata::stdlib_method_return_type("Float64", &[], method),
            Type::Array(elem_ty) => crate::metadata::stdlib_method_return_type(
                "Array",
                &[(*elem_ty.clone())],
                method,
            ),
            Type::String => crate::metadata::stdlib_method_return_type("String", &[], method),
            Type::Rune => crate::metadata::stdlib_method_return_type("Rune", &[], method),
            Type::Option(inner) => crate::metadata::stdlib_method_return_type(
                "Option",
                &[(*inner.clone())],
                method,
            ),
            Type::Result(ok, err) => crate::metadata::stdlib_method_return_type(
                "Result",
                &[(*ok.clone()), (*err.clone())],
                method,
            ),
            Type::Slice(elem_ty) => crate::metadata::stdlib_method_return_type(
                "Slice",
                &[(*elem_ty.clone())],
                method,
            ),
            Type::Map(key_ty, value_ty) => crate::metadata::stdlib_method_return_type(
                "Map",
                &[(*key_ty.clone()), (*value_ty.clone())],
                method,
            ),
            _ => None,
        }
    }

    /// 推断字段类型（查 struct_fields + class_fields，未知保守推断为 I32）
    pub fn infer_field_type(&self, obj_ty: &Type, field: &str) -> Result<Type, String> {
        Ok(self.lookup_field_type(obj_ty, field).unwrap_or(Type::Int32))
    }

    fn lookup_field_type(&self, obj_ty: &Type, field: &str) -> Option<Type> {
        match obj_ty {
            Type::Struct(name, type_args) => {
                let names = Self::resolve_type_names(name, type_args);
                for n in &names {
                    if let Some(fields) = self.struct_fields.get(n.as_str()) {
                        if let Some(ty) = fields.get(field) {
                            return Some(ty.clone());
                        }
                    }
                    if let Some(fields) = self.class_fields.get(n.as_str()) {
                        if let Some(ty) = fields.get(field) {
                            return Some(ty.clone());
                        }
                    }
                }
                crate::metadata::stdlib_field_type(name, type_args, field)
            }
            Type::Array(elem_ty) => crate::metadata::stdlib_field_type(
                "Array",
                &[(*elem_ty.clone())],
                field,
            ),
            Type::String => crate::metadata::stdlib_field_type("String", &[], field),
            Type::Range => crate::metadata::stdlib_field_type("Range", &[], field),
            _ => None,
        }
    }

    fn resolve_type_names(name: &str, type_args: &[Type]) -> Vec<String> {
        let mut names = Vec::new();
        if !type_args.is_empty() {
            names.push(crate::monomorph::mangle_name(name, type_args));
        }
        names.push(name.to_string());
        names
    }

    /// 收集函数中的局部变量类型
    pub fn collect_locals_from_function(&mut self, func: &Function) {
        // 添加参数
        for param in &func.params {
            self.add_local_with_mutability(param.name.clone(), param.ty.clone(), param.is_inout);
        }

        // 遍历函数体收集 let/var 声明
        for stmt in &func.body {
            self.collect_locals_from_stmt(stmt);
        }
    }

    /// 从语句中收集局部变量
    fn collect_locals_from_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
                let var_ty = if let Some(t) = ty {
                    t.clone()
                } else if let Ok(inferred) = self.infer_expr(value) {
                    inferred
                } else {
                    Type::Int32 // 保守推断为对象引用
                };
                let mutable = matches!(stmt, Stmt::Var { .. });
                self.collect_pattern_bindings(pattern, &var_ty, mutable);
            }
            // 递归处理嵌套语句
            Stmt::While { cond: _, body } | Stmt::Loop { body } => {
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::For { var, iterable, body } => {
                let iter_ty = match iterable {
                    Expr::Range { .. } => Type::Int64,
                    Expr::Array(elems) => elems
                        .first()
                        .and_then(|expr| self.infer_expr(expr).ok())
                        .unwrap_or(Type::Int64),
                    _ => Type::Int32,
                };
                self.add_local_with_mutability(var.clone(), iter_ty, true);
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::WhileLet { pattern, expr, body } => {
                let expr_ty = self.infer_expr(expr).unwrap_or(Type::Int32);
                self.collect_pattern_bindings(pattern, &expr_ty, false);
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::DoWhile { body, .. } => {
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::Expr(expr) => {
                self.collect_locals_from_expr(expr);
            }
            Stmt::Return(Some(expr)) => {
                self.collect_locals_from_expr(expr);
            }
            _ => {}
        }
    }

    /// 从模式中收集绑定变量
    fn collect_pattern_bindings(&mut self, pattern: &Pattern, ty: &Type, mutable: bool) {
        self.bind_pattern_types(pattern, ty, mutable);
    }

    fn bind_pattern_types(&mut self, pattern: &Pattern, ty: &Type, mutable: bool) {
        match pattern {
            Pattern::Binding(name) => {
                self.add_local_with_mutability(name.clone(), ty.clone(), mutable);
            }
            Pattern::Tuple(pats) => {
                if let Type::Tuple(items) = ty {
                    for (idx, p) in pats.iter().enumerate() {
                        let elem_ty = items.get(idx).unwrap_or(ty);
                        self.bind_pattern_types(p, elem_ty, mutable);
                    }
                } else {
                    for p in pats {
                        self.bind_pattern_types(p, ty, mutable);
                    }
                }
            }
            Pattern::Variant {
                enum_name,
                variant_name,
                payload: Some(payload),
            } => {
                if let Some(payload_ty) =
                    self.variant_payload_type(ty, enum_name.as_str(), variant_name.as_str())
                {
                    self.bind_pattern_types(payload, &payload_ty, mutable);
                }
            }
            Pattern::Struct { fields, .. } => {
                if let Type::Struct(name, type_args) = ty {
                    let names = Self::resolve_type_names(name, type_args);
                    for (field_name, field_pat) in fields {
                        let field_ty = names
                            .iter()
                            .find_map(|resolved| {
                                self.struct_fields
                                    .get(resolved.as_str())
                                    .and_then(|fields| fields.get(field_name))
                                    .cloned()
                                    .or_else(|| {
                                        self.class_fields
                                            .get(resolved.as_str())
                                            .and_then(|fields| fields.get(field_name))
                                            .cloned()
                                    })
                            })
                            .unwrap_or(Type::Int32);
                        self.bind_pattern_types(field_pat, &field_ty, mutable);
                    }
                }
            }
            Pattern::Or(pats) => {
                for p in pats {
                    self.bind_pattern_types(p, ty, mutable);
                }
            }
            Pattern::TypeTest { binding, ty } => {
                self.add_local_with_mutability(binding.clone(), ty.clone(), mutable);
            }
            _ => {}
        }
    }

    fn variant_payload_type(&self, subject_ty: &Type, enum_name: &str, variant_name: &str) -> Option<Type> {
        match (enum_name, variant_name, subject_ty) {
            ("Option", "Some", Type::Option(inner)) => Some((**inner).clone()),
            ("Option", "Some", Type::Struct(name, args)) if name == "Option" => {
                Some(args.first().cloned().unwrap_or(Type::Int64))
            }
            ("Result", "Ok", Type::Result(ok, _)) => Some((**ok).clone()),
            ("Result", "Err", Type::Result(_, err)) => Some((**err).clone()),
            ("Result", "Ok", Type::Struct(name, args)) if name == "Result" => {
                Some(args.first().cloned().unwrap_or(Type::Int64))
            }
            ("Result", "Err", Type::Struct(name, args)) if name == "Result" => {
                Some(args.get(1).cloned().unwrap_or(Type::String))
            }
            _ => None,
        }
    }

    /// 从表达式中收集局部变量（处理 Block、If、Match 等）
    fn collect_locals_from_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Block(stmts, result) => {
                for s in stmts {
                    self.collect_locals_from_stmt(s);
                }
                if let Some(e) = result {
                    self.collect_locals_from_expr(e);
                }
            }
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_locals_from_expr(then_branch);
                if let Some(else_expr) = else_branch {
                    self.collect_locals_from_expr(else_expr);
                }
            }
            Expr::IfLet {
                pattern,
                expr,
                then_branch,
                else_branch,
            } => {
                let subject_ty = self.infer_expr(expr).unwrap_or(Type::Int32);
                self.bind_pattern_types(pattern, &subject_ty, false);
                self.collect_locals_from_expr(then_branch);
                if let Some(else_expr) = else_branch {
                    self.collect_locals_from_expr(else_expr);
                }
            }
            Expr::Match { arms, .. } => {
                if let Expr::Match { expr: subject, .. } = expr {
                    if let Ok(subject_ty) = self.infer_expr(subject) {
                        for arm in arms {
                            self.bind_pattern_types(&arm.pattern, &subject_ty, false);
                            if let Some(guard) = &arm.guard {
                                self.collect_locals_from_expr(guard);
                            }
                            self.collect_locals_from_expr(&arm.body);
                        }
                        return;
                    }
                }
                for arm in arms {
                    self.collect_locals_from_expr(&arm.body);
                }
            }
            _ => {}
        }
    }
}

impl Default for TypeInferenceContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        ClassDef, ClassMethod, FieldDef, Function, MatchArm as AstMatchArm, Param, StructDef,
        Visibility,
    };

    fn empty_program() -> Program {
        Program {
            functions: vec![],
            structs: vec![],
            classes: vec![],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
            imports: vec![],
            package_name: None,
        }
    }

    fn make_param(name: &str, ty: Type) -> Param {
        Param {
            name: name.into(),
            ty,
            default: None,
            variadic: false,
            is_named: false,
            is_inout: false,
        }
    }

    fn make_field(name: &str, ty: Type) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty,
            default: None,
        }
    }

    fn make_function(
        name: &str,
        params: Vec<Param>,
        return_type: Option<Type>,
        body: Vec<Stmt>,
    ) -> Function {
        Function {
            visibility: Visibility::Public,
            name: name.into(),
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: None,
            body,
            extern_import: None,
        }
    }

    // ─── 字面量推断 ───

    #[test]
    fn test_infer_literal() {
        let ctx = TypeInferenceContext::new();
        assert_eq!(ctx.infer_expr(&Expr::Integer(42)).unwrap(), Type::Int64);
        assert_eq!(ctx.infer_expr(&Expr::Bool(true)).unwrap(), Type::Bool);
        assert_eq!(
            ctx.infer_expr(&Expr::String("hello".into())).unwrap(),
            Type::String
        );
    }

    #[test]
    fn test_infer_float_literals() {
        let ctx = TypeInferenceContext::new();
        assert_eq!(ctx.infer_expr(&Expr::Float(3.14)).unwrap(), Type::Float64);
        assert_eq!(ctx.infer_expr(&Expr::Float32(1.0)).unwrap(), Type::Float32);
        assert_eq!(ctx.infer_expr(&Expr::Rune('A')).unwrap(), Type::Rune);
    }

    #[test]
    fn test_infer_interpolate() {
        let ctx = TypeInferenceContext::new();
        assert_eq!(
            ctx.infer_expr(&Expr::Interpolate(vec![])).unwrap(),
            Type::String
        );
    }

    // ─── 变量推断 ───

    #[test]
    fn test_infer_var_local() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("x".into(), Type::Float64);
        assert_eq!(
            ctx.infer_expr(&Expr::Var("x".into())).unwrap(),
            Type::Float64
        );
    }

    #[test]
    fn test_infer_var_global() {
        let mut ctx = TypeInferenceContext::new();
        ctx.globals.insert("G".into(), Type::Bool);
        assert_eq!(ctx.infer_expr(&Expr::Var("G".into())).unwrap(), Type::Bool);
    }

    #[test]
    fn test_infer_var_math_constants() {
        let ctx = TypeInferenceContext::new();
        for name in &["PI", "E", "TAU", "INF", "NAN"] {
            assert_eq!(
                ctx.infer_expr(&Expr::Var(name.to_string())).unwrap(),
                Type::Float64
            );
        }
    }

    #[test]
    fn test_infer_var_unknown() {
        let ctx = TypeInferenceContext::new();
        assert_eq!(
            ctx.infer_expr(&Expr::Var("unknown".into())).unwrap(),
            Type::Int32
        );
    }

    // ─── 二元运算推断 ───

    #[test]
    fn test_infer_binary_arithmetic() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_binary_comparison() {
        let ctx = TypeInferenceContext::new();
        for op in &[
            BinOp::Lt,
            BinOp::Gt,
            BinOp::Eq,
            BinOp::NotEq,
            BinOp::LtEq,
            BinOp::GtEq,
        ] {
            let expr = Expr::Binary {
                op: op.clone(),
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(2)),
            };
            assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
        }
    }

    #[test]
    fn test_infer_binary_mixed_numeric_promotes_integral() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local_with_mutability("a".into(), Type::Int8, true);
        ctx.add_local_with_mutability("b".into(), Type::Int16, true);
        let expr = Expr::Binary {
            op: BinOp::Mul,
            left: Box::new(Expr::Var("a".into())),
            right: Box::new(Expr::Var("b".into())),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int32);
    }

    #[test]
    fn test_infer_binary_mixed_int32_int64_promotes_to_int64() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local_with_mutability("a".into(), Type::Int32, true);
        ctx.add_local_with_mutability("b".into(), Type::Int64, true);
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Var("a".into())),
            right: Box::new(Expr::Var("b".into())),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_shift_keeps_left_operand_type() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local_with_mutability("a".into(), Type::Int8, true);
        ctx.add_local_with_mutability("b".into(), Type::UInt64, true);
        let expr = Expr::Binary {
            op: BinOp::Shl,
            left: Box::new(Expr::Var("a".into())),
            right: Box::new(Expr::Var("b".into())),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int8);
    }

    #[test]
    fn test_infer_binary_mod_float_is_error() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Binary {
            op: BinOp::Mod,
            left: Box::new(Expr::Float32(1.0)),
            right: Box::new(Expr::Float32(1.0)),
        };
        let err = ctx.infer_expr(&expr).unwrap_err();
        assert!(err.contains("invalid binary operator"));
    }

    #[test]
    fn test_is_assignable_type_with_nominal_subtype() {
        let mut ctx = TypeInferenceContext::new();
        // C <: B <: A <: J <: I
        ctx.nominal_supertypes.insert("C".into(), vec!["B".into()]);
        ctx.nominal_supertypes.insert("B".into(), vec!["A".into()]);
        ctx.nominal_supertypes.insert("A".into(), vec!["J".into()]);
        ctx.nominal_supertypes.insert("J".into(), vec!["I".into()]);

        assert!(ctx.is_assignable_type(
            &Type::Struct("A".into(), vec![]),
            &Type::Struct("C".into(), vec![])
        ));
        assert!(ctx.is_assignable_type(
            &Type::Struct("I".into(), vec![]),
            &Type::Struct("C".into(), vec![])
        ));
        assert!(ctx.is_assignable_type(
            &Type::Struct("Object".into(), vec![]),
            &Type::Struct("C".into(), vec![])
        ));
    }

    #[test]
    fn test_is_assignable_type_integral_widths() {
        let ctx = TypeInferenceContext::new();
        assert!(ctx.is_assignable_type(&Type::Int32, &Type::Int64));
        assert!(ctx.is_assignable_type(&Type::Int64, &Type::Int32));
    }

    #[test]
    fn test_is_assignable_type_option_result_bridge_with_struct_form() {
        let ctx = TypeInferenceContext::new();
        assert!(ctx.is_assignable_type(
            &Type::Struct("Option".into(), vec![Type::Int64]),
            &Type::Option(Box::new(Type::Nothing))
        ));
        assert!(ctx.is_assignable_type(
            &Type::Option(Box::new(Type::Int64)),
            &Type::Struct("Option".into(), vec![Type::Int64])
        ));
        assert!(ctx.is_assignable_type(
            &Type::Struct("Result".into(), vec![Type::Int64, Type::String]),
            &Type::Result(Box::new(Type::Int64), Box::new(Type::Nothing))
        ));
        assert!(ctx.is_assignable_type(
            &Type::Result(Box::new(Type::Int64), Box::new(Type::String)),
            &Type::Struct("Result".into(), vec![Type::Int64, Type::String])
        ));
    }

    #[test]
    fn test_is_assignable_type_allows_value_to_option_autowrap() {
        let ctx = TypeInferenceContext::new();
        assert!(ctx.is_assignable_type(
            &Type::Option(Box::new(Type::Int8)),
            &Type::Int64
        ));
    }

    #[test]
    fn test_infer_binary_result_allows_bool_integral_equality() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Binary {
            op: BinOp::NotEq,
            left: Box::new(Expr::Bool(true)),
            right: Box::new(Expr::Integer(0)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }

    #[test]
    fn test_infer_eq_allows_option_none_compatibility() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("opt".into(), Type::Option(Box::new(Type::Int64)));
        let expr = Expr::Binary {
            op: BinOp::NotEq,
            left: Box::new(Expr::Var("opt".into())),
            right: Box::new(Expr::None),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }

    #[test]
    fn test_is_assignable_type_function_return_covariant() {
        let mut ctx = TypeInferenceContext::new();
        // C <: A
        ctx.nominal_supertypes.insert("C".into(), vec!["A".into()]);

        let source = Type::Function {
            params: vec![
                Type::Struct("A".into(), vec![]),
                Type::Struct("B".into(), vec![]),
            ],
            ret: Box::new(Some(Type::Tuple(vec![
                Type::Struct("A".into(), vec![]),
                Type::Struct("C".into(), vec![]),
            ]))),
        };
        let target = Type::Function {
            params: vec![
                Type::Struct("A".into(), vec![]),
                Type::Struct("B".into(), vec![]),
            ],
            ret: Box::new(Some(Type::Tuple(vec![
                Type::Struct("A".into(), vec![]),
                Type::Struct("A".into(), vec![]),
            ]))),
        };

        // (A, B) -> (A, C) 是 (A, B) -> (A, A) 的子类型（返回值协变）
        assert!(ctx.is_assignable_type(&target, &source));
    }

    #[test]
    fn test_infer_binary_logical() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Binary {
            op: BinOp::LogicalAnd,
            left: Box::new(Expr::Bool(true)),
            right: Box::new(Expr::Bool(false)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }

    #[test]
    fn test_infer_binary_bitwise() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Binary {
            op: BinOp::BitAnd,
            left: Box::new(Expr::Integer(0xFF)),
            right: Box::new(Expr::Integer(0x0F)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    // ─── 一元运算推断 ───

    #[test]
    fn test_infer_unary_not() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(Expr::Bool(true)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }

    #[test]
    fn test_infer_unary_neg() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Integer(42)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    // ─── 函数调用推断 ───

    #[test]
    fn test_infer_call_known_function() {
        let mut ctx = TypeInferenceContext::new();
        ctx.functions.insert(
            "foo".into(),
            FunctionSignature {
                name: "foo".into(),
                params: vec![Type::Int64],
                return_ty: Type::Float64,
            },
        );
        let expr = Expr::Call {
            name: "foo".into(),
            args: vec![Expr::Integer(1)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float64);
    }

    #[test]
    fn test_infer_call_mangled() {
        let mut ctx = TypeInferenceContext::new();
        ctx.functions.insert(
            "foo$2".into(),
            FunctionSignature {
                name: "foo".into(),
                params: vec![Type::Int64, Type::Int64],
                return_ty: Type::Bool,
            },
        );
        let expr = Expr::Call {
            name: "foo".into(),
            args: vec![Expr::Integer(1), Expr::Integer(2)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }

    #[test]
    fn test_infer_call_builtins() {
        let ctx = TypeInferenceContext::new();
        let println_expr = Expr::Call {
            name: "println".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&println_expr).unwrap(), Type::Unit);

        let readln_expr = Expr::Call {
            name: "readln".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&readln_expr).unwrap(), Type::String);

        let exit_expr = Expr::Call {
            name: "exit".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&exit_expr).unwrap(), Type::Nothing);
    }

    #[test]
    fn test_infer_call_type_conversion() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Call {
            name: "Int32".into(),
            args: vec![Expr::Integer(42)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int32);

        let expr = Expr::Call {
            name: "Float64".into(),
            args: vec![Expr::Integer(42)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float64);

        let expr = Expr::Call {
            name: "toString".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::String);
    }

    #[test]
    fn test_infer_call_constructor() {
        let mut ctx = TypeInferenceContext::new();
        ctx.struct_fields.insert("Point".into(), HashMap::new());
        let expr = Expr::Call {
            name: "Point".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(
            ctx.infer_expr(&expr).unwrap(),
            Type::Struct("Point".into(), vec![])
        );
    }

    // ─── 方法调用推断 ───

    #[test]
    fn test_infer_method_return_user_defined() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("obj".into(), Type::Struct("MyClass".into(), vec![]));
        let mut methods = HashMap::new();
        methods.insert("getValue".into(), Type::Float64);
        ctx.class_method_returns.insert("MyClass".into(), methods);

        let expr = Expr::MethodCall {
            object: Box::new(Expr::Var("obj".into())),
            method: "getValue".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float64);
    }

    #[test]
    fn test_infer_method_return_array_builtin() {
        let ctx = TypeInferenceContext::new();
        let arr = Expr::Array(vec![Expr::Integer(1)]);
        let expr = Expr::MethodCall {
            object: Box::new(arr),
            method: "size".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_method_return_tostring() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::MethodCall {
            object: Box::new(Expr::Integer(42)),
            method: "toString".into(),
            args: vec![],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::String);
    }

    #[test]
    fn test_infer_field_type_for_range_properties() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Field {
            object: Box::new(Expr::Range {
                start: Box::new(Expr::Integer(1)),
                end: Box::new(Expr::Integer(3)),
                inclusive: false,
                step: None,
            }),
            field: "start".into(),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_method_return_extension_on_primitive_type() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("n".into(), Type::Int64);
        ctx.class_method_returns.insert(
            "Int64".into(),
            HashMap::from([(
                "checkedPow".into(),
                Type::Option(Box::new(Type::Int64)),
            )]),
        );

        let expr = Expr::MethodCall {
            object: Box::new(Expr::Var("n".into())),
            method: "checkedPow".into(),
            args: vec![Expr::Integer(2)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(
            ctx.infer_expr(&expr).unwrap(),
            Type::Option(Box::new(Type::Int64))
        );
    }

    #[test]
    fn test_infer_method_return_hashmap_runtime_overrides_metadata() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local(
            "map".into(),
            Type::Struct("HashMap".into(), vec![Type::Int64, Type::Int64]),
        );

        let get_expr = Expr::MethodCall {
            object: Box::new(Expr::Var("map".into())),
            method: "get".into(),
            args: vec![Expr::Integer(1)],
            type_args: None,
            named_args: vec![],
        };
        let contains_expr = Expr::MethodCall {
            object: Box::new(Expr::Var("map".into())),
            method: "containsKey".into(),
            args: vec![Expr::Integer(1)],
            type_args: None,
            named_args: vec![],
        };

        assert_eq!(ctx.infer_expr(&get_expr).unwrap(), Type::Int64);
        assert_eq!(ctx.infer_expr(&contains_expr).unwrap(), Type::Int64);
    }

    // ─── 字段推断 ───

    #[test]
    fn test_infer_field_type_struct() {
        let mut ctx = TypeInferenceContext::new();
        let mut fields = HashMap::new();
        fields.insert("x".into(), Type::Float64);
        fields.insert("y".into(), Type::Float64);
        ctx.struct_fields.insert("Point".into(), fields);

        let result = ctx
            .infer_field_type(&Type::Struct("Point".into(), vec![]), "x")
            .unwrap();
        assert_eq!(result, Type::Float64);
    }

    #[test]
    fn test_infer_field_type_class() {
        let mut ctx = TypeInferenceContext::new();
        let mut fields = HashMap::new();
        fields.insert("count".into(), Type::Int64);
        ctx.class_fields.insert("Counter".into(), fields);

        let result = ctx
            .infer_field_type(&Type::Struct("Counter".into(), vec![]), "count")
            .unwrap();
        assert_eq!(result, Type::Int64);
    }

    #[test]
    fn test_infer_field_type_unknown() {
        let ctx = TypeInferenceContext::new();
        let result = ctx
            .infer_field_type(&Type::Struct("Unknown".into(), vec![]), "x")
            .unwrap();
        assert_eq!(result, Type::Int32);
    }

    #[test]
    fn test_infer_field_expr() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("p".into(), Type::Struct("Point".into(), vec![]));
        let mut fields = HashMap::new();
        fields.insert("x".into(), Type::Float64);
        ctx.struct_fields.insert("Point".into(), fields);

        let expr = Expr::Field {
            object: Box::new(Expr::Var("p".into())),
            field: "x".into(),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float64);
    }

    #[test]
    fn test_infer_field_reject_instance_access_static_member() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("c".into(), Type::Struct("C".into(), vec![]));
        ctx.class_fields
            .insert("C".into(), HashMap::from([("x".into(), Type::Int64)]));
        ctx.class_static_fields
            .insert("C".into(), HashSet::from(["x".into()]));

        let expr = Expr::Field {
            object: Box::new(Expr::Var("c".into())),
            field: "x".into(),
        };
        let err = ctx.infer_expr(&expr).unwrap_err();
        assert!(err.contains("static member"));
    }

    #[test]
    fn test_infer_option_result_constructors() {
        let ctx = TypeInferenceContext::new();
        assert_eq!(
            ctx.infer_expr(&Expr::VariantConst {
                enum_name: "CasingOption".into(),
                variant_name: "TR".into(),
                arg: None,
            })
            .unwrap(),
            Type::Struct("CasingOption".into(), vec![])
        );
        assert_eq!(
            ctx.infer_expr(&Expr::Some(Box::new(Expr::Integer(1)))).unwrap(),
            Type::Option(Box::new(Type::Int64))
        );
        assert_eq!(
            ctx.infer_expr(&Expr::None).unwrap(),
            Type::Option(Box::new(Type::Nothing))
        );
        assert_eq!(
            ctx.infer_expr(&Expr::Ok(Box::new(Expr::Integer(1)))).unwrap(),
            Type::Result(Box::new(Type::Int64), Box::new(Type::Nothing))
        );
        assert_eq!(
            ctx.infer_expr(&Expr::Err(Box::new(Expr::String("e".into()))))
                .unwrap(),
            Type::Result(Box::new(Type::Nothing), Box::new(Type::String))
        );
    }

    #[test]
    fn test_infer_null_coalesce_uses_option_payload_type() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local("opt".into(), Type::Option(Box::new(Type::Int64)));
        let expr = Expr::NullCoalesce {
            option: Box::new(Expr::Var("opt".into())),
            default: Box::new(Expr::Integer(0)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_field_allow_type_access_static_member() {
        let mut ctx = TypeInferenceContext::new();
        ctx.class_fields
            .insert("C".into(), HashMap::from([("x".into(), Type::Int64)]));
        ctx.class_static_fields
            .insert("C".into(), HashSet::from(["x".into()]));

        let expr = Expr::Field {
            object: Box::new(Expr::Var("C".into())),
            field: "x".into(),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_postfix_incr_reject_non_assignable_tuple_index() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local(
            "t".into(),
            Type::Tuple(vec![Type::Int64, Type::Int64, Type::Int64]),
        );
        let expr = Expr::PostfixIncr(Box::new(Expr::Index {
            array: Box::new(Expr::Var("t".into())),
            index: Box::new(Expr::Integer(1)),
        }));

        let err = ctx.infer_expr(&expr).unwrap_err();
        assert!(err.contains("not assignable"));
    }

    #[test]
    fn test_infer_postfix_incr_allow_assignable_var() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local_with_mutability("x".into(), Type::Int64, true);
        let expr = Expr::PostfixIncr(Box::new(Expr::Var("x".into())));
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    // ─── 数组/元组推断 ───

    #[test]
    fn test_infer_array() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Array(vec![Expr::Integer(1), Expr::Integer(2)]);
        let ty = ctx.infer_expr(&expr).unwrap();
        assert_eq!(ty, Type::Array(Box::new(Type::Int64)));
    }

    #[test]
    fn test_infer_empty_array() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Array(vec![]);
        assert_eq!(
            ctx.infer_expr(&expr).unwrap(),
            Type::Array(Box::new(Type::Int64))
        );
    }

    #[test]
    fn test_infer_index() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Index {
            array: Box::new(Expr::Array(vec![Expr::Float(1.0)])),
            index: Box::new(Expr::Integer(0)),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float64);
    }

    #[test]
    fn test_infer_tuple() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Tuple(vec![Expr::Integer(1), Expr::Bool(true)]);
        let ty = ctx.infer_expr(&expr).unwrap();
        assert_eq!(ty, Type::Tuple(vec![Type::Int64, Type::Bool]));
    }

    // ─── 结构体/构造函数推断 ───

    #[test]
    fn test_infer_struct_init() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::StructInit {
            name: "Point".into(),
            fields: vec![],
            type_args: None,
        };
        assert_eq!(
            ctx.infer_expr(&expr).unwrap(),
            Type::Struct("Point".into(), vec![])
        );
    }

    #[test]
    fn test_infer_constructor_call() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::ConstructorCall {
            name: "Array".into(),
            args: vec![],
            type_args: Some(vec![Type::Bool]),
            named_args: vec![],
        };
        assert_eq!(
            ctx.infer_expr(&expr).unwrap(),
            Type::Array(Box::new(Type::Bool))
        );
    }

    #[test]
    fn test_infer_constructor_numeric_cast() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::ConstructorCall {
            name: "Float32".into(),
            args: vec![Expr::Integer(42)],
            type_args: None,
            named_args: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Float32);
    }

    // ─── If/Match 推断 ───

    #[test]
    fn test_infer_if_with_else() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: Some(Box::new(Expr::Integer(2))),
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_if_without_else() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: None,
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Unit);
    }

    #[test]
    fn test_infer_match() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Match {
            expr: Box::new(Expr::Integer(1)),
            arms: vec![AstMatchArm {
                pattern: Pattern::Binding("x".into()),
                guard: None,
                body: Box::new(Expr::String("hello".into())),
            }],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::String);
    }

    #[test]
    fn test_infer_match_empty() {
        let ctx = TypeInferenceContext::new();
        let expr = Expr::Match {
            expr: Box::new(Expr::Integer(1)),
            arms: vec![],
        };
        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Unit);
    }

    // ─── add_local / get_local ───

    #[test]
    fn test_add_get_local() {
        let mut ctx = TypeInferenceContext::new();
        assert!(ctx.get_local("x").is_none());
        ctx.add_local("x".into(), Type::Bool);
        assert_eq!(ctx.get_local("x").unwrap(), &Type::Bool);
    }

    // ─── from_program ───

    #[test]
    fn test_from_program_functions() {
        let mut prog = empty_program();
        prog.functions.push(make_function(
            "add",
            vec![make_param("a", Type::Int64), make_param("b", Type::Int64)],
            Some(Type::Int64),
            vec![],
        ));
        let ctx = TypeInferenceContext::from_program(&prog);
        assert!(ctx.functions.contains_key("add"));
        assert!(ctx.functions.contains_key("add$2"));
    }

    #[test]
    fn test_from_program_structs() {
        let mut prog = empty_program();
        prog.structs.push(StructDef {
            visibility: Visibility::Public,
            name: "Point".into(),
            type_params: vec![],
            constraints: vec![],
            fields: vec![
                make_field("x", Type::Float64),
                make_field("y", Type::Float64),
            ],
        });
        let ctx = TypeInferenceContext::from_program(&prog);
        let fields = ctx.struct_fields.get("Point").unwrap();
        assert_eq!(fields.get("x").unwrap(), &Type::Float64);
        assert_eq!(fields.get("y").unwrap(), &Type::Float64);
    }

    #[test]
    fn test_from_program_classes_with_methods() {
        let mut prog = empty_program();
        prog.classes.push(ClassDef {
            visibility: Visibility::Public,
            name: "Counter".into(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: None,
            implements: vec![],
            fields: vec![make_field("count", Type::Int64)],
            init: None,
            deinit: None,
            static_init: None,
            primary_ctor_params: vec![],
            methods: vec![ClassMethod {
                override_: false,
                func: make_function(
                    "Counter.get",
                    vec![make_param("this", Type::Struct("Counter".into(), vec![]))],
                    Some(Type::Int64),
                    vec![],
                ),
            }],
        });
        let ctx = TypeInferenceContext::from_program(&prog);
        assert!(ctx.class_fields.contains_key("Counter"));
        let methods = ctx.class_method_returns.get("Counter").unwrap();
        assert_eq!(methods.get("get").unwrap(), &Type::Int64);
        assert!(ctx.functions.contains_key("Counter.get"));
    }

    #[test]
    fn test_from_program_inheritance() {
        let mut prog = empty_program();
        let base_class = ClassDef {
            visibility: Visibility::Public,
            name: "Base".into(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: true,
            extends: None,
            implements: vec![],
            fields: vec![make_field("id", Type::Int64)],
            init: None,
            deinit: None,
            static_init: None,
            primary_ctor_params: vec![],
            methods: vec![],
        };
        let child_class = ClassDef {
            visibility: Visibility::Public,
            name: "Child".into(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: Some("Base".into()),
            implements: vec![],
            fields: vec![make_field("name", Type::String)],
            init: None,
            deinit: None,
            static_init: None,
            primary_ctor_params: vec![],
            methods: vec![],
        };
        prog.classes.push(base_class);
        prog.classes.push(child_class);
        let ctx = TypeInferenceContext::from_program(&prog);
        let child_fields = ctx.class_fields.get("Child").unwrap();
        assert_eq!(child_fields.get("name").unwrap(), &Type::String);
        assert_eq!(child_fields.get("id").unwrap(), &Type::Int64);
    }

    // ─── collect_locals_from_function ───

    #[test]
    fn test_collect_locals_from_function() {
        let mut ctx = TypeInferenceContext::new();
        let func = make_function(
            "test",
            vec![make_param("a", Type::Int64)],
            Some(Type::Bool),
            vec![
                Stmt::Let {
                    pattern: Pattern::Binding("x".into()),
                    ty: Some(Type::Float64),
                    value: Expr::Float(1.0),
                },
                Stmt::Var {
                    pattern: Pattern::Binding("y".into()),
                    ty: None,
                    value: Expr::Integer(0),
                },
            ],
        );
        ctx.collect_locals_from_function(&func);
        assert_eq!(ctx.get_local("a").unwrap(), &Type::Int64);
        assert_eq!(ctx.get_local("x").unwrap(), &Type::Float64);
        assert_eq!(ctx.get_local("y").unwrap(), &Type::Int64);
    }

    #[test]
    fn test_collect_locals_nested() {
        let mut ctx = TypeInferenceContext::new();
        let func = make_function(
            "test",
            vec![],
            None,
            vec![
                Stmt::While {
                    cond: Expr::Bool(true),
                    body: vec![Stmt::Let {
                        pattern: Pattern::Binding("inner".into()),
                        ty: Some(Type::Bool),
                        value: Expr::Bool(false),
                    }],
                },
                Stmt::For {
                    var: "i".into(),
                    iterable: Expr::Integer(0),
                    body: vec![],
                },
            ],
        );
        ctx.collect_locals_from_function(&func);
        assert_eq!(ctx.get_local("inner").unwrap(), &Type::Bool);
        assert!(ctx.get_local("i").is_some());
    }

    #[test]
    fn test_collect_locals_while_let_payload_and_if_let_payload() {
        let mut ctx = TypeInferenceContext::new();
        let some_x = Pattern::Variant {
            enum_name: "Option".into(),
            variant_name: "Some".into(),
            payload: Some(Box::new(Pattern::Binding("x".into()))),
        };
        let some_n = Pattern::Variant {
            enum_name: "Option".into(),
            variant_name: "Some".into(),
            payload: Some(Box::new(Pattern::Binding("n".into()))),
        };
        let func = make_function(
            "test",
            vec![make_param("opt", Type::Option(Box::new(Type::Int64)))],
            Some(Type::Int64),
            vec![
                Stmt::Expr(Expr::IfLet {
                    pattern: some_x,
                    expr: Box::new(Expr::Var("opt".into())),
                    then_branch: Box::new(Expr::Var("x".into())),
                    else_branch: Some(Box::new(Expr::Integer(0))),
                }),
                Stmt::WhileLet {
                    pattern: some_n,
                    expr: Box::new(Expr::Var("opt".into())),
                    body: vec![Stmt::Expr(Expr::Var("n".into()))],
                },
            ],
        );
        ctx.collect_locals_from_function(&func);
        assert_eq!(ctx.get_local("x"), Some(&Type::Int64));
        assert_eq!(ctx.get_local("n"), Some(&Type::Int64));
    }

    #[test]
    fn test_collect_locals_if_let_payload_with_struct_option_type() {
        let mut ctx = TypeInferenceContext::new();
        let some_x = Pattern::Variant {
            enum_name: "Option".into(),
            variant_name: "Some".into(),
            payload: Some(Box::new(Pattern::Binding("x".into()))),
        };
        let func = make_function(
            "test",
            vec![make_param(
                "opt",
                Type::Struct("Option".into(), vec![Type::Int64]),
            )],
            Some(Type::Int64),
            vec![Stmt::Expr(Expr::IfLet {
                pattern: some_x,
                expr: Box::new(Expr::Var("opt".into())),
                then_branch: Box::new(Expr::Var("x".into())),
                else_branch: Some(Box::new(Expr::Integer(0))),
            })],
        );
        ctx.collect_locals_from_function(&func);
        assert_eq!(ctx.get_local("x"), Some(&Type::Int64));
    }

    #[test]
    fn test_collect_locals_struct_destructure_and_match_fields() {
        let mut ctx = TypeInferenceContext::new();
        ctx.struct_fields.insert(
            "Point".into(),
            HashMap::from([
                ("x".into(), Type::Int64),
                ("y".into(), Type::Int64),
            ]),
        );
        let func = make_function(
            "test",
            vec![make_param("p", Type::Struct("Point".into(), vec![]))],
            Some(Type::Int64),
            vec![
                Stmt::Let {
                    pattern: Pattern::Struct {
                        name: "Point".into(),
                        fields: vec![
                            ("x".into(), Pattern::Binding("x".into())),
                            ("y".into(), Pattern::Binding("y".into())),
                        ],
                    },
                    ty: None,
                    value: Expr::Var("p".into()),
                },
                Stmt::Expr(Expr::Match {
                    expr: Box::new(Expr::Var("p".into())),
                    arms: vec![AstMatchArm {
                        pattern: Pattern::Struct {
                            name: "Point".into(),
                            fields: vec![
                                ("x".into(), Pattern::Binding("mx".into())),
                                ("y".into(), Pattern::Binding("my".into())),
                            ],
                        },
                        guard: None,
                        body: Box::new(Expr::Binary {
                            op: BinOp::Add,
                            left: Box::new(Expr::Var("mx".into())),
                            right: Box::new(Expr::Var("my".into())),
                        }),
                    }],
                }),
            ],
        );
        ctx.collect_locals_from_function(&func);
        assert_eq!(ctx.get_local("x"), Some(&Type::Int64));
        assert_eq!(ctx.get_local("y"), Some(&Type::Int64));
        assert_eq!(ctx.get_local("mx"), Some(&Type::Int64));
        assert_eq!(ctx.get_local("my"), Some(&Type::Int64));
    }

    // ─── infer_return_type_from_body ───

    #[test]
    fn test_infer_return_type_from_body() {
        let ctx = TypeInferenceContext::new();
        let body = vec![
            Stmt::Expr(Expr::Integer(0)),
            Stmt::Return(Some(Expr::Float(3.14))),
        ];
        let result = infer_return_type_from_body(&body, &ctx);
        assert_eq!(result, Some(Type::Float64));
    }

    #[test]
    fn test_infer_return_type_from_body_none() {
        let ctx = TypeInferenceContext::new();
        let body = vec![Stmt::Expr(Expr::Integer(0))];
        assert_eq!(infer_return_type_from_body(&body, &ctx), None);
    }

    #[test]
    fn test_infer_return_type_nested_in_loop() {
        let ctx = TypeInferenceContext::new();
        let body = vec![Stmt::While {
            cond: Expr::Bool(true),
            body: vec![Stmt::Return(Some(Expr::String("done".into())))],
        }];
        let result = infer_return_type_from_body(&body, &ctx);
        assert_eq!(result, Some(Type::String));
    }

    // ─── Default trait ───

    #[test]
    fn test_default() {
        let ctx = TypeInferenceContext::default();
        assert!(ctx.locals.is_empty());
        assert!(ctx.functions.is_empty());
    }
}
