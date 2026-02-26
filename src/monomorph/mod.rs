//! 泛型单态化：将泛型函数与结构体按类型实参生成特化版本。

use crate::ast::{ClassDef, ClassMethod, EnumDef, EnumVariant, Expr, FieldDef, Function, InitDef, MatchArm, Param, Pattern, Program, Stmt, StructDef};
use std::collections::{HashMap, HashSet};
use crate::ast::Type;

/// 类型实参到名字修饰后缀
fn type_mangle_suffix(ty: &Type) -> String {
    match ty {
        Type::Int8 => "Int8".to_string(),
        Type::Int16 => "Int16".to_string(),
        Type::Int32 => "Int32".to_string(),
        Type::Int64 => "Int64".to_string(),
        Type::UInt8 => "UInt8".to_string(),
        Type::UInt16 => "UInt16".to_string(),
        Type::UInt32 => "UInt32".to_string(),
        Type::UInt64 => "UInt64".to_string(),
        Type::Float32 => "Float32".to_string(),
        Type::Float64 => "Float64".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Rune => "Rune".to_string(),
        Type::IntNative => "IntNative".to_string(),
        Type::UIntNative => "UIntNative".to_string(),
        Type::Float16 => "Float16".to_string(),
        Type::Nothing => "Nothing".to_string(),
        Type::Unit => "Unit".to_string(),
        Type::String => "String".to_string(),
        Type::Array(inner) => format!("Array_{}", type_mangle_suffix(inner)),
        Type::Tuple(types) => format!("Tuple_{}", types.iter().map(type_mangle_suffix).collect::<Vec<_>>().join("_")),
        Type::Struct(s, args) => {
            if args.is_empty() {
                s.clone()
            } else {
                format!(
                    "{}_{}",
                    s,
                    args.iter().map(type_mangle_suffix).collect::<Vec<_>>().join("_")
                )
            }
        }
        Type::Range => "Range".to_string(),
        Type::Function { params, ret } => {
            let params_str = params
                .iter()
                .map(type_mangle_suffix)
                .collect::<Vec<_>>()
                .join("_");
            let ret_str = ret
                .as_ref()
                .as_ref()
                .map(type_mangle_suffix)
                .unwrap_or_else(|| "Unit".to_string());
            format!("Fn_{}_{}", params_str, ret_str)
        }
        Type::Option(inner) => format!("Option_{}", type_mangle_suffix(inner)),
        Type::Result(ok, err) => {
            format!(
                "Result_{}_{}",
                type_mangle_suffix(ok),
                type_mangle_suffix(err)
            )
        }
        Type::TypeParam(name) => name.clone(),
        Type::Slice(inner) => format!("Slice_{}", type_mangle_suffix(inner)),
        Type::Map(k, v) => format!(
            "Map_{}_{}",
            type_mangle_suffix(k),
            type_mangle_suffix(v)
        ),
    }
}

/// 生成单态化后的名字：name$T1$T2
pub fn mangle_name(name: &str, type_args: &[Type]) -> String {
    if type_args.is_empty() {
        format!("{}$_", name)
    } else {
        format!(
            "{}${}",
            name,
            type_args
                .iter()
                .map(type_mangle_suffix)
                .collect::<Vec<_>>()
                .join("$")
        )
    }
}

/// 在类型中替换 TypeParam
fn substitute_type(ty: &Type, subst: &HashMap<String, Type>) -> Type {
    match ty {
        Type::TypeParam(name) => subst
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute_type(inner, subst))),
        Type::Tuple(types) => Type::Tuple(types.iter().map(|t| substitute_type(t, subst)).collect()),
        Type::Struct(name, args) => Type::Struct(
            name.clone(),
            args.iter().map(|t| substitute_type(t, subst)).collect(),
        ),
        Type::Function { params, ret } => Type::Function {
            params: params.iter().map(|t| substitute_type(t, subst)).collect(),
            ret: Box::new(match ret.as_ref() {
                Some(t) => Some(substitute_type(t, subst)),
                None => None,
            }),
        },
        Type::Option(inner) => Type::Option(Box::new(substitute_type(inner, subst))),
        Type::Result(ok, err) => Type::Result(
            Box::new(substitute_type(ok, subst)),
            Box::new(substitute_type(err, subst)),
        ),
        Type::Slice(inner) => Type::Slice(Box::new(substitute_type(inner, subst))),
        Type::Map(key, val) => Type::Map(
            Box::new(substitute_type(key, subst)),
            Box::new(substitute_type(val, subst)),
        ),
        _ => ty.clone(),
    }
}

