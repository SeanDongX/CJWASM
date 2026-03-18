//! CHIR 优化 Pass
//!
//! Pass 1: 小函数内联
//! Pass 2: 冗余 local.set/local.get 消除

use std::collections::HashMap;
use wasm_encoder::ValType;
use crate::chir::types::*;

const IMPORT_COUNT: u32 = 4;

/// 入口：对 CHIRProgram 执行所有优化 pass
pub fn optimize_chir(program: &mut CHIRProgram) {
    let before_inline = count_calls(program);
    inline_small_functions(program);
    let after_inline = count_calls(program);

    let before_locals = count_locals(program);
    eliminate_redundant_locals(program);
    let after_locals = count_locals(program);

    eprintln!("[optimize] inline: calls {} -> {} (eliminated {})",
        before_inline, after_inline, before_inline - after_inline);
    eprintln!("[optimize] locals: lets {} -> {} (eliminated {})",
        before_locals, after_locals, before_locals - after_locals);
}

fn count_calls(program: &CHIRProgram) -> usize {
    program.functions.iter().map(|f| count_calls_in_block(&f.body)).sum()
}

fn count_calls_in_block(block: &CHIRBlock) -> usize {
    block.stmts.iter().map(count_calls_in_stmt).sum::<usize>()
        + block.result.as_ref().map_or(0, |e| count_calls_in_expr(e))
}

fn count_calls_in_stmt(stmt: &CHIRStmt) -> usize {
    match stmt {
        CHIRStmt::Let { value, .. } => count_calls_in_expr(value),
        CHIRStmt::Assign { value, .. } => count_calls_in_expr(value),
        CHIRStmt::Expr(e) => count_calls_in_expr(e),
        CHIRStmt::Return(Some(e)) => count_calls_in_expr(e),
        CHIRStmt::While { cond, body } => count_calls_in_expr(cond) + count_calls_in_block(body),
        CHIRStmt::Loop { body } => count_calls_in_block(body),
        _ => 0,
    }
}

fn count_calls_in_expr(expr: &CHIRExpr) -> usize {
    match &expr.kind {
        CHIRExprKind::Call { args, .. } => 1 + args.iter().map(count_calls_in_expr).sum::<usize>(),
        CHIRExprKind::Binary { left, right, .. } => count_calls_in_expr(left) + count_calls_in_expr(right),
        CHIRExprKind::Unary { expr, .. } => count_calls_in_expr(expr),
        CHIRExprKind::Cast { expr, .. } => count_calls_in_expr(expr),
        CHIRExprKind::If { cond, then_block, else_block } => {
            count_calls_in_expr(cond)
                + count_calls_in_block(then_block)
                + else_block.as_ref().map_or(0, |b| count_calls_in_block(b))
        }
        CHIRExprKind::Block(b) => count_calls_in_block(b),
        _ => 0,
    }
}

fn count_locals(program: &CHIRProgram) -> usize {
    program.functions.iter().map(|f| count_lets_in_block(&f.body)).sum()
}

