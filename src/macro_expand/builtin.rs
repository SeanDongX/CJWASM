//! 内建宏：编译器内置的宏展开实现。
//!
//! 这些宏不需要 wasmtime 执行，直接在 Rust 层面生成 AST。
//! 当 macro-system feature 未启用时，所有宏都走此路径。

use crate::ast::{BinOp, Expr, Stmt};

/// 尝试展开内建宏，返回 Some(stmts) 表示成功
pub fn try_expand_builtin(name: &str, args: &[Expr]) -> Option<Vec<Stmt>> {
    match name {
        "Log" => Some(expand_log(args)),
        "Debug" => Some(expand_debug(args)),
        "Stringify" => Some(expand_stringify(args)),
        "Todo" => Some(expand_todo(args)),
        _ => None,
    }
}

/// @Log[msg] → println("[LOG] " + msg)
fn expand_log(args: &[Expr]) -> Vec<Stmt> {
    let msg = args.first().cloned().unwrap_or(Expr::String("".to_string()));
    vec![Stmt::Expr(Expr::Call {
        name: "println".to_string(),
        type_args: None,
        args: vec![Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::String("[LOG] ".to_string())),
            right: Box::new(msg),
        }],
        named_args: vec![],
    })]
}

/// @Debug[expr] → println("DEBUG: " + toString(expr))
fn expand_debug(args: &[Expr]) -> Vec<Stmt> {
    let expr = args.first().cloned().unwrap_or(Expr::String("".to_string()));
    let expr_str = format!("{:?}", expr);
    vec![Stmt::Expr(Expr::Call {
        name: "println".to_string(),
        type_args: None,
        args: vec![Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::String(format!("DEBUG {}: ", expr_str))),
            right: Box::new(Expr::Call {
                name: "toString".to_string(),
                type_args: None,
                args: vec![expr],
                named_args: vec![],
            }),
        }],
        named_args: vec![],
    })]
}

/// @Stringify[expr] → 将表达式转为字符串字面量
fn expand_stringify(args: &[Expr]) -> Vec<Stmt> {
    let expr = args.first().cloned().unwrap_or(Expr::String("".to_string()));
    let s = format!("{:?}", expr);
    vec![Stmt::Expr(Expr::String(s))]
}

/// @Todo[msg] → println("TODO: " + msg); unreachable
fn expand_todo(args: &[Expr]) -> Vec<Stmt> {
    let msg = args.first().cloned().unwrap_or(Expr::String("not implemented".to_string()));
    vec![
        Stmt::Expr(Expr::Call {
            name: "println".to_string(),
            type_args: None,
            args: vec![Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::String("TODO: ".to_string())),
                right: Box::new(msg),
            }],
            named_args: vec![],
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_log() {
        let result = try_expand_builtin("Log", &[Expr::String("hello".to_string())]);
        assert!(result.is_some());
        let stmts = result.unwrap();
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_builtin_debug() {
        let result = try_expand_builtin("Debug", &[Expr::Integer(42)]);
        assert!(result.is_some());
    }

    #[test]
    fn test_builtin_unknown() {
        let result = try_expand_builtin("Unknown", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_builtin_todo() {
        let result = try_expand_builtin("Todo", &[Expr::String("fix later".to_string())]);
        assert!(result.is_some());
    }

    #[test]
    fn test_builtin_stringify() {
        let result = try_expand_builtin("Stringify", &[Expr::Integer(42)]);
        assert!(result.is_some());
    }
}
