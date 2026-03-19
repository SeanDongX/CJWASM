//! AST → CHIR 完整降低（Lowering）

use crate::ast::{Function, Param, Program, Type, TypeConstraint, Visibility};
use crate::chir::lower_expr::LoweringContext;
use crate::chir::type_inference::TypeInferenceContext;
use crate::chir::{CHIRFunction, CHIRParam, CHIRProgram};
use std::collections::{HashMap, HashSet};

/// 继承合法性检查：避免自继承/循环继承导致 lowering 或 codegen 死循环。
fn validate_class_inheritance(program: &Program) -> Result<(), String> {
    let extends: HashMap<String, String> = program
        .classes
        .iter()
        .filter_map(|c| c.extends.as_ref().map(|p| (c.name.clone(), p.clone())))
        .collect();

    // cjc 对齐：直接自继承给出专用错误。
    for class_def in &program.classes {
        if class_def.extends.as_ref() == Some(&class_def.name) {
            return Err(format!(
                "declaration '{}' cannot inherit itself",
                class_def.name
            ));
        }
    }

    // 通用循环继承检测：A <: B <: ... <: A
    for class_def in &program.classes {
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(class_def.name.clone());
        let mut chain = vec![class_def.name.clone()];
        let mut parent = class_def.extends.clone();

        while let Some(parent_name) = parent {
            chain.push(parent_name.clone());
            if !seen.insert(parent_name.clone()) {
                return Err(format!(
                    "cyclic class inheritance detected: {}",
                    chain.join(" <: ")
                ));
            }
            parent = extends.get(&parent_name).cloned();
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct MethodSig {
    name: String,
    params: Vec<Type>,
    return_type: Option<Type>,
    visibility: Visibility,
    is_static: bool,
    type_params: Vec<String>,
    constraints: Vec<TypeConstraint>,
}

#[derive(Debug, Clone)]
struct ClassMethodSig {
    sig: MethodSig,
    is_override: bool,
}

#[derive(Debug, Clone)]
struct InterfaceMethodReq {
    sig: MethodSig,
    required: bool,
}

fn method_short_name(owner: &str, full_name: &str) -> String {
    if let Some(short) = full_name.strip_prefix(&format!("{owner}.")) {
        return short.to_string();
    }
    full_name
        .rsplit_once('.')
        .map(|(_, short)| short.to_string())
        .unwrap_or_else(|| full_name.to_string())
}

fn split_receiver(params: &[Param]) -> (bool, Vec<Type>) {
    let has_receiver = params
        .first()
        .map(|p| p.name == "this" || p.name == "self")
        .unwrap_or(false);
    let start = if has_receiver { 1 } else { 0 };
    (
        !has_receiver,
        params[start..].iter().map(|p| p.ty.clone()).collect(),
    )
}

fn class_method_sig(owner: &str, method: &crate::ast::ClassMethod) -> ClassMethodSig {
    let short_name = method_short_name(owner, &method.func.name);
    let (is_static, params) = split_receiver(&method.func.params);
    ClassMethodSig {
        sig: MethodSig {
            name: short_name,
            params,
            return_type: method.func.return_type.clone(),
            visibility: method.func.visibility.clone(),
            is_static,
            type_params: method.func.type_params.clone(),
            constraints: method.func.constraints.clone(),
        },
        is_override: method.override_,
    }
}

fn top_level_method_sig(owner: &str, func: &Function) -> MethodSig {
    let short_name = method_short_name(owner, &func.name);
    let (is_static, params) = split_receiver(&func.params);
    MethodSig {
        name: short_name,
        params,
        return_type: func.return_type.clone(),
        visibility: func.visibility.clone(),
        is_static,
        type_params: func.type_params.clone(),
        constraints: func.constraints.clone(),
    }
}

fn interface_method_sig(method: &crate::ast::InterfaceMethod) -> MethodSig {
    MethodSig {
        name: method.name.clone(),
        params: method.params.iter().map(|p| p.ty.clone()).collect(),
        return_type: method.return_type.clone(),
        visibility: Visibility::Public,
        is_static: false,
        type_params: method.type_params.clone(),
        constraints: method.constraints.clone(),
    }
}

fn sig_key(sig: &MethodSig) -> String {
    format!(
        "{}|{}|{:?}|{:?}",
        sig.name,
        sig.type_params.len(),
        sig.params,
        sig.return_type
    )
}

fn same_name_params(lhs: &MethodSig, rhs: &MethodSig) -> bool {
    lhs.name == rhs.name
        && lhs.params == rhs.params
        && lhs.type_params.len() == rhs.type_params.len()
}

fn visibility_rank(v: &Visibility) -> u8 {
    match v {
        Visibility::Private => 0,
        Visibility::Internal => 1,
        Visibility::Protected => 2,
        Visibility::Public => 3,
    }
}

fn visibility_at_least(child: &Visibility, base: &Visibility) -> bool {
    visibility_rank(child) >= visibility_rank(base)
}

fn validate_upper_bounds(
    constraints: &[crate::ast::TypeConstraint],
    class_names: &HashSet<String>,
    interface_names: &HashSet<String>,
    type_params: &HashSet<String>,
) -> Result<(), String> {
    for constraint in constraints {
        for bound in &constraint.bounds {
            if class_names.contains(bound)
                || interface_names.contains(bound)
                || bound == "Object"
                || bound == "Any"
                || type_params.contains(bound)
            {
                continue;
            }
            return Err(format!(
                "the upper bound '{}' of generic parameter 'Generics-{}' must be class or interface",
                bound, constraint.param
            ));
        }
    }
    Ok(())
}

fn constraints_by_param_index(sig: &MethodSig) -> HashMap<usize, Vec<String>> {
    let index_map: HashMap<String, usize> = sig
        .type_params
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.clone(), idx))
        .collect();
    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    for c in &sig.constraints {
        if let Some(&idx) = index_map.get(&c.param) {
            let entry = out.entry(idx).or_default();
            for b in &c.bounds {
                if !entry.contains(b) {
                    entry.push(b.clone());
                }
            }
        }
    }
    out
}

fn is_super_class_of(
    candidate_super: &str,
    maybe_sub: &str,
    class_parent: &HashMap<String, Option<String>>,
) -> bool {
    let mut cur = Some(maybe_sub.to_string());
    while let Some(name) = cur {
        if name == candidate_super {
            return true;
        }
        cur = class_parent.get(&name).cloned().flatten();
    }
    false
}

fn is_super_interface_of(
    candidate_super: &str,
    maybe_sub: &str,
    interface_map: &HashMap<String, &crate::ast::InterfaceDef>,
    visiting: &mut HashSet<String>,
) -> bool {
    if candidate_super == maybe_sub {
        return true;
    }
    if !visiting.insert(maybe_sub.to_string()) {
        return false;
    }
    let Some(iface) = interface_map.get(maybe_sub) else {
        visiting.remove(maybe_sub);
        return false;
    };
    for parent in &iface.parents {
        if is_super_interface_of(candidate_super, parent, interface_map, visiting) {
            visiting.remove(maybe_sub);
            return true;
        }
    }
    visiting.remove(maybe_sub);
    false
}

fn is_bound_looser_or_equal(
    child_bound: &str,
    parent_bound: &str,
    class_parent: &HashMap<String, Option<String>>,
    interface_map: &HashMap<String, &crate::ast::InterfaceDef>,
) -> bool {
    if child_bound == parent_bound || child_bound == "Any" {
        return true;
    }
    if parent_bound == "Any" {
        return child_bound == "Any";
    }
    if child_bound == "Object" {
        return parent_bound == "Object"
            || class_parent.contains_key(parent_bound)
            || interface_map.contains_key(parent_bound);
    }
    if parent_bound == "Object" {
        return false;
    }
    if class_parent.contains_key(parent_bound) {
        return class_parent.contains_key(child_bound)
            && is_super_class_of(child_bound, parent_bound, class_parent);
    }
    if interface_map.contains_key(parent_bound) {
        if !interface_map.contains_key(child_bound) {
            return false;
        }
        let mut visiting = HashSet::new();
        return is_super_interface_of(child_bound, parent_bound, interface_map, &mut visiting);
    }
    // 泛型上界（如 T1 <: T2）当前仅支持同名等价比较。
    child_bound == parent_bound
}

fn constraints_not_tighter_than_parent(
    child: &MethodSig,
    parent: &MethodSig,
    class_parent: &HashMap<String, Option<String>>,
    interface_map: &HashMap<String, &crate::ast::InterfaceDef>,
) -> bool {
    if child.type_params.is_empty() && parent.type_params.is_empty() {
        return true;
    }
    if child.type_params.len() != parent.type_params.len() {
        return false;
    }

    let child_constraints = constraints_by_param_index(child);
    let parent_constraints = constraints_by_param_index(parent);

    for idx in 0..child.type_params.len() {
        let child_bounds = child_constraints.get(&idx).cloned().unwrap_or_default();
        if child_bounds.is_empty() {
            continue;
        }
        let parent_bounds = parent_constraints.get(&idx).cloned().unwrap_or_default();
        if parent_bounds.is_empty() {
            if child_bounds.iter().all(|b| b == "Any") {
                continue;
            }
            return false;
        }
        for child_bound in child_bounds {
            if !parent_bounds.iter().any(|parent_bound| {
                is_bound_looser_or_equal(&child_bound, parent_bound, class_parent, interface_map)
            }) {
                return false;
            }
        }
    }
    true
}

fn collect_interface_methods(
    iface_name: &str,
    interface_map: &HashMap<String, &crate::ast::InterfaceDef>,
    cache: &mut HashMap<String, HashMap<String, InterfaceMethodReq>>,
    visiting: &mut HashSet<String>,
) -> Result<HashMap<String, InterfaceMethodReq>, String> {
    if let Some(cached) = cache.get(iface_name) {
        return Ok(cached.clone());
    }
    if !visiting.insert(iface_name.to_string()) {
        return Err(format!(
            "cyclic interface inheritance detected at '{}'",
            iface_name
        ));
    }

    let iface = interface_map
        .get(iface_name)
        .ok_or_else(|| format!("undeclared type name '{}'", iface_name))?;

    let mut methods: HashMap<String, InterfaceMethodReq> = HashMap::new();
    for parent in &iface.parents {
        let parent_methods = collect_interface_methods(parent, interface_map, cache, visiting)?;
        for (key, req) in parent_methods {
            methods.entry(key).or_insert(req);
        }
    }

    for method in &iface.methods {
        let sig = interface_method_sig(method);
        methods.insert(
            sig_key(&sig),
            InterfaceMethodReq {
                sig,
                required: method.default_body.is_none(),
            },
        );
    }

    visiting.remove(iface_name);
    cache.insert(iface_name.to_string(), methods.clone());
    Ok(methods)
}

fn has_chain_method_with_same_name_params(
    class_name: &str,
    expected: &MethodSig,
    class_methods: &HashMap<String, HashMap<String, Vec<ClassMethodSig>>>,
    class_parent: &HashMap<String, Option<String>>,
) -> bool {
    let mut cur = Some(class_name.to_string());
    while let Some(name) = cur {
        if let Some(methods_by_name) = class_methods.get(&name) {
            if let Some(methods) = methods_by_name.get(&expected.name) {
                if methods.iter().any(|m| same_name_params(&m.sig, expected)) {
                    return true;
                }
            }
        }
        cur = class_parent.get(&name).cloned().flatten();
    }
    false
}

/// P1-2：类/接口语义校验（CHIR 路径）。
fn validate_class_interface_semantics(program: &Program) -> Result<(), String> {
    let class_map: HashMap<String, &crate::ast::ClassDef> = program
        .classes
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();
    let interface_map: HashMap<String, &crate::ast::InterfaceDef> = program
        .interfaces
        .iter()
        .map(|i| (i.name.clone(), i))
        .collect();
    let class_names: HashSet<String> = class_map.keys().cloned().collect();
    let interface_names: HashSet<String> = interface_map.keys().cloned().collect();

    // 接口父接口合法性与重复检查
    for iface in &program.interfaces {
        let mut seen = HashSet::new();
        for parent in &iface.parents {
            if !interface_map.contains_key(parent.as_str()) {
                return Err(format!("undeclared type name '{}'", parent));
            }
            if !seen.insert(parent.clone()) {
                return Err(format!(
                    "interface '{}' inherits or implements duplicate interface '{}'",
                    iface.name, parent
                ));
            }
        }
    }

    // 泛型上界必须是 class 或 interface（CJC 对齐）
    for class_def in &program.classes {
        let class_type_params: HashSet<String> = class_def.type_params.iter().cloned().collect();
        validate_upper_bounds(
            &class_def.constraints,
            &class_names,
            &interface_names,
            &class_type_params,
        )?;
        for method in &class_def.methods {
            let method_type_params: HashSet<String> = class_def
                .type_params
                .iter()
                .cloned()
                .chain(method.func.type_params.iter().cloned())
                .collect();
            validate_upper_bounds(
                &method.func.constraints,
                &class_names,
                &interface_names,
                &method_type_params,
            )?;
        }
    }
    for iface in &program.interfaces {
        for method in &iface.methods {
            let method_type_params: HashSet<String> = method.type_params.iter().cloned().collect();
            validate_upper_bounds(
                &method.constraints,
                &class_names,
                &interface_names,
                &method_type_params,
            )?;
        }
    }
    for struct_def in &program.structs {
        let struct_type_params: HashSet<String> = struct_def.type_params.iter().cloned().collect();
        validate_upper_bounds(
            &struct_def.constraints,
            &class_names,
            &interface_names,
            &struct_type_params,
        )?;
    }

    let mut class_parent: HashMap<String, Option<String>> = HashMap::new();
    let mut class_direct_ifaces: HashMap<String, Vec<String>> = HashMap::new();
    let mut class_methods: HashMap<String, HashMap<String, Vec<ClassMethodSig>>> = HashMap::new();

    // 预处理 class：规范化 extends/implements，并收集方法签名
    for class_def in &program.classes {
        if class_def.is_sealed && !class_def.is_abstract {
            return Err("non-abstract class cannot be modified by 'sealed'".to_string());
        }

        let mut parent = None;
        let mut interfaces = class_def.implements.clone();
        if let Some(ext) = &class_def.extends {
            if class_map.contains_key(ext.as_str()) {
                parent = Some(ext.clone());
            } else if interface_map.contains_key(ext.as_str()) {
                interfaces.push(ext.clone());
            } else {
                return Err(format!("undeclared type name '{}'", ext));
            }
        }

        let mut seen = HashSet::new();
        for iface in &interfaces {
            if !interface_map.contains_key(iface.as_str()) {
                return Err(format!("undeclared type name '{}'", iface));
            }
            if !seen.insert(iface.clone()) {
                return Err(format!(
                    "class '{}' inherits or implements duplicate interface '{}'",
                    class_def.name, iface
                ));
            }
        }

        class_parent.insert(class_def.name.clone(), parent);
        class_direct_ifaces.insert(class_def.name.clone(), interfaces);

        let mut by_name: HashMap<String, Vec<ClassMethodSig>> = HashMap::new();
        for method in &class_def.methods {
            let sig = class_method_sig(&class_def.name, method);
            by_name.entry(sig.sig.name.clone()).or_default().push(sig);
        }
        class_methods.insert(class_def.name.clone(), by_name);
    }

    // struct 方法签名（顶层函数形式: StructName.method）
    let struct_names: HashSet<String> = program.structs.iter().map(|s| s.name.clone()).collect();
    let mut struct_methods: HashMap<String, HashMap<String, Vec<MethodSig>>> = HashMap::new();
    for func in &program.functions {
        if let Some((owner, _)) = func.name.split_once('.') {
            if struct_names.contains(owner) {
                let sig = top_level_method_sig(owner, func);
                struct_methods
                    .entry(owner.to_string())
                    .or_default()
                    .entry(sig.name.clone())
                    .or_default()
                    .push(sig);
            }
        }
    }

    let mut iface_cache: HashMap<String, HashMap<String, InterfaceMethodReq>> = HashMap::new();

    // class 语义校验
    for class_def in &program.classes {
        if let Some(parent_name) = class_parent.get(&class_def.name).cloned().flatten() {
            if let Some(parent) = class_map.get(parent_name.as_str()) {
                if !parent.is_open && !parent.is_abstract && !parent.is_sealed {
                    return Err(format!("super class '{}' is not inheritable", parent_name));
                }
            }
        }

        // 收集父类方法（用于 override/实现签名校验）
        let mut parent_methods_by_name: HashMap<String, Vec<MethodSig>> = HashMap::new();
        let mut cur_parent = class_parent.get(&class_def.name).cloned().flatten();
        while let Some(parent_name) = cur_parent {
            if let Some(methods_by_name) = class_methods.get(&parent_name) {
                for (name, methods) in methods_by_name {
                    for method in methods {
                        parent_methods_by_name
                            .entry(name.clone())
                            .or_default()
                            .push(method.sig.clone());
                    }
                }
            }
            cur_parent = class_parent.get(&parent_name).cloned().flatten();
        }

        // 收集当前类及祖先类声明的 interfaces
        let mut inherited_ifaces = Vec::new();
        let mut seen_ifaces = HashSet::new();
        let mut cur = Some(class_def.name.clone());
        while let Some(class_name) = cur {
            if let Some(ifaces) = class_direct_ifaces.get(&class_name) {
                for iface in ifaces {
                    if seen_ifaces.insert(iface.clone()) {
                        inherited_ifaces.push(iface.clone());
                    }
                }
            }
            cur = class_parent.get(&class_name).cloned().flatten();
        }

        let mut iface_methods_by_name: HashMap<String, Vec<MethodSig>> = HashMap::new();
        let mut required_iface_methods: Vec<MethodSig> = Vec::new();
        for iface_name in inherited_ifaces {
            let mut visiting = HashSet::new();
            let methods = collect_interface_methods(
                &iface_name,
                &interface_map,
                &mut iface_cache,
                &mut visiting,
            )?;
            for req in methods.values() {
                iface_methods_by_name
                    .entry(req.sig.name.clone())
                    .or_default()
                    .push(req.sig.clone());
                if req.required {
                    required_iface_methods.push(req.sig.clone());
                }
            }
        }

        if let Some(current_methods) = class_methods.get(&class_def.name) {
            for methods in current_methods.values() {
                for method in methods {
                    if method.is_override && method.sig.is_static {
                        return Err(
                            "'static' and 'override' modifiers conflict on function declaration"
                                .to_string(),
                        );
                    }

                    let mut base_match: Option<MethodSig> = None;
                    if let Some(parent_candidates) = parent_methods_by_name.get(&method.sig.name) {
                        if let Some(found) = parent_candidates.iter().find(|base| {
                            same_name_params(base, &method.sig)
                                && base.is_static == method.sig.is_static
                        }) {
                            base_match = Some(found.clone());
                        }
                    }
                    if base_match.is_none() {
                        if let Some(iface_candidates) = iface_methods_by_name.get(&method.sig.name)
                        {
                            if let Some(found) = iface_candidates
                                .iter()
                                .find(|base| same_name_params(base, &method.sig))
                            {
                                base_match = Some(found.clone());
                            }
                        }
                    }

                    if method.is_override && base_match.is_none() {
                        return Err(format!(
                            "'override' function '{}' does not have an overridden function in its supertype",
                            method.sig.name
                        ));
                    }

                    if let Some(base) = base_match {
                        if !visibility_at_least(&method.sig.visibility, &base.visibility) {
                            return Err(
                                "a deriving member must be at least as visible as its base member"
                                    .to_string(),
                            );
                        }
                        if !constraints_not_tighter_than_parent(
                            &method.sig,
                            &base,
                            &class_parent,
                            &interface_map,
                        ) {
                            return Err(
                                "the constraint of type parameter is not looser than parent's constraint"
                                    .to_string(),
                            );
                        }
                        if method.sig.return_type != base.return_type {
                            if method.sig.name.starts_with("__get_")
                                || method.sig.name.starts_with("__set_")
                            {
                                return Err(
                                    "The type of the override/implement property must be the same"
                                        .to_string(),
                                );
                            }
                            return Err(format!(
                                "return type of '{}' is not identical or not a subtype of the overridden/redefined/implement function",
                                method.sig.name
                            ));
                        }
                    }
                }
            }
        }

        // 非 abstract class 必须实现接口中未提供默认实现的方法
        if !class_def.is_abstract {
            for required in &required_iface_methods {
                if !has_chain_method_with_same_name_params(
                    &class_def.name,
                    required,
                    &class_methods,
                    &class_parent,
                ) {
                    return Err(format!(
                        "implementation of function '{}' is needed in '{}'",
                        required.name, class_def.name
                    ));
                }
            }
        }
    }

    // struct 实现 interface 的完整性校验
    for struct_def in &program.structs {
        let mut implemented_ifaces = Vec::new();
        for constraint in &struct_def.constraints {
            if constraint.param == struct_def.name {
                implemented_ifaces.extend(constraint.bounds.iter().cloned());
            }
        }
        if implemented_ifaces.is_empty() {
            continue;
        }

        let mut seen = HashSet::new();
        for iface in &implemented_ifaces {
            if !interface_map.contains_key(iface.as_str()) {
                return Err(format!("undeclared type name '{}'", iface));
            }
            if !seen.insert(iface.clone()) {
                return Err(format!(
                    "class '{}' inherits or implements duplicate interface '{}'",
                    struct_def.name, iface
                ));
            }
        }

        let methods_by_name = struct_methods
            .get(&struct_def.name)
            .cloned()
            .unwrap_or_default();

        for iface_name in implemented_ifaces {
            let mut visiting = HashSet::new();
            let methods = collect_interface_methods(
                &iface_name,
                &interface_map,
                &mut iface_cache,
                &mut visiting,
            )?;
            for req in methods.values().filter(|m| m.required) {
                let implemented = methods_by_name
                    .get(&req.sig.name)
                    .map(|cands| cands.iter().any(|m| same_name_params(m, &req.sig)))
                    .unwrap_or(false);
                if !implemented {
                    return Err(format!(
                        "implementation of function '{}' is needed in '{}'",
                        req.sig.name, struct_def.name
                    ));
                }
            }
        }
    }

    Ok(())
}

/// 降低函数
pub fn lower_function(
    func: &Function,
    base_type_ctx: &TypeInferenceContext,
    func_indices: &HashMap<String, u32>,
    func_params: &HashMap<String, Vec<Param>>,
    struct_field_offsets: &HashMap<String, HashMap<String, u32>>,
    class_field_offsets: &HashMap<String, HashMap<String, u32>>,
    class_field_info: &HashMap<String, HashMap<String, (u32, Type)>>,
    class_extends: &HashMap<String, String>,
    func_return_types: &HashMap<String, Type>,
    enum_defs: &[crate::ast::EnumDef],
    current_class_name: Option<&str>,
    lambda_base: u32,
) -> Result<CHIRFunction, String> {
    // 为每个函数创建局部类型推断上下文（包含参数和局部变量）
    let mut type_ctx = base_type_ctx.clone();
    for param in &func.params {
        type_ctx.add_local_with_mutability(param.name.clone(), param.ty.clone(), param.is_inout);
    }
    // 类方法：将类字段作为局部变量注册到类型推断上下文
    // 使 infer_expr(Expr::Var("fieldName")) 能正确推断字段类型
    if let Some(class_name) = current_class_name {
        // 注册 this/self 的类型，使 this.field 能正确推断
        let class_ty = Type::Struct(class_name.to_string(), vec![]);
        if !type_ctx.locals.contains_key("this") {
            type_ctx.add_local("this".to_string(), class_ty.clone());
        }
        if !type_ctx.locals.contains_key("self") {
            type_ctx.add_local("self".to_string(), class_ty);
        }
        if let Some(fields) = class_field_info.get(class_name) {
            for (field_name, (_, field_ty)) in fields {
                if !type_ctx.locals.contains_key(field_name) {
                    type_ctx.add_local(field_name.clone(), field_ty.clone());
                }
            }
        }
    }
    // 预扫描函数体中的 let/var 声明类型
    type_ctx.collect_locals_from_function(func);

    let return_ty = func.return_type.clone().unwrap_or(crate::ast::Type::Unit);
    let return_wasm = match &return_ty {
        crate::ast::Type::Unit | crate::ast::Type::Nothing => None,
        t => Some(t.to_wasm()),
    };

    let mut ctx = LoweringContext::new(
        &type_ctx,
        func_indices,
        func_params,
        struct_field_offsets,
        class_field_offsets,
        class_field_info,
    );
    ctx.return_wasm_ty = return_wasm;
    ctx.class_extends = class_extends.clone();
    ctx.func_return_types = func_return_types.clone();
    ctx.enum_defs = enum_defs.to_vec();
    ctx.lambda_counter = lambda_base;

    // 处理参数（分配局部变量索引，同时记录类型供赋值时强制类型转换）
    let mut params = Vec::new();
    for param in &func.params {
        let wasm_ty = match &param.ty {
            crate::ast::Type::Unit | crate::ast::Type::Nothing => wasm_encoder::ValType::I32,
            t => t.to_wasm(),
        };
        // 使用 alloc_local_typed 记录参数 WASM 类型，
        // 使 Stmt::Assign 对参数赋值时能正确插入类型强制转换（如 TCO 生成的 param = tmp）
        let local_idx = ctx.alloc_local_typed(param.name.clone(), wasm_ty);
        ctx.local_ast_types
            .insert(param.name.clone(), param.ty.clone());
        params.push(CHIRParam {
            name: param.name.clone(),
            ty: param.ty.clone(),
            wasm_ty,
            local_idx,
        });
    }

    // 如果是类实例方法，设置隐式 this 字段访问上下文
    // 实例方法 params[0] 名为 "this"，且调用者提供了类名
    if let Some(class_name) = current_class_name {
        let is_init = func.name.starts_with("__") && func.name.ends_with("_init");
        if is_init {
            // init 函数：allocate this local after params（由 chir_codegen prologue 赋值）
            let this_idx = ctx.alloc_local_typed("this".to_string(), wasm_encoder::ValType::I32);
            ctx.current_class = Some((class_name.to_string(), this_idx));
        } else if let Some(this_param) = params.first() {
            if this_param.name == "this" {
                ctx.current_class = Some((class_name.to_string(), this_param.local_idx));
            }
        }
    }

    // 转换函数体
    let body = ctx.lower_stmts_to_block(&func.body)?;

    // 返回类型
    let return_ty = func.return_type.clone().unwrap_or(Type::Unit);
    let return_wasm_ty = match &return_ty {
        Type::Unit | Type::Nothing => wasm_encoder::ValType::I32, // 占位，Unit 函数无返回值
        t => t.to_wasm(),
    };

    Ok(CHIRFunction {
        name: func.name.clone(),
        params,
        return_ty,
        return_wasm_ty,
        locals: Vec::new(),
        body,
        local_wasm_types: ctx.local_wasm_tys.clone(),
    })
}

/// 降低程序
pub fn lower_program(program: &Program) -> Result<CHIRProgram, String> {
    validate_class_inheritance(program)?;
    validate_class_interface_semantics(program)?;

    // 构建类型推断上下文
    let type_ctx = TypeInferenceContext::from_program(program);

    // 构建函数索引表（偏移 4 跳过 WASI 导入：fd_write=0, proc_exit=1, clock_time_get=2, random_get=3）
    // 同名不同参数的函数（重载）使用 "name$arity" 修饰名，优先精确匹配
    let import_offset: u32 = 4;
    let mut func_indices = HashMap::new();
    let mut all_funcs: Vec<&Function> = program.functions.iter().collect();

    // 将类方法提取为顶级函数
    let class_methods_owned: Vec<Function> = program
        .classes
        .iter()
        .flat_map(|c| c.methods.iter().map(|m| m.func.clone()))
        .collect();
    all_funcs.extend(class_methods_owned.iter());

    // 为有 init 的类生成 __ClassName_init 函数
    let init_funcs_owned: Vec<Function> = program
        .classes
        .iter()
        .filter(|c| c.type_params.is_empty())
        .filter_map(|c| {
            c.init.as_ref().map(|init_def| Function {
                visibility: crate::ast::Visibility::Public,
                name: format!("__{}_init", c.name),
                type_params: vec![],
                constraints: vec![],
                params: init_def.params.clone(),
                return_type: Some(Type::Struct(c.name.clone(), vec![])),
                throws: None,
                body: init_def.body.clone(),
                extern_import: None,
            })
        })
        .collect();
    all_funcs.extend(init_funcs_owned.iter());

    // extend 方法提取为顶级函数（parser 已完成 TypeName.method 命名 + this 首参插入）
    let extend_methods_owned: Vec<Function> = program
        .extends
        .iter()
        .flat_map(|ext| ext.methods.iter().cloned())
        .collect();
    all_funcs.extend(extend_methods_owned.iter());

    // Lambda 预扫描：收集所有 Lambda 表达式并生成 __lambda_N 函数
    let mut lambda_counter = 0u32;
    let mut lambda_funcs: Vec<Function> = Vec::new();
    {
        let funcs_snapshot: Vec<&Function> = all_funcs.clone();
        for func in &funcs_snapshot {
            for stmt in &func.body {
                collect_lambdas_from_stmt(stmt, &mut lambda_counter, &mut lambda_funcs);
            }
        }
    }
    let lambda_funcs_owned = lambda_funcs;
    all_funcs.extend(lambda_funcs_owned.iter());

    for (i, func) in all_funcs.iter().enumerate() {
        let idx = import_offset + i as u32;
        // 修饰名（按参数数量）：精确匹配重载版本
        let mangled = format!("{}${}", func.name, func.params.len());
        func_indices.insert(mangled, idx);
        // 原名：仅当尚未注册时插入（保留首个定义的覆盖规则；重载场景应走修饰名路径）
        func_indices.entry(func.name.clone()).or_insert(idx);
    }

    // 注册运行时助手函数索引（与 CHIRCodeGen 中的 RT_NAMES 顺序一致）
    let rt_base = import_offset + all_funcs.len() as u32;
    let rt_names = [
        "__rt_println_i64",
        "__rt_print_i64",
        "__rt_println_str",
        "__rt_print_str",
        "__rt_println_bool",
        "__rt_print_bool",
        "__rt_println_empty",
        "__alloc",
        "sin",
        "cos",
        "tan",
        "exp",
        "log",
        "pow",
        "__i64_to_str",
        "__bool_to_str",
        "__str_to_i64",
        "__str_concat",
        "__f64_to_str",
        "now",
        "randomInt64",
        "randomFloat64",
        "__str_contains",
        "__str_starts_with",
        "__str_ends_with",
        "__str_trim",
        "__str_to_array",
        "__str_index_of",
        "__str_replace",
        // Collections (must match RT_NAMES in chir_codegen.rs)
        "__arraylist_new",
        "__arraylist_append",
        "__arraylist_get",
        "__arraylist_set",
        "__arraylist_remove",
        "__arraylist_size",
        "__hashmap_new",
        "__hashmap_put",
        "__hashmap_get",
        "__hashmap_contains",
        "__hashmap_remove",
        "__hashmap_size",
        "__hashset_new",
        "__hashset_add",
        "__hashset_contains",
        "__hashset_size",
        "__pow_i64",
        "__pow_f64",
    ];
    for (i, name) in rt_names.iter().enumerate() {
        func_indices.insert(name.to_string(), rt_base + i as u32);
    }

    // 构建结构体字段偏移表
    let mut struct_field_offsets = HashMap::new();
    for struct_def in &program.structs {
        let mut offsets = HashMap::new();
        let mut offset = 0u32;
        for field in &struct_def.fields {
            offsets.insert(field.name.clone(), offset);
            offset += field.ty.size() as u32;
        }
        struct_field_offsets.insert(struct_def.name.clone(), offsets);
    }
    // Range 虚拟结构体：[start:i64(8 bytes), end:i64(8 bytes)]
    {
        let mut range_offsets = HashMap::new();
        range_offsets.insert("start".to_string(), 0u32);
        range_offsets.insert("end".to_string(), 8u32);
        struct_field_offsets.insert("Range".to_string(), range_offsets);
    }

    // 构建类字段偏移表 + 完整字段信息（含类型）
    let mut class_field_offsets = HashMap::new();
    let mut class_field_info: HashMap<String, HashMap<String, (u32, Type)>> = HashMap::new();
    // struct 字段也加入 class_field_info，供 struct 方法中 this.field 访问
    for struct_def in &program.structs {
        let mut offsets = HashMap::new();
        let mut info = HashMap::new();
        let mut offset = 0u32;
        for field in &struct_def.fields {
            offsets.insert(field.name.clone(), offset);
            info.insert(field.name.clone(), (offset, field.ty.clone()));
            offset += field.ty.size() as u32;
        }
        class_field_offsets.insert(struct_def.name.clone(), offsets);
        class_field_info.insert(struct_def.name.clone(), info);
    }
    // 预计算 has_vtable：有继承关系的类需要 vtable
    let mut has_children: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cd in &program.classes {
        if let Some(ref parent) = cd.extends {
            has_children.insert(parent.clone());
        }
    }
    // 构建每个类的完整字段布局（父类字段在前，子类字段在后）
    // 先构建类定义映射
    let class_defs: HashMap<String, &crate::ast::ClassDef> = program
        .classes
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();
    // 递归计算类的字段布局
    fn build_class_fields(
        class_name: &str,
        class_defs: &HashMap<String, &crate::ast::ClassDef>,
        has_children: &std::collections::HashSet<String>,
        cache: &mut HashMap<String, (HashMap<String, u32>, HashMap<String, (u32, Type)>)>,
    ) {
        if cache.contains_key(class_name) {
            return;
        }
        let cd = match class_defs.get(class_name) {
            Some(cd) => cd,
            None => return,
        };
        let needs_vtable = cd.extends.is_some() || has_children.contains(class_name);
        let mut offsets = HashMap::new();
        let mut info = HashMap::new();
        let mut offset = if needs_vtable { 4u32 } else { 0u32 };
        // 先添加父类字段
        if let Some(ref parent) = cd.extends {
            build_class_fields(parent, class_defs, has_children, cache);
            if let Some((p_offsets, p_info)) = cache.get(parent) {
                for (name, &off) in p_offsets {
                    offsets.insert(name.clone(), off);
                }
                for (name, val) in p_info {
                    info.insert(name.clone(), val.clone());
                }
                offset = p_offsets.values().copied().max().unwrap_or(offset);
                if let Some(max_entry) = p_info.values().max_by_key(|(o, _)| *o) {
                    offset = max_entry.0 + max_entry.1.size();
                }
            }
        }
        // 再添加自己的字段
        for field in &cd.fields {
            if !offsets.contains_key(&field.name) {
                offsets.insert(field.name.clone(), offset);
                info.insert(field.name.clone(), (offset, field.ty.clone()));
                offset += field.ty.size() as u32;
            }
        }
        cache.insert(class_name.to_string(), (offsets, info));
    }
    let mut field_cache: HashMap<String, (HashMap<String, u32>, HashMap<String, (u32, Type)>)> =
        HashMap::new();
    for cd in &program.classes {
        build_class_fields(&cd.name, &class_defs, &has_children, &mut field_cache);
    }
    for (name, (offsets, info)) in field_cache {
        class_field_offsets.insert(name.clone(), offsets);
        class_field_info.insert(name, info);
    }

    // 构建"方法名 → 类名"映射，用于 lower_function 时传入类上下文
    let mut method_class_map: HashMap<String, String> = HashMap::new();
    for class_def in &program.classes {
        for method in &class_def.methods {
            method_class_map.insert(method.func.name.clone(), class_def.name.clone());
        }
        let init_name = format!("__{}_init", class_def.name);
        method_class_map.insert(init_name, class_def.name.clone());
    }
    // struct 方法（parser 已转为 "StructName.method" 顶级函数）也加入映射
    let struct_names: std::collections::HashSet<String> =
        program.structs.iter().map(|s| s.name.clone()).collect();
    for func in &all_funcs {
        if let Some(dot) = func.name.find('.') {
            let prefix = &func.name[..dot];
            if struct_names.contains(prefix) && !method_class_map.contains_key(&func.name) {
                method_class_map.insert(func.name.clone(), prefix.to_string());
            }
        }
    }

    // 注册内建 Option / Result 枚举（若用户未自定义）
    let has_user_option = program.enums.iter().any(|e| e.name == "Option");
    let has_user_result = program.enums.iter().any(|e| e.name == "Result");
    let mut all_enums = program.enums.clone();
    if !has_user_option {
        all_enums.push(crate::ast::EnumDef {
            visibility: crate::ast::Visibility::Public,
            name: "Option".to_string(),
            type_params: vec![],
            constraints: vec![],
            variants: vec![
                crate::ast::EnumVariant {
                    name: "None".to_string(),
                    payload: None,
                },
                crate::ast::EnumVariant {
                    name: "Some".to_string(),
                    payload: Some(Type::Int64),
                },
            ],
        });
    }
    if !has_user_result {
        all_enums.push(crate::ast::EnumDef {
            visibility: crate::ast::Visibility::Public,
            name: "Result".to_string(),
            type_params: vec![],
            constraints: vec![],
            variants: vec![
                crate::ast::EnumVariant {
                    name: "Ok".to_string(),
                    payload: Some(Type::Int64),
                },
                crate::ast::EnumVariant {
                    name: "Err".to_string(),
                    payload: Some(Type::String),
                },
            ],
        });
    }

    // 构建函数参数表（含修饰名和原名），用于命名参数默认值补全
    let mut func_params: HashMap<String, Vec<Param>> = HashMap::new();
    for func in &all_funcs {
        let params = func.params.clone();
        let mangled = format!("{}${}", func.name, func.params.len());
        func_params.insert(mangled, params.clone());
        func_params.entry(func.name.clone()).or_insert(params);
    }

    // 构建函数返回类型表
    let mut func_return_types: HashMap<String, crate::ast::Type> = HashMap::new();
    for func in &all_funcs {
        if let Some(ref ret_ty) = func.return_type {
            func_return_types.insert(func.name.clone(), ret_ty.clone());
        }
    }

    // 构建类继承关系图
    let class_extends_map: HashMap<String, String> = program
        .classes
        .iter()
        .filter_map(|c| c.extends.as_ref().map(|p| (c.name.clone(), p.clone())))
        .collect();

    // 预计算每个函数中的 lambda 数量，以便正确分配全局 lambda 索引
    let mut lambda_counts: Vec<u32> = Vec::new();
    for func in &all_funcs {
        let mut cnt = 0u32;
        let mut dummy = Vec::new();
        for stmt in &func.body {
            collect_lambdas_from_stmt(stmt, &mut cnt, &mut dummy);
        }
        lambda_counts.push(cnt);
    }

    // 转换所有函数（包含类方法）
    let mut chir_functions = Vec::new();
    let mut global_lambda_offset = 0u32;
    for (fi, func) in all_funcs.iter().enumerate() {
        let current_class_name = method_class_map.get(&func.name).map(|s| s.as_str());
        let lambda_base = global_lambda_offset;
        global_lambda_offset += lambda_counts[fi];
        match lower_function(
            func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_field_offsets,
            &class_field_offsets,
            &class_field_info,
            &class_extends_map,
            &func_return_types,
            &all_enums,
            current_class_name,
            lambda_base,
        ) {
            Ok(chir_func) => {
                chir_functions.push(chir_func);
            }
            Err(_e) => {
                if _e.starts_with("semantic error:") {
                    return Err(_e
                        .strip_prefix("semantic error:")
                        .map(|s| s.trim().to_string())
                        .unwrap_or(_e));
                }
                // 生成空函数占位，避免索引错位
                let empty_body = crate::chir::CHIRBlock {
                    stmts: vec![],
                    result: None,
                };
                let return_ty = func.return_type.clone().unwrap_or(Type::Unit);
                let return_wasm_ty = match &return_ty {
                    Type::Unit | Type::Nothing => wasm_encoder::ValType::I32,
                    t => t.to_wasm(),
                };
                let params: Vec<CHIRParam> = func
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let wt = match &p.ty {
                            Type::Unit | Type::Nothing => wasm_encoder::ValType::I32,
                            t => t.to_wasm(),
                        };
                        CHIRParam {
                            name: p.name.clone(),
                            ty: p.ty.clone(),
                            wasm_ty: wt,
                            local_idx: i as u32,
                        }
                    })
                    .collect();
                chir_functions.push(CHIRFunction {
                    name: func.name.clone(),
                    params,
                    return_ty,
                    return_wasm_ty,
                    locals: vec![],
                    body: empty_body,
                    local_wasm_types: std::collections::HashMap::new(),
                });
            }
        }
    }

    // 复制结构体、类、枚举定义
    let structs = program.structs.clone();
    let classes = program.classes.clone();
    let enums = program.enums.clone();

    // 全局变量（暂时为空）
    let globals = Vec::new();

    Ok(CHIRProgram {
        functions: chir_functions,
        structs,
        classes,
        enums,
        globals,
    })
}