/// 替换表达式中的类型与引用
fn substitute_expr(expr: Expr, subst: &HashMap<String, Type>, rewrites: &RewriteMap) -> Expr {
    use crate::ast::Expr::*;
    match expr {
        Call {
            name,
            type_args: ta_opt,
            args,
            named_args,
        } => {
            let new_name = match ta_opt.as_ref() {
                Option::Some(tas) => rewrites
                    .func_rewrites
                    .get(&(name.clone(), tas.clone()))
                    .cloned()
                    .unwrap_or(name),
                Option::None => name,
            };
            Call {
                name: new_name,
                type_args: Option::None,
                args: args
                    .into_iter()
                    .map(|a| substitute_expr(a, subst, rewrites))
                    .collect(),
                named_args: named_args
                    .into_iter()
                    .map(|(n, e)| (n, substitute_expr(e, subst, rewrites)))
                    .collect(),
            }
        }
        StructInit {
            name,
            type_args: ta_opt,
            fields,
        } => {
            let (new_name, new_type_args) = match ta_opt.as_ref() {
                Option::Some(tas) => rewrites
                    .struct_rewrites
                    .get(&(name.clone(), tas.clone()))
                    .cloned()
                    .map(|n| (n, Option::None))
                    .unwrap_or_else(|| (name.clone(), ta_opt)),
                Option::None => (name, ta_opt),
            };
            StructInit {
                name: new_name,
                type_args: new_type_args,
                fields: fields
                    .into_iter()
                    .map(|(k, v)| (k, substitute_expr(v, subst, rewrites)))
                    .collect(),
            }
        }
        ConstructorCall {
            name,
            type_args: ta_opt,
            args,
            named_args,
        } => {
            let (new_name, new_type_args) = match ta_opt.as_ref() {
                Option::Some(tas) => rewrites
                    .struct_rewrites
                    .get(&(name.clone(), tas.clone()))
                    .cloned()
                    .map(|n| (n, Option::None))
                    .unwrap_or_else(|| (name.clone(), ta_opt)),
                Option::None => (name, ta_opt),
            };
            ConstructorCall {
                name: new_name,
                type_args: new_type_args,
                args: args
                    .into_iter()
                    .map(|a| substitute_expr(a, subst, rewrites))
                    .collect(),
                named_args: named_args
                    .into_iter()
                    .map(|(n, e)| (n, substitute_expr(e, subst, rewrites)))
                    .collect(),
            }
        }
        Unary { op, expr } => Unary {
            op,
            expr: Box::new(substitute_expr(*expr, subst, rewrites)),
        },
        Binary {
            op,
            left,
            right,
        } => Binary {
            op,
            left: Box::new(substitute_expr(*left, subst, rewrites)),
            right: Box::new(substitute_expr(*right, subst, rewrites)),
        },
        MethodCall {
            object,
            method,
            args,
            named_args,
        } => MethodCall {
            object: Box::new(substitute_expr(*object, subst, rewrites)),
            method,
            args: args
                .into_iter()
                .map(|a| substitute_expr(a, subst, rewrites))
                .collect(),
            named_args: named_args
                .into_iter()
                .map(|(n, e)| (n, substitute_expr(e, subst, rewrites)))
                .collect(),
        },
        SuperCall { method, args, named_args } => SuperCall {
            method,
            args: args
                .into_iter()
                .map(|a| substitute_expr(a, subst, rewrites))
                .collect(),
            named_args: named_args
                .into_iter()
                .map(|(n, e)| (n, substitute_expr(e, subst, rewrites)))
                .collect(),
        },
        If {
            cond,
            then_branch,
            else_branch,
        } => If {
            cond: Box::new(substitute_expr(*cond, subst, rewrites)),
            then_branch: Box::new(substitute_expr(*then_branch, subst, rewrites)),
            else_branch: else_branch
                .map(|e| Box::new(substitute_expr(*e, subst, rewrites))),
        },
        IfLet {
            pattern,
            expr,
            then_branch,
            else_branch,
        } => IfLet {
            pattern: substitute_pattern(pattern, subst, rewrites),
            expr: Box::new(substitute_expr(*expr, subst, rewrites)),
            then_branch: Box::new(substitute_expr(*then_branch, subst, rewrites)),
            else_branch: else_branch
                .map(|e| Box::new(substitute_expr(*e, subst, rewrites))),
        },
        Block(stmts, expr) => Block(
            stmts
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            expr.map(|e| Box::new(substitute_expr(*e, subst, rewrites))),
        ),
        Array(elems) => Array(
            elems
                .into_iter()
                .map(|e| substitute_expr(e, subst, rewrites))
                .collect(),
        ),
        Index { array, index } => Index {
            array: Box::new(substitute_expr(*array, subst, rewrites)),
            index: Box::new(substitute_expr(*index, subst, rewrites)),
        },
        Field { object, field } => Field {
            object: Box::new(substitute_expr(*object, subst, rewrites)),
            field,
        },
        Range {
            start,
            end,
            inclusive,
            step,
        } => Range {
            start: Box::new(substitute_expr(*start, subst, rewrites)),
            end: Box::new(substitute_expr(*end, subst, rewrites)),
            inclusive,
            step: step.map(|s| Box::new(substitute_expr(*s, subst, rewrites))),
        },
        VariantConst {
            enum_name,
            variant_name,
            arg,
        } => VariantConst {
            enum_name,
            variant_name,
            arg: arg.map(|e| Box::new(substitute_expr(*e, subst, rewrites))),
        },
        Match { expr, arms } => Match {
            expr: Box::new(substitute_expr(*expr, subst, rewrites)),
            arms: arms
                .into_iter()
                .map(|a| substitute_match_arm(a, subst, rewrites))
                .collect(),
        },
        Cast { expr, target_ty } => Cast {
            expr: Box::new(substitute_expr(*expr, subst, rewrites)),
            target_ty: substitute_type(&target_ty, subst),
        },
        Lambda {
            params,
            return_type,
            body,
        } => Lambda {
            params: params
                .into_iter()
                .map(|(n, t)| (n, substitute_type(&t, subst)))
                .collect(),
            return_type: return_type.map(|t| substitute_type(&t, subst)),
            body: Box::new(substitute_expr(*body, subst, rewrites)),
        },
        Some(inner) => Some(Box::new(substitute_expr(*inner, subst, rewrites))),
        Ok(inner) => Ok(Box::new(substitute_expr(*inner, subst, rewrites))),
        Err(inner) => Err(Box::new(substitute_expr(*inner, subst, rewrites))),
        Try(inner) => Try(Box::new(substitute_expr(*inner, subst, rewrites))),
        Throw(inner) => Throw(Box::new(substitute_expr(*inner, subst, rewrites))),
        TryBlock {
            resources,
            body,
            catch_var,
            catch_body,
            finally_body,
        } => TryBlock {
            resources: resources
                .into_iter()
                .map(|(n, e)| (n, substitute_expr(e, subst, rewrites)))
                .collect(),
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            catch_var,
            catch_body: catch_body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            finally_body: finally_body.map(|stmts| {
                stmts
                    .into_iter()
                    .map(|s| substitute_stmt(s, subst, rewrites))
                    .collect()
            }),
        },
        Tuple(elems) => Tuple(
            elems
                .into_iter()
                .map(|e| substitute_expr(e, subst, rewrites))
                .collect(),
        ),
        TupleIndex { object, index } => TupleIndex {
            object: Box::new(substitute_expr(*object, subst, rewrites)),
            index,
        },
        NullCoalesce { option, default } => NullCoalesce {
            option: Box::new(substitute_expr(*option, subst, rewrites)),
            default: Box::new(substitute_expr(*default, subst, rewrites)),
        },
        SliceExpr { array, start, end } => SliceExpr {
            array: Box::new(substitute_expr(*array, subst, rewrites)),
            start: Box::new(substitute_expr(*start, subst, rewrites)),
            end: Box::new(substitute_expr(*end, subst, rewrites)),
        },
        PostfixIncr(inner) => PostfixIncr(Box::new(substitute_expr(*inner, subst, rewrites))),
        PostfixDecr(inner) => PostfixDecr(Box::new(substitute_expr(*inner, subst, rewrites))),
        Break => Break,
        Continue => Continue,
        MapLiteral { entries } => MapLiteral {
            entries: entries
                .into_iter()
                .map(|(k, v)| (substitute_expr(k, subst, rewrites), substitute_expr(v, subst, rewrites)))
                .collect(),
        },
        Rune(c) => Rune(c),
        Var(n) => Var(n),
        Integer(i) => Integer(i),
        Float(f) => Float(f),
        Float32(f) => Float32(f),
        Bool(b) => Bool(b),
        String(s) => String(s),
        Interpolate(parts) => Interpolate(
            parts
                .into_iter()
                .map(|p| match p {
                    crate::ast::InterpolatePart::Literal(s) => crate::ast::InterpolatePart::Literal(s),
                    crate::ast::InterpolatePart::Expr(e) => {
                        crate::ast::InterpolatePart::Expr(Box::new(substitute_expr(*e, subst, rewrites)))
                    }
                })
                .collect(),
        ),
        Expr::None => Expr::None,
        e => e,
    }
}

