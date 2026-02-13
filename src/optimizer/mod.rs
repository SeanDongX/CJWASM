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
    // Pass 1: 常量折叠
    for stmt in &mut func.body {
        fold_stmt(stmt);
    }
    for param in &mut func.params {
        if let Some(ref mut default) = param.default {
            *default = fold_expr(default.clone());
        }
    }
    // Pass 2: 死代码消除 (return/break/continue 后的语句)
    eliminate_dead_code(&mut func.body);
    // Pass 3: 尾递归优化
    optimize_tail_recursion(func);
}

/// 死代码消除：移除 return/break/continue 后不可达的语句
fn eliminate_dead_code(stmts: &mut Vec<Stmt>) {
    // 找到第一个终止语句（return/break/continue）的位置
    let mut terminator_pos = None;
    for (i, stmt) in stmts.iter().enumerate() {
        match stmt {
            Stmt::Return(_) | Stmt::Break | Stmt::Continue => {
                terminator_pos = Some(i);
                break;
            }
            _ => {}
        }
    }
    // 截断终止语句之后的所有语句
    if let Some(pos) = terminator_pos {
        stmts.truncate(pos + 1);
    }
    // 递归处理嵌套块
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body } => {
                eliminate_dead_code(body);
            }
            Stmt::WhileLet { body, .. } => {
                eliminate_dead_code(body);
            }
            _ => {}
        }
    }
}

