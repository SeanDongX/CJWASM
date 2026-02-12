//! 泛型单态化：将泛型函数与结构体按类型实参生成特化版本。

use crate::ast::{Expr, FieldDef, Function, MatchArm, Param, Pattern, Program, Stmt, StructDef};
use std::collections::{HashMap, HashSet};
use crate::ast::Type;

/// 类型实参到名字修饰后缀
fn type_mangle_suffix(ty: &Type) -> String {
    match ty {
        Type::Int32 => "Int32".to_string(),
        Type::Int64 => "Int64".to_string(),
        Type::Float32 => "Float32".to_string(),
        Type::Float64 => "Float64".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Unit => "Unit".to_string(),
        Type::String => "String".to_string(),
        Type::Array(inner) => format!("Array_{}", type_mangle_suffix(inner)),
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
        } => MethodCall {
            object: Box::new(substitute_expr(*object, subst, rewrites)),
            method,
            args: args
                .into_iter()
                .map(|a| substitute_expr(a, subst, rewrites))
                .collect(),
        },
        SuperCall { method, args } => SuperCall {
            method,
            args: args
                .into_iter()
                .map(|a| substitute_expr(a, subst, rewrites))
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
        } => Range {
            start: Box::new(substitute_expr(*start, subst, rewrites)),
            end: Box::new(substitute_expr(*end, subst, rewrites)),
            inclusive,
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
            body,
            catch_var,
            catch_body,
        } => TryBlock {
            body: body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
            catch_var,
            catch_body: catch_body
                .into_iter()
                .map(|s| substitute_stmt(s, subst, rewrites))
                .collect(),
        },
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
        Unit => Unit,
        Expr::None => Expr::None,
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
            binding,
        } => Variant {
            enum_name,
            variant_name,
            binding,
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
        Var { name, ty, value } => Var {
            name,
            ty: ty.map(|t| substitute_type(&t, subst)),
            value: substitute_expr(value, subst, rewrites),
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
    }
}

/// 重写映射：(原名, 类型实参) -> 单态化后名
struct RewriteMap {
    func_rewrites: HashMap<(String, Vec<Type>), String>,
    struct_rewrites: HashMap<(String, Vec<Type>), String>,
    struct_pattern_rewrites: HashMap<String, String>,
}

/// 收集所有需要的泛型实例化
fn collect_instantiations(program: &Program) -> (HashSet<(String, Vec<Type>)>, HashSet<(String, Vec<Type>)>) {
    let mut func_insts = HashSet::new();
    let mut struct_insts = HashSet::new();

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

    fn visit_expr(
        expr: &Expr,
        gf: &std::collections::HashMap<String, usize>,
        gs: &std::collections::HashMap<String, usize>,
        fi: &mut HashSet<(String, Vec<Type>)>,
        si: &mut HashSet<(String, Vec<Type>)>,
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
                }
            }
            ConstructorCall { name, type_args, .. } => {
                if let Option::Some(tas) = type_args.as_ref() {
                    if let Option::Some(n) = gs.get(name) {
                        if *n == tas.len() {
                            si.insert((name.clone(), tas.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for func in &program.functions {
        for stmt in &func.body {
            let mut visit = |e: &Expr| visit_expr(e, &generic_functions, &generic_structs, &mut func_insts, &mut struct_insts);
            stmt.walk(&mut visit);
        }
    }

    (func_insts, struct_insts)
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
            _ => {}
        }
    }
}

impl AstWalk for Stmt {
    fn walk<F: FnMut(&Expr)>(&self, f: &mut F) {
        use crate::ast::Stmt::*;
        match self {
            Let { value, .. } => value.walk(f),
            Var { value, .. } => value.walk(f),
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
            Var { value, .. } => value.walk(f),
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

/// 对程序执行单态化
pub fn monomorphize_program(program: &mut Program) {
    let (func_insts, struct_insts) = collect_instantiations(program);

    let mut func_rewrites: HashMap<(String, Vec<Type>), String> = HashMap::new();
    let mut struct_rewrites: HashMap<(String, Vec<Type>), String> = HashMap::new();
    let mut struct_pattern_rewrites: HashMap<String, String> = HashMap::new();

    for (name, type_args) in &struct_insts {
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
            })
            .collect();
        new_structs.push(StructDef {
            visibility: def.visibility.clone(),
            name: mangled_name,
            type_params: vec![],
            fields,
        });
    }
    program.structs.append(&mut new_structs);

    let mut new_functions = Vec::new();
    for (name, type_args) in &func_insts {
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

        let mangled_name = mangle_name(name, type_args);
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
            params,
            return_type,
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
