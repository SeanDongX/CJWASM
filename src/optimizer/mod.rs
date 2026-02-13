//! 优化器：常量折叠等 AST 级优化。

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};
use std::ops::{Add, Div, Mul, Rem, Shl, Shr, Sub};

/// 对程序做一次常量折叠（及后续可扩展的优化）。
pub fn optimize_program(program: &mut crate::ast::Program) {
    for func in &mut program.functions {
        optimize_function(func);
    }
    for class in &mut program.classes {
        for m in &mut class.methods {
            optimize_function(&mut m.func);
        }
        if let Some(ref mut init) = class.init {
            for stmt in &mut init.body {
                fold_stmt(stmt);
            }
        }
        if let Some(ref mut deinit) = class.deinit {
            for stmt in deinit {
                fold_stmt(stmt);
            }
        }
    }
}

fn optimize_function(func: &mut crate::ast::Function) {
    for stmt in &mut func.body {
        fold_stmt(stmt);
    }
    for param in &mut func.params {
        if let Some(ref mut default) = param.default {
            *default = fold_expr(default.clone());
        }
    }
}

fn fold_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Let { value, .. } => *value = fold_expr(value.clone()),
        Stmt::Var { value, .. } => *value = fold_expr(value.clone()),
        Stmt::Assign { value, .. } => *value = fold_expr(value.clone()),
        Stmt::Expr(e) => *e = fold_expr(e.clone()),
        Stmt::Return(Some(e)) => *e = fold_expr(e.clone()),
        Stmt::While { cond, body } => {
            *cond = fold_expr(cond.clone());
            for s in body {
                fold_stmt(s);
            }
        }
        Stmt::WhileLet { expr, body, .. } => {
            *expr = Box::new(fold_expr((**expr).clone()));
            for s in body {
                fold_stmt(s);
            }
        }
        Stmt::For { iterable, body, .. } => {
            *iterable = fold_expr(iterable.clone());
            for s in body {
                fold_stmt(s);
            }
        }
        Stmt::Loop { body } => {
            for s in body {
                fold_stmt(s);
            }
        }
        _ => {}
    }
}

fn fold_expr(expr: Expr) -> Expr {
    use crate::ast::Expr::*;
    match expr {
        Binary { op, left, right } => {
            let left = fold_expr(*left);
            let right = fold_expr(*right);
            match (&left, &right) {
                (Integer(a), Integer(b)) => fold_binary_int(*a, *b, &op).unwrap_or(Binary {
                    op: op.clone(),
                    left: Box::new(left),
                    right: Box::new(right),
                }),
                (Float(x), Float(y)) => fold_binary_float(*x, *y, &op).unwrap_or(Binary {
                    op: op.clone(),
                    left: Box::new(left),
                    right: Box::new(right),
                }),
                _ => Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            }
        }
        Unary { op, expr } => {
            let inner = fold_expr(*expr);
            match (&op, &inner) {
                (UnaryOp::Neg, Integer(n)) => Integer(n.saturating_neg()),
                (UnaryOp::Not, Bool(b)) => Bool(!b),
                _ => Unary {
                    op,
                    expr: Box::new(inner),
                },
            }
        }
        If {
            cond,
            then_branch,
            else_branch,
        } => If {
            cond: Box::new(fold_expr(*cond)),
            then_branch: Box::new(fold_expr(*then_branch)),
            else_branch: else_branch.map(|e| Box::new(fold_expr(*e))),
        },
        IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => IfLet {
            pattern,
            expr: Box::new(fold_expr(*expr)),
            then_branch: Box::new(fold_expr(*then_branch)),
            else_branch: else_branch.map(|e| Box::new(fold_expr(*e))),
        },
        Block(stmts, tail) => {
            let stmts: Vec<Stmt> = stmts
                .into_iter()
                .map(|mut s| {
                    fold_stmt(&mut s);
                    s
                })
                .collect();
            let tail = tail.map(|e| Box::new(fold_expr(*e)));
            Block(stmts, tail)
        }
        Call { name, type_args, args } => Call {
            name,
            type_args,
            args: args.into_iter().map(fold_expr).collect(),
        },
        MethodCall {
            object,
            method,
            args,
        } => MethodCall {
            object: Box::new(fold_expr(*object)),
            method,
            args: args.into_iter().map(fold_expr).collect(),
        },
        Array(elems) => Array(elems.into_iter().map(fold_expr).collect()),
        Tuple(elems) => Tuple(elems.into_iter().map(fold_expr).collect()),
        TupleIndex { object, index } => TupleIndex {
            object: Box::new(fold_expr(*object)),
            index,
        },
        NullCoalesce { option, default } => NullCoalesce {
            option: Box::new(fold_expr(*option)),
            default: Box::new(fold_expr(*default)),
        },
        Index { array, index } => Index {
            array: Box::new(fold_expr(*array)),
            index: Box::new(fold_expr(*index)),
        },
        Field { object, field } => Field {
            object: Box::new(fold_expr(*object)),
            field,
        },
        StructInit { name, type_args, fields } => StructInit {
            name,
            type_args,
            fields: fields
                .into_iter()
                .map(|(n, e)| (n, fold_expr(e)))
                .collect(),
        },
        Match { expr, arms } => Match {
            expr: Box::new(fold_expr(*expr)),
            arms: arms
                .into_iter()
                .map(|arm| crate::ast::MatchArm {
                    guard: arm.guard.map(|g| Box::new(fold_expr(*g))),
                    body: Box::new(fold_expr(*arm.body)),
                    ..arm
                })
                .collect(),
        },
        Range { start, end, inclusive } => Range {
            start: Box::new(fold_expr(*start)),
            end: Box::new(fold_expr(*end)),
            inclusive,
        },
        VariantConst { enum_name, variant_name, arg } => VariantConst {
            enum_name,
            variant_name,
            arg: arg.map(|e| Box::new(fold_expr(*e))),
        },
        Some(inner) => Some(Box::new(fold_expr(*inner))),
        Ok(inner) => Ok(Box::new(fold_expr(*inner))),
        Err(inner) => Err(Box::new(fold_expr(*inner))),
        Try(inner) => Try(Box::new(fold_expr(*inner))),
        Throw(inner) => Throw(Box::new(fold_expr(*inner))),
        TryBlock {
            body,
            catch_var,
            catch_body,
        } => {
            let body: Vec<Stmt> = body
                .into_iter()
                .map(|mut s| {
                    fold_stmt(&mut s);
                    s
                })
                .collect();
            let catch_body: Vec<Stmt> = catch_body
                .into_iter()
                .map(|mut s| {
                    fold_stmt(&mut s);
                    s
                })
                .collect();
            TryBlock {
                body,
                catch_var,
                catch_body,
            }
        }
        Cast { expr, target_ty } => Cast {
            expr: Box::new(fold_expr(*expr)),
            target_ty,
        },
        Interpolate(parts) => Interpolate(
            parts
                .into_iter()
                .map(|p| match p {
                    crate::ast::InterpolatePart::Literal(s) => crate::ast::InterpolatePart::Literal(s),
                    crate::ast::InterpolatePart::Expr(e) => {
                        crate::ast::InterpolatePart::Expr(Box::new(fold_expr(*e)))
                    }
                })
                .collect(),
        ),
        e => e,
    }
}