/// 尾递归优化：将尾递归函数转换为循环
/// 检测模式：func f(params) { ... return f(new_args) }
fn optimize_tail_recursion(func: &mut crate::ast::Function) {
    use crate::ast::AssignTarget;
    let func_name = func.name.clone();
    let param_names: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();

    if param_names.is_empty() || func.body.is_empty() {
        return;
    }

    // 检查最后一条语句是否为 return func_name(args)
    let is_tail_call = match func.body.last() {
        Some(Stmt::Return(Some(Expr::Call { name, args, .. }))) => {
            name == &func_name && args.len() == param_names.len()
        }
        Some(Stmt::Expr(Expr::Call { name, args, .. })) => {
            name == &func_name && args.len() == param_names.len()
        }
        _ => false,
    };

    if !is_tail_call {
        return;
    }

    // 提取尾调用的参数
    let tail_args = match func.body.pop() {
        Some(Stmt::Return(Some(Expr::Call { args, .. }))) => args,
        Some(Stmt::Expr(Expr::Call { args, .. })) => args,
        _ => return,
    };

    // 转换为循环：
    // loop {
    //     <original body without last statement>
    //     param1 = new_arg1
    //     param2 = new_arg2
    //     continue
    // }
    let mut loop_body = func.body.clone();

    // 添加参数重新赋值
    for (param_name, arg) in param_names.iter().zip(tail_args) {
        loop_body.push(Stmt::Assign {
            target: AssignTarget::Var(param_name.clone()),
            value: arg,
        });
    }
    loop_body.push(Stmt::Continue);

    // 替换整个函数体为 loop
    func.body = vec![Stmt::Loop { body: loop_body }];
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
            finally_body,
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
            let finally_body = finally_body.map(|stmts| {
                stmts
                    .into_iter()
                    .map(|mut s| {
                        fold_stmt(&mut s);
                        s
                    })
                    .collect()
            });
            TryBlock {
                body,
                catch_var,
                catch_body,
                finally_body,
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
        // Phase 9: 折叠切片和 Map 字面量中的子表达式
        SliceExpr { array, start, end } => SliceExpr {
            array: Box::new(fold_expr(*array)),
            start: Box::new(fold_expr(*start)),
            end: Box::new(fold_expr(*end)),
        },
        MapLiteral { entries } => MapLiteral {
            entries: entries
                .into_iter()
                .map(|(k, v)| (fold_expr(k), fold_expr(v)))
                .collect(),
        },
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

    // === 覆盖率补充：optimizer 单元测试 ===

    #[test]
    fn fold_int_sub() {
        let e = Expr::Binary { op: BinOp::Sub, left: Box::new(Expr::Integer(10)), right: Box::new(Expr::Integer(3)) };
        assert!(matches!(fold_expr(e), Expr::Integer(7)));
    }

    #[test]
    fn fold_int_div() {
        let e = Expr::Binary { op: BinOp::Div, left: Box::new(Expr::Integer(10)), right: Box::new(Expr::Integer(3)) };
        assert!(matches!(fold_expr(e), Expr::Integer(3)));
    }

    #[test]
    fn fold_int_div_zero() {
        let e = Expr::Binary { op: BinOp::Div, left: Box::new(Expr::Integer(10)), right: Box::new(Expr::Integer(0)) };
        // 除零不折叠，应保持为 Binary
        assert!(matches!(fold_expr(e), Expr::Binary { .. }));
    }

    #[test]
    fn fold_int_mod() {
        let e = Expr::Binary { op: BinOp::Mod, left: Box::new(Expr::Integer(10)), right: Box::new(Expr::Integer(3)) };
        assert!(matches!(fold_expr(e), Expr::Integer(1)));
    }

    #[test]
    fn fold_int_mod_zero() {
        let e = Expr::Binary { op: BinOp::Mod, left: Box::new(Expr::Integer(10)), right: Box::new(Expr::Integer(0)) };
        assert!(matches!(fold_expr(e), Expr::Binary { .. }));
    }

    #[test]
    fn fold_int_comparisons() {
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Eq, left: Box::new(Expr::Integer(1)), right: Box::new(Expr::Integer(1)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::NotEq, left: Box::new(Expr::Integer(1)), right: Box::new(Expr::Integer(2)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Lt, left: Box::new(Expr::Integer(1)), right: Box::new(Expr::Integer(2)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Gt, left: Box::new(Expr::Integer(2)), right: Box::new(Expr::Integer(1)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::LtEq, left: Box::new(Expr::Integer(1)), right: Box::new(Expr::Integer(1)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::GtEq, left: Box::new(Expr::Integer(2)), right: Box::new(Expr::Integer(2)) }), Expr::Bool(true)));
    }

    #[test]
    fn fold_int_bitwise() {
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::BitAnd, left: Box::new(Expr::Integer(0xFF)), right: Box::new(Expr::Integer(0x0F)) }), Expr::Integer(0x0F)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::BitOr, left: Box::new(Expr::Integer(0xF0)), right: Box::new(Expr::Integer(0x0F)) }), Expr::Integer(0xFF)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::BitXor, left: Box::new(Expr::Integer(0xFF)), right: Box::new(Expr::Integer(0x0F)) }), Expr::Integer(0xF0)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Shl, left: Box::new(Expr::Integer(1)), right: Box::new(Expr::Integer(4)) }), Expr::Integer(16)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Shr, left: Box::new(Expr::Integer(16)), right: Box::new(Expr::Integer(2)) }), Expr::Integer(4)));
    }

    #[test]
    fn fold_int_ushr() {
        let e = Expr::Binary { op: BinOp::UShr, left: Box::new(Expr::Integer(256)), right: Box::new(Expr::Integer(2)) };
        assert!(matches!(fold_expr(e), Expr::Integer(64)));
    }

    #[test]
    fn fold_int_logical_returns_none() {
        // LogicalAnd, LogicalOr, Pow 不折叠整数
        let e = Expr::Binary { op: BinOp::Pow, left: Box::new(Expr::Integer(2)), right: Box::new(Expr::Integer(3)) };
        assert!(matches!(fold_expr(e), Expr::Binary { .. }));
    }

    #[test]
    fn fold_float_add() {
        let e = Expr::Binary { op: BinOp::Add, left: Box::new(Expr::Float(1.5)), right: Box::new(Expr::Float(2.5)) };
        if let Expr::Float(v) = fold_expr(e) { assert!((v - 4.0).abs() < 0.001); } else { panic!("expected float"); }
    }

    #[test]
    fn fold_float_sub() {
        let e = Expr::Binary { op: BinOp::Sub, left: Box::new(Expr::Float(5.0)), right: Box::new(Expr::Float(2.0)) };
        if let Expr::Float(v) = fold_expr(e) { assert!((v - 3.0).abs() < 0.001); } else { panic!("expected float"); }
    }

    #[test]
    fn fold_float_mul() {
        let e = Expr::Binary { op: BinOp::Mul, left: Box::new(Expr::Float(3.0)), right: Box::new(Expr::Float(4.0)) };
        if let Expr::Float(v) = fold_expr(e) { assert!((v - 12.0).abs() < 0.001); } else { panic!("expected float"); }
    }

    #[test]
    fn fold_float_div() {
        let e = Expr::Binary { op: BinOp::Div, left: Box::new(Expr::Float(10.0)), right: Box::new(Expr::Float(4.0)) };
        if let Expr::Float(v) = fold_expr(e) { assert!((v - 2.5).abs() < 0.001); } else { panic!("expected float"); }
    }

    #[test]
    fn fold_float_comparisons() {
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Eq, left: Box::new(Expr::Float(1.0)), right: Box::new(Expr::Float(1.0)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::NotEq, left: Box::new(Expr::Float(1.0)), right: Box::new(Expr::Float(2.0)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Lt, left: Box::new(Expr::Float(1.0)), right: Box::new(Expr::Float(2.0)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Gt, left: Box::new(Expr::Float(2.0)), right: Box::new(Expr::Float(1.0)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::LtEq, left: Box::new(Expr::Float(1.0)), right: Box::new(Expr::Float(1.0)) }), Expr::Bool(true)));
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::GtEq, left: Box::new(Expr::Float(2.0)), right: Box::new(Expr::Float(2.0)) }), Expr::Bool(true)));
    }

    #[test]
    fn fold_float_unsupported_ops() {
        // Mod, BitAnd 等不支持浮点
        assert!(matches!(fold_expr(Expr::Binary { op: BinOp::Mod, left: Box::new(Expr::Float(1.0)), right: Box::new(Expr::Float(2.0)) }), Expr::Binary { .. }));
    }

    #[test]
    fn fold_unary_neg_int() {
        use crate::ast::UnaryOp;
        let e = Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::Integer(42)) };
        assert!(matches!(fold_expr(e), Expr::Integer(-42)));
    }

    #[test]
    fn fold_unary_neg_float_not_folded() {
        use crate::ast::UnaryOp;
        // Neg on Float is not folded by the optimizer (only Integer)
        let e = Expr::Unary { op: UnaryOp::Neg, expr: Box::new(Expr::Float(3.14)) };
        assert!(matches!(fold_expr(e), Expr::Unary { .. }));
    }

    #[test]
    fn fold_unary_not() {
        use crate::ast::UnaryOp;
        let e = Expr::Unary { op: UnaryOp::Not, expr: Box::new(Expr::Bool(true)) };
        assert!(matches!(fold_expr(e), Expr::Bool(false)));
        let e2 = Expr::Unary { op: UnaryOp::Not, expr: Box::new(Expr::Bool(false)) };
        assert!(matches!(fold_expr(e2), Expr::Bool(true)));
    }

    #[test]
    fn fold_unary_bitnot_not_folded() {
        use crate::ast::UnaryOp;
        // BitNot is not folded by the optimizer
        let e = Expr::Unary { op: UnaryOp::BitNot, expr: Box::new(Expr::Integer(0)) };
        assert!(matches!(fold_expr(e), Expr::Unary { .. }));
    }

    #[test]
    fn fold_if_recursive() {
        // If 应递归折叠子表达式
        let e = Expr::If {
            cond: Box::new(Expr::Binary {
                op: BinOp::Eq,
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(1)),
            }),
            then_branch: Box::new(Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::Integer(2)),
                right: Box::new(Expr::Integer(3)),
            }),
            else_branch: Some(Box::new(Expr::Integer(0))),
        };
        let folded = fold_expr(e);
        // cond should become Bool(true), then_branch should become Integer(5)
        if let Expr::If { cond, then_branch, .. } = folded {
            assert!(matches!(*cond, Expr::Bool(true)));
            assert!(matches!(*then_branch, Expr::Integer(5)));
        } else {
            panic!("Expected If");
        }
    }

    #[test]
    fn fold_if_let_recursive() {
        use crate::ast::Pattern;
        let e = Expr::IfLet {
            pattern: Pattern::Wildcard,
            expr: Box::new(Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(2)),
            }),
            then_branch: Box::new(Expr::Integer(10)),
            else_branch: Some(Box::new(Expr::Integer(0))),
        };
        let folded = fold_expr(e);
        if let Expr::IfLet { expr, .. } = folded {
            assert!(matches!(*expr, Expr::Integer(3)));
        } else {
            panic!("Expected IfLet");
        }
    }

    #[test]
    fn fold_block_recursive() {
        let e = Expr::Block(
            vec![],
            Some(Box::new(Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(2)),
            })),
        );
        let folded = fold_expr(e);
        if let Expr::Block(_, Some(tail)) = folded {
            assert!(matches!(*tail, Expr::Integer(3)));
        } else {
            panic!("Expected Block");
        }
    }

    #[test]
    fn fold_try_block_recursive() {
        use crate::ast::Stmt;
        let e = Expr::TryBlock {
            body: vec![Stmt::Expr(Expr::Binary {
                op: BinOp::Add,
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(2)),
            })],
            catch_var: None,
            catch_body: vec![],
            finally_body: Some(vec![Stmt::Expr(Expr::Binary {
                op: BinOp::Mul,
                left: Box::new(Expr::Integer(3)),
                right: Box::new(Expr::Integer(4)),
            })]),
        };
        let folded = fold_expr(e);
        if let Expr::TryBlock { body, finally_body, .. } = folded {
            if let Stmt::Expr(Expr::Integer(3)) = &body[0] {
                // body folded correctly
            } else {
                panic!("Body should be folded to Integer(3)");
            }
            let fb = finally_body.unwrap();
            if let Stmt::Expr(Expr::Integer(12)) = &fb[0] {
                // finally folded correctly
            } else {
                panic!("Finally should be folded to Integer(12)");
            }
        } else {
            panic!("Expected TryBlock");
        }
    }
}
