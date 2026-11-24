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

// ─── 语义合法性校验 ─────────────────────────────────────────────────────────
//
// 在语法解析之后、代码生成之前，对 AST 做名称/签名层面的合法性检查。
// 只实现可精确定位的规则，避免误伤合法重载：
//   1. 同一容器（class/struct/enum/interface）内同名 + 同参数类型的成员函数重定义；
//   2. class/struct/enum 中 static 与实例成员函数同名（6.3.2(1)）；
//   3. 顶层同名 + 同签名的重复函数（泛型与非泛型同名不视为重定义，5.1.6(3)；
//       泛型函数按声明顺序做 alpha 等价，5.1.6(5)/(7)）；
//   4. enum 重复构造器（同名同载荷）/ 容器重复字段 / interface 默认实现重复；
//   5. override/redef 方法与父类/接口方法签名不匹配，或父类型中不存在可覆盖函数；
//   6. 非抽象类未实现本地接口的抽象方法。
//
// 所有规则都能在单文件内基于 AST 判定，父类型无法在本地解析时保守跳过。

use crate::ast::{
    AssignTarget, ClassDef, EnumDef, Function, InterfaceDef, InterfaceMethod, StructDef, Visibility,
};
use std::collections::HashSet;

mod expr_check;

/// 一条语义校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub message: String,
}

impl std::fmt::Display for SemanticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 对单个 `Program` 做语义合法性校验。
///
/// 成功返回 `Ok(())`；发现错误时返回全部错误（编译应以非 0 退出）。
/// 校验是纯只读的，不修改 AST。
pub fn validate_program(program: &Program) -> Result<(), Vec<SemanticError>> {
    let mut errors = Vec::new();
    let index = ProgramIndex::new(program);

    check_function_groups(program, &index, &mut errors);
    for cls in &program.classes {
        check_class(cls, &index, &mut errors);
    }
    for st in &program.structs {
        check_struct_interfaces(st, program, &index, &mut errors);
    }
    for iface in &program.interfaces {
        check_interface(iface, &mut errors);
    }
    for en in &program.enums {
        check_enum_variants(en, &mut errors);
        check_enum_interfaces(en, program, &index, &mut errors);
    }

    // 不允许实例化抽象类 / 接口；不能继承 struct/enum。
    check_instantiation_targets(program, &mut errors);

    // 表达式/赋值类型检查：运算符操作数类型、可赋值性、不可变绑定等。
    expr_check::check_program(program, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ─── 签名表示与比较 ─────────────────────────────────────────────────────────

/// 成员函数签名（已剥离隐式 this/self 接收者）。
#[derive(Debug, Clone)]
struct Sig {
    /// 不带所属类型前缀的成员名（如 `Class.method` → `method`）。
    name: String,
    /// 参数类型（不含接收者）。
    params: Vec<Type>,
    /// 返回类型；None 表示未标注（Unit 推断）。
    ret: Option<Type>,
    /// 是否带方法级泛型参数；None 表示语言层不可见（接口方法）。
    generic: Option<bool>,
    /// 声明顺序的类型参数（类级在前、方法级在后）。
    /// 用于按声明顺序做 alpha 等价；为空时按首次出现顺序编号。
    tp_order: Vec<String>,
    /// 是否为静态函数（无实例接收者）。
    is_static: bool,
    /// 声明可见性；`None` 表示语言层不可见（接口方法在 AST 中不保存可见性）。
    vis: Option<Visibility>,
    /// 用于错误消息的全名。
    display: String,
}

fn is_receiver_param(name: &str) -> bool {
    name == "this" || name == "self"
}

/// 剥离第一个隐式/显式接收者参数，返回（其余参数, 是否静态）。
fn split_receiver(params: &[crate::ast::Param]) -> (Vec<Type>, bool) {
    if params
        .first()
        .map_or(false, |p| is_receiver_param(&p.name))
    {
        (params[1..].iter().map(|p| p.ty.clone()).collect(), false)
    } else {
        (params.iter().map(|p| p.ty.clone()).collect(), true)
    }
}

fn sig_of_function(func: &Function, outer_tp: &[String]) -> Sig {
    let (params, is_static) = split_receiver(&func.params);
    // 属性访问器脱糖方法名形如 `Class.__get_p`，按成员名比较时去掉前缀。
    let name = func
        .name
        .rsplit_once('.')
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| func.name.clone());
    let mut tp_order = outer_tp.to_vec();
    tp_order.extend(func.type_params.iter().cloned());
    Sig {
        name,
        params,
        ret: func.return_type.clone(),
        generic: Some(!func.type_params.is_empty()),
        tp_order,
        is_static,
        vis: Some(func.visibility.clone()),
        display: func.name.clone(),
    }
}

fn sig_of_interface_method(method: &InterfaceMethod) -> Sig {
    let (params, _) = split_receiver(&method.params);
    Sig {
        name: method.name.clone(),
        params,
        ret: method.return_type.clone(),
        // 接口方法在 AST 中不保存泛型参数，无法判断泛型/非泛型，按未知处理。
        generic: None,
        // 接口的泛型参数未进入 AST，按首次出现顺序做 alpha 等价。
        tp_order: Vec::new(),
        // 接口的 static 修饰符不进入 AST，static/实例区分不可用。
        is_static: false,
        // 接口方法在 AST 中不保存可见性。
        vis: None,
        display: method.name.clone(),
    }
}

/// 所属类型名（`A.b` → Some("A")，`b` → None）。
fn owner_name(full: &str) -> Option<&str> {
    full.rsplit_once('.').map(|(o, _)| o)
}

/// 把类型结构序列化为规范字符串。
///
/// 泛型参数优先按 `tp_order`（声明顺序）映射编号；不在 `tp_order` 中的
/// 类型参数按首次出现顺序追加编号。这样 `f<X,Y>(a:X,b:Y)` 与
/// `f<Y,X>(a:X,b:Y)` 的非泛型部分不同，不误判为重定义。
fn canon_type(ty: &Type, order: &[String], map: &mut HashMap<String, usize>) -> String {
    match ty {
        Type::TypeParam(name) => {
            if let Some(pos) = order.iter().position(|n| n == name) {
                format!("#{}", pos)
            } else {
                let next = order.len() + map.len();
                let idx = *map.entry(name.clone()).or_insert(next);
                format!("#{}", idx)
            }
        }
        Type::Array(inner) => format!("Array<{}>", canon_type(inner, order, map)),
        Type::Option(inner) => format!("Option<{}>", canon_type(inner, order, map)),
        Type::Slice(inner) => format!("Slice<{}>", canon_type(inner, order, map)),
        Type::Struct(name, args) => {
            if args.is_empty() {
                format!("S:{}", name)
            } else {
                let inner: Vec<String> = args.iter().map(|a| canon_type(a, order, map)).collect();
                format!("S:{}<{}>", name, inner.join(","))
            }
        }
        Type::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(|a| canon_type(a, order, map)).collect();
            format!("({})", inner.join(","))
        }
        Type::Result(ok, err) => {
            format!(
                "Result<{},{}>",
                canon_type(ok, order, map),
                canon_type(err, order, map)
            )
        }
        Type::Map(k, v) => format!(
            "Map<{},{}>",
            canon_type(k, order, map),
            canon_type(v, order, map)
        ),
        Type::Function { params, ret } => {
            let ps: Vec<String> = params.iter().map(|p| canon_type(p, order, map)).collect();
            let r = match ret.as_ref() {
                Some(t) => canon_type(t, order, map),
                None => "Unit".to_string(),
            };
            format!("Fn({})->{}", ps.join(","), r)
        }
        Type::Qualified(parts) => format!("Q:{}", parts.join(".")),
        other => format!("{:?}", other),
    }
}

/// 仅参数列表的规范键（用于重定义判定）。
fn canon_params(sig: &Sig) -> String {
    let mut map = HashMap::new();
    sig.params
        .iter()
        .map(|t| canon_type(t, &sig.tp_order, &mut map))
        .collect::<Vec<_>>()
        .join(",")
}