fn collect_lambdas_from_stmt(stmt: &crate::ast::Stmt, counter: &mut u32, out: &mut Vec<Function>) {
    use crate::ast::{Expr, Stmt};
    match stmt {
        Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
            collect_lambdas_from_expr(value, counter, out);
        }
        Stmt::Assign { value, .. } => {
            collect_lambdas_from_expr(value, counter, out);
        }
        Stmt::Expr(e) => collect_lambdas_from_expr(e, counter, out),
        Stmt::Return(Some(e)) => collect_lambdas_from_expr(e, counter, out),
        Stmt::While { cond, body, .. } => {
            collect_lambdas_from_expr(cond, counter, out);
            for s in body {
                collect_lambdas_from_stmt(s, counter, out);
            }
        }
        Stmt::DoWhile { body, cond } => {
            for s in body {
                collect_lambdas_from_stmt(s, counter, out);
            }
            collect_lambdas_from_expr(cond, counter, out);
        }
        Stmt::For { iterable, body, .. } => {
            collect_lambdas_from_expr(iterable, counter, out);
            for s in body {
                collect_lambdas_from_stmt(s, counter, out);
            }
        }
        Stmt::Loop { body, .. } => {
            for s in body {
                collect_lambdas_from_stmt(s, counter, out);
            }
        }
        Stmt::UnsafeBlock { body } => {
            for s in body {
                collect_lambdas_from_stmt(s, counter, out);
            }
        }
        Stmt::Const { value, .. } => {
            collect_lambdas_from_expr(value, counter, out);
        }
        _ => {}
    }
}

