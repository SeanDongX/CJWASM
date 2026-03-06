//! 类型推断器 - 遍历 AST 推断表达式类型

use crate::ast::{Expr, Stmt, Type, Function, Program, BinOp, UnaryOp, Pattern};
use std::collections::HashMap;

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
            current_return_ty: None,
            globals: HashMap::new(),
        }
    }

    /// 从程序构建上下文
    pub fn from_program(program: &Program) -> Self {
        let mut ctx = Self::new();

        // 收集函数签名
        for func in &program.functions {
            let sig = FunctionSignature {
                name: func.name.clone(),
                params: func.params.iter().map(|p| p.ty.clone()).collect(),
                return_ty: func.return_type.clone().unwrap_or(Type::Unit),
            };
            ctx.functions.insert(func.name.clone(), sig);
        }

        // 收集结构体字段
        for struct_def in &program.structs {
            let mut fields = HashMap::new();
            for field in &struct_def.fields {
                fields.insert(field.name.clone(), field.ty.clone());
            }
            ctx.struct_fields.insert(struct_def.name.clone(), fields);
        }

        // 收集类字段
        for class_def in &program.classes {
            let mut fields = HashMap::new();
            for field in &class_def.fields {
                fields.insert(field.name.clone(), field.ty.clone());
            }
            ctx.class_fields.insert(class_def.name.clone(), fields);
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
                Err(format!("变量未定义: {}", name))
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
                if let Some(sig) = self.functions.get(name) {
                    Ok(sig.return_ty.clone())
                } else {
                    // 内置函数
                    match name.as_str() {
                        "println" | "print" | "eprintln" | "eprint" => Ok(Type::Unit),
                        "readln" => Ok(Type::String),
                        "exit" => Ok(Type::Nothing),
                        "abs" | "min" | "max" if !args.is_empty() => {
                            self.infer_expr(&args[0])
                        }
                        // 类型转换构造函数
                        "Float32" => Ok(Type::Float32),
                        "Float64" => Ok(Type::Float64),
                        "Int32" | "UInt32" => Ok(Type::Int32),
                        "Int64" | "UInt64" => Ok(Type::Int64),
                        _ => Ok(Type::Int64), // 默认返回 Int64
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

            // 其他表达式
            _ => Ok(Type::Int64), // 默认类型
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
    fn infer_method_return(&self, _obj_ty: &Type, method: &str, _args: &[Expr]) -> Result<Type, String> {
        // 简化实现：根据方法名推断
        match method {
            "toString" => Ok(Type::String),
            "size" | "length" => Ok(Type::Int64),
            "get" => Ok(Type::Int64), // 简化
            "append" | "remove" | "clear" => Ok(Type::Unit),
            _ => Ok(Type::Int64),
        }
    }

    /// 推断字段类型
    pub fn infer_field_type(&self, obj_ty: &Type, field: &str) -> Result<Type, String> {
        match obj_ty {
            Type::Struct(name, _) => {
                if let Some(fields) = self.struct_fields.get(name) {
                    fields.get(field)
                        .cloned()
                        .ok_or_else(|| format!("字段未找到: {}.{}", name, field))
                } else {
                    Err(format!("结构体未定义: {}", name))
                }
            }
            _ => Ok(Type::Int64), // 默认类型
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
                if let Pattern::Binding(name) = pattern {
                    let var_ty = if let Some(t) = ty {
                        t.clone()
                    } else if let Ok(inferred) = self.infer_expr(value) {
                        inferred
                    } else {
                        Type::Int64 // 默认类型
                    };
                    self.add_local(name.clone(), var_ty);
                }
            }
            // 递归处理嵌套语句
            Stmt::While { body, .. } | Stmt::Loop { body } => {
                for s in body {
                    self.collect_locals_from_stmt(s);
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