/// 参数 + 返回类型的规范键（用于 override/实现匹配）。
fn canon_signature(sig: &Sig) -> (String, Option<String>) {
    let mut map = HashMap::new();
    let params = sig
        .params
        .iter()
        .map(|t| canon_type(t, &sig.tp_order, &mut map))
        .collect::<Vec<_>>()
        .join(",");
    let ret = sig
        .ret
        .as_ref()
        .map(|t| canon_type(t, &sig.tp_order, &mut map));
    (params, ret)
}

/// 泛型性质是否兼容：已知泛型/非泛型不同则不构成重定义（5.1.6(3)）。
fn generic_compatible(a: Option<bool>, b: Option<bool>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// 两签名是否构成重定义（同名且参数类型一致，泛型性质兼容）。
fn is_redefinition(a: &Sig, b: &Sig) -> bool {
    a.params.len() == b.params.len()
        && generic_compatible(a.generic, b.generic)
        && canon_params(a) == canon_params(b)
}

// ─── 容器内重复成员检测 ────────────────────────────────────────────────────

fn is_synthetic_member(name: &str) -> bool {
    name == "__static_init" || name == "__init"
}

fn check_signature_group(
    owner: &str,
    group: &[Sig],
    check_static_conflict: bool,
    errors: &mut Vec<SemanticError>,
) {
    if group.len() < 2 {
        return;
    }
    let name = group[0].name.clone();

    // static 与实例成员函数同名：不允许互为重载（6.3.2(1)）。
    if check_static_conflict {
        let has_static = group.iter().any(|s| s.is_static);
        let has_instance = group.iter().any(|s| !s.is_static);
        if has_static && has_instance {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 中静态成员函数与实例成员函数同名 '{}'，不允许互为重载",
                    owner, name
                ),
            });
            return;
        }
    }

    // 同名 + 同参数类型的成员函数构成重定义。
    for i in 0..group.len() {
        for j in (i + 1)..group.len() {
            if is_redefinition(&group[i], &group[j]) {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 中成员函数 '{}' 被重复定义（{} 与 {}）",
                        owner, name, group[i].display, group[j].display
                    ),
                });
                return;
            }
        }
    }
}

/// 校验 program.functions 中的函数组（顶层函数 + struct/enum 成员方法）。
fn check_function_groups(
    program: &Program,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut groups: HashMap<String, Vec<Sig>> = HashMap::new();
    for func in &program.functions {
        if func.extern_import.is_some() {
            continue;
        }
        let owner = owner_name(&func.name).unwrap_or("").to_string();
        let outer = index.type_params_of(&owner);
        let sig = sig_of_function(func, outer);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        groups.entry(owner).or_default().push(sig);
    }

    for (owner, sigs) in groups {
        // 顶层函数没有 static/实例之分；struct/enum 成员函数需要检查。
        let check_static = !owner.is_empty()
            && (index.structs.contains(owner.as_str()) || index.enums.contains(owner.as_str()));
        let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
        for sig in sigs {
            by_name.entry(sig.name.clone()).or_default().push(sig);
        }
        let owner_desc = if owner.is_empty() {
            "<顶层>"
        } else {
            owner.as_str()
        };
        for group in by_name.values() {
            check_signature_group(owner_desc, group, check_static, errors);
        }
    }
}

fn check_class(cls: &ClassDef, index: &ProgramIndex<'_>, errors: &mut Vec<SemanticError>) {
    let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
    for method in &cls.methods {
        let sig = sig_of_function(&method.func, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        by_name.entry(sig.name.clone()).or_default().push(sig);
    }
    for group in by_name.values() {
        check_signature_group(&cls.name, group, true, errors);
    }

    // 重复字段
    check_duplicate_fields(
        &cls.name,
        cls.fields.iter().map(|f| f.name.as_str()),
        errors,
    );

    // override / redef 签名校验
    check_class_overrides(cls, index, errors);

    // 覆写/重定义不得降低访问级别
    check_member_visibility_lowering(cls, index, errors);

    // 实现接口属性时类型必须完全一致
    if let Some(parents) = collect_parent_methods(cls, index) {
        if parents.iter().any(|p| p.is_interface) {
            let own: Vec<Sig> = cls
                .methods
                .iter()
                .map(|m| sig_of_function(&m.func, &cls.type_params))
                .collect();
            check_interface_property_types(&cls.name, &own, &parents, index, errors);
        }
    }

    // 与父类同名函数的 static/实例冲突
    check_inherited_static_conflicts(cls, index, errors);

    // 接口实现完整性
    check_interface_implementation(cls, index, errors);

    // 接口 const 函数必须由 const 函数实现
    check_const_interface_impls(cls, index, errors);

    // 父类型来源合法性：单继承、可继承性、不能继承 struct/enum
    check_class_supertypes(cls, index, errors);
}

/// 可见性级别：private < internal(默认) < protected < public。
fn visibility_rank(v: &Visibility) -> u8 {
    match v {
        Visibility::Private => 0,
        Visibility::Internal => 1,
        Visibility::Protected => 2,
        Visibility::Public => 3,
    }
}

/// 校验 class 的父类型来源（6.1.1.2(1)、6.1.1.2(11)、6.1.1.3(2)(4)、6.1.1.1(6)）：
/// - 类只能单继承：`implements` 中出现的名字不得解析为本地类；
/// - 只能继承 open / abstract / sealed 类（sealed 在同包内可继承）；
/// - 不得继承 struct / enum，也不得把 struct / enum 当作接口实现。
///
/// 父类型无法在本地解析时保守跳过。
fn check_class_supertypes(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if let Some(parent) = cls.extends.as_deref() {
        if index.classes.contains_key(parent) {
            let p = index.classes[parent];
            // sealed 类允许在同一包（单文件视为同包）内被继承；abstract 类天然可继承。
            if !p.is_open && !p.is_abstract && !p.is_sealed {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 不能继承非 open/abstract/sealed 类 '{}'",
                        cls.name, parent
                    ),
                });
            }
        } else if index.structs.contains(parent) || index.enums.contains(parent) {
            errors.push(SemanticError {
                message: format!("类 '{}' 不能继承 struct/enum 类型 '{}'", cls.name, parent),
            });
        }
    }
    for name in &cls.implements {
        if let Some(p) = index.classes.get(name.as_str()) {
            // 抽象类可能被误解析进 implements（如多模板标注用例）；仅对非抽象类报错，
            // 避免误伤合法/宏相关用例。open 类出现在 implements 一定是多继承。
            if !p.is_abstract {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 只能单继承，不能从另一个类 '{}' 继承（父类须位于 <: 列表首位）",
                        cls.name, name
                    ),
                });
            }
        } else if index.structs.contains(name.as_str()) || index.enums.contains(name.as_str()) {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 不能将 struct/enum 类型 '{}' 当作接口实现",
                    cls.name, name
                ),
            });
        }
    }
}

