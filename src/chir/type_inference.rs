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

    /// 类方法返回类型：class_name → method_name → return_type
    pub class_method_returns: HashMap<String, HashMap<String, Type>>,

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
            class_method_returns: HashMap::new(),
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

        // 注册 extend 方法签名
        for ext in &program.extends {
            for method in &ext.methods {
                let sig = FunctionSignature {
                    name: method.name.clone(),
                    params: method.params.iter().map(|p| p.ty.clone()).collect(),
                    return_ty: method.return_type.clone().unwrap_or(Type::Unit),
                };
                let mangled = format!("{}${}", method.name, method.params.len());
                ctx.functions.insert(mangled, sig.clone());
                ctx.functions.entry(method.name.clone()).or_insert(sig);

                if let Some(dot_pos) = method.name.find('.') {
                    let type_name = &method.name[..dot_pos];
                    let method_name = &method.name[dot_pos + 1..];
                    let ret_ty = method.return_type.clone().unwrap_or(Type::Unit);
                    ctx.class_method_returns
                        .entry(type_name.to_string())
                        .or_default()
                        .entry(method_name.to_string())
                        .or_insert(ret_ty);
                }
            }
        }

        // 收集类字段 + 类方法签名
        for class_def in &program.classes {
            let mut fields = HashMap::new();
            for field in &class_def.fields {
                fields.insert(field.name.clone(), field.ty.clone());
            }
            ctx.class_fields.insert(class_def.name.clone(), fields);

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
                self.infer_field_type(&obj_ty, field)
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
            Expr::Index { array, .. } => {
                let arr_ty = self.infer_expr(array)?;
                match arr_ty {
                    Type::Array(elem_ty) => Ok(*elem_ty),
                    Type::Tuple(types) => {
                        // 元组索引，返回第一个元素类型（简化）
                        Ok(types.first().cloned().unwrap_or(Type::Int64))
                    }
                    _ => Ok(Type::Int64),
                }
            }

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

            // Match 表达式
            Expr::Match { arms, .. } => {
                if arms.is_empty() {
                    Ok(Type::Unit)
                } else {
                    self.infer_expr(&arms[0].body)
                }
            }

            // 其他表达式：保守推断为对象引用 (I32)
            _ => Ok(Type::Int32),
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
                if left == right {
                    Ok(Type::Bool)
                } else {
                    invalid()
                }
            }
            BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                if left == right
                    && (Self::is_numeric(left) || matches!(left, Type::Rune | Type::String))
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
                if left == right && Self::is_integral(left) {
                    Ok(left.clone())
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
                } else {
                    invalid()
                }
            }
            // 位运算返回整数类型
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                if left == right && Self::is_integral(left) {
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
            UnaryOp::Not => Ok(Type::Bool),
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
            Type::Struct(n, _) => n.as_str(),
            Type::Array(_) => "Array",
            _ => "",
        };

        // 优先查找用户定义的类方法真实返回类型
        if !obj_type_name.is_empty() {
            if let Some(methods) = self.class_method_returns.get(obj_type_name) {
                if let Some(ret_ty) = methods.get(method) {
                    return Ok(ret_ty.clone());
                }
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
        // 通用方法名推断（fallback）
        // 原则：宁可返回 Int64（emit 一个零值），也不能错误地返回 Unit（导致 empty-stack）
        // 只有确定对任何对象类型都是 void 的方法才返回 Unit
        match method {
            "toString" => Ok(Type::String),
            // 默认返回 Int64（保守推断，避免 empty-stack 错误）
            _ => Ok(Type::Int64),
        }
    }

    /// 推断字段类型（查 struct_fields + class_fields，未知保守推断为 I32）
    pub fn infer_field_type(&self, obj_ty: &Type, field: &str) -> Result<Type, String> {
        match obj_ty {
            Type::Struct(name, type_args) => {
                let names = Self::resolve_type_names(name, type_args);
                for n in &names {
                    if let Some(fields) = self.struct_fields.get(n.as_str()) {
                        if let Some(ty) = fields.get(field) {
                            return Ok(ty.clone());
                        }
                    }
                    if let Some(fields) = self.class_fields.get(n.as_str()) {
                        if let Some(ty) = fields.get(field) {
                            return Ok(ty.clone());
                        }
                    }
                }
                Ok(Type::Int32)
            }
            _ => Ok(Type::Int32),
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
            Stmt::For { var, body, .. } => {
                // For 循环变量：类型保守推断为 I32（迭代器元素常为对象或整数）
                self.add_local_with_mutability(var.clone(), Type::Int32, true);
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::WhileLet { pattern, body, .. } => {
                self.collect_pattern_bindings(pattern, &Type::Int32, false);
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
        match pattern {
            Pattern::Binding(name) => {
                self.add_local_with_mutability(name.clone(), ty.clone(), mutable);
            }
            Pattern::Tuple(pats) => {
                for p in pats {
                    self.collect_pattern_bindings(p, ty, mutable);
                }
            }
            _ => {}
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
            Expr::Match { arms, .. } => {
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
    fn test_infer_binary_mixed_numeric_is_error() {
        let mut ctx = TypeInferenceContext::new();
        ctx.add_local_with_mutability("a".into(), Type::Int8, true);
        ctx.add_local_with_mutability("b".into(), Type::Int16, true);
        let expr = Expr::Binary {
            op: BinOp::Mul,
            left: Box::new(Expr::Var("a".into())),
            right: Box::new(Expr::Var("b".into())),
        };
        let err = ctx.infer_expr(&expr).unwrap_err();
        assert!(err.contains("invalid binary operator"));
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
