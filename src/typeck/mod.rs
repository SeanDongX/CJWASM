//! 函数级类型预解析模块（多遍编译架构 Pass 5）
//!
//! 在 CodeGen 为每个函数生成 WASM 指令之前，先对函数体做一次纯前向扫描，
//! 确定所有局部变量的精确 WASM 类型，避免 `collect_locals` 中 `I64` 错误回退。
//!
//! 设计原则：
//! - 纯只读扫描，不修改 AST
//! - 零 panic，遇到不确定类型统一回退到 `I32`（对象引用比 I64 更安全）
//! - 仅对 Integer 字面量赋值的变量才用 `I64`
//! - 与 CodeGen 解耦，通过 `FunctionTypeGlobal` 借用符号表引用

use crate::ast::{
    BinOp, Expr, Function as FuncDef, InterpolatePart, Pattern, Stmt, StructDef, Type,
};
use crate::codegen::ClassInfo;
use std::collections::HashMap;
use wasm_encoder::ValType;

/// 程序级只读符号视图（从 CodeGen 借用，不拷贝）
pub(crate) struct FunctionTypeGlobal<'a> {
    /// 函数名 → WASM 返回类型（None 表示 void），来自已注册的函数签名
    pub func_return_wasm_types: &'a HashMap<String, Option<ValType>>,
    /// 结构体定义
    pub structs: &'a HashMap<String, StructDef>,
    /// 类信息
    pub classes: &'a HashMap<String, ClassInfo>,
    /// 全局变量 AST 类型
    pub global_var_types: &'a HashMap<String, Type>,
}

/// 每函数类型上下文：记录函数内所有局部变量的解析后 WASM 类型
#[derive(Debug, Default)]
pub(crate) struct FunctionTypeContext {
    /// 局部变量名 → 精确 WASM 类型（预解析，在 collect_locals 前完成）
    pub local_types: HashMap<String, ValType>,
}

impl FunctionTypeContext {
    /// 对一个函数做前向类型扫描，返回其局部变量类型映射
    pub fn resolve(func: &FuncDef, global: &FunctionTypeGlobal<'_>) -> Self {
        let mut ctx = FunctionTypeContext::default();

        // 1. 参数：直接用声明类型
        for param in &func.params {
            let actual_ty = if param.variadic {
                Type::Array(Box::new(param.ty.clone()))
            } else {
                param.ty.clone()
            };
            ctx.local_types
                .insert(param.name.clone(), actual_ty.to_wasm());
        }

        // 2. 扫描函数体
        for stmt in &func.body {
            scan_stmt(stmt, &mut ctx, global);
        }

        ctx
    }
}

// ─── 内部扫描函数 ────────────────────────────────────────────────────────────