/// 扫描所有函数体，禁止实例化抽象类或接口。
fn check_instantiation_targets(
    program: &Program,
    errors: &mut Vec<SemanticError>,
) {
    let abstract_classes: HashSet<&str> = program
        .classes
        .iter()
        .filter(|c| c.is_abstract)
        .map(|c| c.name.as_str())
        .collect();
    let interfaces: HashSet<&str> = program.interfaces.iter().map(|i| i.name.as_str()).collect();
    if abstract_classes.is_empty() && interfaces.is_empty() {
        return;
    }

    // 可能遮蔽类型名的可调用名/成员名。若调用的裸名与这些名字重名，按“调用函数
    // 而非实例化类型”处理并跳过判定（名称遮蔽用例不得误伤）。
    let mut shadowed: HashSet<&str> = HashSet::new();
    for f in &program.functions {
        shadowed.insert(f.name.rsplit('.').next().unwrap_or(f.name.as_str()));
    }
    for cls in &program.classes {
        for m in &cls.methods {
            shadowed.insert(m.func.name.rsplit('.').next().unwrap_or(m.func.name.as_str()));
        }
        for f in &cls.fields {
            shadowed.insert(f.name.as_str());
        }
        for p in &cls.primary_ctor_params {
            shadowed.insert(p.name.as_str());
        }
    }
    for st in &program.structs {
        for f in &st.fields {
            shadowed.insert(f.name.as_str());
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            shadowed.insert(f.name.rsplit('.').next().unwrap_or(f.name.as_str()));
        }
    }

    let mut ctx = InstantiationScan {
        abstract_classes,
        interfaces,
        shadowed,
        scopes: Vec::new(),
        errors,
    };

    for func in &program.functions {
        ctx.scan_function(func);
    }
    for cls in &program.classes {
        for m in &cls.methods {
            ctx.scan_function(&m.func);
        }
        if let Some(init) = &cls.init {
            ctx.push_scope();
            for p in &init.params {
                ctx.bind(&p.name);
                if let Some(d) = &p.default {
                    ctx.scan_expr(d);
                }
            }
            for s in &init.body {
                ctx.scan_stmt(s);
            }
            ctx.pop_scope();
        }
        if let Some(body) = &cls.deinit {
            ctx.scan_stmts(body);
        }
        if let Some(body) = &cls.static_init {
            ctx.scan_stmts(body);
        }
    }
    for ext in &program.extends {
        for f in &ext.methods {
            ctx.scan_function(f);
        }
    }
    for iface in &program.interfaces {
        for m in &iface.methods {
            if let Some(body) = &m.default_body {
                ctx.scan_stmts(body);
            }
        }
    }
    for c in &program.constants {
        ctx.scan_expr(&c.init);
    }
}

/// 递归扫描函数体中的调用，识别对抽象类/接口的实例化。
struct InstantiationScan<'a, 'i> {
    abstract_classes: HashSet<&'a str>,
    interfaces: HashSet<&'a str>,
    /// 可能与类型名重名的可调用/成员名（遮蔽时跳过实例化判定）。
    shadowed: HashSet<&'a str>,
    /// 局部作用域栈：记录参数、局部变量、局部函数等名字。
    scopes: Vec<HashSet<String>>,
    errors: &'i mut Vec<SemanticError>,
}

impl<'a, 'i> InstantiationScan<'a, 'i> {
    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string());
        }
    }

    fn bind_pattern(&mut self, pattern: &crate::ast::Pattern) {
        use crate::ast::Pattern;
        match pattern {
            Pattern::Binding(n) => self.bind(n),
            Pattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.bind_pattern(p);
                }
            }
            Pattern::Tuple(items) | Pattern::Or(items) => {
                for p in items {
                    self.bind_pattern(p);
                }
            }
            Pattern::Variant { payload, .. } => {
                if let Some(p) = payload {
                    self.bind_pattern(p);
                }
            }
            Pattern::TypeTest { binding, .. } => self.bind(binding),
            _ => {}
        }
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.contains(name) || self.scopes.iter().any(|s| s.contains(name))
    }

    fn scan_function(&mut self, func: &Function) {
        self.push_scope();
        for p in &func.params {
            self.bind(&p.name);
            if let Some(d) = &p.default {
                self.scan_expr(d);
            }
        }
        // 函数体与参数共享同一作用域，避免为参数再压一层。
        for stmt in &func.body {
            self.scan_stmt(stmt);
        }
        self.pop_scope();
    }

    fn scan_stmts(&mut self, stmts: &[Stmt]) {
        self.push_scope();
        for stmt in stmts {
            self.scan_stmt(stmt);
        }
        self.pop_scope();
    }

    fn scan_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
                self.scan_expr(value);
                self.bind_pattern(pattern);
            }
            Stmt::Const { name, value, .. } => {
                self.scan_expr(value);
                self.bind(name);
            }
            Stmt::Assign { target, value } => {
                self.scan_assign_target(target);
                self.scan_expr(value);
            }
            Stmt::Expr(e) => self.scan_expr(e),
            Stmt::Return(Some(e)) => self.scan_expr(e),
            Stmt::While { cond, body } => {
                self.scan_expr(cond);
                self.scan_stmts(body);
            }
            Stmt::WhileLet { pattern, expr, body } => {
                self.scan_expr(expr);
                self.push_scope();
                self.bind_pattern(pattern);
                for s in body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::DoWhile { body, cond } => {
                self.scan_stmts(body);
                self.scan_expr(cond);
            }
            Stmt::For { var, iterable, body } => {
                self.scan_expr(iterable);
                self.push_scope();
                self.bind(var);
                for s in body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::Loop { body } | Stmt::UnsafeBlock { body } => self.scan_stmts(body),
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            Stmt::LocalFunc(f) => {
                let simple = f.name.rsplit('.').next().unwrap_or(f.name.as_str()).to_string();
                self.bind(&simple);
                self.scan_function(f);
            }
            Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        }
    }

    fn scan_assign_target(&mut self, target: &AssignTarget) {
        match target {
            AssignTarget::Index { index, .. } => self.scan_expr(index),
            AssignTarget::IndexPath { index, .. } => self.scan_expr(index),
            AssignTarget::ExprIndex { expr, index } => {
                self.scan_expr(expr);
                self.scan_expr(index);
            }
            AssignTarget::ExprField { object, .. } => self.scan_expr(object),
            AssignTarget::Slice { array, start, end } => {
                self.scan_expr(array);
                self.scan_expr(start);
                self.scan_expr(end);
            }
            AssignTarget::Tuple(items) => {
                for t in items {
                    self.scan_assign_target(t);
                }
            }
            _ => {}
        }
    }

    fn check_ctor_name(&mut self, name: &str) {
        if name.contains('.') || self.is_shadowed(name) {
            return;
        }
        if self.abstract_classes.contains(name) {
            self.errors.push(SemanticError {
                message: format!("抽象类 '{}' 不能被实例化", name),
            });
        } else if self.interfaces.contains(name) {
            self.errors.push(SemanticError {
                message: format!("接口 '{}' 不能被实例化", name),
            });
        }
    }

    fn scan_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { name, args, named_args, .. }
            | Expr::ConstructorCall { name, args, named_args, .. } => {
                self.check_ctor_name(name);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::MethodCall { object, args, named_args, .. } => {
                self.scan_expr(object);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::SuperCall { args, named_args, .. } => {
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::Unary { expr, .. }
            | Expr::Try(expr)
            | Expr::Throw(expr)
            | Expr::Cast { expr, .. }
            | Expr::IsType { expr, .. }
            | Expr::PostfixIncr(expr)
            | Expr::PostfixDecr(expr)
            | Expr::PrefixIncr(expr)
            | Expr::PrefixDecr(expr)
            | Expr::Field { object: expr, .. }
            | Expr::OptionalChain { object: expr, .. }
            | Expr::TupleIndex { object: expr, .. } => self.scan_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            Expr::If { cond, then_branch, else_branch } => {
                self.scan_expr(cond);
                self.scan_expr(then_branch);
                if let Some(e) = else_branch {
                    self.scan_expr(e);
                }
            }
            Expr::IfLet { expr, then_branch, else_branch, .. } => {
                self.scan_expr(expr);
                self.scan_expr(then_branch);
                if let Some(e) = else_branch {
                    self.scan_expr(e);
                }
            }
            Expr::Block(stmts, tail) => {
                self.scan_stmts(stmts);
                if let Some(e) = tail {
                    self.scan_expr(e);
                }
            }
            Expr::Tuple(items) | Expr::Array(items) => self.scan_exprs(items),
            Expr::Index { array, index } => {
                self.scan_expr(array);
                self.scan_expr(index);
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.scan_expr(e);
                }
            }
            Expr::Range { start, end, step, .. } => {
                self.scan_expr(start);
                self.scan_expr(end);
                if let Some(s) = step {
                    self.scan_expr(s);
                }
            }
            Expr::VariantConst { arg, .. } => {
                if let Some(a) = arg {
                    self.scan_expr(a);
                }
            }
            Expr::Match { expr, arms } => {
                self.scan_expr(expr);
                for arm in arms {
                    self.push_scope();
                    self.bind_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.scan_expr(g);
                    }
                    self.scan_expr(&arm.body);
                    self.pop_scope();
                }
            }
            Expr::Lambda { params, body, .. } => {
                self.push_scope();
                for (n, _) in params {
                    self.bind(n);
                }
                self.scan_expr(body);
                self.pop_scope();
            }
            Expr::Some(e) | Expr::Ok(e) | Expr::Err(e) => self.scan_expr(e),
            Expr::NullCoalesce { option, default } => {
                self.scan_expr(option);
                self.scan_expr(default);
            }
            Expr::Return(Some(e)) => self.scan_expr(e),
            Expr::TryBlock {
                resources,
                body,
                catch_var,
                catch_body,
                finally_body,
                ..
            } => {
                for (n, e) in resources {
                    self.bind(n);
                    self.scan_expr(e);
                }
                self.scan_stmts(body);
                self.push_scope();
                if let Some(v) = catch_var {
                    self.bind(v);
                }
                for s in catch_body {
                    self.scan_stmt(s);
                }
                self.pop_scope();
                if let Some(f) = finally_body {
                    self.scan_stmts(f);
                }
            }
            Expr::SliceExpr { array, start, end } => {
                self.scan_expr(array);
                self.scan_expr(start);
                self.scan_expr(end);
            }
            Expr::MapLiteral { entries } => {
                for (k, v) in entries {
                    self.scan_expr(k);
                    self.scan_expr(v);
                }
            }
            Expr::Spawn { body } => self.scan_stmts(body),
            Expr::Synchronized { lock, body } => {
                self.scan_expr(lock);
                self.scan_stmts(body);
            }
            Expr::TrailingClosure { callee, args, closure } => {
                self.scan_expr(callee);
                self.scan_exprs(args);
                self.scan_expr(closure);
            }
            Expr::Invoke { callee, args, named_args } => {
                self.scan_expr(callee);
                self.scan_exprs(args);
                self.scan_named(named_args);
            }
            Expr::Macro { args, .. } => self.scan_exprs(args),
            Expr::Interpolate(parts) => {
                for p in parts {
                    if let crate::ast::InterpolatePart::Expr(e) = p {
                        self.scan_expr(e);
                    }
                }
            }
            Expr::Integer(_)
            | Expr::Float(_)
            | Expr::Float32(_)
            | Expr::Bool(_)
            | Expr::Rune(_)
            | Expr::String(_)
            | Expr::Var(_)
            | Expr::SuperFieldAccess { .. }
            | Expr::None
            | Expr::Return(None)
            | Expr::Break
            | Expr::Continue => {}
        }
    }

    fn scan_exprs(&mut self, exprs: &[Expr]) {
        for e in exprs {
            self.scan_expr(e);
        }
    }

    fn scan_named(&mut self, named: &[(String, Expr)]) {
        for (_, e) in named {
            self.scan_expr(e);
        }
    }
}