fn fold_binary_int(a: i64, b: i64, op: &BinOp) -> Option<Expr> {
    use crate::ast::Expr::{Bool, Integer};
    use BinOp::*;
    let out = match op {
        Add => Integer(a.add(b)),
        Sub => Integer(a.sub(b)),
        Mul => Integer(a.mul(b)),
        Div => {
            if b == 0 {
                return None;
            }
            Integer(a.div(b))
        }
        Mod => {
            if b == 0 {
                return None;
            }
            Integer(a.rem(b))
        }
        Eq => Bool(a == b),
        NotEq => Bool(a != b),
        Lt => Bool(a < b),
        Gt => Bool(a > b),
        LtEq => Bool(a <= b),
        GtEq => Bool(a >= b),
        BitAnd => Integer(a & b),
        BitOr => Integer(a | b),
        BitXor => Integer(a ^ b),
        Shl => Integer(a.shl(b as u32)),
        Shr => Integer(a.shr(b as u32)),
        UShr => Integer(((a as u64).shr(b as u32)) as i64),
        LogicalAnd | LogicalOr | Pow => return None,
    };
    Some(out)
}

fn fold_binary_float(x: f64, y: f64, op: &BinOp) -> Option<Expr> {
    use crate::ast::Expr::{Bool, Float};
    use BinOp::*;
    let out = match op {
        Add => Float(x + y),
        Sub => Float(x - y),
        Mul => Float(x * y),
        Div => Float(x / y),
        Eq => Bool(x == y),
        NotEq => Bool(x != y),
        Lt => Bool(x < y),
        Gt => Bool(x > y),
        LtEq => Bool(x <= y),
        GtEq => Bool(x >= y),
        Mod | BitAnd | BitOr | BitXor | Shl | Shr | UShr | LogicalAnd | LogicalOr | Pow => return None,
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, Expr};

    #[test]
    fn fold_int_add() {
        let e = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Integer(2)),
            right: Box::new(Expr::Integer(3)),
        };
        let got = fold_expr(e);
        assert!(matches!(got, Expr::Integer(5)));
    }

    #[test]
    fn fold_nested() {
        let e = Expr::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::Binary {
                op: BinOp::Mul,
                left: Box::new(Expr::Integer(2)),
                right: Box::new(Expr::Integer(3)),
            }),
            right: Box::new(Expr::Integer(4)),
        };
        let got = fold_expr(e);
        assert!(matches!(got, Expr::Integer(10)));
    }
}
