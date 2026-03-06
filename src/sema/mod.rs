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