/// 实现接口属性时，属性类型必须与接口声明完全一致（6.2.4.1(7)）。
///
/// 既覆盖抽象属性（已有签名匹配逻辑），也覆盖带默认实现的属性：
/// 后者不会进入“必须实现”的集合，但仍要求同类型（不允许协变/逆变）。
fn check_interface_property_types(
    owner: &str,
    own_sigs: &[Sig],
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    for p in parents.iter().filter(|p| p.is_interface) {
        let is_get = p.sig.name.starts_with("__get_");
        if !is_get && !p.sig.name.starts_with("__set_") {
            continue;
        }
        for s in own_sigs {
            if s.name != p.sig.name || s.params.len() != p.sig.params.len() {
                continue;
            }
            let comparable = s
                .params
                .iter()
                .chain(s.ret.iter())
                .chain(p.sig.params.iter())
                .chain(p.sig.ret.iter())
                .all(|t| type_is_comparable(t, index));
            if !comparable {
                continue;
            }
            let (sp, sr) = canon_signature(s);
            let (pp, pr) = canon_signature(&p.sig);
            let ok = sp == pp && (!is_get || sr == pr);
            if !ok {
                errors.push(SemanticError {
                    message: format!(
                        "类型 '{}' 实现的属性 '{}' 类型与接口声明不一致（属性类型不允许协变/逆变）",
                        owner, p.sig.display
                    ),
                });
                break;
            }
        }
    }
}

fn check_duplicate_fields<'a, I: Iterator<Item = &'a str>>(
    owner: &str,
    fields: I,
    errors: &mut Vec<SemanticError>,
) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for name in fields {
        *seen.entry(name).or_insert(0) += 1;
    }
    for (name, count) in seen {
        if count > 1 {
            errors.push(SemanticError {
                message: format!("类型 '{}' 中字段 '{}' 被重复定义", owner, name),
            });
        }
    }
}

fn check_interface(iface: &InterfaceDef, errors: &mut Vec<SemanticError>) {
    let mut by_name: HashMap<String, Vec<Sig>> = HashMap::new();
    for method in &iface.methods {
        let sig = sig_of_interface_method(method);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        by_name.entry(sig.name.clone()).or_default().push(sig);
    }
    for group in by_name.values() {
        check_signature_group(&iface.name, group, false, errors);
    }
}

fn check_enum_variants(en: &EnumDef, errors: &mut Vec<SemanticError>) {
    // 同名但载荷类型不同的构造器是合法的重载；仅同名同载荷构成重定义。
    for i in 0..en.variants.len() {
        for j in (i + 1)..en.variants.len() {
            let a = &en.variants[i];
            let b = &en.variants[j];
            if a.name != b.name {
                continue;
            }
            let payload_key = |p: &Option<Type>| -> String {
                match p {
                    Some(t) => canon_type(t, &en.type_params, &mut HashMap::new()),
                    None => "()".to_string(),
                }
            };
            if payload_key(&a.payload) == payload_key(&b.payload) {
                errors.push(SemanticError {
                    message: format!("枚举 '{}' 中构造器 '{}' 被重复定义", en.name, a.name),
                });
                break;
            }
        }
    }
}

// ─── 父类型索引与覆写/实现校验 ─────────────────────────────────────────────

struct ProgramIndex<'a> {
    classes: HashMap<&'a str, &'a ClassDef>,
    interfaces: HashMap<&'a str, &'a InterfaceDef>,
    structs: HashSet<&'a str>,
    enums: HashSet<&'a str>,
    /// 各类型的声明类型参数（class/struct/enum）。接口的泛型参数未进入 AST。
    type_params: HashMap<&'a str, Vec<String>>,
    /// 本地可解析的类型名（用于判断签名是否可严格比较）。
    known_types: HashSet<&'a str>,
    /// 类型别名目标
    aliases: HashMap<&'a str, &'a Type>,
    /// extend 块提供的函数：目标类型名 → 方法列表
    extends: HashMap<&'a str, Vec<&'a Function>>,
}