fn substitute_pattern(
    pattern: Pattern,
    subst: &HashMap<String, Type>,
    rewrites: &RewriteMap,
) -> Pattern {
    use crate::ast::Pattern::*;
    match pattern {
        Struct { name, fields } => {
            let new_name = rewrites
                .struct_pattern_rewrites
                .get(&name)
                .cloned()
                .unwrap_or(name);
            Struct {
                name: new_name,
                fields: fields
                    .into_iter()
                    .map(|(k, p)| (k, substitute_pattern(p, subst, rewrites)))
                    .collect(),
            }
        }
        Variant {
            enum_name,
            variant_name,
            bindings,
        } => Variant {
            enum_name,
            variant_name,
            bindings,
        },
        Or(ps) => Or(ps
            .into_iter()
            .map(|p| substitute_pattern(p, subst, rewrites))
            .collect()),
        Tuple(ps) => Tuple(ps
            .into_iter()
            .map(|p| substitute_pattern(p, subst, rewrites))
            .collect()),
        Range {
            start,
            end,
            inclusive,
        } => Range {
            start,
            end,
            inclusive,
        },
        Literal(l) => Literal(l),
        Binding(n) => Binding(n),
        Wildcard => Wildcard,
        TypeTest { binding, ty } => TypeTest {
            binding,
            ty: substitute_type(&ty, subst),
        },
        Field { object, field } => Field { object, field },
        Guard(expr) => Guard(Box::new(substitute_expr(*expr, subst, rewrites))),
    }
}

fn substitute_match_arm(
    arm: MatchArm,
    subst: &HashMap<String, Type>,
    rewrites: &RewriteMap,
) -> MatchArm {
    MatchArm {
        pattern: substitute_pattern(arm.pattern, subst, rewrites),
        guard: arm
            .guard
            .map(|e| Box::new(substitute_expr(*e, subst, rewrites))),
        body: Box::new(substitute_expr(*arm.body, subst, rewrites)),
    }
}