fn count_lets_in_block(block: &CHIRBlock) -> usize {
    block.stmts.iter().map(|s| match s {
        CHIRStmt::Let { .. } => 1,
        CHIRStmt::While { body, .. } => count_lets_in_block(body),
        CHIRStmt::Loop { body } => count_lets_in_block(body),
        _ => 0,
    }).sum()
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass 1: 小函数内联
// ─────────────────────────────────────────────────────────────────────────────

fn is_inlinable(func: &CHIRFunction) -> bool {
    // 提取可内联的 result 表达式：
    // 形式1: stmts=[], result=Some(expr)
    // 形式2: stmts=[Return(expr)], result=None
    let result: &CHIRExpr = if func.body.stmts.is_empty() {
        match &func.body.result {
            Some(r) => r,
            None => return false,
        }
    } else if func.body.stmts.len() == 1 {
        match &func.body.stmts[0] {
            CHIRStmt::Return(Some(e)) => e,
            _ => return false,
        }
    } else {
        return false;
    };
    if count_expr_nodes(result) > 8 {
        return false;
    }
    if has_side_effects(result) {
        return false;
    }
    // result 的 wasm_ty 必须与函数返回类型一致，否则内联后类型不匹配
    if result.wasm_ty != func.return_wasm_ty {
        return false;
    }
    true
}

/// 提取可内联函数的 result 表达式（与 is_inlinable 逻辑对应）
fn get_inlinable_result(func: &CHIRFunction) -> Option<&CHIRExpr> {
    if func.body.stmts.is_empty() {
        func.body.result.as_deref()
    } else if func.body.stmts.len() == 1 {
        match &func.body.stmts[0] {
            CHIRStmt::Return(Some(e)) => Some(e),
            _ => None,
        }
    } else {
        None
    }
}

fn count_expr_nodes(expr: &CHIRExpr) -> usize {
    1 + match &expr.kind {
        CHIRExprKind::Binary { left, right, .. } => {
            count_expr_nodes(left) + count_expr_nodes(right)
        }
        CHIRExprKind::Unary { expr, .. } => count_expr_nodes(expr),
        CHIRExprKind::Cast { expr, .. } => count_expr_nodes(expr),
        CHIRExprKind::FieldGet { object, .. } => count_expr_nodes(object),
        CHIRExprKind::TupleGet { tuple, .. } => count_expr_nodes(tuple),
        CHIRExprKind::Load { ptr, .. } => count_expr_nodes(ptr),
        _ => 0,
    }
}

fn has_side_effects(expr: &CHIRExpr) -> bool {
    match &expr.kind {
        CHIRExprKind::Call { .. }
        | CHIRExprKind::MethodCall { .. }
        | CHIRExprKind::CallIndirect { .. }
        | CHIRExprKind::Store { .. }
        | CHIRExprKind::FieldSet { .. }
        | CHIRExprKind::ArraySet { .. }
        | CHIRExprKind::Print { .. } => true,
        CHIRExprKind::Binary { left, right, .. } => {
            has_side_effects(left) || has_side_effects(right)
        }
        CHIRExprKind::Unary { expr, .. } => has_side_effects(expr),
        CHIRExprKind::Cast { expr, .. } => has_side_effects(expr),
        CHIRExprKind::FieldGet { object, .. } => has_side_effects(object),
        CHIRExprKind::TupleGet { tuple, .. } => has_side_effects(tuple),
        CHIRExprKind::Load { ptr, .. } => has_side_effects(ptr),
        CHIRExprKind::If { cond, then_block, else_block } => {
            has_side_effects(cond)
                || has_side_effects_block(then_block)
                || else_block.as_ref().map_or(false, |b| has_side_effects_block(b))
        }
        CHIRExprKind::Block(b) => has_side_effects_block(b),
        CHIRExprKind::Match { subject, arms } => {
            has_side_effects(subject)
                || arms.iter().any(|a| has_side_effects_block(&a.body))
        }
        _ => false,
    }
}

fn has_side_effects_block(block: &CHIRBlock) -> bool {
    block.stmts.iter().any(|s| match s {
        CHIRStmt::Let { value, .. } => has_side_effects(value),
        CHIRStmt::Assign { value, .. } => has_side_effects(value),
        CHIRStmt::Expr(e) => has_side_effects(e),
        // Return 是控制流副作用，内联含 Return 的 block 会导致 caller 提前返回
        CHIRStmt::Return(_) => true,
        CHIRStmt::While { cond, body } => has_side_effects(cond) || has_side_effects_block(body),
        CHIRStmt::Loop { body } => has_side_effects_block(body),
        _ => false,
    }) || block.result.as_ref().map_or(false, |r| has_side_effects(r))
}

fn has_call_to(expr: &CHIRExpr, target_idx: u32) -> bool {
    match &expr.kind {
        CHIRExprKind::Call { func_idx, args } => {
            if *func_idx == target_idx {
                return true;
            }
            args.iter().any(|a| has_call_to(a, target_idx))
        }
        CHIRExprKind::Binary { left, right, .. } => {
            has_call_to(left, target_idx) || has_call_to(right, target_idx)
        }
        CHIRExprKind::Unary { expr, .. } => has_call_to(expr, target_idx),
        CHIRExprKind::Cast { expr, .. } => has_call_to(expr, target_idx),
        CHIRExprKind::FieldGet { object, .. } => has_call_to(object, target_idx),
        CHIRExprKind::TupleGet { tuple, .. } => has_call_to(tuple, target_idx),
        CHIRExprKind::Load { ptr, .. } => has_call_to(ptr, target_idx),
        CHIRExprKind::If { cond, then_block, else_block } => {
            has_call_to(cond, target_idx)
                || has_call_to_block(then_block, target_idx)
                || else_block.as_ref().map_or(false, |b| has_call_to_block(b, target_idx))
        }
        CHIRExprKind::Block(b) => has_call_to_block(b, target_idx),
        CHIRExprKind::Match { subject, arms } => {
            has_call_to(subject, target_idx)
                || arms.iter().any(|a| has_call_to_block(&a.body, target_idx))
        }
        _ => false,
    }
}

fn has_call_to_block(block: &CHIRBlock, target_idx: u32) -> bool {
    block.stmts.iter().any(|s| match s {
        CHIRStmt::Let { value, .. } => has_call_to(value, target_idx),
        CHIRStmt::Assign { value, .. } => has_call_to(value, target_idx),
        CHIRStmt::Expr(e) => has_call_to(e, target_idx),
        CHIRStmt::Return(Some(e)) => has_call_to(e, target_idx),
        CHIRStmt::While { cond, body } => has_call_to(cond, target_idx) || has_call_to_block(body, target_idx),
        CHIRStmt::Loop { body } => has_call_to_block(body, target_idx),
        _ => false,
    }) || block.result.as_ref().map_or(false, |r| has_call_to(r, target_idx))
}

/// 将 expr 中所有 Local(param_local) 替换为对应 arg，并将被内联函数的非参数 locals 偏移
fn substitute_and_remap(
    expr: CHIRExpr,
    params: &[CHIRParam],
    args: &[CHIRExpr],
    caller_next_local: u32,
) -> CHIRExpr {
    let param_count = params.len() as u32;
    subst_expr(expr, params, args, caller_next_local, param_count)
}

fn subst_expr(
    expr: CHIRExpr,
    params: &[CHIRParam],
    args: &[CHIRExpr],
    caller_next_local: u32,
    param_count: u32,
) -> CHIRExpr {
    let new_kind = match expr.kind {
        CHIRExprKind::Local(idx) => {
            // 查找是否是参数
            if let Some(pos) = params.iter().position(|p| p.local_idx == idx) {
                if pos < args.len() {
                    return args[pos].clone();
                }
            }
            // 非参数 local：偏移
            if idx >= param_count {
                CHIRExprKind::Local(caller_next_local + (idx - param_count))
            } else {
                CHIRExprKind::Local(idx)
            }
        }
        CHIRExprKind::Binary { op, left, right } => CHIRExprKind::Binary {
            op,
            left: Box::new(subst_expr(*left, params, args, caller_next_local, param_count)),
            right: Box::new(subst_expr(*right, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::Unary { op, expr: inner } => CHIRExprKind::Unary {
            op,
            expr: Box::new(subst_expr(*inner, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::Cast { expr: inner, from_ty, to_ty } => CHIRExprKind::Cast {
            expr: Box::new(subst_expr(*inner, params, args, caller_next_local, param_count)),
            from_ty,
            to_ty,
        },
        CHIRExprKind::FieldGet { object, field_offset, field_ty } => CHIRExprKind::FieldGet {
            object: Box::new(subst_expr(*object, params, args, caller_next_local, param_count)),
            field_offset,
            field_ty,
        },
        CHIRExprKind::TupleGet { tuple, index } => CHIRExprKind::TupleGet {
            tuple: Box::new(subst_expr(*tuple, params, args, caller_next_local, param_count)),
            index,
        },
        CHIRExprKind::Load { ptr, offset, align } => CHIRExprKind::Load {
            ptr: Box::new(subst_expr(*ptr, params, args, caller_next_local, param_count)),
            offset,
            align,
        },
        // 控制流：必须递归进入 block，以便重映射其中的 Let local_idx
        CHIRExprKind::If { cond, then_block, else_block } => CHIRExprKind::If {
            cond: Box::new(subst_expr(*cond, params, args, caller_next_local, param_count)),
            then_block: subst_block(then_block, params, args, caller_next_local, param_count),
            else_block: else_block.map(|b| subst_block(b, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::Block(b) => CHIRExprKind::Block(
            subst_block(b, params, args, caller_next_local, param_count)
        ),
        CHIRExprKind::Match { subject, arms } => CHIRExprKind::Match {
            subject: Box::new(subst_expr(*subject, params, args, caller_next_local, param_count)),
            arms: arms.into_iter().map(|arm| CHIRMatchArm {
                pattern: subst_pattern(arm.pattern, param_count, caller_next_local),
                guard: arm.guard.map(|g| subst_expr(g, params, args, caller_next_local, param_count)),
                body: subst_block(arm.body, params, args, caller_next_local, param_count),
            }).collect(),
        },
        CHIRExprKind::Call { func_idx, args: call_args } => CHIRExprKind::Call {
            func_idx,
            args: call_args.into_iter()
                .map(|a| subst_expr(a, params, args, caller_next_local, param_count))
                .collect(),
        },
        CHIRExprKind::MethodCall { vtable_offset, func_idx, receiver, args: call_args } => CHIRExprKind::MethodCall {
            vtable_offset,
            func_idx,
            receiver: Box::new(subst_expr(*receiver, params, args, caller_next_local, param_count)),
            args: call_args.into_iter()
                .map(|a| subst_expr(a, params, args, caller_next_local, param_count))
                .collect(),
        },
        CHIRExprKind::CallIndirect { type_idx, args: call_args, callee } => CHIRExprKind::CallIndirect {
            type_idx,
            args: call_args.into_iter()
                .map(|a| subst_expr(a, params, args, caller_next_local, param_count))
                .collect(),
            callee: Box::new(subst_expr(*callee, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::ArrayGet { array, index } => CHIRExprKind::ArrayGet {
            array: Box::new(subst_expr(*array, params, args, caller_next_local, param_count)),
            index: Box::new(subst_expr(*index, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::ArraySet { array, index, value } => CHIRExprKind::ArraySet {
            array: Box::new(subst_expr(*array, params, args, caller_next_local, param_count)),
            index: Box::new(subst_expr(*index, params, args, caller_next_local, param_count)),
            value: Box::new(subst_expr(*value, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::ArrayNew { len, init } => CHIRExprKind::ArrayNew {
            len: Box::new(subst_expr(*len, params, args, caller_next_local, param_count)),
            init: Box::new(subst_expr(*init, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::ArrayLiteral { elements } => CHIRExprKind::ArrayLiteral {
            elements: elements.into_iter()
                .map(|e| subst_expr(e, params, args, caller_next_local, param_count))
                .collect(),
        },
        CHIRExprKind::TupleNew { elements } => CHIRExprKind::TupleNew {
            elements: elements.into_iter()
                .map(|e| subst_expr(e, params, args, caller_next_local, param_count))
                .collect(),
        },
        CHIRExprKind::StructNew { struct_name, fields } => CHIRExprKind::StructNew {
            struct_name,
            fields: fields.into_iter()
                .map(|(n, e)| (n, subst_expr(e, params, args, caller_next_local, param_count)))
                .collect(),
        },
        CHIRExprKind::Store { ptr, value, offset, align } => CHIRExprKind::Store {
            ptr: Box::new(subst_expr(*ptr, params, args, caller_next_local, param_count)),
            value: Box::new(subst_expr(*value, params, args, caller_next_local, param_count)),
            offset,
            align,
        },
        CHIRExprKind::FieldSet { object, field_offset, value } => CHIRExprKind::FieldSet {
            object: Box::new(subst_expr(*object, params, args, caller_next_local, param_count)),
            field_offset,
            value: Box::new(subst_expr(*value, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::Print { arg, newline, fd } => CHIRExprKind::Print {
            arg: arg.map(|a| Box::new(subst_expr(*a, params, args, caller_next_local, param_count))),
            newline,
            fd,
        },
        CHIRExprKind::MathUnary { op, arg } => CHIRExprKind::MathUnary {
            op,
            arg: Box::new(subst_expr(*arg, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::MathBinary { op, left, right } => CHIRExprKind::MathBinary {
            op,
            left: Box::new(subst_expr(*left, params, args, caller_next_local, param_count)),
            right: Box::new(subst_expr(*right, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::BuiltinAbs { val, tmp_local } => {
            let new_tmp = if tmp_local >= param_count {
                caller_next_local + (tmp_local - param_count)
            } else {
                tmp_local
            };
            CHIRExprKind::BuiltinAbs {
                val: Box::new(subst_expr(*val, params, args, caller_next_local, param_count)),
                tmp_local: new_tmp,
            }
        }
        CHIRExprKind::BuiltinCompareTo { left, right } => CHIRExprKind::BuiltinCompareTo {
            left: Box::new(subst_expr(*left, params, args, caller_next_local, param_count)),
            right: Box::new(subst_expr(*right, params, args, caller_next_local, param_count)),
        },
        CHIRExprKind::BuiltinStringIsEmpty { val } => CHIRExprKind::BuiltinStringIsEmpty {
            val: Box::new(subst_expr(*val, params, args, caller_next_local, param_count)),
        },
        other => other,
    };
    CHIRExpr { kind: new_kind, ..expr }
}

fn subst_block(
    block: CHIRBlock,
    params: &[CHIRParam],
    args: &[CHIRExpr],
    caller_next_local: u32,
    param_count: u32,
) -> CHIRBlock {
    CHIRBlock {
        stmts: block.stmts.into_iter()
            .map(|s| subst_stmt(s, params, args, caller_next_local, param_count))
            .collect(),
        result: block.result.map(|r| Box::new(subst_expr(*r, params, args, caller_next_local, param_count))),
    }
}

fn subst_stmt(
    stmt: CHIRStmt,
    params: &[CHIRParam],
    args: &[CHIRExpr],
    caller_next_local: u32,
    param_count: u32,
) -> CHIRStmt {
    match stmt {
        CHIRStmt::Let { local_idx, value } => {
            // 重映射 Let 的写目标 local_idx
            let new_idx = if local_idx >= param_count {
                caller_next_local + (local_idx - param_count)
            } else {
                local_idx
            };
            CHIRStmt::Let {
                local_idx: new_idx,
                value: subst_expr(value, params, args, caller_next_local, param_count),
            }
        }
        CHIRStmt::Assign { target, value } => CHIRStmt::Assign {
            target: subst_lvalue(target, params, args, caller_next_local, param_count),
            value: subst_expr(value, params, args, caller_next_local, param_count),
        },
        CHIRStmt::Expr(e) => CHIRStmt::Expr(
            subst_expr(e, params, args, caller_next_local, param_count)
        ),
        CHIRStmt::Return(Some(e)) => CHIRStmt::Return(Some(
            subst_expr(e, params, args, caller_next_local, param_count)
        )),
        CHIRStmt::While { cond, body } => CHIRStmt::While {
            cond: subst_expr(cond, params, args, caller_next_local, param_count),
            body: subst_block(body, params, args, caller_next_local, param_count),
        },
        CHIRStmt::Loop { body } => CHIRStmt::Loop {
            body: subst_block(body, params, args, caller_next_local, param_count),
        },
        other => other,
    }
}

fn subst_lvalue(
    lvalue: CHIRLValue,
    params: &[CHIRParam],
    args: &[CHIRExpr],
    caller_next_local: u32,
    param_count: u32,
) -> CHIRLValue {
    match lvalue {
        CHIRLValue::Local(idx) => {
            let new_idx = if idx >= param_count {
                caller_next_local + (idx - param_count)
            } else {
                idx
            };
            CHIRLValue::Local(new_idx)
        }
        CHIRLValue::Field { object, offset } => CHIRLValue::Field {
            object: Box::new(subst_expr(*object, params, args, caller_next_local, param_count)),
            offset,
        },
        CHIRLValue::Index { array, index } => CHIRLValue::Index {
            array: Box::new(subst_expr(*array, params, args, caller_next_local, param_count)),
            index: Box::new(subst_expr(*index, params, args, caller_next_local, param_count)),
        },
    }
}

fn subst_pattern(pattern: CHIRPattern, param_count: u32, caller_next_local: u32) -> CHIRPattern {
    let remap = |idx: u32| -> u32 {
        if idx >= param_count { caller_next_local + (idx - param_count) } else { idx }
    };
    match pattern {
        CHIRPattern::Binding(idx) => CHIRPattern::Binding(remap(idx)),
        CHIRPattern::Variant { discriminant, payload_binding, enum_has_payload } => {
            CHIRPattern::Variant {
                discriminant,
                payload_binding: payload_binding.map(remap),
                enum_has_payload,
            }
        }
        CHIRPattern::Struct { fields } => CHIRPattern::Struct {
            fields: fields.into_iter().map(|field| match field {
                StructPatternField::Binding { offset, local_idx, wasm_ty } =>
                    StructPatternField::Binding { offset, local_idx: remap(local_idx), wasm_ty },
                StructPatternField::NestedBinding { outer_offset, inner_offset, local_idx, wasm_ty } =>
                    StructPatternField::NestedBinding { outer_offset, inner_offset, local_idx: remap(local_idx), wasm_ty },
                other => other,
            }).collect(),
        },
        other => other,
    }
}

fn inline_in_expr(
    expr: &mut CHIRExpr,
    inlinable: &HashMap<u32, CHIRFunction>,
    next_local: &mut u32,
    extra_locals: &mut Vec<(u32, ValType)>,
) {
    match &mut expr.kind {
        CHIRExprKind::Call { func_idx, args } => {
            // 先递归处理 args
            let fi = *func_idx;
            for arg in args.iter_mut() {
                inline_in_expr(arg, inlinable, next_local, extra_locals);
            }
            if let Some(callee) = inlinable.get(&fi) {
                let param_count = callee.params.len() as u32;
                // callee.locals 始终为空（lower.rs 中 locals: Vec::new()）
                // 必须从 local_wasm_types 中读取非参数 locals
                let mut callee_extra: Vec<(u32, ValType)> = callee
                    .local_wasm_types
                    .iter()
                    .filter(|(&idx, _)| idx >= param_count)
                    .map(|(&idx, &ty)| (idx, ty))
                    .collect();
                callee_extra.sort_by_key(|&(idx, _)| idx);
                // 为被内联函数的非参数 locals 分配新索引
                let base = *next_local;
                for &(orig_idx, wasm_ty) in &callee_extra {
                    let new_idx = base + (orig_idx - param_count);
                    extra_locals.push((new_idx, wasm_ty));
                }
                *next_local += callee_extra.len() as u32;

                let result = match get_inlinable_result(callee) {
                    Some(r) => r.clone(),
                    None => return,
                };
                let original_wasm_ty = expr.wasm_ty;
                let original_ty = expr.ty.clone();
                let args_cloned: Vec<CHIRExpr> = match &expr.kind {
                    CHIRExprKind::Call { args, .. } => args.clone(),
                    _ => unreachable!(),
                };
                let mut inlined = substitute_and_remap(result, &callee.params, &args_cloned, base);
                // 如果内联结果的 wasm_ty 与原 Call 不一致，插入 Cast 保证类型正确
                if inlined.wasm_ty != original_wasm_ty {
                    inlined = CHIRExpr {
                        kind: CHIRExprKind::Cast {
                            expr: Box::new(inlined),
                            from_ty: callee.return_wasm_ty,
                            to_ty: original_wasm_ty,
                        },
                        ty: original_ty,
                        wasm_ty: original_wasm_ty,
                        span: None,
                    };
                }
                *expr = inlined;
                return;
            }
        }
        CHIRExprKind::Binary { left, right, .. } => {
            inline_in_expr(left, inlinable, next_local, extra_locals);
            inline_in_expr(right, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::Unary { expr: inner, .. } => {
            inline_in_expr(inner, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::Cast { expr: inner, .. } => {
            inline_in_expr(inner, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::FieldGet { object, .. } => {
            inline_in_expr(object, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::TupleGet { tuple, .. } => {
            inline_in_expr(tuple, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::Load { ptr, .. } => {
            inline_in_expr(ptr, inlinable, next_local, extra_locals);
        }
        CHIRExprKind::If { cond, then_block, else_block } => {
            inline_in_expr(cond, inlinable, next_local, extra_locals);
            inline_in_block(then_block, inlinable, next_local, extra_locals);
            if let Some(eb) = else_block {
                inline_in_block(eb, inlinable, next_local, extra_locals);
            }
        }
        CHIRExprKind::Block(block) => {
            inline_in_block(block, inlinable, next_local, extra_locals);
        }
        _ => {}
    }
}

fn inline_in_block(
    block: &mut CHIRBlock,
    inlinable: &HashMap<u32, CHIRFunction>,
    next_local: &mut u32,
    extra_locals: &mut Vec<(u32, ValType)>,
) {
    for stmt in &mut block.stmts {
        inline_in_stmt(stmt, inlinable, next_local, extra_locals);
    }
    if let Some(result) = &mut block.result {
        inline_in_expr(result, inlinable, next_local, extra_locals);
    }
}

fn inline_in_stmt(
    stmt: &mut CHIRStmt,
    inlinable: &HashMap<u32, CHIRFunction>,
    next_local: &mut u32,
    extra_locals: &mut Vec<(u32, ValType)>,
) {
    match stmt {
        CHIRStmt::Let { value, .. } => {
            inline_in_expr(value, inlinable, next_local, extra_locals);
        }
        CHIRStmt::Assign { value, .. } => {
            inline_in_expr(value, inlinable, next_local, extra_locals);
        }
        CHIRStmt::Expr(e) => {
            inline_in_expr(e, inlinable, next_local, extra_locals);
        }
        CHIRStmt::Return(Some(e)) => {
            inline_in_expr(e, inlinable, next_local, extra_locals);
        }
        CHIRStmt::While { cond, body } => {
            inline_in_expr(cond, inlinable, next_local, extra_locals);
            inline_in_block(body, inlinable, next_local, extra_locals);
        }
        CHIRStmt::Loop { body } => {
            inline_in_block(body, inlinable, next_local, extra_locals);
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass 2: 冗余 local.set/local.get 消除
// ─────────────────────────────────────────────────────────────────────────────

fn is_pure(expr: &CHIRExpr) -> bool {
    match &expr.kind {
        CHIRExprKind::Integer(_)
        | CHIRExprKind::Float(_)
        | CHIRExprKind::Float32(_)
        | CHIRExprKind::Bool(_)
        | CHIRExprKind::Local(_)
        | CHIRExprKind::Nop => true,
        CHIRExprKind::Binary { left, right, .. } => is_pure(left) && is_pure(right),
        CHIRExprKind::Unary { expr, .. } => is_pure(expr),
        CHIRExprKind::Cast { expr, .. } => is_pure(expr),
        CHIRExprKind::FieldGet { object, .. } => is_pure(object),
        CHIRExprKind::TupleGet { tuple, .. } => is_pure(tuple),
        _ => false,
    }
}

fn count_local_reads_in_expr(expr: &CHIRExpr, idx: u32) -> u32 {
    match &expr.kind {
        CHIRExprKind::Local(i) => if *i == idx { 1 } else { 0 },
        CHIRExprKind::Binary { left, right, .. } => {
            count_local_reads_in_expr(left, idx) + count_local_reads_in_expr(right, idx)
        }
        CHIRExprKind::Unary { expr, .. } => count_local_reads_in_expr(expr, idx),
        CHIRExprKind::Cast { expr, .. } => count_local_reads_in_expr(expr, idx),
        CHIRExprKind::FieldGet { object, .. } => count_local_reads_in_expr(object, idx),
        CHIRExprKind::TupleGet { tuple, .. } => count_local_reads_in_expr(tuple, idx),
        CHIRExprKind::Load { ptr, .. } => count_local_reads_in_expr(ptr, idx),
        CHIRExprKind::Call { args, .. } => args.iter().map(|a| count_local_reads_in_expr(a, idx)).sum(),
        CHIRExprKind::MethodCall { receiver, args, .. } => {
            count_local_reads_in_expr(receiver, idx)
                + args.iter().map(|a| count_local_reads_in_expr(a, idx)).sum::<u32>()
        }
        CHIRExprKind::If { cond, then_block, else_block } => {
            count_local_reads_in_expr(cond, idx)
                + count_local_reads_in_block(then_block, idx)
                + else_block.as_ref().map_or(0, |b| count_local_reads_in_block(b, idx))
        }
        CHIRExprKind::Block(b) => count_local_reads_in_block(b, idx),
        CHIRExprKind::ArrayGet { array, index } => {
            count_local_reads_in_expr(array, idx) + count_local_reads_in_expr(index, idx)
        }
        CHIRExprKind::ArraySet { array, index, value } => {
            count_local_reads_in_expr(array, idx)
                + count_local_reads_in_expr(index, idx)
                + count_local_reads_in_expr(value, idx)
        }
        CHIRExprKind::TupleNew { elements } | CHIRExprKind::ArrayLiteral { elements } => {
            elements.iter().map(|e| count_local_reads_in_expr(e, idx)).sum()
        }
        CHIRExprKind::FieldSet { object, value, .. } => {
            count_local_reads_in_expr(object, idx) + count_local_reads_in_expr(value, idx)
        }
        CHIRExprKind::Store { ptr, value, .. } => {
            count_local_reads_in_expr(ptr, idx) + count_local_reads_in_expr(value, idx)
        }
        CHIRExprKind::Print { arg, .. } => {
            arg.as_ref().map_or(0, |a| count_local_reads_in_expr(a, idx))
        }
        CHIRExprKind::MathUnary { arg, .. } => count_local_reads_in_expr(arg, idx),
        CHIRExprKind::MathBinary { left, right, .. } => {
            count_local_reads_in_expr(left, idx) + count_local_reads_in_expr(right, idx)
        }
        CHIRExprKind::BuiltinAbs { val, .. } => count_local_reads_in_expr(val, idx),
        CHIRExprKind::BuiltinCompareTo { left, right } => {
            count_local_reads_in_expr(left, idx) + count_local_reads_in_expr(right, idx)
        }
        CHIRExprKind::BuiltinStringIsEmpty { val } => count_local_reads_in_expr(val, idx),
        CHIRExprKind::CallIndirect { args, callee, .. } => {
            count_local_reads_in_expr(callee, idx)
                + args.iter().map(|a| count_local_reads_in_expr(a, idx)).sum::<u32>()
        }
        CHIRExprKind::ArrayNew { len, init } => {
            count_local_reads_in_expr(len, idx) + count_local_reads_in_expr(init, idx)
        }
        CHIRExprKind::StructNew { fields, .. } => {
            fields.iter().map(|(_, e)| count_local_reads_in_expr(e, idx)).sum()
        }
        CHIRExprKind::Match { subject, arms } => {
            count_local_reads_in_expr(subject, idx)
                + arms.iter().map(|a| {
                    a.guard.as_ref().map_or(0, |g| count_local_reads_in_expr(g, idx))
                        + count_local_reads_in_block(&a.body, idx)
                }).sum::<u32>()
        }
        _ => 0,
    }
}

fn count_local_reads_in_block(block: &CHIRBlock, idx: u32) -> u32 {
    let stmt_reads: u32 = block.stmts.iter().map(|s| count_local_reads_in_stmt(s, idx)).sum();
    let result_reads = block.result.as_ref().map_or(0, |r| count_local_reads_in_expr(r, idx));
    stmt_reads + result_reads
}

fn count_local_reads_in_stmt(stmt: &CHIRStmt, idx: u32) -> u32 {
    match stmt {
        CHIRStmt::Let { value, .. } => count_local_reads_in_expr(value, idx),
        CHIRStmt::Assign { value, target } => {
            count_local_reads_in_expr(value, idx)
                + match target {
                    CHIRLValue::Local(i) => if *i == idx { 0 } else { 0 },
                    CHIRLValue::Field { object, .. } => count_local_reads_in_expr(object, idx),
                    CHIRLValue::Index { array, index } => {
                        count_local_reads_in_expr(array, idx) + count_local_reads_in_expr(index, idx)
                    }
                }
        }
        CHIRStmt::Expr(e) => count_local_reads_in_expr(e, idx),
        CHIRStmt::Return(Some(e)) => count_local_reads_in_expr(e, idx),
        CHIRStmt::While { cond, body } => {
            count_local_reads_in_expr(cond, idx) + count_local_reads_in_block(body, idx)
        }
        CHIRStmt::Loop { body } => count_local_reads_in_block(body, idx),
        _ => 0,
    }
}

/// 统计函数内每个 local 的 (write_count, read_count)
fn count_local_usage_in_function(func: &CHIRFunction) -> HashMap<u32, (u32, u32)> {
    let mut usage: HashMap<u32, (u32, u32)> = HashMap::new();
    count_usage_in_block(&func.body, &mut usage);
    usage
}

fn count_usage_in_block(block: &CHIRBlock, usage: &mut HashMap<u32, (u32, u32)>) {
    for stmt in &block.stmts {
        count_usage_in_stmt(stmt, usage);
    }
    if let Some(r) = &block.result {
        let reads = count_all_local_reads_in_expr(r);
        for (idx, cnt) in reads {
            usage.entry(idx).or_default().1 += cnt;
        }
    }
}

fn count_usage_in_stmt(stmt: &CHIRStmt, usage: &mut HashMap<u32, (u32, u32)>) {
    match stmt {
        CHIRStmt::Let { local_idx, value } => {
            usage.entry(*local_idx).or_default().0 += 1;
            let reads = count_all_local_reads_in_expr(value);
            for (idx, cnt) in reads {
                usage.entry(idx).or_default().1 += cnt;
            }
        }
        CHIRStmt::Assign { target, value } => {
            if let CHIRLValue::Local(idx) = target {
                usage.entry(*idx).or_default().0 += 1;
            }
            let reads = count_all_local_reads_in_expr(value);
            for (idx, cnt) in reads {
                usage.entry(idx).or_default().1 += cnt;
            }
        }
        CHIRStmt::Expr(e) => {
            let reads = count_all_local_reads_in_expr(e);
            for (idx, cnt) in reads {
                usage.entry(idx).or_default().1 += cnt;
            }
        }
        CHIRStmt::Return(Some(e)) => {
            let reads = count_all_local_reads_in_expr(e);
            for (idx, cnt) in reads {
                usage.entry(idx).or_default().1 += cnt;
            }
        }
        CHIRStmt::While { cond, body } => {
            let reads = count_all_local_reads_in_expr(cond);
            for (idx, cnt) in reads {
                usage.entry(idx).or_default().1 += cnt;
            }
            count_usage_in_block(body, usage);
        }
        CHIRStmt::Loop { body } => {
            count_usage_in_block(body, usage);
        }
        _ => {}
    }
}

fn count_all_local_reads_in_expr(expr: &CHIRExpr) -> HashMap<u32, u32> {
    let mut map: HashMap<u32, u32> = HashMap::new();
    collect_reads(expr, &mut map);
    map
}

fn collect_reads(expr: &CHIRExpr, map: &mut HashMap<u32, u32>) {
    match &expr.kind {
        CHIRExprKind::Local(i) => { *map.entry(*i).or_default() += 1; }
        CHIRExprKind::Binary { left, right, .. } => { collect_reads(left, map); collect_reads(right, map); }
        CHIRExprKind::Unary { expr, .. } => collect_reads(expr, map),
        CHIRExprKind::Cast { expr, .. } => collect_reads(expr, map),
        CHIRExprKind::FieldGet { object, .. } => collect_reads(object, map),
        CHIRExprKind::TupleGet { tuple, .. } => collect_reads(tuple, map),
        CHIRExprKind::Load { ptr, .. } => collect_reads(ptr, map),
        CHIRExprKind::Call { args, .. } => { for a in args { collect_reads(a, map); } }
        CHIRExprKind::MethodCall { receiver, args, .. } => {
            collect_reads(receiver, map);
            for a in args { collect_reads(a, map); }
        }
        CHIRExprKind::If { cond, then_block, else_block } => {
            collect_reads(cond, map);
            collect_reads_block(then_block, map);
            if let Some(b) = else_block { collect_reads_block(b, map); }
        }
        CHIRExprKind::Block(b) => collect_reads_block(b, map),
        CHIRExprKind::ArrayGet { array, index } => { collect_reads(array, map); collect_reads(index, map); }
        CHIRExprKind::ArraySet { array, index, value } => {
            collect_reads(array, map); collect_reads(index, map); collect_reads(value, map);
        }
        CHIRExprKind::TupleNew { elements } | CHIRExprKind::ArrayLiteral { elements } => {
            for e in elements { collect_reads(e, map); }
        }
        CHIRExprKind::FieldSet { object, value, .. } => { collect_reads(object, map); collect_reads(value, map); }
        CHIRExprKind::Store { ptr, value, .. } => { collect_reads(ptr, map); collect_reads(value, map); }
        CHIRExprKind::Print { arg, .. } => { if let Some(a) = arg { collect_reads(a, map); } }
        CHIRExprKind::MathUnary { arg, .. } => collect_reads(arg, map),
        CHIRExprKind::MathBinary { left, right, .. } => { collect_reads(left, map); collect_reads(right, map); }
        CHIRExprKind::BuiltinAbs { val, .. } => collect_reads(val, map),
        CHIRExprKind::BuiltinCompareTo { left, right } => { collect_reads(left, map); collect_reads(right, map); }
        CHIRExprKind::BuiltinStringIsEmpty { val } => collect_reads(val, map),
        CHIRExprKind::CallIndirect { args, callee, .. } => {
            collect_reads(callee, map);
            for a in args { collect_reads(a, map); }
        }
        CHIRExprKind::ArrayNew { len, init } => { collect_reads(len, map); collect_reads(init, map); }
        CHIRExprKind::StructNew { fields, .. } => { for (_, e) in fields { collect_reads(e, map); } }
        CHIRExprKind::Match { subject, arms } => {
            collect_reads(subject, map);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_reads(g, map); }
                collect_reads_block(&arm.body, map);
            }
        }
        _ => {}
    }
}

fn collect_reads_block(block: &CHIRBlock, map: &mut HashMap<u32, u32>) {
    for stmt in &block.stmts {
        collect_reads_stmt(stmt, map);
    }
    if let Some(r) = &block.result { collect_reads(r, map); }
}

fn collect_reads_stmt(stmt: &CHIRStmt, map: &mut HashMap<u32, u32>) {
    match stmt {
        CHIRStmt::Let { value, .. } => collect_reads(value, map),
        CHIRStmt::Assign { value, .. } => collect_reads(value, map),
        CHIRStmt::Expr(e) => collect_reads(e, map),
        CHIRStmt::Return(Some(e)) => collect_reads(e, map),
        CHIRStmt::While { cond, body } => { collect_reads(cond, map); collect_reads_block(body, map); }
        CHIRStmt::Loop { body } => collect_reads_block(body, map),
        _ => {}
    }
}

fn substitute_local(expr: CHIRExpr, idx: u32, replacement: &CHIRExpr) -> CHIRExpr {
    let sl = |e: CHIRExpr| substitute_local(e, idx, replacement);
    let sl_block = |b: CHIRBlock| substitute_local_block(b, idx, replacement);
    let new_kind = match expr.kind {
        CHIRExprKind::Local(i) if i == idx => return replacement.clone(),
        CHIRExprKind::Binary { op, left, right } => CHIRExprKind::Binary {
            op,
            left: Box::new(sl(*left)),
            right: Box::new(sl(*right)),
        },
        CHIRExprKind::Unary { op, expr: inner } => CHIRExprKind::Unary {
            op,
            expr: Box::new(sl(*inner)),
        },
        CHIRExprKind::Cast { expr: inner, from_ty, to_ty } => CHIRExprKind::Cast {
            expr: Box::new(sl(*inner)),
            from_ty,
            to_ty,
        },
        CHIRExprKind::FieldGet { object, field_offset, field_ty } => CHIRExprKind::FieldGet {
            object: Box::new(sl(*object)),
            field_offset,
            field_ty,
        },
        CHIRExprKind::TupleGet { tuple, index } => CHIRExprKind::TupleGet {
            tuple: Box::new(sl(*tuple)),
            index,
        },
        CHIRExprKind::Load { ptr, offset, align } => CHIRExprKind::Load {
            ptr: Box::new(sl(*ptr)),
            offset,
            align,
        },
        CHIRExprKind::Call { func_idx, args } => CHIRExprKind::Call {
            func_idx,
            args: args.into_iter().map(sl).collect(),
        },
        CHIRExprKind::MethodCall { vtable_offset, func_idx, receiver, args } => CHIRExprKind::MethodCall {
            vtable_offset,
            func_idx,
            receiver: Box::new(sl(*receiver)),
            args: args.into_iter().map(sl).collect(),
        },
        CHIRExprKind::CallIndirect { type_idx, args, callee } => CHIRExprKind::CallIndirect {
            type_idx,
            args: args.into_iter().map(sl).collect(),
            callee: Box::new(sl(*callee)),
        },
        CHIRExprKind::If { cond, then_block, else_block } => CHIRExprKind::If {
            cond: Box::new(sl(*cond)),
            then_block: sl_block(then_block),
            else_block: else_block.map(sl_block),
        },
        CHIRExprKind::Block(b) => CHIRExprKind::Block(sl_block(b)),
        CHIRExprKind::Match { subject, arms } => CHIRExprKind::Match {
            subject: Box::new(sl(*subject)),
            arms: arms.into_iter().map(|arm| CHIRMatchArm {
                pattern: arm.pattern,
                guard: arm.guard.map(sl),
                body: sl_block(arm.body),
            }).collect(),
        },
        CHIRExprKind::ArrayGet { array, index } => CHIRExprKind::ArrayGet {
            array: Box::new(sl(*array)),
            index: Box::new(sl(*index)),
        },
        CHIRExprKind::ArraySet { array, index, value } => CHIRExprKind::ArraySet {
            array: Box::new(sl(*array)),
            index: Box::new(sl(*index)),
            value: Box::new(sl(*value)),
        },
        CHIRExprKind::ArrayNew { len, init } => CHIRExprKind::ArrayNew {
            len: Box::new(sl(*len)),
            init: Box::new(sl(*init)),
        },
        CHIRExprKind::ArrayLiteral { elements } => CHIRExprKind::ArrayLiteral {
            elements: elements.into_iter().map(sl).collect(),
        },
        CHIRExprKind::TupleNew { elements } => CHIRExprKind::TupleNew {
            elements: elements.into_iter().map(sl).collect(),
        },
        CHIRExprKind::StructNew { struct_name, fields } => CHIRExprKind::StructNew {
            struct_name,
            fields: fields.into_iter().map(|(n, e)| (n, sl(e))).collect(),
        },
        CHIRExprKind::Store { ptr, value, offset, align } => CHIRExprKind::Store {
            ptr: Box::new(sl(*ptr)),
            value: Box::new(sl(*value)),
            offset,
            align,
        },
        CHIRExprKind::FieldSet { object, field_offset, value } => CHIRExprKind::FieldSet {
            object: Box::new(sl(*object)),
            field_offset,
            value: Box::new(sl(*value)),
        },
        CHIRExprKind::Print { arg, newline, fd } => CHIRExprKind::Print {
            arg: arg.map(|a| Box::new(sl(*a))),
            newline,
            fd,
        },
        CHIRExprKind::MathUnary { op, arg } => CHIRExprKind::MathUnary {
            op,
            arg: Box::new(sl(*arg)),
        },
        CHIRExprKind::MathBinary { op, left, right } => CHIRExprKind::MathBinary {
            op,
            left: Box::new(sl(*left)),
            right: Box::new(sl(*right)),
        },
        CHIRExprKind::BuiltinAbs { val, tmp_local } => CHIRExprKind::BuiltinAbs {
            val: Box::new(sl(*val)),
            tmp_local,
        },
        CHIRExprKind::BuiltinCompareTo { left, right } => CHIRExprKind::BuiltinCompareTo {
            left: Box::new(sl(*left)),
            right: Box::new(sl(*right)),
        },
        CHIRExprKind::BuiltinStringIsEmpty { val } => CHIRExprKind::BuiltinStringIsEmpty {
            val: Box::new(sl(*val)),
        },
        other => other,
    };
    CHIRExpr { kind: new_kind, ..expr }
}

fn substitute_local_block(block: CHIRBlock, idx: u32, replacement: &CHIRExpr) -> CHIRBlock {
    CHIRBlock {
        stmts: block.stmts.into_iter()
            .map(|s| substitute_local_stmt(s, idx, replacement))
            .collect(),
        result: block.result.map(|r| Box::new(substitute_local(*r, idx, replacement))),
    }
}

fn substitute_local_stmt(stmt: CHIRStmt, idx: u32, replacement: &CHIRExpr) -> CHIRStmt {
    let sl = |e: CHIRExpr| substitute_local(e, idx, replacement);
    let sl_block = |b: CHIRBlock| substitute_local_block(b, idx, replacement);
    match stmt {
        CHIRStmt::Let { local_idx, value } => CHIRStmt::Let { local_idx, value: sl(value) },
        CHIRStmt::Assign { target, value } => CHIRStmt::Assign { target, value: sl(value) },
        CHIRStmt::Expr(e) => CHIRStmt::Expr(sl(e)),
        CHIRStmt::Return(Some(e)) => CHIRStmt::Return(Some(sl(e))),
        CHIRStmt::While { cond, body } => CHIRStmt::While { cond: sl(cond), body: sl_block(body) },
        CHIRStmt::Loop { body } => CHIRStmt::Loop { body: sl_block(body) },
        other => other,
    }
}

/// 在单个 block 内做线性扫描，消除满足条件的 Let
fn eliminate_in_block(
    block: &mut CHIRBlock,
    usage: &HashMap<u32, (u32, u32)>,
    param_count: u32,
) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < block.stmts.len() {
        // 先递归处理子 block
        match &mut block.stmts[i] {
            CHIRStmt::While { body, .. } => {
                let c = eliminate_in_block(body, usage, param_count);
                changed |= c;
            }
            CHIRStmt::Loop { body } => {
                let c = eliminate_in_block(body, usage, param_count);
                changed |= c;
            }
            _ => {}
        }

        // 检查是否是可消除的 Let
        let can_elim = if let CHIRStmt::Let { local_idx, value } = &block.stmts[i] {
            let lidx = *local_idx;
            if lidx < param_count {
                false
            } else {
                let (writes, reads) = usage.get(&lidx).copied().unwrap_or((0, 0));
                writes == 1 && reads == 1 && is_pure(value)
            }
        } else {
            false
        };

        if can_elim {
            let (local_idx, value) = if let CHIRStmt::Let { local_idx, value } = block.stmts.remove(i) {
                (local_idx, value)
            } else {
                unreachable!()
            };

            // 在紧邻下一条 stmt 或 block.result 中替换
            let mut substituted = false;
            if i < block.stmts.len() {
                let reads_in_next = count_local_reads_in_stmt(&block.stmts[i], local_idx);
                if reads_in_next == 1 {
                    let stmt = block.stmts.remove(i);
                    let new_stmt = subst_in_stmt(stmt, local_idx, &value);
                    block.stmts.insert(i, new_stmt);
                    substituted = true;
                    changed = true;
                }
            }
            if !substituted {
                if let Some(result) = &block.result {
                    let reads = count_local_reads_in_expr(result, local_idx);
                    if reads == 1 {
                        let result_owned = *block.result.take().unwrap();
                        block.result = Some(Box::new(substitute_local(result_owned, local_idx, &value)));
                        substituted = true;
                        changed = true;
                    }
                }
            }
            if !substituted {
                // 无法替换，放回
                block.stmts.insert(i, CHIRStmt::Let { local_idx, value });
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    changed
}

fn subst_in_stmt(stmt: CHIRStmt, idx: u32, replacement: &CHIRExpr) -> CHIRStmt {
    substitute_local_stmt(stmt, idx, replacement)
}

fn eliminate_redundant_locals(program: &mut CHIRProgram) {
    for func in &mut program.functions {
        let param_count = func.params.len() as u32;
        let usage = count_local_usage_in_function(func);
        loop {
            let changed = eliminate_in_block(&mut func.body, &usage, param_count);
            if !changed {
                break;
            }
        }
    }
}

/// 扫描函数体中所有 Let/Assign 写入的最大 local_idx（不含参数）
fn max_written_local_in_block(block: &CHIRBlock) -> Option<u32> {
    let mut max: Option<u32> = None;
    for stmt in &block.stmts {
        let m = max_written_local_in_stmt(stmt);
        max = opt_max(max, m);
    }
    max
}

fn max_written_local_in_stmt(stmt: &CHIRStmt) -> Option<u32> {
    match stmt {
        CHIRStmt::Let { local_idx, .. } => Some(*local_idx),
        CHIRStmt::Assign { target: CHIRLValue::Local(idx), .. } => Some(*idx),
        CHIRStmt::While { body, .. } | CHIRStmt::Loop { body } => {
            max_written_local_in_block(body)
        }
        _ => None,
    }
}

fn opt_max(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn inline_small_functions(program: &mut CHIRProgram) {
    // 构建可内联函数表（深拷贝，避免借用冲突）
    let mut inlinable: HashMap<u32, CHIRFunction> = HashMap::new();
    for (i, func) in program.functions.iter().enumerate() {
        let func_idx = IMPORT_COUNT + i as u32;
        if !is_inlinable(func) {
            continue;
        }
        // 检查不递归调用自身
        let result = match get_inlinable_result(func) {
            Some(r) => r,
            None => continue,
        };
        if has_call_to(result, func_idx) {
            continue;
        }
        inlinable.insert(func_idx, func.clone());
    }

    if inlinable.is_empty() {
        return;
    }

    // 对每个函数执行内联
    for func in &mut program.functions {
        let param_count = func.params.len() as u32;
        // 从 local_wasm_types 计算已分配的最大 local_idx（这是权威来源）
        let wasm_types_max = func.local_wasm_types.keys().copied().max();
        let body_max = max_written_local_in_block(&func.body);
        let mut next_local = wasm_types_max
            .map_or(param_count, |m| m + 1)
            .max(body_max.map_or(0, |m| m + 1))
            .max(param_count);
        let mut extra_locals: Vec<(u32, ValType)> = Vec::new();

        inline_in_block(&mut func.body, &inlinable, &mut next_local, &mut extra_locals);

        // 将新增 locals 注册到函数
        for (idx, wasm_ty) in extra_locals {
            use crate::ast::Type;
            let ty = match wasm_ty {
                ValType::I32 => Type::Int32,
                ValType::I64 => Type::Int64,
                ValType::F32 => Type::Float32,
                ValType::F64 => Type::Float64,
                _ => Type::Int64,
            };
            func.locals.push(CHIRLocal {
                name: format!("__inline_{}", idx),
                ty,
                wasm_ty,
                local_idx: idx,
            });
            func.local_wasm_types.insert(idx, wasm_ty);
        }
    }
}