impl<'a> ProgramIndex<'a> {
    fn new(program: &'a Program) -> Self {
        let mut classes = HashMap::new();
        let mut interfaces = HashMap::new();
        let mut type_params = HashMap::new();
        let mut known_types = HashSet::new();
        let mut structs = HashSet::new();
        let mut enums = HashSet::new();

        for c in &program.classes {
            classes.insert(c.name.as_str(), c);
            type_params.insert(c.name.as_str(), c.type_params.clone());
            known_types.insert(c.name.as_str());
        }
        for i in &program.interfaces {
            interfaces.insert(i.name.as_str(), i);
            known_types.insert(i.name.as_str());
        }
        for s in &program.structs {
            structs.insert(s.name.as_str());
            type_params.insert(s.name.as_str(), s.type_params.clone());
            known_types.insert(s.name.as_str());
        }
        for e in &program.enums {
            enums.insert(e.name.as_str());
            type_params.insert(e.name.as_str(), e.type_params.clone());
            known_types.insert(e.name.as_str());
        }
        let mut aliases = HashMap::new();
        for (name, ty) in &program.type_aliases {
            aliases.insert(name.as_str(), ty);
            known_types.insert(name.as_str());
        }

        let mut extends: HashMap<&str, Vec<&Function>> = HashMap::new();
        for ext in &program.extends {
            extends
                .entry(ext.target_type.as_str())
                .or_default()
                .extend(ext.methods.iter());
        }

        ProgramIndex {
            classes,
            interfaces,
            structs,
            enums,
            type_params,
            known_types,
            aliases,
            extends,
        }
    }

    fn type_params_of(&self, name: &str) -> &[String] {
        self.type_params
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// 判断 child 是否为 parent 的（本地可见的）子类型。
    fn is_subtype_name(&self, child: &str, parent: &str) -> bool {
        if child == parent {
            return true;
        }
        let mut stack: Vec<&str> = vec![child];
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(name) = stack.pop() {
            if !seen.insert(name) {
                continue;
            }
            if name == parent {
                return true;
            }
            if let Some(c) = self.classes.get(name) {
                if let Some(p) = c.extends.as_deref() {
                    stack.push(p);
                }
                for i in &c.implements {
                    stack.push(i.as_str());
                }
            }
            if let Some(i) = self.interfaces.get(name) {
                for p in &i.parents {
                    stack.push(p.as_str());
                }
            }
            if let Some(Type::Struct(pn, _)) = self.aliases.get(name) {
                stack.push(pn.as_str());
            }
        }
        false
    }
}

/// 判断类型是否可以在本地严格比较（无未解析泛型参数 / 外部类型）。
fn type_is_comparable(ty: &Type, index: &ProgramIndex<'_>) -> bool {
    match ty {
        Type::TypeParam(_) | Type::This => false,
        Type::Struct(name, args) => {
            index.known_types.contains(name.as_str())
                && args.iter().all(|a| type_is_comparable(a, index))
        }
        Type::Array(inner) | Type::Option(inner) | Type::Slice(inner) => {
            type_is_comparable(inner, index)
        }
        Type::Tuple(items) => items.iter().all(|t| type_is_comparable(t, index)),
        Type::Result(a, b) | Type::Map(a, b) => {
            type_is_comparable(a, index) && type_is_comparable(b, index)
        }
        Type::Function { params, ret } => {
            params.iter().all(|p| type_is_comparable(p, index))
                && match &**ret {
                    Some(t) => type_is_comparable(t, index),
                    None => true,
                }
        }
        Type::Qualified(_) => false,
        _ => true,
    }
}

fn return_compatible(child: &Type, parent: &Type, index: &ProgramIndex<'_>) -> bool {
    if child == parent {
        return true;
    }
    // This/Nothing 与泛型/协变场景：不做严格判定，避免误伤。
    if matches!(child, Type::This | Type::Nothing) || matches!(parent, Type::This) {
        return true;
    }
    match (child, parent) {
        (Type::Struct(c, _), Type::Struct(p, _)) => index.is_subtype_name(c, p),
        (Type::Option(c), Type::Option(p)) => return_compatible(c, p, index),
        _ => false,
    }
}

/// 覆写/实现匹配。
///
/// `strict_types == false` 表示父类型带类级泛型参数，子类型可能以具体类型覆写，
/// 此时只比较参数个数。签名含未解析泛型/外部类型时同样只比较参数个数。
fn signature_matches(a: &Sig, b: &Sig, strict_types: bool, index: &ProgramIndex<'_>) -> bool {
    if a.params.len() != b.params.len() {
        return false;
    }
    if !strict_types {
        return true;
    }
    let comparable = a
        .params
        .iter()
        .chain(a.ret.iter())
        .chain(b.params.iter())
        .chain(b.ret.iter())
        .all(|t| type_is_comparable(t, index));
    if !comparable {
        return true;
    }
    if canon_params(a) != canon_params(b) {
        return false;
    }
    // 属性访问器要求实现类型与接口声明完全一致（属性类型不变，不支持协变）。
    if a.name.starts_with("__get_") || a.name.starts_with("__set_") {
        let (_, ra) = canon_signature(a);
        let (_, rb) = canon_signature(b);
        return match (ra, rb) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        };
    }
    match (&a.ret, &b.ret) {
        (Some(x), Some(y)) => return_compatible(x, y, index),
        // 任一侧未标注返回类型时无法比较，视为兼容（不误伤）。
        _ => true,
    }
}

/// 父类型方法的来源签名。
struct ParentMethod {
    sig: Sig,
    /// 是否可以严格比较参数/返回类型（泛型父类型只比较参数个数）。
    strict_types: bool,
    /// 是否为接口中的抽象方法（无默认实现）。
    abstract_: bool,
    /// 是否来自接口（接口方法的 static/实例修饰符不进入 AST，无法区分）。
    is_interface: bool,
    /// 声明该方法的类型名（用于识别多个接口的默认实现冲突）。
    owner: String,
}

/// 收集一个 class 的所有本地父类型（父类 + 接口，含传递）的方法。
///
/// 任何父类型无法在本地解析时返回 `None`，调用方应保守跳过校验。
fn collect_parent_methods<'a>(
    cls: &'a ClassDef,
    index: &ProgramIndex<'a>,
) -> Option<Vec<ParentMethod>> {
    let mut out = Vec::new();
    let mut visited: HashSet<&'a str> = HashSet::new();
    let mut class_chain: Vec<&'a str> = Vec::new();
    let mut iface_names: Vec<&'a str> = Vec::new();

    // 直接父类型（extends 可能是父类或接口，implements 为接口）
    let mut direct: Vec<&'a str> = Vec::new();
    if let Some(parent) = cls.extends.as_deref() {
        direct.push(parent);
    }
    for name in &cls.implements {
        direct.push(name.as_str());
    }

    for name in direct {
        if index.classes.contains_key(name) {
            class_chain.push(name);
        } else if index.interfaces.contains_key(name) {
            iface_names.push(name);
        } else if index.structs.contains(name) || index.enums.contains(name) {
            // 继承 struct/enum 不合法，但语法上已解析；无法提供方法签名
            return None;
        } else {
            // 外部/未解析的父类型：保守跳过
            return None;
        }
    }

    // 沿父类链向上收集（每条链上的类都要求本地可解析）
    let mut chain_idx = 0;
    while chain_idx < class_chain.len() {
        let name = class_chain[chain_idx];
        chain_idx += 1;
        if !visited.insert(name) {
            continue;
        }
        let Some(parent) = index.classes.get(name) else {
            return None;
        };
        let strict = parent.type_params.is_empty();
        for m in &parent.methods {
            out.push(ParentMethod {
                sig: sig_of_function(&m.func, &parent.type_params),
                strict_types: strict,
                // 父类方法（无论有无函数体）都视为可用的实现来源；
                // 抽象类未实现的方法不在此强制子类实现（保守，避免误伤）。
                abstract_: false,
                is_interface: false,
                owner: name.to_string(),
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    is_interface: false,
                    owner: name.to_string(),
                });
            }
        }
        if let Some(p) = parent.extends.as_deref() {
            if index.classes.contains_key(p) {
                class_chain.push(p);
            } else if index.interfaces.contains_key(p) {
                iface_names.push(p);
            } else {
                return None;
            }
        }
        for i in &parent.implements {
            if index.interfaces.contains_key(i.as_str()) {
                iface_names.push(i.as_str());
            } else if !index.classes.contains_key(i.as_str()) {
                return None;
            }
        }
    }

    // 展开接口集合（含接口继承）
    let mut iface_idx = 0;
    while iface_idx < iface_names.len() {
        let name = iface_names[iface_idx];
        iface_idx += 1;
        if !visited.insert(name) {
            continue;
        }
        let Some(iface) = index.interfaces.get(name) else {
            return None;
        };
        for m in &iface.methods {
            out.push(ParentMethod {
                sig: sig_of_interface_method(m),
                strict_types: true,
                abstract_: m.default_body.is_none(),
                is_interface: true,
                owner: name.to_string(),
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    is_interface: true,
                    owner: name.to_string(),
                });
            }
        }
        for p in &iface.parents {
            if index.interfaces.contains_key(p.as_str()) {
                iface_names.push(p.as_str());
            } else if !index.classes.contains_key(p.as_str()) {
                return None;
            }
        }
    }

    Some(out)
}