fn scan_stmt(stmt: &Stmt, ctx: &mut FunctionTypeContext, global: &FunctionTypeGlobal<'_>) {
    match stmt {
        Stmt::Let { pattern, ty, value } | Stmt::Var { pattern, ty, value } => {
            if let Pattern::Binding(name) = pattern {
                let wt = if let Some(t) = ty {
                    t.to_wasm()
                } else {
                    resolve_expr_type(value, ctx, global)
                };
                ctx.local_types.entry(name.clone()).or_insert(wt);
            }
            // 元组解构：子变量用 I64（与旧行为兼容，codegen 会再做协调）
            if let Pattern::Tuple(pats) = pattern {
                for pat in pats {
                    if let Pattern::Binding(name) = pat {
                        ctx.local_types.entry(name.clone()).or_insert(ValType::I64);
                    }
                }
            }
            // 递归扫描 value 中的嵌套 let
            scan_expr(value, ctx, global);
        }
        Stmt::Const { name, ty, value } => {
            let wt = if let Some(t) = ty {
                t.to_wasm()
            } else {
                resolve_expr_type(value, ctx, global)
            };
            ctx.local_types.entry(name.clone()).or_insert(wt);
            scan_expr(value, ctx, global);
        }
        Stmt::Assign { value, .. } => {
            scan_expr(value, ctx, global);
        }
        Stmt::Expr(e) => {
            scan_expr(e, ctx, global);
        }
        Stmt::Return(Some(e)) => {
            scan_expr(e, ctx, global);
        }
        Stmt::While { cond, body } => {
            scan_expr(cond, ctx, global);
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Stmt::DoWhile { body, cond } => {
            for s in body {
                scan_stmt(s, ctx, global);
            }
            scan_expr(cond, ctx, global);
        }
        Stmt::Loop { body } | Stmt::UnsafeBlock { body } => {
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Stmt::For { var, iterable, body } => {
            // for 循环变量类型：Range → I64（整数索引），数组 → 元素类型
            let loop_var_ty = resolve_for_loop_var_type(iterable, ctx, global);
            ctx.local_types.entry(var.clone()).or_insert(loop_var_ty);
            // 数组 for 循环的内部临时变量
            if !matches!(iterable, Expr::Range { .. }) {
                ctx.local_types
                    .entry(format!("__{}_idx", var))
                    .or_insert(ValType::I64);
                ctx.local_types
                    .entry(format!("__{}_len", var))
                    .or_insert(ValType::I64);
                ctx.local_types
                    .entry(format!("__{}_arr", var))
                    .or_insert(ValType::I32);
            }
            scan_expr(iterable, ctx, global);
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Stmt::WhileLet { pattern, expr, body } => {
            scan_expr(expr, ctx, global);
            // while let 绑定变量：保守用 I32（枚举载荷通常是引用）
            if let Pattern::Binding(name) = pattern {
                ctx.local_types.entry(name.clone()).or_insert(ValType::I32);
            }
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
            scan_expr(left, ctx, global);
            scan_expr(right, ctx, global);
        }
        _ => {}
    }
}

fn scan_expr(expr: &Expr, ctx: &mut FunctionTypeContext, global: &FunctionTypeGlobal<'_>) {
    match expr {
        Expr::If { cond, then_branch, else_branch, .. } => {
            scan_expr(cond, ctx, global);
            scan_expr(then_branch, ctx, global);
            if let Some(eb) = else_branch {
                scan_expr(eb, ctx, global);
            }
        }
        Expr::Block(stmts, trailing) => {
            for s in stmts {
                scan_stmt(s, ctx, global);
            }
            if let Some(te) = trailing {
                scan_expr(te, ctx, global);
            }
        }
        Expr::Match { expr: sub, arms } => {
            scan_expr(sub, ctx, global);
            for arm in arms {
                collect_pattern_vars(&arm.pattern, ctx);
                scan_expr(&arm.body, ctx, global);
                if let Some(g) = &arm.guard {
                    scan_expr(g, ctx, global);
                }
            }
        }
        Expr::IfLet { pattern, expr, then_branch, else_branch } => {
            scan_expr(expr, ctx, global);
            collect_pattern_vars(pattern, ctx);
            scan_expr(then_branch, ctx, global);
            if let Some(eb) = else_branch {
                scan_expr(eb, ctx, global);
            }
        }
        Expr::Lambda { body, .. } => {
            scan_expr(body, ctx, global);
        }
        Expr::TryBlock { resources, body, catch_body, catch_var, finally_body, .. } => {
            for (res_name, res_expr) in resources {
                let wt = resolve_expr_type(res_expr, ctx, global);
                ctx.local_types.entry(res_name.clone()).or_insert(wt);
                scan_expr(res_expr, ctx, global);
            }
            for s in body {
                scan_stmt(s, ctx, global);
            }
            if let Some(cv) = catch_var {
                ctx.local_types.entry(cv.clone()).or_insert(ValType::I32);
            }
            for s in catch_body {
                scan_stmt(s, ctx, global);
            }
            if let Some(fb) = finally_body {
                for s in fb {
                    scan_stmt(s, ctx, global);
                }
            }
        }
        Expr::Spawn { body } => {
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Expr::Synchronized { lock, body } => {
            scan_expr(lock, ctx, global);
            for s in body {
                scan_stmt(s, ctx, global);
            }
        }
        Expr::Call { args, named_args, .. }
        | Expr::MethodCall { args, named_args, .. }
        | Expr::ConstructorCall { args, named_args, .. } => {
            for a in args {
                scan_expr(a, ctx, global);
            }
            for (_, a) in named_args {
                scan_expr(a, ctx, global);
            }
        }
        Expr::SuperCall { args, named_args, .. } => {
            for a in args {
                scan_expr(a, ctx, global);
            }
            for (_, a) in named_args {
                scan_expr(a, ctx, global);
            }
        }
        Expr::Binary { left, right, .. } => {
            scan_expr(left, ctx, global);
            scan_expr(right, ctx, global);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::IsType { expr: inner, .. }
        | Expr::Some(inner)
        | Expr::Ok(inner)
        | Expr::Err(inner)
        | Expr::Throw(inner)
        | Expr::Try(inner)
        | Expr::PostfixIncr(inner)
        | Expr::PostfixDecr(inner)
        | Expr::PrefixIncr(inner)
        | Expr::PrefixDecr(inner) => {
            scan_expr(inner, ctx, global);
        }
        Expr::Index { array: object, index } => {
            scan_expr(object, ctx, global);
            scan_expr(index, ctx, global);
        }
        Expr::Field { object, .. } | Expr::OptionalChain { object, .. } => {
            scan_expr(object, ctx, global);
        }
        Expr::TupleIndex { object, .. } => {
            scan_expr(object, ctx, global);
        }
        Expr::Array(elems) | Expr::Tuple(elems) => {
            for e in elems {
                scan_expr(e, ctx, global);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                scan_expr(e, ctx, global);
            }
        }
        Expr::NullCoalesce { option, default } => {
            scan_expr(option, ctx, global);
            scan_expr(default, ctx, global);
        }
        Expr::Range { start, end, step, .. } => {
            scan_expr(start, ctx, global);
            scan_expr(end, ctx, global);
            if let Some(s) = step {
                scan_expr(s, ctx, global);
            }
        }
        Expr::VariantConst { arg: Some(e), .. } => {
            scan_expr(e, ctx, global);
        }
        Expr::Interpolate(parts) => {
            for part in parts {
                if let InterpolatePart::Expr(e) = part {
                    scan_expr(e, ctx, global);
                }
            }
        }
        Expr::TrailingClosure { callee, args, closure } => {
            scan_expr(callee, ctx, global);
            for a in args {
                scan_expr(a, ctx, global);
            }
            scan_expr(closure, ctx, global);
        }
        Expr::Macro { args, .. } => {
            for a in args {
                scan_expr(a, ctx, global);
            }
        }
        _ => {}
    }
}

/// 收集 pattern 中的绑定变量（保守类型：I32）
fn collect_pattern_vars(pattern: &Pattern, ctx: &mut FunctionTypeContext) {
    match pattern {
        Pattern::Binding(name) => {
            ctx.local_types.entry(name.clone()).or_insert(ValType::I32);
        }
        Pattern::Tuple(pats) | Pattern::Or(pats) => {
            for p in pats {
                collect_pattern_vars(p, ctx);
            }
        }
        Pattern::Variant { payload: Some(p), .. } => {
            collect_pattern_vars(p, ctx);
        }
        Pattern::Struct { fields, .. } => {
            for (_, p) in fields {
                collect_pattern_vars(p, ctx);
            }
        }
        Pattern::TypeTest { binding, ty } => {
            ctx.local_types
                .entry(binding.clone())
                .or_insert(ty.to_wasm());
        }
        _ => {}
    }
}

// ─── 表达式类型推断（保守、仅用于 local 类型推断） ─────────────────────────

/// 对一个赋值右侧表达式推断其 WASM 类型。
/// 规则：整数字面量 → I64；其余未知 → I32（对象引用，比 I64 更安全）。
pub(crate) fn resolve_expr_type(
    expr: &Expr,
    ctx: &FunctionTypeContext,
    global: &FunctionTypeGlobal<'_>,
) -> ValType {
    match expr {
        // 整数字面量（唯一确定用 I64 的情况）
        Expr::Integer(_) => ValType::I64,
        // 浮点
        Expr::Float(_) => ValType::F64,
        Expr::Float32(_) => ValType::F32,
        // 其余值类型：Bool/Rune/String/Array/Struct 全是 I32
        Expr::Bool(_) | Expr::Rune(_) | Expr::String(_) | Expr::Interpolate(_) => ValType::I32,
        Expr::Array(_) | Expr::Tuple(_) | Expr::SliceExpr { .. } | Expr::Range { .. } => {
            ValType::I32
        }
        Expr::StructInit { .. } => ValType::I32,
        // 构造函数调用：原始类型构造函数与 Type::T.to_wasm() 保持一致
        Expr::ConstructorCall { name, .. } => match name.as_str() {
            "Int8" | "Int16" | "Int32" | "UInt8" | "UInt16" | "UInt32" => ValType::I32,
            "Int64" | "UInt64" | "IntNative" | "UIntNative" => ValType::I64,
            "Float16" | "Float32" => ValType::F32,
            "Float64" => ValType::F64,
            _ => ValType::I32, // 用户定义的类/结构体
        },
        Expr::Some(_) | Expr::None => ValType::I32,
        Expr::Ok(_) | Expr::Err(_) => ValType::I32,
        Expr::VariantConst { .. } => ValType::I32,
        // 变量：优先查已解析的 local_types，再查全局变量
        Expr::Var(name) => {
            if let Some(&vt) = ctx.local_types.get(name.as_str()) {
                return vt;
            }
            if let Some(ty) = global.global_var_types.get(name.as_str()) {
                return ty.to_wasm();
            }
            // 类名 / 结构体名 → I32 指针
            if global.structs.contains_key(name.as_str())
                || global.classes.contains_key(name.as_str())
            {
                return ValType::I32;
            }
            ValType::I32 // 未知变量保守 I32
        }
        // 函数调用：查签名表（需与 codegen::infer_type 保持一致）
        Expr::Call { name, args, .. } => {
            // 原始类型转换构造函数（Int64(x), Int32(x) 等），与 Type::T.to_wasm() 一致
            match name.as_str() {
                "Int8" | "Int16" | "Int32" | "UInt8" | "UInt16" | "UInt32" => {
                    return ValType::I32
                }
                "Int64" | "UInt64" | "IntNative" | "UIntNative" => return ValType::I64,
                "Float16" | "Float32" => return ValType::F32,
                "Float64" => return ValType::F64,
                _ => {}
            }
            // I/O 内置函数：void 返回
            if matches!(
                name.as_str(),
                "println" | "print" | "eprintln" | "eprint"
            ) {
                return ValType::I32;
            }
            // 字符串读取
            if name == "readln" {
                return ValType::I32;
            }
            // 数学内置函数 min/max (2参数) → I64 整数
            if (name == "min" || name == "max") && args.len() == 2 {
                return ValType::I64;
            }
            // abs (1参数) → I64 整数（注意：用户可重定义，先查签名）
            if name == "abs" && args.len() == 1 {
                // 优先查用户定义的 abs 签名
                if let Some(&rt) = global.func_return_wasm_types.get("abs") {
                    return rt.unwrap_or(ValType::I64);
                }
                return ValType::I64;
            }
            // 结构体/类构造函数 → I32 指针
            if global.structs.contains_key(name.as_str())
                || global.classes.contains_key(name.as_str())
            {
                return ValType::I32;
            }
            // 查已注册的函数签名
            global
                .func_return_wasm_types
                .get(name.as_str())
                .copied()
                .flatten()
                .unwrap_or(ValType::I32) // 未知函数保守 I32
        }
        // 方法调用：返回 I32（无法精确推断，codegen 层通过 AST 推断修正）
        Expr::MethodCall { .. } | Expr::SuperCall { .. } => ValType::I32,
        // 类型转换：使用目标类型
        Expr::Cast { target_ty, .. } => target_ty.to_wasm(),
        // IsType 返回 Bool (i32)
        Expr::IsType { .. } => ValType::I32,
        // 二元运算：比较运算 → I32，算术运算 → 取左右侧类型中最大的
        Expr::Binary { op, left, right } => match op {
            BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::LtEq
            | BinOp::Gt
            | BinOp::GtEq
            | BinOp::LogicalAnd
            | BinOp::LogicalOr
            | BinOp::NotIn => ValType::I32, // 比较 → Bool (i32)
            _ => {
                let lt = resolve_expr_type(left, ctx, global);
                let rt = resolve_expr_type(right, ctx, global);
                // 如果其中有 I64 参与，整体认为是 I64
                if lt == ValType::I64 || rt == ValType::I64 {
                    ValType::I64
                } else {
                    lt
                }
            }
        },
        // 一元运算：取操作数类型
        Expr::Unary { expr: inner, .. } => resolve_expr_type(inner, ctx, global),
        // If 表达式：取 then 分支类型
        Expr::If { then_branch, .. } => resolve_expr_type(then_branch, ctx, global),
        // Block：取最后一个 Expr 语句的类型，或 trailing expr 类型
        Expr::Block(stmts, trailing) => {
            if let Some(te) = trailing {
                return resolve_expr_type(te, ctx, global);
            }
            if let Some(Stmt::Expr(last)) = stmts.last() {
                resolve_expr_type(last, ctx, global)
            } else {
                ValType::I32
            }
        }
        // Lambda → I32（函数引用 / table index）
        Expr::Lambda { .. } => ValType::I32,
        // 前后缀运算：类型与操作数一致
        Expr::PostfixIncr(inner)
        | Expr::PostfixDecr(inner)
        | Expr::PrefixIncr(inner)
        | Expr::PrefixDecr(inner) => resolve_expr_type(inner, ctx, global),
        // NullCoalesce：取默认值的类型
        Expr::NullCoalesce { default, .. } => resolve_expr_type(default, ctx, global),
        // 其他全部 → I32
        _ => ValType::I32,
    }
}

/// 推断 for 循环变量类型
fn resolve_for_loop_var_type(
    iterable: &Expr,
    ctx: &FunctionTypeContext,
    global: &FunctionTypeGlobal<'_>,
) -> ValType {
    match iterable {
        Expr::Range { .. } => ValType::I64, // Range 迭代 → i64 整数
        Expr::Array(elems) => {
            // 数组字面量：元素类型
            elems
                .first()
                .map(|e| resolve_expr_type(e, ctx, global))
                .unwrap_or(ValType::I32)
        }
        _ => ValType::I64, // 未知迭代器 → 保守 I64（与旧行为兼容）
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Type;

    fn empty_global<'a>(
        frwt: &'a HashMap<String, Option<ValType>>,
        structs: &'a HashMap<String, StructDef>,
        classes: &'a HashMap<String, ClassInfo>,
        gvt: &'a HashMap<String, Type>,
    ) -> FunctionTypeGlobal<'a> {
        FunctionTypeGlobal {
            func_return_wasm_types: frwt,
            structs,
            classes,
            global_var_types: gvt,
        }
    }

    #[test]
    fn test_integer_literal_resolves_to_i64() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        assert_eq!(
            resolve_expr_type(&Expr::Integer(42), &ctx, &global),
            ValType::I64
        );
    }

    #[test]
    fn test_bool_literal_resolves_to_i32() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        assert_eq!(
            resolve_expr_type(&Expr::Bool(true), &ctx, &global),
            ValType::I32
        );
    }

    #[test]
    fn test_unknown_var_resolves_to_i32() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        // 未知变量应回退到 I32 而不是 I64
        assert_eq!(
            resolve_expr_type(&Expr::Var("unknown".to_string()), &ctx, &global),
            ValType::I32
        );
    }

    #[test]
    fn test_let_with_integer_value_resolves_to_i64() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        // func f() { let x = 42; }
        let func = FuncDef {
            visibility: crate::ast::Visibility::Private,
            name: "f".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![],
            return_type: None,
            body: vec![Stmt::Let {
                pattern: Pattern::Binding("x".to_string()),
                ty: None,
                value: Expr::Integer(42),
            }],
            extern_import: None,
            throws: None,
        };
        let result = FunctionTypeContext::resolve(&func, &global);
        assert_eq!(result.local_types.get("x"), Some(&ValType::I64));
    }

    #[test]
    fn test_const_with_bool_value_resolves_to_i32() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let func = FuncDef {
            visibility: crate::ast::Visibility::Private,
            name: "f".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![],
            return_type: None,
            body: vec![Stmt::Const {
                name: "FLAG".to_string(),
                ty: None,
                value: Expr::Bool(true),
            }],
            extern_import: None,
            throws: None,
        };
        let result = FunctionTypeContext::resolve(&func, &global);
        assert_eq!(result.local_types.get("FLAG"), Some(&ValType::I32));
    }

    #[test]
    fn test_var_from_local_types_resolves() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let mut gvt = HashMap::new();
        gvt.insert("g".to_string(), Type::Int64);
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let func = FuncDef {
            visibility: crate::ast::Visibility::Private,
            name: "f".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![crate::ast::Param {
                name: "x".to_string(),
                ty: Type::Int64,
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            return_type: None,
            body: vec![Stmt::Let {
                pattern: Pattern::Binding("y".to_string()),
                ty: None,
                value: Expr::Var("x".to_string()),
            }],
            extern_import: None,
            throws: None,
        };
        let result = FunctionTypeContext::resolve(&func, &global);
        assert_eq!(result.local_types.get("y"), Some(&ValType::I64));
    }

    #[test]
    fn test_let_with_tuple_pattern() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let func = FuncDef {
            visibility: crate::ast::Visibility::Private,
            name: "f".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![],
            return_type: None,
            body: vec![Stmt::Let {
                pattern: Pattern::Tuple(vec![
                    Pattern::Binding("a".to_string()),
                    Pattern::Binding("b".to_string()),
                ]),
                ty: None,
                value: Expr::Tuple(vec![Expr::Integer(1), Expr::Integer(2)]),
            }],
            extern_import: None,
            throws: None,
        };
        let result = FunctionTypeContext::resolve(&func, &global);
        assert_eq!(result.local_types.get("a"), Some(&ValType::I64));
        assert_eq!(result.local_types.get("b"), Some(&ValType::I64));
    }

    #[test]
    fn test_for_with_array_iterable() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let func = FuncDef {
            visibility: crate::ast::Visibility::Private,
            name: "f".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![],
            return_type: None,
            body: vec![Stmt::For {
                var: "x".to_string(),
                iterable: Expr::Array(vec![Expr::Integer(1), Expr::Integer(2)]),
                body: vec![],
            }],
            extern_import: None,
            throws: None,
        };
        let result = FunctionTypeContext::resolve(&func, &global);
        assert_eq!(result.local_types.get("x"), Some(&ValType::I64));
        assert_eq!(result.local_types.get("__x_idx"), Some(&ValType::I64));
        assert_eq!(result.local_types.get("__x_len"), Some(&ValType::I64));
        assert_eq!(result.local_types.get("__x_arr"), Some(&ValType::I32));
    }

    #[test]
    fn test_resolve_expr_call_int64_constructor() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        let call = Expr::Call {
            name: "Int64".to_string(),
            type_args: None,
            args: vec![Expr::Integer(42)],
            named_args: vec![],
        };
        assert_eq!(resolve_expr_type(&call, &ctx, &global), ValType::I64);
    }

    #[test]
    fn test_resolve_expr_call_println() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        let call = Expr::Call {
            name: "println".to_string(),
            type_args: None,
            args: vec![Expr::Integer(1)],
            named_args: vec![],
        };
        assert_eq!(resolve_expr_type(&call, &ctx, &global), ValType::I32);
    }

    #[test]
    fn test_resolve_expr_binary_add_i64() {
        let frwt = HashMap::new();
        let structs = HashMap::new();
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        let bin = Expr::Binary {
            op: crate::ast::BinOp::Add,
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(resolve_expr_type(&bin, &ctx, &global), ValType::I64);
    }

    #[test]
    fn test_resolve_expr_struct_name_in_global() {
        let frwt = HashMap::new();
        let mut structs = HashMap::new();
        structs.insert(
            "Point".to_string(),
            crate::ast::StructDef {
                visibility: crate::ast::Visibility::default(),
                name: "Point".to_string(),
                type_params: vec![],
                constraints: vec![],
                fields: vec![],
            },
        );
        let classes = HashMap::new();
        let gvt = HashMap::new();
        let global = empty_global(&frwt, &structs, &classes, &gvt);
        let ctx = FunctionTypeContext::default();
        assert_eq!(
            resolve_expr_type(&Expr::Var("Point".to_string()), &ctx, &global),
            ValType::I32
        );
    }
}