fn substitute_stmt(stmt: Stmt, subst: &HashMap<String, Type>, rewrites: &RewriteMap) -> Stmt {
    use crate::ast::Stmt::*;
    match stmt {
        Let {
            pattern,
            ty,
            value,
        } => Let {
            pattern: substitute_pattern(pattern, subst, rewrites),
            ty: ty.map(|t| substitute_type(&t, subst)),
            value: substitute_expr(value, subst, rewrites),
        },
        Var { pattern, ty, value } => Var {
            pattern: substitute_pattern(pattern, subst, rewrites),
            ty: ty.map(|t| substitute_type(&t, subst)),
            value: value.map(|v| substitute_expr(v, subst, rewrites)),
        },
        Assign { target, value } => Assign {
            target,
            value: substitute_expr(value, subst, rewrites),
        },
        Expr(e) => Expr(substitute_expr(e, subst, rewrites)),
        Return(opt) => Return(opt.map(|e| substitute_expr(e, subst, rewrites))),
        While { cond, body } => While {
            cond: substitute_expr(cond, subst, rewrites),
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
        WhileLet {
            pattern,
            expr,
            body,
        } => WhileLet {
            pattern: substitute_pattern(pattern, subst, rewrites),
            expr: Box::new(substitute_expr(*expr, subst, rewrites)),
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
        For {
            var,
            iterable,
            body,
        } => For {
            var,
            iterable: substitute_expr(iterable, subst, rewrites),
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
        Loop { body } => Loop {
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
        Break => Break,
        Continue => Continue,
        Assert { left, right, line } => Assert {
            left: substitute_expr(left, subst, rewrites),
            right: substitute_expr(right, subst, rewrites),
            line,
        },
        Expect { left, right, line } => Expect {
            left: substitute_expr(left, subst, rewrites),
            right: substitute_expr(right, subst, rewrites),
            line,
        },
        DoWhile { body, cond } => DoWhile {
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            cond: substitute_expr(cond, subst, rewrites),
        },
        Const { name, ty, value } => Const {
            name,
            ty: ty.map(|t| substitute_type(&t, subst)),
            value: substitute_expr(value, subst, rewrites),
        },
        UnsafeBlock { body } => UnsafeBlock {
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
        LocalFunc(f) => LocalFunc(crate::ast::Function {
            visibility: f.visibility.clone(),
            name: f.name.clone(),
            type_params: f.type_params.clone(),
            constraints: f.constraints.clone(),
            params: f
                .params
                .iter()
                .map(|p| crate::ast::Param {
                    name: p.name.clone(),
                    ty: substitute_type(&p.ty, subst),
                    default: p.default.as_ref().map(|e| substitute_expr(e.clone(), subst, rewrites)),
                    variadic: p.variadic,
                    is_named: p.is_named,
                    is_inout: p.is_inout,
                })
                .collect(),
            return_type: f.return_type.as_ref().map(|t| substitute_type(t, subst)),
            throws: f.throws.clone(),
            body: f
                .body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            extern_import: f.extern_import.clone(),
        }),
    }
}

/// 重写映射：(原名, 类型实参) -> 单态化后名
struct RewriteMap {
    func_rewrites: HashMap<(String, Vec<Type>), String>,
    struct_rewrites: HashMap<(String, Vec<Type>), String>,
    struct_pattern_rewrites: HashMap<String, String>,
}

/// 收集所有需要的泛型实例化
fn collect_instantiations(program: &Program) -> (
    HashSet<(String, Vec<Type>)>,
    HashSet<(String, Vec<Type>)>,
    HashSet<(String, Vec<Type>)>,
    HashSet<(String, Vec<Type>)>,
) {
    let mut func_insts = HashSet::new();
    let mut struct_insts = HashSet::new();
    let mut enum_insts = HashSet::new();
    let mut class_insts = HashSet::new();

    let generic_functions: std::collections::HashMap<_, _> = program
        .functions
        .iter()
        .filter(|f| !f.type_params.is_empty() && f.extern_import.is_none())
        .map(|f| (f.name.clone(), f.type_params.len()))
        .collect();

    let generic_structs: std::collections::HashMap<_, _> = program
        .structs
        .iter()
        .filter(|s| !s.type_params.is_empty())
        .map(|s| (s.name.clone(), s.type_params.len()))
        .collect();

    let generic_enums: std::collections::HashMap<_, _> = program
        .enums
        .iter()
        .filter(|e| !e.type_params.is_empty())
        .map(|e| (e.name.clone(), e.type_params.len()))
        .collect();

    let generic_classes: std::collections::HashMap<_, _> = program
        .classes
        .iter()
        .filter(|c| !c.type_params.is_empty())
        .map(|c| (c.name.clone(), c.type_params.len()))
        .collect();

    fn visit_expr(
        expr: &Expr,
        gf: &std::collections::HashMap<String, usize>,
        gs: &std::collections::HashMap<String, usize>,
        ge: &std::collections::HashMap<String, usize>,
        gc: &std::collections::HashMap<String, usize>,
        fi: &mut HashSet<(String, Vec<Type>)>,
        si: &mut HashSet<(String, Vec<Type>)>,
        ei: &mut HashSet<(String, Vec<Type>)>,
        ci: &mut HashSet<(String, Vec<Type>)>,
    ) {
        use crate::ast::Expr::*;
        match expr {
            Call { name, type_args, .. } => {
                if let Option::Some(tas) = type_args.as_ref() {
                    if let Option::Some(n) = gf.get(name) {
                        if *n == tas.len() {
                            fi.insert((name.clone(), tas.clone()));
                        }
                    }
                }
            }
            StructInit { name, type_args, .. } => {
                if let Option::Some(tas) = type_args.as_ref() {
                    if let Option::Some(n) = gs.get(name) {
                        if *n == tas.len() {
                            si.insert((name.clone(), tas.clone()));
                        }
                    }
                    // 也检查泛型类
                    if let Option::Some(n) = gc.get(name) {
                        if *n == tas.len() {
                            ci.insert((name.clone(), tas.clone()));
                        }
                    }
                }
            }
            ConstructorCall { name, type_args, .. } => {
                if let Option::Some(tas) = type_args.as_ref() {
                    if let Option::Some(n) = gs.get(name) {
                        if *n == tas.len() {
                            si.insert((name.clone(), tas.clone()));
                        }
                    }
                    // 也检查泛型枚举
                    if let Option::Some(n) = ge.get(name) {
                        if *n == tas.len() {
                            ei.insert((name.clone(), tas.clone()));
                        }
                    }
                    // 也检查泛型类
                    if let Option::Some(n) = gc.get(name) {
                        if *n == tas.len() {
                            ci.insert((name.clone(), tas.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for func in &program.functions {
        for stmt in &func.body {
            let mut visit = |e: &Expr| visit_expr(e, &generic_functions, &generic_structs, &generic_enums, &generic_classes, &mut func_insts, &mut struct_insts, &mut enum_insts, &mut class_insts);
            stmt.walk(&mut visit);
        }
    }

    (func_insts, struct_insts, enum_insts, class_insts)
}

/// 为 Expr/Stmt 提供 walk
trait AstWalk {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F);
}

impl AstWalk for Box<Expr> {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F) {
        self.as_ref().walk(f);
    }
}

impl AstWalk for Expr {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F) {
        f(self);
        use crate::ast::Expr::*;
        match self {
            Unary { expr, .. } => expr.walk(f),
            Binary { left, right, .. } => {
                left.walk(f);
                right.walk(f);
            }
            Call { args, .. } => args.iter().for_each(|a| a.walk(f)),
            MethodCall { object, args, .. } => {
                object.walk(f);
                args.iter().for_each(|a| a.walk(f));
            }
            SuperCall { args, .. } => args.iter().for_each(|a| a.walk(f)),
            If {
                cond,
                then_branch,
                else_branch,
            } => {
                cond.walk(f);
                then_branch.walk(f);
                if let Option::Some(e) = else_branch.as_ref() {
                    e.as_ref().walk(f);
                }
            }
            IfLet {
                expr,
                then_branch,
                else_branch,
                ..
            } => {
                expr.walk(f);
                then_branch.walk(f);
                if let Option::Some(e) = else_branch.as_ref() {
                    e.as_ref().walk(f);
                }
            }
            Block(stmts, expr) => {
                for s in stmts {
                    s.walk(f);
                }
                if let Option::Some(e) = expr.as_ref() {
                    e.as_ref().walk(f);
                }
            }
            Array(elems) => elems.iter().for_each(|a| a.walk(f)),
            Tuple(elems) => elems.iter().for_each(|a| a.walk(f)),
            TupleIndex { object, .. } => object.walk(f),
            NullCoalesce { option, default } => {
                option.walk(f);
                default.walk(f);
            }
            Interpolate(parts) => {
                for p in parts {
                    if let crate::ast::InterpolatePart::Expr(ref e) = p {
                        e.as_ref().walk(f);
                    }
                }
            }
            Index { array, index } => {
                array.walk(f);
                index.walk(f);
            }
            StructInit { fields, .. } => {
                for (_, v) in fields {
                    v.walk(f);
                }
            }
            ConstructorCall { args, .. } => args.iter().for_each(|a| a.walk(f)),
            Field { object, .. } => object.walk(f),
            Range { start, end, .. } => {
                start.walk(f);
                end.walk(f);
            }
            VariantConst { arg, .. } => {
                if let Option::Some(e) = arg.as_ref() {
                    e.as_ref().walk(f);
                }
            }
            Match { expr, arms, .. } => {
                expr.walk(f);
                for a in arms {
                    a.body.as_ref().walk(f);
                    if let Option::Some(g) = a.guard.as_ref() {
                        g.as_ref().walk(f);
                    }
                }
            }
            Cast { expr, .. } => expr.walk(f),
            Lambda { body, .. } => body.walk(f),
            Some(e) | Ok(e) | Err(e) | Try(e) | Throw(e) => e.as_ref().walk(f),
            TryBlock { body, catch_body, .. } => {
                for s in body {
                    s.walk(f);
                }
                for s in catch_body {
                    s.walk(f);
                }
            }
            SliceExpr { array, start, end } => {
                array.walk(f);
                start.walk(f);
                end.walk(f);
            }
            PostfixIncr(inner) | PostfixDecr(inner) => inner.walk(f),
            Break | Continue => {}
            MapLiteral { entries } => {
                for (k, v) in entries {
                    k.walk(f);
                    v.walk(f);
                }
            }
            _ => {}
        }
    }
}

impl AstWalk for Stmt {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F) {
        use crate::ast::Stmt::*;
        match self {
            Let { value, .. } => value.walk(f),
            Var { value: Some(value), .. } => value.walk(f),
            Var { value: None, .. } => {}
            Expr(e) => e.walk(f),
            Assign { value, .. } => value.walk(f),
            Return(Some(e)) => e.walk(f),
            While { cond, body } => {
                cond.walk(f);
                for s in body {
                    s.walk(f);
                }
            }
            WhileLet { expr, body, .. } => {
                expr.walk(f);
                for s in body {
                    s.walk(f);
                }
            }
            For { iterable, body, .. } => {
                iterable.walk(f);
                for s in body {
                    s.walk(f);
                }
            }
            Loop { body } => {
                for s in body {
                    s.walk(f);
                }
            }
            _ => {}
        }
    }
}

trait StmtWalkExprs {
    fn walk_exprs<F: FnMut(&Expr)>(&self, f: &mut F);
}

impl StmtWalkExprs for Stmt {
    fn walk_exprs<F: FnMut(&Expr)>(&self, f: &mut F) {
        use crate::ast::Stmt::*;
        match self {
            Let { value, .. } => value.walk(f),
            Var { value: Some(value), .. } => value.walk(f),
            Var { value: None, .. } => {}
            Expr(e) => e.walk(f),
            Assign { value, .. } => value.walk(f),
            Return(Some(e)) => e.walk(f),
            While { cond, body } => {
                cond.walk(f);
                for s in body {
                    s.walk_exprs(f);
                }
            }
            WhileLet { expr, body, .. } => {
                expr.walk(f);
                for s in body {
                    s.walk_exprs(f);
                }
            }
            For { iterable, body, .. } => {
                iterable.walk(f);
                for s in body {
                    s.walk_exprs(f);
                }
            }
            Loop { body } => {
                for s in body {
                    s.walk_exprs(f);
                }
            }
            _ => {}
        }
    }
}

impl Pattern {
    fn walk<F: FnMut(&Expr)>(&self, _f: &mut F) {
        match self {
            Pattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    p.walk(_f);
                }
            }
            _ => {}
        }
    }
}

impl MatchArm {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F) {
        self.body.walk(f);
        if let Some(ref g) = self.guard {
            g.walk(f);
        }
    }
}

impl Pattern {
    fn walk_exprs<F: FnMut(&Expr)>(&self, _f: &mut F) {}
}

/// 收集类型的接口实现信息，用于约束检查
fn collect_type_implementations(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut impls: HashMap<String, HashSet<String>> = HashMap::new();
    // 类的 implements 声明
    for c in &program.classes {
        let entry = impls.entry(c.name.clone()).or_default();
        for iface in &c.implements {
            entry.insert(iface.clone());
        }
    }
    // 可扩展：也可从 extension/conform 声明中收集
    impls
}

/// 检查类型约束是否满足
/// 返回不满足的约束描述列表（空列表表示全部满足）
fn check_constraints(
    constraints: &[crate::ast::TypeConstraint],
    type_params: &[String],
    type_args: &[Type],
    type_impls: &HashMap<String, HashSet<String>>,
) -> Vec<String> {
    let subst: HashMap<_, _> = type_params.iter().cloned().zip(type_args.iter().cloned()).collect();
    let mut violations = Vec::new();

    for constraint in constraints {
        if let Some(actual_type) = subst.get(&constraint.param) {
            let type_name = match actual_type {
                Type::Int8 => "Int8",
                Type::Int16 => "Int16",
                Type::Int32 => "Int32",
                Type::Int64 => "Int64",
                Type::UInt8 => "UInt8",
                Type::UInt16 => "UInt16",
                Type::UInt32 => "UInt32",
                Type::UInt64 => "UInt64",
                Type::Float32 => "Float32",
                Type::Float64 => "Float64",
                Type::Bool => "Bool",
                Type::Rune => "Rune",
                Type::IntNative => "IntNative",
                Type::UIntNative => "UIntNative",
                Type::Float16 => "Float16",
                Type::Nothing => "Nothing",
                Type::String => "String",
                Type::Struct(name, _) => name.as_str(),
                _ => continue, // 复杂类型跳过约束检查
            };

            // 内建类型隐含实现常见接口
            let builtin_impls: HashSet<&str> = match type_name {
                "Int8" | "Int16" | "Int32" | "Int64" |
                "UInt8" | "UInt16" | "UInt32" | "UInt64" |
                "Float32" | "Float64" =>
                    ["Comparable", "Hashable", "Equatable", "ToString", "Numeric"]
                        .iter().copied().collect(),
                "Bool" =>
                    ["Comparable", "Hashable", "Equatable", "ToString"]
                        .iter().copied().collect(),
                "Char" =>
                    ["Comparable", "Hashable", "Equatable", "ToString"]
                        .iter().copied().collect(),
                "String" =>
                    ["Comparable", "Hashable", "Equatable", "ToString", "Collection"]
                        .iter().copied().collect(),
                _ => HashSet::new(),
            };

            for bound in &constraint.bounds {
                let satisfied = builtin_impls.contains(bound.as_str())
                    || type_impls
                        .get(type_name)
                        .map(|s| s.contains(bound))
                        .unwrap_or(false);
                if !satisfied {
                    violations.push(format!(
                        "类型 {} 不满足约束 {}: {}",
                        type_name, constraint.param, bound
                    ));
                }
            }
        }
    }
    violations
}

/// 对程序执行单态化
pub fn monomorphize_program(program: &mut Program) {
    let (func_insts, struct_insts, enum_insts, class_insts) = collect_instantiations(program);

    // 收集类型实现信息用于约束检查
    let type_impls = collect_type_implementations(program);

    // 约束检查：泛型函数
    for (name, type_args) in &func_insts {
        if let Some(def) = program.functions.iter().find(|f| &f.name == name && f.type_params.len() == type_args.len()) {
            if !def.constraints.is_empty() {
                let violations = check_constraints(&def.constraints, &def.type_params, type_args, &type_impls);
                for v in &violations {
                    eprintln!("⚠ 泛型约束警告 (函数 {}): {}", name, v);
                }
            }
        }
    }

    // 约束检查：泛型结构体
    for (name, type_args) in &struct_insts {
        if let Some(def) = program.structs.iter().find(|s| &s.name == name && s.type_params.len() == type_args.len()) {
            if !def.constraints.is_empty() {
                let violations = check_constraints(&def.constraints, &def.type_params, type_args, &type_impls);
                for v in &violations {
                    eprintln!("⚠ 泛型约束警告 (结构体 {}): {}", name, v);
                }
            }
        }
    }

    // 约束检查：泛型枚举
    for (name, type_args) in &enum_insts {
        if let Some(def) = program.enums.iter().find(|e| &e.name == name && e.type_params.len() == type_args.len()) {
            if !def.constraints.is_empty() {
                let violations = check_constraints(&def.constraints, &def.type_params, type_args, &type_impls);
                for v in &violations {
                    eprintln!("⚠ 泛型约束警告 (枚举 {}): {}", name, v);
                }
            }
        }
    }

    // 约束检查：泛型类
    for (name, type_args) in &class_insts {
        if let Some(def) = program.classes.iter().find(|c| &c.name == name && c.type_params.len() == type_args.len()) {
            if !def.constraints.is_empty() {
                let violations = check_constraints(&def.constraints, &def.type_params, type_args, &type_impls);
                for v in &violations {
                    eprintln!("⚠ 泛型约束警告 (类 {}): {}", name, v);
                }
            }
        }
    }

    let mut func_rewrites: HashMap<(String, Vec<Type>), String> = HashMap::new();
    let mut struct_rewrites: HashMap<(String, Vec<Type>), String> = HashMap::new();
    let mut struct_pattern_rewrites: HashMap<String, String> = HashMap::new();

    for (name, type_args) in &struct_insts {
        let mangled = mangle_name(name, type_args);
        struct_rewrites.insert((name.clone(), type_args.clone()), mangled.clone());
        struct_pattern_rewrites.insert(mangled.clone(), mangled.clone());
    }

    // 泛型枚举也加入 struct_rewrites（枚举实例化名也需要重写）
    for (name, type_args) in &enum_insts {
        let mangled = mangle_name(name, type_args);
        struct_rewrites.insert((name.clone(), type_args.clone()), mangled.clone());
        struct_pattern_rewrites.insert(mangled.clone(), mangled.clone());
    }

    // 泛型类也加入 struct_rewrites
    for (name, type_args) in &class_insts {
        let mangled = mangle_name(name, type_args);
        struct_rewrites.insert((name.clone(), type_args.clone()), mangled.clone());
        struct_pattern_rewrites.insert(mangled.clone(), mangled.clone());
    }

    for (name, type_args) in &func_insts {
        let mangled = mangle_name(name, type_args);
        func_rewrites.insert((name.clone(), type_args.clone()), mangled);
    }

    let rewrites = RewriteMap {
        func_rewrites,
        struct_rewrites: struct_rewrites.clone(),
        struct_pattern_rewrites,
    };

    let mut new_structs = Vec::new();
    for (name, type_args) in &struct_insts {
        let def = program
            .structs
            .iter()
            .find(|s| &s.name == name && s.type_params.len() == type_args.len())
            .expect("泛型结构体定义未找到");
        let subst: HashMap<_, _> = def
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let mangled_name = mangle_name(name, type_args);
        let fields: Vec<FieldDef> = def
            .fields
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                ty: substitute_type(&f.ty, &subst),
                default: f.default.as_ref().map(|d| substitute_expr(d.clone(), &subst, &rewrites)),
            })
            .collect();
        new_structs.push(StructDef {
            visibility: def.visibility.clone(),
            name: mangled_name,
            type_params: vec![],
            constraints: vec![],
            fields,
        });
    }
    program.structs.append(&mut new_structs);

    // 泛型枚举单态化
    let mut new_enums = Vec::new();
    for (name, type_args) in &enum_insts {
        let def = program
            .enums
            .iter()
            .find(|e| &e.name == name && e.type_params.len() == type_args.len())
            .expect("泛型枚举定义未找到");
        let subst: HashMap<_, _> = def
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let mangled_name = mangle_name(name, type_args);
        let variants: Vec<EnumVariant> = def
            .variants
            .iter()
            .map(|v| EnumVariant {
                name: v.name.clone(),
                payload: v.payload.as_ref().map(|t| substitute_type(t, &subst)),
            })
            .collect();
        new_enums.push(EnumDef {
            visibility: def.visibility.clone(),
            name: mangled_name,
            type_params: vec![],
            constraints: vec![],
            variants,
        });
    }
    program.enums.append(&mut new_enums);

    // 泛型类单态化
    let mut new_classes = Vec::new();
    for (name, type_args) in &class_insts {
        let def = program
            .classes
            .iter()
            .find(|c| &c.name == name && c.type_params.len() == type_args.len())
            .expect("泛型类定义未找到");
        let subst: HashMap<_, _> = def
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let mangled_name = mangle_name(name, type_args);
        let fields: Vec<FieldDef> = def
            .fields
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                ty: substitute_type(&f.ty, &subst),
                default: f.default.as_ref().map(|d| substitute_expr(d.clone(), &subst, &rewrites)),
            })
            .collect();
        let methods: Vec<ClassMethod> = def
            .methods
            .iter()
            .map(|m| {
                // 替换方法名中的类名：OrigName.method → MangledName.method
                let method_name = if m.func.name.starts_with(&format!("{}.", name)) {
                    m.func.name.replacen(name, &mangled_name, 1)
                } else {
                    m.func.name.clone()
                };
                ClassMethod {
                    override_: m.override_,
                    func: Function {
                        visibility: m.func.visibility.clone(),
                        name: method_name,
                        type_params: vec![],
                        constraints: vec![],
                        params: m.func.params.iter().map(|p| Param {
                            name: p.name.clone(),
                            ty: substitute_type(&p.ty, &subst),
                            default: p.default.as_ref().map(|d| substitute_expr(d.clone(), &subst, &rewrites)),
                            variadic: p.variadic,
                            is_named: p.is_named,
                            is_inout: p.is_inout,
                        }).collect(),
                        return_type: m.func.return_type.as_ref().map(|t| substitute_type(t, &subst)),
                        throws: m.func.throws.clone(),
                        body: m.func.body.iter().cloned().map(|s| substitute_stmt(s, &subst, &rewrites)).collect(),
                        extern_import: None,
                    },
                }
            })
            .collect();
        let init = def.init.as_ref().map(|i| InitDef {
            params: i.params.iter().map(|p| Param {
                name: p.name.clone(),
                ty: substitute_type(&p.ty, &subst),
                default: p.default.as_ref().map(|d| substitute_expr(d.clone(), &subst, &rewrites)),
                variadic: p.variadic,
                is_named: p.is_named,
                is_inout: p.is_inout,
            }).collect(),
            body: i.body.iter().cloned().map(|s| substitute_stmt(s, &subst, &rewrites)).collect(),
        });
        let deinit = def.deinit.as_ref().map(|d| d.iter().cloned().map(|s| substitute_stmt(s, &subst, &rewrites)).collect());
        new_classes.push(ClassDef {
            visibility: def.visibility.clone(),
            name: mangled_name,
            type_params: vec![],
            constraints: vec![],
            is_abstract: def.is_abstract,
            is_sealed: def.is_sealed,
            is_open: def.is_open,
            extends: def.extends.clone(),
            implements: def.implements.clone(),
            fields,
            init,
            deinit,
            static_init: def.static_init.as_ref().map(|s| s.iter().cloned().map(|st| substitute_stmt(st, &subst, &rewrites)).collect()),
            methods,
            primary_ctor_params: vec![],
        });
    }
    program.classes.append(&mut new_classes);

    // 泛型函数单态化（支持特化：若已存在同名非泛型函数，则优先使用）
    let mut new_functions = Vec::new();
    for (name, type_args) in &func_insts {
        let mangled_name = mangle_name(name, type_args);

        // 泛型特化检查：若程序中已存在名为 mangled_name 的非泛型函数，跳过生成
        let has_specialization = program
            .functions
            .iter()
            .any(|f| f.name == mangled_name && f.type_params.is_empty());
        if has_specialization {
            // 已有特化实现，直接使用
            continue;
        }

        let def = program
            .functions
            .iter()
            .find(|f| &f.name == name && f.type_params.len() == type_args.len() && f.extern_import.is_none())
            .expect("泛型函数定义未找到");
        let subst: HashMap<_, _> = def
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();

        let params: Vec<Param> = def
            .params
            .iter()
            .map(|p| Param {
                name: p.name.clone(),
                ty: substitute_type(&p.ty, &subst),
                default: p
                    .default
                    .as_ref()
                    .map(|d| substitute_expr(d.clone(), &subst, &rewrites)),
                variadic: p.variadic,
                is_named: p.is_named,
                is_inout: p.is_inout,
            })
            .collect();
        let return_type = def
            .return_type
            .as_ref()
            .map(|t| substitute_type(t, &subst));
        let body: Vec<Stmt> = def
            .body
            .iter()
            .cloned()
            .map(|s| substitute_stmt(s, &subst, &rewrites))
            .collect();

        new_functions.push(Function {
            visibility: def.visibility.clone(),
            name: mangled_name,
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: def.throws.clone(),
            body,
            extern_import: None,
        });
    }
    program.functions.append(&mut new_functions);

    for func in &mut program.functions {
        if func.extern_import.is_some() {
            continue;
        }
        let subst = HashMap::new();
        let mut new_body = Vec::new();
        for stmt in std::mem::take(&mut func.body) {
            new_body.push(substitute_stmt(stmt, &subst, &rewrites));
        }
        func.body = new_body;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Type;

    #[test]
    fn test_type_mangle_suffix_basic() {
        assert_eq!(type_mangle_suffix(&Type::Int8), "Int8");
        assert_eq!(type_mangle_suffix(&Type::Int16), "Int16");
        assert_eq!(type_mangle_suffix(&Type::Int32), "Int32");
        assert_eq!(type_mangle_suffix(&Type::Int64), "Int64");
        assert_eq!(type_mangle_suffix(&Type::UInt8), "UInt8");
        assert_eq!(type_mangle_suffix(&Type::UInt16), "UInt16");
        assert_eq!(type_mangle_suffix(&Type::UInt32), "UInt32");
        assert_eq!(type_mangle_suffix(&Type::UInt64), "UInt64");
        assert_eq!(type_mangle_suffix(&Type::Float32), "Float32");
        assert_eq!(type_mangle_suffix(&Type::Float64), "Float64");
        assert_eq!(type_mangle_suffix(&Type::Bool), "Bool");
        assert_eq!(type_mangle_suffix(&Type::Rune), "Rune");
        assert_eq!(type_mangle_suffix(&Type::Unit), "Unit");
        assert_eq!(type_mangle_suffix(&Type::String), "String");
        assert_eq!(type_mangle_suffix(&Type::Range), "Range");
    }

    #[test]
    fn test_type_mangle_suffix_compound() {
        assert_eq!(
            type_mangle_suffix(&Type::Array(Box::new(Type::Int64))),
            "Array_Int64"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Tuple(vec![Type::Int64, Type::Float64])),
            "Tuple_Int64_Float64"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Struct("Foo".to_string(), vec![])),
            "Foo"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Struct("Pair".to_string(), vec![Type::Int64, Type::String])),
            "Pair_Int64_String"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Option(Box::new(Type::Int64))),
            "Option_Int64"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            "Result_Int64_String"
        );
        assert_eq!(
            type_mangle_suffix(&Type::Function {
                params: vec![Type::Int64],
                ret: Box::new(Some(Type::Bool)),
            }),
            "Fn_Int64_Bool"
        );
        assert_eq!(
            type_mangle_suffix(&Type::TypeParam("T".to_string())),
            "T"
        );
    }

    #[test]
    fn test_mangle_name() {
        assert_eq!(mangle_name("foo", &[Type::Int64]), "foo$Int64");
        assert_eq!(mangle_name("bar", &[Type::Int64, Type::String]), "bar$Int64$String");
        assert_eq!(mangle_name("baz", &[]), "baz$_");
    }

    #[test]
    fn test_monomorphize_empty_program() {
        let mut program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        monomorphize_program(&mut program);
        assert!(program.functions.is_empty());
    }

    #[test]
    fn test_monomorphize_no_generics() {
        let mut program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![Function {
                visibility: crate::ast::Visibility::default(),
                name: "main".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Integer(42)))],
                extern_import: None,
            }],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };
        monomorphize_program(&mut program);
        assert_eq!(program.functions.len(), 1);
    }
}