/// unittest 框架注入到 TestCase 的生命周期钩子名。
/// `@Test` 宏会把测试类挂到 TestCase 下，并允许用 override 覆盖这些钩子；
/// 宏展开受限时父类型不可见，override 检查需对它们放行。
fn is_unittest_lifecycle_hook(name: &str) -> bool {
    matches!(name, "beforeAll" | "beforeEach" | "afterAll" | "afterEach")
}

/// override/redef 方法必须能在父类型中找到签名匹配的函数。
fn check_class_overrides(cls: &ClassDef, index: &ProgramIndex<'_>, errors: &mut Vec<SemanticError>) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        if !method.override_ {
            continue;
        }
        let sig = sig_of_function(&method.func, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        // 抽象的 mut prop 只会脱糖出 __get_x（解析器不生成抽象 setter），
        // 因此 setter 的 override 只要能找到同名 getter 就视为合法。
        let alt_getter = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let same_name: Vec<&ParentMethod> = parents
            .iter()
            .filter(|p| {
                p.sig.name == sig.name || alt_getter.as_deref() == Some(p.sig.name.as_str())
            })
            .collect();
        if same_name.is_empty() {
            // unittest 测试框架宏（@Test/@TestCase）会给测试类注入 TestCase 父类，
            // 从而提供 beforeAll/beforeEach/afterAll/afterEach 这些生命周期方法。
            // 由于宏展开能力有限，这里对生命周期钩子的“无父类可覆盖”错误放行，
            // 避免 std.unittest 用例因宏未展开而编译失败。
            if is_unittest_lifecycle_hook(&sig.name) {
                continue;
            }
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 使用了 override/redef，但父类型中不存在可覆盖的同名函数",
                    cls.name, sig.display
                ),
            });
            continue;
        }
        let matched = same_name.iter().any(|p| {
            (p.sig.name == sig.name && signature_matches(&sig, &p.sig, p.strict_types, index))
                || alt_getter.as_deref() == Some(p.sig.name.as_str())
        });
        if !matched {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的 '{}' 与父类型函数的参数列表或返回类型不一致",
                    cls.name, sig.display
                ),
            });
        }
    }
}

/// 覆写 / 重定义（含未写 override 关键字的同名静态重定义）不得降低可见性（6.3.5(2)）。
fn check_member_visibility_lowering(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        let sig = sig_of_function(&method.func, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let Some(child_rank) = sig.vis.as_ref().map(visibility_rank) else {
            continue;
        };
        // 属性的 setter 只有父接口可能只声明 getter。
        let alt_getter = sig
            .name
            .strip_prefix("__set_")
            .map(|rest| format!("__get_{}", rest));
        let parent_rank = parents
            .iter()
            .filter(|p| !p.is_interface)
            .filter(|p| {
                p.sig.is_static == sig.is_static
                    && (p.sig.name == sig.name || alt_getter.as_deref() == Some(p.sig.name.as_str()))
                    && (signature_matches(&sig, &p.sig, p.strict_types, index)
                        || alt_getter.as_deref() == Some(p.sig.name.as_str()))
            })
            .filter_map(|p| p.sig.vis.as_ref().map(visibility_rank))
            .max();
        if let Some(parent_rank) = parent_rank {
            if child_rank < parent_rank {
                errors.push(SemanticError {
                    message: format!(
                        "类 '{}' 的 '{}' 覆写/重定义时降低了访问级别（父类型更宽松）",
                        cls.name, sig.display
                    ),
                });
            }
        }
    }
}

/// 检查类自身方法与父类（非接口）方法之间的 static/实例同名冲突
/// （6.3.2(1) 中“包括继承自父类”的部分）。
fn check_inherited_static_conflicts(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    for method in &cls.methods {
        let sig = sig_of_function(&method.func, &cls.type_params);
        if is_synthetic_member(&sig.name) {
            continue;
        }
        let conflict = parents.iter().any(|p| {
            !p.is_interface
                && p.sig.name == sig.name
                && p.sig.is_static != sig.is_static
                && !is_synthetic_member(&p.sig.name)
        });
        if conflict {
            errors.push(SemanticError {
                message: format!(
                    "类 '{}' 的成员函数 '{}' 与父类同名函数在 static/实例属性上冲突，不允许互为重载",
                    cls.name, sig.display
                ),
            });
        }
    }
}

/// 把“自身可用实现 + 父类型中带实现的方法”与接口抽象方法比对，报告缺失/签名不匹配。
fn check_abstract_methods_implemented(
    owner: &str,
    own_impls: Vec<Sig>,
    parents: &[ParentMethod],
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    // 真实实现：实现类型自身的方法 + 父类（非接口）方法 + extend 方法。
    let mut real_impls: Vec<Sig> = own_impls;
    for p in parents {
        if !p.is_interface && !p.abstract_ {
            real_impls.push(p.sig.clone());
        }
    }
    // 接口默认实现也可以满足另一个接口的抽象方法。
    let mut owned_impls: Vec<Sig> = real_impls.clone();
    for p in parents {
        if p.is_interface && !p.abstract_ {
            owned_impls.push(p.sig.clone());
        }
    }
    let impls: Vec<&Sig> = owned_impls.iter().collect();

    let mut reported: HashSet<String> = HashSet::new();
    for p in parents {
        if !p.abstract_ || is_synthetic_member(&p.sig.name) {
            continue;
        }
        // 属性抽象声明 `prop p` 会脱糖为 `__get_p`，setter 不参与强制实现。
        let has_same_name = impls.iter().any(|s| s.name == p.sig.name);
        let matched = impls.iter().any(|s| {
            s.name == p.sig.name && signature_matches(s, &p.sig, p.strict_types, index)
        });
        if matched {
            continue;
        }
        if reported.insert(p.sig.name.clone()) {
            let reason = if has_same_name {
                "参数列表或返回类型不匹配"
            } else {
                "缺少实现"
            };
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 未正确实现接口方法 '{}'（{}）",
                    owner, p.sig.display, reason
                ),
            });
        }
    }

    // 多个接口为同名同签名的函数提供默认实现时，实现类型不能依赖继承默认实现（6.2.4.1(9)）。
    let mut defaults: HashMap<(String, String), (HashSet<String>, Sig)> = HashMap::new();
    for p in parents {
        if p.is_interface && !p.abstract_ && !is_synthetic_member(&p.sig.name) {
            let entry = defaults
                .entry((p.sig.name.clone(), canon_params(&p.sig)))
                .or_insert_with(|| (HashSet::new(), p.sig.clone()));
            entry.0.insert(p.owner.clone());
        }
    }
    for ((name, _params), (owners, sample)) in defaults {
        if owners.len() < 2 {
            continue;
        }
        // 若某个提供默认实现的接口是另一个的父接口，子接口的默认实现会覆盖父接口的，
        // 不构成“多个默认实现冲突”，只保留最派生的实现者。
        let owners_vec: Vec<&String> = owners.iter().collect();
        let maximal: Vec<&&String> = owners_vec
            .iter()
            .filter(|a| {
                !owners_vec
                    .iter()
                    .any(|b| b != *a && index.is_subtype_name(b, a))
            })
            .collect();
        if maximal.len() < 2 {
            continue;
        }
        let real = real_impls
            .iter()
            .any(|s| s.name == name && signature_matches(s, &sample, true, index));
        if !real {
            errors.push(SemanticError {
                message: format!(
                    "类型 '{}' 的函数 '{}' 在多个接口中存在默认实现，必须显式实现（不能依赖继承）",
                    owner, sample.display
                ),
            });
        }
    }
}

