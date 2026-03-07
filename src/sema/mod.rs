//! 语义分析模块 (P3/P4)
//!
//! 在代码生成前对 AST 做轻量级预分析：
//! - 推断无显式返回类型标注的函数/方法返回类型
//! - 构建全局符号类型表，提供给 CodeGen 使用
//!
//! 设计目标：零 panic、无 I/O、纯函数，与 CodeGen 解耦。

use crate::ast::{Expr, Program, Stmt, Type};
use std::collections::HashMap;

/// 语义分析上下文：保存预分析推断出的类型信息
#[derive(Debug, Default)]
pub struct SemanticContext {
    /// 函数名 → 推断的返回类型（仅限无显式标注的函数）
    /// key 格式与 `func_return_types` 一致（函数全限定名，含 ClassName.methodName）
    pub inferred_return_types: HashMap<String, Type>,
}

/// 对整个 Program 做语义预分析，返回增强的类型上下文
pub fn analyze(program: &Program) -> SemanticContext {
    let mut ctx = SemanticContext::default();

    // 第一轮：收集所有已有返回类型标注的符号作为已知类型表
    let mut known: HashMap<String, Type> = HashMap::new();

    for func in &program.functions {
        if let Some(ref ret) = func.return_type {
            if *ret != Type::Unit && *ret != Type::Nothing {
                known.insert(func.name.clone(), ret.clone());
            }
        }
    }
    for cls in &program.classes {
        for m in &cls.methods {
            if let Some(ref ret) = m.func.return_type {
                if *ret != Type::Unit && *ret != Type::Nothing {
                    // m.func.name 已由解析器限定为 "ClassName.methodName"
                    known.insert(m.func.name.clone(), ret.clone());
                }
            }
        }
    }

    // 第二轮（多次迭代，传播类型信息）：推断无标注顶层函数的返回类型
    for _pass in 0..3 {
        let mut changed = false;

        for func in &program.functions {
            if func.return_type.is_some()
                || func.extern_import.is_some()
                || !func.type_params.is_empty()
                || func.body.is_empty()
                || known.contains_key(&func.name)
            {
                continue;
            }
            if let Some(inferred) = infer_return_from_body(&func.body, &known) {
                known.insert(func.name.clone(), inferred.clone());
                ctx.inferred_return_types.insert(func.name.clone(), inferred);
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    // P4.5: 第三轮 — 推断无标注 class method 的返回类型
    // class method 已由解析器将 func.name 限定为 "ClassName.methodName"
    for _pass in 0..3 {
        let mut changed = false;

        for cls in &program.classes {
            if !cls.type_params.is_empty() {
                continue; // 跳过泛型类（单态化前无法推断）
            }
            for m in &cls.methods {
                if m.func.return_type.is_some()
                    || m.func.extern_import.is_some()
                    || !m.func.type_params.is_empty()
                    || m.func.body.is_empty()
                    || known.contains_key(&m.func.name)
                {
                    continue;
                }
                if let Some(inferred) = infer_return_from_body(&m.func.body, &known) {
                    known.insert(m.func.name.clone(), inferred.clone());
                    ctx.inferred_return_types.insert(m.func.name.clone(), inferred);
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    ctx
}

// ─── 内部辅助函数 ────────────────────────────────────────────────────────────

/// 从函数体推断返回类型
fn infer_return_from_body(body: &[Stmt], known: &HashMap<String, Type>) -> Option<Type> {
    // 先扫描显式 return 语句
    for stmt in body {
        if let Some(ty) = scan_stmt_for_return(stmt, known) {
            return Some(ty);
        }
    }
    // 再检查末尾表达式语句
    if let Some(Stmt::Expr(last_expr)) = body.last() {
        return infer_expr(last_expr, known);
    }
    None
}

/// 递归扫描语句，寻找 return 语句的类型
fn scan_stmt_for_return(stmt: &Stmt, known: &HashMap<String, Type>) -> Option<Type> {
    match stmt {
        Stmt::Return(Some(expr)) => infer_expr(expr, known),
        Stmt::Expr(expr) => {
            // 只有 if/match/block 才可能是末尾值；其他表达式不作为 return 源
            match expr {
                Expr::If { then_branch, .. } => infer_expr(then_branch, known),
                Expr::Match { arms, .. } => {
                    for arm in arms {
                        if let Some(ty) = infer_expr(&arm.body, known) {
                            return Some(ty);
                        }
                    }
                    None
                }
                Expr::Block(stmts, trailing) => {
                    if let Some(tr) = trailing {
                        return infer_expr(tr, known);
                    }
                    for s in stmts.iter().rev() {
                        if let Some(ty) = scan_stmt_for_return(s, known) {
                            return Some(ty);
                        }
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 从表达式推断 AST 类型（不依赖 CodeGen 状态，纯静态推断）
pub fn infer_expr(expr: &Expr, known: &HashMap<String, Type>) -> Option<Type> {
    match expr {
        // ── 字面量 ──────────────────────────────────────────────────────────
        Expr::Integer(_) => Some(Type::Int64),
        Expr::Float(_) => Some(Type::Float64),
        Expr::Float32(_) => Some(Type::Float32),
        Expr::Bool(_) => Some(Type::Bool),
        Expr::Rune(_) => Some(Type::Rune),
        Expr::String(_) | Expr::Interpolate(_) => Some(Type::String),

        // ── 类型转换 ─────────────────────────────────────────────────────────
        Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
        Expr::IsType { .. } => Some(Type::Bool),

        // ── Option/Result 构造 ────────────────────────────────────────────────
        Expr::Some(inner) => infer_expr(inner, known).map(|t| Type::Option(Box::new(t))),
        Expr::Ok(inner) => infer_expr(inner, known)
            .map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
        Expr::Err(_) => Some(Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
        Expr::None => None,

        // ── 构造函数 / 结构体 ─────────────────────────────────────────────────
        Expr::StructInit { name, type_args, .. } => {
            let ta = type_args.clone().unwrap_or_default();
            Some(Type::Struct(name.clone(), ta))
        }
        Expr::ConstructorCall { name, type_args, .. } => {
            let ta = type_args.clone().unwrap_or_default();
            Some(Type::Struct(name.clone(), ta))
        }

        // ── 函数调用 ──────────────────────────────────────────────────────────
        Expr::Call { name, type_args, .. } => {
            // 优先查已知符号表
            if let Some(ty) = known.get(name.as_str()) {
                return Some(ty.clone());
            }
            // 大写开头名字 → 构造函数，返回 Struct 类型
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                let ta = type_args.clone().unwrap_or_default();
                return Some(Type::Struct(name.clone(), ta));
            }
            None
        }

        // ── 变量引用 ──────────────────────────────────────────────────────────
        Expr::Var(name) => known.get(name.as_str()).cloned(),

        // ── 二元运算 ──────────────────────────────────────────────────────────
        Expr::Binary { op, left, right } => {
            use crate::ast::BinOp;
            match op {
                BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::LogicalAnd
                | BinOp::LogicalOr
                | BinOp::NotIn => Some(Type::Bool),
                BinOp::Add => {
                    let lt = infer_expr(left, known);
                    let rt = infer_expr(right, known);
                    if lt == Some(Type::String) || rt == Some(Type::String) {
                        Some(Type::String)
                    } else {
                        lt.or(rt)
                    }
                }
                _ => infer_expr(left, known).or_else(|| infer_expr(right, known)),
            }
        }

        // ── 一元运算 ──────────────────────────────────────────────────────────
        Expr::Unary { op, expr } => {
            use crate::ast::UnaryOp;
            match op {
                UnaryOp::Not => Some(Type::Bool),
                _ => infer_expr(expr, known),
            }
        }

        // ── 控制流表达式 ──────────────────────────────────────────────────────
        Expr::If { then_branch, .. } => infer_expr(then_branch, known),
        Expr::IfLet { then_branch, .. } => infer_expr(then_branch, known),
        Expr::Block(stmts, trailing) => {
            if let Some(tr) = trailing {
                return infer_expr(tr, known);
            }
            for stmt in stmts.iter().rev() {
                if let Stmt::Expr(e) = stmt {
                    if let Some(ty) = infer_expr(e, known) {
                        return Some(ty);
                    }
                }
                if let Some(ty) = scan_stmt_for_return(stmt, known) {
                    return Some(ty);
                }
            }
            None
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                if let Some(ty) = infer_expr(&arm.body, known) {
                    return Some(ty);
                }
            }
            None
        }

        // ── 元组 ──────────────────────────────────────────────────────────────
        Expr::Tuple(elems) => {
            let types: Vec<Type> = elems
                .iter()
                .map(|e| infer_expr(e, known).unwrap_or(Type::Int64))
                .collect();
            Some(Type::Tuple(types))
        }

        // ── Lambda ────────────────────────────────────────────────────────────
        Expr::Lambda {
            params,
            return_type,
            ..
        } => {
            let param_types: Vec<Type> = params.iter().map(|(_, t)| t.clone()).collect();
            let ret = return_type.clone();
            Some(Type::Function {
                params: param_types,
                ret: Box::new(ret),
            })
        }

        // ── Try / NullCoalesce ───────────────────────────────────────────────
        Expr::Try(inner) => infer_expr(inner, known),
        Expr::NullCoalesce { default, .. } => infer_expr(default, known),

        // ── 数组 ──────────────────────────────────────────────────────────────
        Expr::Array(elems) => {
            let elem_ty = elems
                .first()
                .and_then(|e| infer_expr(e, known))
                .unwrap_or(Type::Int64);
            Some(Type::Array(Box::new(elem_ty)))
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        BinOp, ClassDef, ClassMethod, Expr, Function, InterpolatePart, MatchArm, Param, Pattern,
        Program, Stmt, Type, UnaryOp, Visibility,
    };
    use std::collections::HashMap;

    fn make_param(name: &str, ty: Type) -> Param {
        Param {
            name: name.to_string(),
            ty,
            default: None,
            variadic: false,
            is_named: false,
            is_inout: false,
        }
    }

    // ─── infer_expr: literals ─────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_integer() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Integer(42), &known),
            Some(Type::Int64)
        );
    }

    #[test]
    fn test_infer_expr_float() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Float(3.14), &known),
            Some(Type::Float64)
        );
    }

    #[test]
    fn test_infer_expr_float32() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::Float32(1.0f32), &known),
            Some(Type::Float32)
        );
    }

    #[test]
    fn test_infer_expr_bool() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Bool(true), &known), Some(Type::Bool));
    }

    #[test]
    fn test_infer_expr_rune() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Rune('x'), &known), Some(Type::Rune));
    }

    #[test]
    fn test_infer_expr_string() {
        let known = HashMap::new();
        assert_eq!(
            infer_expr(&Expr::String("hello".to_string()), &known),
            Some(Type::String)
        );
    }

    #[test]
    fn test_infer_expr_interpolate() {
        let known = HashMap::new();
        let expr = Expr::Interpolate(vec![InterpolatePart::Literal("hi".to_string())]);
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }

    // ─── infer_expr: Cast, IsType ────────────────────────────────────────────

    #[test]
    fn test_infer_expr_cast() {
        let known = HashMap::new();
        let expr = Expr::Cast {
            expr: Box::new(Expr::Integer(1)),
            target_ty: Type::Float64,
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Float64));
    }

    #[test]
    fn test_infer_expr_is_type() {
        let known = HashMap::new();
        let expr = Expr::IsType {
            expr: Box::new(Expr::Var("x".to_string())),
            target_ty: Type::String,
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }

    // ─── infer_expr: Some, Ok, Err, None ─────────────────────────────────────

    #[test]
    fn test_infer_expr_some() {
        let known = HashMap::new();
        let expr = Expr::Some(Box::new(Expr::Integer(1)));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Option(Box::new(Type::Int64)))
        );
    }

    #[test]
    fn test_infer_expr_ok() {
        let known = HashMap::new();
        let expr = Expr::Ok(Box::new(Expr::String("ok".to_string())));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Result(
                Box::new(Type::String),
                Box::new(Type::String),
            ))
        );
    }

    #[test]
    fn test_infer_expr_err() {
        let known = HashMap::new();
        let expr = Expr::Err(Box::new(Expr::String("err".to_string())));
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Result(
                Box::new(Type::Int64),
                Box::new(Type::String),
            ))
        );
    }

    #[test]
    fn test_infer_expr_none() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::None, &known), None);
    }

    // ─── infer_expr: StructInit, ConstructorCall ───────────────────────────────

    #[test]
    fn test_infer_expr_struct_init() {
        let known = HashMap::new();
        let expr = Expr::StructInit {
            name: "Point".to_string(),
            type_args: Some(vec![Type::Int64, Type::Int64]),
            fields: vec![
                ("x".to_string(), Expr::Integer(1)),
                ("y".to_string(), Expr::Integer(2)),
            ],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct(
                "Point".to_string(),
                vec![Type::Int64, Type::Int64],
            ))
        );
    }

    #[test]
    fn test_infer_expr_constructor_call() {
        let known = HashMap::new();
        let expr = Expr::ConstructorCall {
            name: "Pair".to_string(),
            type_args: Some(vec![Type::Int64, Type::String]),
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct(
                "Pair".to_string(),
                vec![Type::Int64, Type::String],
            ))
        );
    }

    // ─── infer_expr: Call ─────────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_call_known_function() {
        let mut known = HashMap::new();
        known.insert("foo".to_string(), Type::Int64);
        let expr = Expr::Call {
            name: "foo".to_string(),
            type_args: None,
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    #[test]
    fn test_infer_expr_call_uppercase_constructor() {
        let known = HashMap::new();
        let expr = Expr::Call {
            name: "Point".to_string(),
            type_args: Some(vec![Type::Int64]),
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Struct("Point".to_string(), vec![Type::Int64]))
        );
    }

    #[test]
    fn test_infer_expr_call_unknown() {
        let known = HashMap::new();
        let expr = Expr::Call {
            name: "unknownFunc".to_string(),
            type_args: None,
            args: vec![],
            named_args: vec![],
        };
        assert_eq!(infer_expr(&expr, &known), None);
    }

    // ─── infer_expr: Var ──────────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_var_known() {
        let mut known = HashMap::new();
        known.insert("x".to_string(), Type::String);
        assert_eq!(
            infer_expr(&Expr::Var("x".to_string()), &known),
            Some(Type::String)
        );
    }

    #[test]
    fn test_infer_expr_var_unknown() {
        let known = HashMap::new();
        assert_eq!(infer_expr(&Expr::Var("y".to_string()), &known), None);
    }

    // ─── infer_expr: Binary ──────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_binary_comparison() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Eq,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }

    #[test]
    fn test_infer_expr_binary_add_string() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::String("a".to_string())),
            right: Box::new(Expr::String("b".to_string())),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }

    #[test]
    fn test_infer_expr_binary_add_integers() {
        let known = HashMap::new();
        let expr = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    // ─── infer_expr: Unary ───────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_unary_not() {
        let known = HashMap::new();
        let expr = Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(Expr::Bool(true)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Bool));
    }

    #[test]
    fn test_infer_expr_unary_neg() {
        let known = HashMap::new();
        let expr = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(Expr::Integer(1)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    // ─── infer_expr: If, Block, Match ──────────────────────────────────────────

    #[test]
    fn test_infer_expr_if() {
        let known = HashMap::new();
        let expr = Expr::If {
            cond: Box::new(Expr::Bool(true)),
            then_branch: Box::new(Expr::Integer(1)),
            else_branch: Some(Box::new(Expr::Integer(2))),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    #[test]
    fn test_infer_expr_block_with_trailing() {
        let known = HashMap::new();
        let expr = Expr::Block(
            vec![Stmt::Expr(Expr::Integer(0))],
            Some(Box::new(Expr::String("result".to_string()))),
        );
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }

    #[test]
    fn test_infer_expr_match() {
        let known = HashMap::new();
        let expr = Expr::Match {
            expr: Box::new(Expr::Integer(1)),
            arms: vec![MatchArm {
                pattern: Pattern::Wildcard,
                guard: None,
                body: Box::new(Expr::String("matched".to_string())),
            }],
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::String));
    }

    // ─── infer_expr: Tuple, Array ──────────────────────────────────────────────

    #[test]
    fn test_infer_expr_tuple() {
        let known = HashMap::new();
        let expr = Expr::Tuple(vec![
            Expr::Integer(1),
            Expr::String("two".to_string()),
        ]);
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Tuple(vec![Type::Int64, Type::String]))
        );
    }

    #[test]
    fn test_infer_expr_array() {
        let known = HashMap::new();
        let expr = Expr::Array(vec![
            Expr::Integer(1),
            Expr::Integer(2),
        ]);
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Array(Box::new(Type::Int64)))
        );
    }

    // ─── infer_expr: Lambda ───────────────────────────────────────────────────

    #[test]
    fn test_infer_expr_lambda() {
        let known = HashMap::new();
        let expr = Expr::Lambda {
            params: vec![("x".to_string(), Type::Int64)],
            return_type: Some(Type::Int64),
            body: Box::new(Expr::Var("x".to_string())),
        };
        assert_eq!(
            infer_expr(&expr, &known),
            Some(Type::Function {
                params: vec![Type::Int64],
                ret: Box::new(Some(Type::Int64)),
            })
        );
    }

    // ─── infer_expr: Try, NullCoalesce ─────────────────────────────────────────

    #[test]
    fn test_infer_expr_try() {
        let known = HashMap::new();
        let expr = Expr::Try(Box::new(Expr::Integer(42)));
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    #[test]
    fn test_infer_expr_null_coalesce() {
        let known = HashMap::new();
        let expr = Expr::NullCoalesce {
            option: Box::new(Expr::None),
            default: Box::new(Expr::Integer(0)),
        };
        assert_eq!(infer_expr(&expr, &known), Some(Type::Int64));
    }

    // ─── analyze ──────────────────────────────────────────────────────────────

    #[test]
    fn test_analyze_simple_program_with_return_types() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![
                Function {
                    visibility: Visibility::default(),
                    name: "annotated".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![make_param("x", Type::Int64)],
                    return_type: Some(Type::Int64),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
                Function {
                    visibility: Visibility::default(),
                    name: "inferred".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![],
                    return_type: None,
                    throws: None,
                    body: vec![Stmt::Expr(Expr::Integer(42))],
                    extern_import: None,
                },
            ],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert!(ctx.inferred_return_types.contains_key("inferred"));
        assert_eq!(
            ctx.inferred_return_types.get("inferred"),
            Some(&Type::Int64)
        );
    }

    #[test]
    fn test_analyze_function_with_return_stmt() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "with_return".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: None,
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("hello".to_string())))],
                extern_import: None,
            }],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert_eq!(
            ctx.inferred_return_types.get("with_return"),
            Some(&Type::String)
        );
    }

    #[test]
    fn test_analyze_class_method_inference() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![ClassDef {
                visibility: Visibility::default(),
                name: "Foo".to_string(),
                type_params: vec![],
                constraints: vec![],
                is_abstract: false,
                is_sealed: false,
                is_open: false,
                extends: None,
                implements: vec![],
                fields: vec![],
                init: None,
                deinit: None,
                static_init: None,
                primary_ctor_params: vec![],
                methods: vec![ClassMethod {
                    override_: false,
                    func: Function {
                        visibility: Visibility::default(),
                        name: "Foo.getVal".to_string(),
                        type_params: vec![],
                        constraints: vec![],
                        params: vec![],
                        return_type: None,
                        throws: None,
                        body: vec![Stmt::Expr(Expr::Bool(true))],
                        extern_import: None,
                    },
                }],
            }],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        let ctx = analyze(&program);
        assert!(ctx.inferred_return_types.contains_key("Foo.getVal"));
        assert_eq!(
            ctx.inferred_return_types.get("Foo.getVal"),
            Some(&Type::Bool)
        );
    }
}
