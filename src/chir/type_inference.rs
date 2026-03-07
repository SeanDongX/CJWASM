//! 类型推断器 - 遍历 AST 推断表达式类型

use crate::ast::{Expr, Stmt, Type, Function, Program, BinOp, UnaryOp, Pattern};
use std::collections::HashMap;

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
        Stmt::Return(Some(expr)) => {
            ctx.infer_expr(expr).ok().filter(|t| !matches!(t, Type::Unit | Type::Nothing))
        }
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
                let short_name = full_name.strip_prefix(&format!("{}.", class_def.name))
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
            ctx.class_method_returns.insert(class_def.name.clone(), method_returns);
        }

        // 继承合并：将父类字段 + 方法签名传播到子类（多轮直到稳定）
        let class_extends: HashMap<String, Option<String>> = program.classes.iter()
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
                    if let Some(parent_methods) = ctx.class_method_returns.get(parent_name).cloned() {
                        let child_methods = ctx.class_method_returns.get_mut(&class_def.name).unwrap();
                        for (name, ret_ty) in parent_methods {
                            child_methods.entry(name).or_insert(ret_ty);
                        }
                    }
                    parent = class_extends.get(parent_name).and_then(|p| p.clone());
                }
            }
            if !changed { break; }
        }

        ctx
    }

    /// 添加局部变量
    pub fn add_local(&mut self, name: String, ty: Type) {
        self.locals.insert(name, ty);
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
                if matches!(name.as_str(), "PI" | "E" | "TAU" | "INF" | "INFINITY" | "NEG_INF" | "NEG_INFINITY" | "NAN") {
                    return Ok(Type::Float64);
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
                if let Some(sig) = self.functions.get(mangled.as_str())
                    .or_else(|| self.functions.get(name.as_str()))
                {
                    return Ok(sig.return_ty.clone());
                }
                // 内置函数
                match name.as_str() {
                    "println" | "print" | "eprintln" | "eprint" => Ok(Type::Unit),
                    "readln" => Ok(Type::String),
                    "exit" => Ok(Type::Nothing),
                    "abs" | "min" | "max" if !args.is_empty() => {
                        // 优先查函数签名，无签名则从第一个参数推断
                        self.infer_expr(&args[0])
                    }
                    // 整数类型转换构造函数 → 对应整数类型
                    "Int8" | "Int16" | "Int32" | "UInt8" | "UInt16" | "UInt32" => Ok(Type::Int32),
                    "Int64" | "UInt64" | "IntNative" | "UIntNative" => Ok(Type::Int64),
                    // 浮点类型转换构造函数
                    "Float16" | "Float32" => Ok(Type::Float32),
                    "Float64" => Ok(Type::Float64),
                    // 字符串相关
                    "toString" | "format" => Ok(Type::String),
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
            Expr::MethodCall { object, method, args, .. } => {
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

            // 元组
            Expr::Tuple(elems) => {
                let types: Result<Vec<_>, _> = elems.iter().map(|e| self.infer_expr(e)).collect();
                Ok(Type::Tuple(types?))
            }

            // 结构体初始化
            Expr::StructInit { name, .. } => {
                Ok(Type::Struct(name.clone(), vec![]))
            }

            // 构造函数调用
            Expr::ConstructorCall { name, type_args, .. } => {
                match name.as_str() {
                    // 类型转换构造函数
                    "Float32" => return Ok(Type::Float32),
                    "Float64" => return Ok(Type::Float64),
                    "Int32" | "UInt32" => return Ok(Type::Int32),
                    "Int64" | "UInt64" => return Ok(Type::Int64),
                    "Array" => {
                        let elem_ty = type_args.as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        Ok(Type::Array(Box::new(elem_ty)))
                    }
                    "ArrayList" | "LinkedList" => {
                        let elem_ty = type_args.as_ref()
                            .and_then(|ta| ta.first().cloned())
                            .unwrap_or(Type::Int64);
                        Ok(Type::Array(Box::new(elem_ty)))
                    }
                    _ => Ok(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
                }
            }

            // If 表达式
            Expr::If { then_branch, else_branch, .. } => {
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
    fn infer_binary_result(&self, op: &BinOp, left: &Type, _right: &Type) -> Result<Type, String> {
        match op {
            // 比较运算符返回 Bool
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                Ok(Type::Bool)
            }
            // 逻辑运算符返回 Bool
            BinOp::LogicalAnd | BinOp::LogicalOr => Ok(Type::Bool),
            // 算术运算符返回操作数类型
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                // 简化：返回左操作数类型
                Ok(left.clone())
            }
            // 位运算返回整数类型
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                Ok(left.clone())
            }
            _ => Ok(left.clone()),
        }
    }

    /// 推断一元运算结果类型
    fn infer_unary_result(&self, op: &UnaryOp, expr_ty: &Type) -> Result<Type, String> {
        match op {
            UnaryOp::Not => Ok(Type::Bool),
            UnaryOp::Neg | UnaryOp::BitNot => Ok(expr_ty.clone()),
        }
    }

    /// 推断方法返回类型
    fn infer_method_return(&self, obj_ty: &Type, method: &str, _args: &[Expr]) -> Result<Type, String> {
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
            Type::Struct(name, _) => {
                // 先查 struct_fields
                if let Some(fields) = self.struct_fields.get(name.as_str()) {
                    return Ok(fields.get(field).cloned().unwrap_or(Type::Int32));
                }
                // 再查 class_fields
                if let Some(fields) = self.class_fields.get(name.as_str()) {
                    return Ok(fields.get(field).cloned().unwrap_or(Type::Int32));
                }
                Ok(Type::Int32)
            }
            _ => Ok(Type::Int32),
        }
    }

    /// 收集函数中的局部变量类型
    pub fn collect_locals_from_function(&mut self, func: &Function) {
        // 添加参数
        for param in &func.params {
            self.add_local(param.name.clone(), param.ty.clone());
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
                self.collect_pattern_bindings(pattern, &var_ty);
            }
            // 递归处理嵌套语句
            Stmt::While { cond: _, body } | Stmt::Loop { body } => {
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::For { var, body, .. } => {
                // For 循环变量：类型保守推断为 I32（迭代器元素常为对象或整数）
                self.add_local(var.clone(), Type::Int32);
                for s in body {
                    self.collect_locals_from_stmt(s);
                }
            }
            Stmt::WhileLet { pattern, body, .. } => {
                self.collect_pattern_bindings(pattern, &Type::Int32);
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
    fn collect_pattern_bindings(&mut self, pattern: &Pattern, ty: &Type) {
        match pattern {
            Pattern::Binding(name) => {
                self.add_local(name.clone(), ty.clone());
            }
            Pattern::Tuple(pats) => {
                for p in pats {
                    self.collect_pattern_bindings(p, ty);
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
            Expr::If { then_branch, else_branch, .. } => {
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

    #[test]
    fn test_infer_literal() {
        let ctx = TypeInferenceContext::new();

        assert_eq!(ctx.infer_expr(&Expr::Integer(42)).unwrap(), Type::Int64);
        assert_eq!(ctx.infer_expr(&Expr::Bool(true)).unwrap(), Type::Bool);
        assert_eq!(ctx.infer_expr(&Expr::String("hello".to_string())).unwrap(), Type::String);
    }

    #[test]
    fn test_infer_binary() {
        let ctx = TypeInferenceContext::new();

        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };

        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Int64);
    }

    #[test]
    fn test_infer_comparison() {
        let ctx = TypeInferenceContext::new();

        let expr = Expr::Binary {
            op: BinOp::Lt,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };

        assert_eq!(ctx.infer_expr(&expr).unwrap(), Type::Bool);
    }
}