/// 非抽象类必须实现其（本地）接口的抽象方法。
fn check_interface_implementation(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    if cls.is_abstract {
        return;
    }
    let Some(parents) = collect_parent_methods(cls, index) else {
        return;
    };
    if parents.is_empty() {
        return;
    }

    let mut own_impls: Vec<Sig> = cls
        .methods
        .iter()
        .map(|m| sig_of_function(&m.func, &cls.type_params))
        .collect();
    // extend 块中为当前类提供的方法
    if let Some(exts) = index.extends.get(cls.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &cls.type_params));
        }
    }
    check_abstract_methods_implemented(&cls.name, own_impls, &parents, index, errors);
}

/// 接口的 const 函数必须由 const 函数实现（16.4(6) 约束）。
///
/// 同名同签名但未声明 const 的类方法不能实现接口的 const 函数。
fn check_const_interface_impls(
    cls: &ClassDef,
    index: &ProgramIndex<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut queue: Vec<String> = Vec::new();
    if let Some(p) = cls.extends.as_deref() {
        if index.interfaces.contains_key(p) {
            queue.push(p.to_string());
        }
    }
    for i in &cls.implements {
        if index.interfaces.contains_key(i.as_str()) {
            queue.push(i.clone());
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(iface) = index.interfaces.get(name.as_str()) else {
            continue;
        };
        for m in &iface.methods {
            if !m.is_const || is_synthetic_member(&m.name) {
                continue;
            }
            let isig = sig_of_interface_method(m);
            for cm in &cls.methods {
                if cm.is_const {
                    continue;
                }
                let csig = sig_of_function(&cm.func, &cls.type_params);
                if csig.name == isig.name && signature_matches(&csig, &isig, true, index) {
                    errors.push(SemanticError {
                        message: format!(
                            "类 '{}' 的 '{}' 实现接口 '{}' 的 const 函数时必须声明为 const",
                            cls.name, csig.display, name
                        ),
                    });
                }
            }
        }
        for p in &iface.parents {
            queue.push(p.clone());
        }
    }
}

/// 收集一组接口（含传递父接口）的方法；任一接口无法本地解析时返回 None。
fn collect_interface_methods<'a>(
    names: &[&'a str],
    index: &ProgramIndex<'a>,
) -> Option<Vec<ParentMethod>> {
    let mut out = Vec::new();
    let mut visited: HashSet<&'a str> = HashSet::new();
    let mut queue: Vec<&'a str> = names.to_vec();
    let mut i = 0;
    while i < queue.len() {
        let name = queue[i];
        i += 1;
        if !visited.insert(name) {
            continue;
        }
        let iface = index.interfaces.get(name)?;
        for m in &iface.methods {
            out.push(ParentMethod {
                sig: sig_of_interface_method(m),
                strict_types: true,
                abstract_: m.default_body.is_none(),
                is_interface: true,
                owner: name.to_string(),
            });
        }
        if let Some(exts) = index.extends.get(name) {
            for f in exts {
                out.push(ParentMethod {
                    sig: sig_of_function(f, index.type_params_of(name)),
                    strict_types: true,
                    abstract_: false,
                    is_interface: true,
                    owner: name.to_string(),
                });
            }
        }
        for p in &iface.parents {
            queue.push(p.as_str());
        }
    }
    Some(out)
}

/// 结构体通过 `struct S <: I` 实现接口时，必须实现接口抽象方法。
///
/// 解析器把 `struct S <: I & J` 记录为约束 `S: [I, J]`（`param` 为结构体名），
/// 因此这里可以直接复用该信息，无需扩展 AST。
fn check_struct_interfaces<'a>(
    st: &'a StructDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let bounds: Vec<&'a str> = st
        .constraints
        .iter()
        .filter(|c| c.param == st.name)
        .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods(&bounds, index) else {
        return;
    };
    let mut own_impls: Vec<Sig> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name) == Some(st.name.as_str()))
        .map(|f| sig_of_function(f, &st.type_params))
        .collect();
    if let Some(exts) = index.extends.get(st.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &st.type_params));
        }
    }
    if parents.iter().any(|p| p.abstract_) {
        check_abstract_methods_implemented(&st.name, own_impls.clone(), &parents, index, errors);
    }
    check_interface_property_types(&st.name, &own_impls, &parents, index, errors);
}

fn is_property_method_name(name: &str) -> bool {
    name.starts_with("__get_") || name.starts_with("__set_")
}