fn collect_lambdas_from_expr(expr: &crate::ast::Expr, counter: &mut u32, out: &mut Vec<Function>) {
    use crate::ast::{Expr, Param, Visibility};
    match expr {
        Expr::Lambda {
            params,
            return_type,
            body,
        } => {
            let idx = *counter;
            *counter += 1;
            let lambda_name = format!("__lambda_{}", idx);
            let func_params: Vec<Param> = params
                .iter()
                .map(|(name, ty)| Param {
                    name: name.clone(),
                    ty: ty.clone(),
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                })
                .collect();
            // Infer return type: explicit > parameter type > default Int64
            let ret_type = return_type.clone().or_else(|| {
                if let Some((_, ty)) = params.first() {
                    Some(ty.clone())
                } else {
                    Some(Type::Int64)
                }
            });
            let body_stmt = vec![crate::ast::Stmt::Return(Some(*body.clone()))];
            out.push(Function {
                visibility: Visibility::Public,
                name: lambda_name,
                type_params: vec![],
                constraints: vec![],
                params: func_params,
                return_type: ret_type,
                throws: None,
                body: body_stmt,
                extern_import: None,
            });
            collect_lambdas_from_expr(body, counter, out);
        }
        Expr::Binary { left, right, .. } => {
            collect_lambdas_from_expr(left, counter, out);
            collect_lambdas_from_expr(right, counter, out);
        }
        Expr::Unary { expr, .. } => collect_lambdas_from_expr(expr, counter, out),
        Expr::Call { args, .. } => {
            for a in args {
                collect_lambdas_from_expr(a, counter, out);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            collect_lambdas_from_expr(object, counter, out);
            for a in args {
                collect_lambdas_from_expr(a, counter, out);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_lambdas_from_expr(cond, counter, out);
            collect_lambdas_from_expr(then_branch, counter, out);
            if let Some(e) = else_branch {
                collect_lambdas_from_expr(e, counter, out);
            }
        }
        Expr::Block(stmts, expr) => {
            for s in stmts {
                collect_lambdas_from_stmt(s, counter, out);
            }
            if let Some(e) = expr {
                collect_lambdas_from_expr(e, counter, out);
            }
        }
        Expr::Array(elems) | Expr::Tuple(elems) => {
            for e in elems {
                collect_lambdas_from_expr(e, counter, out);
            }
        }
        Expr::ConstructorCall { args, .. } => {
            for a in args {
                collect_lambdas_from_expr(a, counter, out);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields {
                collect_lambdas_from_expr(e, counter, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        ClassDef, ClassMethod, Expr, FieldDef, InterfaceDef, InterfaceMethod, Param, Pattern, Stmt,
        StructDef, TypeConstraint, Visibility,
    };

    fn make_func(name: &str, params: Vec<Param>, body: Vec<Stmt>) -> Function {
        Function {
            name: name.to_string(),
            type_params: vec![],
            params,
            return_type: Some(Type::Int64),
            body,
            constraints: vec![],
            visibility: crate::ast::Visibility::Public,
            throws: None,
            extern_import: None,
        }
    }

    fn make_param(name: &str) -> Param {
        Param {
            name: name.to_string(),
            ty: Type::Int64,
            default: None,
            variadic: false,
            is_named: false,
            is_inout: false,
        }
    }

    fn make_program(functions: Vec<Function>) -> crate::ast::Program {
        crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions,
            structs: vec![],
            classes: vec![],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        }
    }

    #[test]
    fn test_lower_simple_function() {
        let func = make_func("test", vec![], vec![Stmt::Return(Some(Expr::Integer(42)))]);

        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();
        let class_extends_map = HashMap::new();

        let func_return_types = HashMap::new();
        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
            &class_extends_map,
            &func_return_types,
            &[],
            None,
            0,
        )
        .unwrap();

        assert_eq!(chir_func.name, "test");
        assert_eq!(chir_func.return_wasm_ty, wasm_encoder::ValType::I64);
    }

    #[test]
    fn test_lower_function_with_params() {
        let func = make_func(
            "add",
            vec![make_param("a"), make_param("b")],
            vec![Stmt::Return(Some(Expr::Binary {
                op: crate::ast::BinOp::Add,
                left: Box::new(Expr::Var("a".to_string())),
                right: Box::new(Expr::Var("b".to_string())),
            }))],
        );

        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();
        let class_field_info = HashMap::new();
        let class_extends_map = HashMap::new();

        let func_return_types = HashMap::new();
        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
            &class_extends_map,
            &func_return_types,
            &[],
            None,
            0,
        )
        .unwrap();

        assert_eq!(chir_func.params.len(), 2);
        assert_eq!(chir_func.params[0].name, "a");
        assert_eq!(chir_func.params[1].name, "b");
    }

    #[test]
    fn test_lower_program() {
        let program = make_program(vec![make_func(
            "main",
            vec![],
            vec![Stmt::Return(Some(Expr::Integer(0)))],
        )]);

        let chir_program = lower_program(&program).unwrap();

        assert_eq!(chir_program.functions.len(), 1);
        assert_eq!(chir_program.functions[0].name, "main");
    }

    #[test]
    fn test_lower_function_class_method() {
        // Class method with this param and class fields in context
        let this_param = Param {
            name: "this".to_string(),
            ty: Type::Struct("Counter".to_string(), vec![]),
            default: None,
            variadic: false,
            is_named: false,
            is_inout: false,
        };
        let func = make_func(
            "Counter.getN",
            vec![this_param],
            vec![Stmt::Return(Some(Expr::Field {
                object: Box::new(Expr::Var("this".to_string())),
                field: "n".to_string(),
            }))],
        );

        let type_ctx = TypeInferenceContext::new();
        let func_indices = HashMap::new();
        let func_params = HashMap::new();
        let struct_offsets = HashMap::new();
        let mut class_offsets = HashMap::new();
        class_offsets.insert(
            "Counter".to_string(),
            [("n".to_string(), 8u32)].into_iter().collect(),
        );
        let mut class_field_info: HashMap<String, HashMap<String, (u32, Type)>> = HashMap::new();
        let mut info = HashMap::new();
        info.insert("n".to_string(), (8, Type::Int64));
        class_field_info.insert("Counter".to_string(), info);

        let class_extends_map = HashMap::new();
        let func_return_types = HashMap::new();
        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
            &class_extends_map,
            &func_return_types,
            &[],
            Some("Counter"),
            0,
        )
        .unwrap();

        assert_eq!(chir_func.name, "Counter.getN");
        assert_eq!(chir_func.params[0].name, "this");
    }

    #[test]
    fn test_lower_program_with_class() {
        let class_method_func = make_func(
            "Counter.inc",
            vec![Param {
                name: "this".to_string(),
                ty: Type::Struct("Counter".to_string(), vec![]),
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            vec![Stmt::Return(Some(Expr::Integer(1)))],
        );
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "Counter".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: None,
            implements: vec![],
            fields: vec![FieldDef {
                name: "n".to_string(),
                ty: Type::Int64,
                default: None,
            }],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![ClassMethod {
                override_: false,
                func: class_method_func,
            }],
            primary_ctor_params: vec![],
        };

        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func(
                "main",
                vec![],
                vec![Stmt::Return(Some(Expr::Integer(0)))],
            )],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let chir_program = lower_program(&program).unwrap();
        assert_eq!(chir_program.functions.len(), 2); // main + Counter.inc
        assert!(chir_program.classes.len() == 1);
        assert_eq!(chir_program.classes[0].name, "Counter");
    }

    #[test]
    fn test_lower_program_function_fails_placeholder() {
        // Function that triggers lower error (assign to undefined var) -> Err path
        // lower_program pushes empty placeholder on Err
        use crate::ast::AssignTarget;
        let bad_func = make_func(
            "bad",
            vec![],
            vec![Stmt::Assign {
                target: AssignTarget::Var("__nonexistent_var__".to_string()),
                value: Expr::Integer(0),
            }],
        );
        let program = make_program(vec![bad_func]);
        let chir_program = lower_program(&program).unwrap();
        // Should still succeed (placeholder), function count 1
        assert_eq!(chir_program.functions.len(), 1);
        assert_eq!(chir_program.functions[0].name, "bad");
        assert!(chir_program.functions[0].body.stmts.is_empty());
    }

    #[test]
    fn test_lower_program_rejects_self_inheritance() {
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "A".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: true,
            extends: Some("A".to_string()),
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("cannot inherit itself"));
    }

    #[test]
    fn test_lower_program_rejects_non_abstract_sealed_class() {
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "A".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: true,
            is_open: false,
            extends: None,
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("non-abstract class cannot be modified by 'sealed'"));
    }

    #[test]
    fn test_lower_program_rejects_override_without_base() {
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "A".to_string(),
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
            methods: vec![ClassMethod {
                override_: true,
                func: make_func(
                    "A.f",
                    vec![Param {
                        name: "this".to_string(),
                        ty: Type::Struct("A".to_string(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    vec![Stmt::Return(Some(Expr::Integer(1)))],
                ),
            }],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("does not have an overridden function in its supertype"));
    }

    #[test]
    fn test_lower_program_rejects_interface_visibility_reduction() {
        let iface = InterfaceDef {
            visibility: Visibility::Public,
            name: "I".to_string(),
            parents: vec![],
            methods: vec![InterfaceMethod {
                name: "f".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Unit),
                default_body: None,
            }],
            assoc_types: vec![],
        };
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "A".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            // parser 对 `class A <: I` 会把 I 放到 extends
            extends: Some("I".to_string()),
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Protected,
                    name: "A.f".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![Param {
                        name: "this".to_string(),
                        ty: Type::Struct("A".to_string(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    return_type: Some(Type::Unit),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            }],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![iface],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("at least as visible as its base member"));
    }

    #[test]
    fn test_lower_program_rejects_struct_missing_interface_method() {
        let iface = InterfaceDef {
            visibility: Visibility::Public,
            name: "I".to_string(),
            parents: vec![],
            methods: vec![InterfaceMethod {
                name: "f".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                default_body: None,
            }],
            assoc_types: vec![],
        };
        let st = StructDef {
            visibility: Visibility::Public,
            name: "S".to_string(),
            type_params: vec![],
            constraints: vec![TypeConstraint {
                param: "S".to_string(),
                bounds: vec!["I".to_string()],
            }],
            fields: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![st],
            classes: vec![],
            enums: vec![],
            interfaces: vec![iface],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("implementation of function 'f' is needed in 'S'"));
    }

    #[test]
    fn test_lower_program_rejects_invalid_generic_upper_bound() {
        let class_def = ClassDef {
            visibility: Visibility::default(),
            name: "A".to_string(),
            type_params: vec!["T".to_string()],
            constraints: vec![TypeConstraint {
                param: "T".to_string(),
                bounds: vec!["Int64".to_string()],
            }],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: None,
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![class_def],
            enums: vec![],
            interfaces: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("must be class or interface"));
    }

    #[test]
    fn test_lower_program_rejects_tighter_generic_constraint_than_interface() {
        let iface = InterfaceDef {
            visibility: Visibility::Public,
            name: "I".to_string(),
            parents: vec![],
            methods: vec![InterfaceMethod {
                name: "f".to_string(),
                type_params: vec!["T".to_string()],
                constraints: vec![TypeConstraint {
                    param: "T".to_string(),
                    bounds: vec!["BoundBase".to_string()],
                }],
                params: vec![],
                return_type: Some(Type::Unit),
                default_body: None,
            }],
            assoc_types: vec![],
        };
        let bound_base = ClassDef {
            visibility: Visibility::default(),
            name: "BoundBase".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: true,
            extends: None,
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let bound_child = ClassDef {
            visibility: Visibility::default(),
            name: "BoundChild".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: Some("BoundBase".to_string()),
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let impl_class = ClassDef {
            visibility: Visibility::default(),
            name: "Impl".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: Some("I".to_string()),
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "Impl.f".to_string(),
                    type_params: vec!["T".to_string()],
                    constraints: vec![TypeConstraint {
                        param: "T".to_string(),
                        bounds: vec!["BoundChild".to_string()],
                    }],
                    params: vec![Param {
                        name: "this".to_string(),
                        ty: Type::Struct("Impl".to_string(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    return_type: Some(Type::Unit),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            }],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![bound_base, bound_child, impl_class],
            enums: vec![],
            interfaces: vec![iface],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let err = lower_program(&program).unwrap_err();
        assert!(
            err.contains("the constraint of type parameter is not looser than parent's constraint")
        );
    }

    #[test]
    fn test_lower_program_accepts_looser_generic_constraint_than_interface() {
        let iface = InterfaceDef {
            visibility: Visibility::Public,
            name: "I".to_string(),
            parents: vec![],
            methods: vec![InterfaceMethod {
                name: "f".to_string(),
                type_params: vec!["T".to_string()],
                constraints: vec![TypeConstraint {
                    param: "T".to_string(),
                    bounds: vec!["BoundBase".to_string()],
                }],
                params: vec![],
                return_type: Some(Type::Unit),
                default_body: None,
            }],
            assoc_types: vec![],
        };
        let bound_base = ClassDef {
            visibility: Visibility::default(),
            name: "BoundBase".to_string(),
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
            methods: vec![],
            primary_ctor_params: vec![],
        };
        let impl_class = ClassDef {
            visibility: Visibility::default(),
            name: "Impl".to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            extends: Some("I".to_string()),
            implements: vec![],
            fields: vec![],
            init: None,
            deinit: None,
            static_init: None,
            methods: vec![ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "Impl.f".to_string(),
                    type_params: vec!["T".to_string()],
                    constraints: vec![TypeConstraint {
                        param: "T".to_string(),
                        bounds: vec!["Object".to_string()],
                    }],
                    params: vec![Param {
                        name: "this".to_string(),
                        ty: Type::Struct("Impl".to_string(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    return_type: Some(Type::Unit),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            }],
            primary_ctor_params: vec![],
        };
        let program = crate::ast::Program {
            package_name: None,
            imports: vec![],
            functions: vec![make_func("main", vec![], vec![])],
            structs: vec![],
            classes: vec![bound_base, impl_class],
            enums: vec![],
            interfaces: vec![iface],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
        };

        let chir_program = lower_program(&program).unwrap();
        assert!(!chir_program.functions.is_empty());
    }

    #[test]
    fn test_lower_program_rejects_assign_to_immutable() {
        let program = make_program(vec![make_func(
            "main",
            vec![],
            vec![
                Stmt::Let {
                    pattern: Pattern::Binding("x".to_string()),
                    ty: Some(Type::Int64),
                    value: Expr::Integer(1),
                },
                Stmt::Assign {
                    target: crate::ast::AssignTarget::Var("x".to_string()),
                    value: Expr::Integer(2),
                },
            ],
        )]);

        let err = lower_program(&program).unwrap_err();
        assert!(err.contains("cannot assign to immutable value"));
    }
}