/// 枚举通过 `enum E <: I` 实现接口时，必须实现接口的抽象函数。
///
/// 枚举的 `prop` 成员解析时被丢弃（未脱糖为方法），因此跳过接口属性相关检查，
/// 只校验抽象函数，避免误判。
fn check_enum_interfaces<'a>(
    en: &'a EnumDef,
    program: &'a Program,
    index: &ProgramIndex<'a>,
    errors: &mut Vec<SemanticError>,
) {
    let bounds: Vec<&'a str> = en
        .constraints
        .iter()
        .filter(|c| c.param == en.name)
        .flat_map(|c| c.bounds.iter().map(|b| b.as_str()))
        .collect();
    if bounds.is_empty() {
        return;
    }
    let Some(parents) = collect_interface_methods(&bounds, index) else {
        return;
    };
    let parents: Vec<ParentMethod> = parents
        .into_iter()
        .filter(|p| !is_property_method_name(&p.sig.name))
        .collect();
    if !parents.iter().any(|p| p.abstract_) {
        return;
    }
    let mut own_impls: Vec<Sig> = program
        .functions
        .iter()
        .filter(|f| owner_name(&f.name) == Some(en.name.as_str()))
        .map(|f| sig_of_function(f, &en.type_params))
        .collect();
    if let Some(exts) = index.extends.get(en.name.as_str()) {
        for f in exts {
            own_impls.push(sig_of_function(f, &en.type_params));
        }
    }
    check_abstract_methods_implemented(&en.name, own_impls, &parents, index, errors);
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
                    is_const: false,
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

    // ─── validate_program: 语义校验 ─────────────────────────────────────────

    /// 直接走 lexer + parser 得到 AST（不经过 pipeline 的校验），便于测试 validate_program。
    fn program_of(source: &str) -> Program {
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Result<Vec<_>, _> = lexer.collect();
        let tokens = tokens.expect("词法分析应成功");
        crate::parser::Parser::new(tokens)
            .with_source(source)
            .parse_program()
            .expect("语法分析应成功")
    }

    fn assert_valid(source: &str) {
        let program = program_of(source);
        if let Err(errs) = validate_program(&program) {
            panic!("合法程序被误判: {:?}", errs);
        }
    }

    fn assert_invalid(source: &str) -> Vec<SemanticError> {
        let program = program_of(source);
        match validate_program(&program) {
            Ok(()) => panic!("非法程序未被拒绝: {}", source),
            Err(errs) => errs,
        }
    }

    #[test]
    fn test_validate_duplicate_class_method() {
        assert_invalid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   func f(y: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_legal_overload_not_rejected() {
        assert_valid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   func f(x: Int64, y: Bool) {}\n\
             \x20   func f(x: String) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_static_instance_same_name() {
        let errs = assert_invalid(
            "class A {\n\
             \x20   func f(x: Int64) {}\n\
             \x20   static func f() {}\n\
             }\n\
             main() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("静态成员函数与实例成员函数同名")));
    }

    #[test]
    fn test_validate_duplicate_top_level_function() {
        assert_invalid(
            "func f(a: Int64) {}\nfunc f(b: Int64) {}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_generic_vs_non_generic_not_redefinition() {
        // 5.1.6(3): 泛型与非泛型同名不构成重定义
        assert_valid(
            "func f(a: Int32, b!: Int32): Int32 { a }\n\
             func f<A>(a: A, b!: A): A { b }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_generic_type_param_order_not_redefinition() {
        // 5.1.6(7): func f<X,Y>(a:X,b:Y) 与 func f<Y,X>(a:X,b:Y) 不构成重定义
        assert_valid(
            "func f<X, Y>(a: X, b: Y) {}\n\
             func f<Y, X>(a: X, b: Y) {}\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_duplicate_interface_generic_method() {
        // 5.1.6(5): 重命名泛型参数后非泛型部分相同 → 重定义
        assert_invalid(
            "interface I {\n\
             \x20   static func f<A>(a: A): A\n\
             \x20   static func f<B>(a: B): B\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_override_without_parent() {
        let errs = assert_invalid("class A {\n    override func f() {}\n}\nmain() {}\n");
        assert!(errs.iter().any(|e| e.message.contains("不存在可覆盖的同名函数")));
    }

    #[test]
    fn test_validate_override_signature_mismatch() {
        assert_invalid(
            "open class P {\n\
             \x20   open func f() {}\n\
             }\n\
             class C <: P {\n\
             \x20   override func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_override_matching_signature_ok() {
        assert_valid(
            "open class P {\n\
             \x20   open func f(x: Int64) {}\n\
             }\n\
             class C <: P {\n\
             \x20   override func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_override_covariant_return_ok() {
        assert_valid(
            "open class Sup {}\n\
             class Sub <: Sup {}\n\
             open class P {\n\
             \x20   open func f(): Sup { Sup() }\n\
             }\n\
             class C <: P {\n\
             \x20   override func f(): Sub { Sub() }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_interface_missing_implementation() {
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             class A <: I {\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_interface_implementation_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f(x: Int64): Int64\n\
             }\n\
             class A <: I {\n\
             \x20   func f(x: Int64) { x }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_struct_interface_missing_implementation() {
        assert_invalid(
            "interface I {\n\
             \x20   func f(): Unit\n\
             }\n\
             struct S <: I {\n\
             \x20   func f(x: Int64) {}\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_property_type_mismatch() {
        assert_invalid(
            "interface I {\n\
             \x20   mut prop p: Int64\n\
             }\n\
             class A <: I {\n\
             \x20   public mut prop p: String {\n\
             \x20       get() { \"\" }\n\
             \x20       set(v) {}\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_enum_duplicate_variant() {
        assert_invalid("enum E {\n    A(Int8)\n    A(Int8)\n}\nmain() {}\n");
    }

    #[test]
    fn test_validate_enum_variant_overload_ok() {
        // 同名但载荷不同 → 合法重载
        assert_valid("enum E {\n    A\n    A(Int8)\n}\nmain() {}\n");
    }

    #[test]
    fn test_validate_duplicate_field() {
        assert_invalid("class A {\n    var x: Int64 = 1\n    var x: Int64 = 2\n}\nmain() {}\n");
    }

    // ─── validate_program: 类/接口规则补强（06 章） ─────────────────────────

    #[test]
    fn test_validate_abstract_class_instantiation() {
        let errs = assert_invalid(
            "abstract class A {}\nmain() {\n    let a = A()\n}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("抽象类") && e.message.contains("实例化")));
    }

    #[test]
    fn test_validate_interface_instantiation() {
        let errs = assert_invalid(
            "interface I {\n    func f(): Int64\n}\nmain() {\n    let x = I()\n}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("接口") && e.message.contains("实例化")));
    }

    #[test]
    fn test_validate_abstract_class_as_type_ok() {
        // 仅作为类型使用（不实例化）是合法的
        assert_valid(
            "abstract class A {}\nfunc f(a: A) {}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_shadowed_type_name_not_instantiation() {
        // 局部函数遮蔽同名接口时，该调用不是接口实例化（避免误伤遮蔽用例）
        assert_valid(
            "interface I {}\nmain() {\n    func I(): Int64 { 1 }\n    let x = I()\n}\n",
        );
    }

    #[test]
    fn test_validate_multiple_inheritance() {
        let errs = assert_invalid(
            "open class A {}\nopen class B {}\nclass C <: A & B {}\nmain() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("单继承")));
    }

    #[test]
    fn test_validate_interface_before_superclass() {
        // 接口写在父类之前：解析器把首个当成 extends，类名落入 implements
        assert_invalid(
            "interface I {}\nopen class A {}\nclass B <: I & A {}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_inherit_non_open_class() {
        let errs = assert_invalid("class A {}\nclass B <: A {}\nmain() {}\n");
        assert!(errs.iter().any(|e| e.message.contains("非 open")));
    }

    #[test]
    fn test_validate_inherit_open_class_ok() {
        assert_valid("open class A {}\nclass B <: A {}\nmain() {}\n");
    }

    #[test]
    fn test_validate_inherit_sealed_class_same_file_ok() {
        // sealed 类允许在同包（单文件）内被继承
        assert_valid("sealed class A {}\nclass B <: A {}\nmain() {}\n");
    }

    #[test]
    fn test_validate_inherit_struct() {
        assert_invalid("struct S {}\nclass C <: S {}\nmain() {}\n");
    }

    #[test]
    fn test_validate_override_lowers_visibility() {
        let errs = assert_invalid(
            "open class P {\n    public open func f() {}\n}\nclass C <: P {\n    override func f() {}\n}\nmain() {}\n",
        );
        assert!(errs.iter().any(|e| e.message.contains("降低")));
    }

    #[test]
    fn test_validate_override_raises_visibility_ok() {
        assert_valid(
            "open class P {\n    protected open func f() {}\n}\nclass C <: P {\n    public override func f() {}\n}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_static_redef_lowers_visibility() {
        // 未写 override 关键字的同名静态重定义也受可见性约束（6.3.5(2)）
        assert_invalid(
            "open class P {\n    protected static func f() {}\n}\nclass C <: P {\n    private static func f() {}\n}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_struct_interface_property_type_mismatch() {
        assert_invalid(
            "open class Sup {}\n\
             class Sub <: Sup {}\n\
             interface I {\n\
             \x20   prop p: Sub {\n\
             \x20       get() { Sub() }\n\
             \x20   }\n\
             }\n\
             struct A <: I {\n\
             \x20   public prop p: Sup {\n\
             \x20       get() { Sub() }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_struct_interface_property_type_ok() {
        assert_valid(
            "interface I {\n\
             \x20   prop p: Int64 {\n\
             \x20       get() { 0 }\n\
             \x20   }\n\
             }\n\
             struct A <: I {\n\
             \x20   public prop p: Int64 {\n\
             \x20       get() { 1 }\n\
             \x20   }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_validate_enum_missing_interface_function() {
        assert_invalid(
            "interface I {\n    func f(): Int64\n}\nenum E <: I {\n    A\n}\nmain() {}\n",
        );
    }

    #[test]
    fn test_validate_enum_implements_interface_ok() {
        assert_valid(
            "interface I {\n\
             \x20   func f(): Int64\n\
             }\n\
             enum E <: I {\n\
             \x20   A\n\
             \x20   public func f(): Int64 { 1 }\n\
             }\n\
             main() {}\n",
        );
    }

    #[test]
    fn test_w5c_const_interface_func_requires_const_impl() {
        // 接口 const 函数必须由 const 函数实现
        assert_invalid(
            "interface I {\n\
             \x20   const static func f(): Int64\n\
             }\n\
             class C <: I {\n\
             \x20   public static func f() { 1 }\n\
             }\n\
             main() {}\n",
        );
        assert_valid(
            "interface I {\n\
             \x20   const static func f(): Int64\n\
             }\n\
             class C <: I {\n\
             \x20   public const static func f() { 1 }\n\
             }\n\
             main() {}\n",
        );
    }
}
