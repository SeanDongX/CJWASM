//! AST → CHIR 完整降低（Lowering）

use crate::ast::{Function, Param, Program, Type};
use crate::chir::{CHIRFunction, CHIRProgram, CHIRParam};
use crate::chir::type_inference::TypeInferenceContext;
use crate::chir::lower_expr::LoweringContext;
use std::collections::HashMap;

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
        type_ctx.add_local(param.name.clone(), param.ty.clone());
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
        ctx.local_ast_types.insert(param.name.clone(), param.ty.clone());
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
    // 构建类型推断上下文
    let type_ctx = TypeInferenceContext::from_program(program);

    // 构建函数索引表（偏移 4 跳过 WASI 导入：fd_write=0, proc_exit=1, clock_time_get=2, random_get=3）
    // 同名不同参数的函数（重载）使用 "name$arity" 修饰名，优先精确匹配
    let import_offset: u32 = 4;
    let mut func_indices = HashMap::new();
    let mut all_funcs: Vec<&Function> = program.functions.iter().collect();

    // 将类方法提取为顶级函数
    let class_methods_owned: Vec<Function> = program.classes.iter()
        .flat_map(|c| c.methods.iter().map(|m| m.func.clone()))
        .collect();
    all_funcs.extend(class_methods_owned.iter());

    // 为有 init 的类生成 __ClassName_init 函数
    let init_funcs_owned: Vec<Function> = program.classes.iter()
        .filter(|c| c.type_params.is_empty())
        .filter_map(|c| {
            c.init.as_ref().map(|init_def| {
                Function {
                    visibility: crate::ast::Visibility::Public,
                    name: format!("__{}_init", c.name),
                    type_params: vec![],
                    constraints: vec![],
                    params: init_def.params.clone(),
                    return_type: Some(Type::Struct(c.name.clone(), vec![])),
                    throws: None,
                    body: init_def.body.clone(),
                    extern_import: None,
                }
            })
        })
        .collect();
    all_funcs.extend(init_funcs_owned.iter());

    // extend 方法提取为顶级函数（parser 已完成 TypeName.method 命名 + this 首参插入）
    let extend_methods_owned: Vec<Function> = program.extends.iter()
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
        "__rt_println_i64", "__rt_print_i64",
        "__rt_println_str", "__rt_print_str",
        "__rt_println_bool", "__rt_print_bool",
        "__rt_println_empty",
        "__alloc",
        "sin", "cos", "tan", "exp", "log", "pow",
        "__i64_to_str", "__bool_to_str", "__str_to_i64",
        "__str_concat", "__f64_to_str",
        "now", "randomInt64", "randomFloat64",
        "__str_contains", "__str_starts_with", "__str_ends_with", "__str_trim",
        "__str_to_array", "__str_index_of", "__str_replace",
        // Collections (must match RT_NAMES in chir_codegen.rs)
        "__arraylist_new", "__arraylist_append", "__arraylist_get", "__arraylist_set", "__arraylist_remove", "__arraylist_size",
        "__hashmap_new", "__hashmap_put", "__hashmap_get", "__hashmap_contains", "__hashmap_remove", "__hashmap_size",
        "__hashset_new", "__hashset_add", "__hashset_contains", "__hashset_size",
        "__pow_i64", "__pow_f64",
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
    let class_defs: HashMap<String, &crate::ast::ClassDef> = program.classes.iter()
        .map(|c| (c.name.clone(), c))
        .collect();
    // 递归计算类的字段布局
    fn build_class_fields(
        class_name: &str,
        class_defs: &HashMap<String, &crate::ast::ClassDef>,
        has_children: &std::collections::HashSet<String>,
        cache: &mut HashMap<String, (HashMap<String, u32>, HashMap<String, (u32, Type)>)>,
    ) {
        if cache.contains_key(class_name) { return; }
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
    let mut field_cache: HashMap<String, (HashMap<String, u32>, HashMap<String, (u32, Type)>)> = HashMap::new();
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
    let struct_names: std::collections::HashSet<String> = program.structs.iter().map(|s| s.name.clone()).collect();
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
                crate::ast::EnumVariant { name: "None".to_string(), payload: None },
                crate::ast::EnumVariant { name: "Some".to_string(), payload: Some(Type::Int64) },
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
                crate::ast::EnumVariant { name: "Ok".to_string(), payload: Some(Type::Int64) },
                crate::ast::EnumVariant { name: "Err".to_string(), payload: Some(Type::String) },
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
    let class_extends_map: HashMap<String, String> = program.classes.iter()
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
                // 生成空函数占位，避免索引错位
                let empty_body = crate::chir::CHIRBlock { stmts: vec![], result: None };
                let return_ty = func.return_type.clone().unwrap_or(Type::Unit);
                let return_wasm_ty = match &return_ty {
                    Type::Unit | Type::Nothing => wasm_encoder::ValType::I32,
                    t => t.to_wasm(),
                };
                let params: Vec<CHIRParam> = func.params.iter().enumerate().map(|(i, p)| {
                    let wt = match &p.ty { Type::Unit | Type::Nothing => wasm_encoder::ValType::I32, t => t.to_wasm() };
                    CHIRParam { name: p.name.clone(), ty: p.ty.clone(), wasm_ty: wt, local_idx: i as u32 }
                }).collect();
                chir_functions.push(CHIRFunction {
                    name: func.name.clone(), params, return_ty, return_wasm_ty,
                    locals: vec![], body: empty_body,
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
    use crate::ast::{Stmt, Expr};
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
            for s in body { collect_lambdas_from_stmt(s, counter, out); }
        }
        Stmt::DoWhile { body, cond } => {
            for s in body { collect_lambdas_from_stmt(s, counter, out); }
            collect_lambdas_from_expr(cond, counter, out);
        }
        Stmt::For { iterable, body, .. } => {
            collect_lambdas_from_expr(iterable, counter, out);
            for s in body { collect_lambdas_from_stmt(s, counter, out); }
        }
        Stmt::Loop { body, .. } => {
            for s in body { collect_lambdas_from_stmt(s, counter, out); }
        }
        Stmt::UnsafeBlock { body } => {
            for s in body { collect_lambdas_from_stmt(s, counter, out); }
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
        Expr::Lambda { params, return_type, body } => {
            let idx = *counter;
            *counter += 1;
            let lambda_name = format!("__lambda_{}", idx);
            let func_params: Vec<Param> = params.iter().map(|(name, ty)| Param {
                name: name.clone(),
                ty: ty.clone(),
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }).collect();
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
            for a in args { collect_lambdas_from_expr(a, counter, out); }
        }
        Expr::MethodCall { object, args, .. } => {
            collect_lambdas_from_expr(object, counter, out);
            for a in args { collect_lambdas_from_expr(a, counter, out); }
        }
        Expr::If { cond, then_branch, else_branch, .. } => {
            collect_lambdas_from_expr(cond, counter, out);
            collect_lambdas_from_expr(then_branch, counter, out);
            if let Some(e) = else_branch { collect_lambdas_from_expr(e, counter, out); }
        }
        Expr::Block(stmts, expr) => {
            for s in stmts { collect_lambdas_from_stmt(s, counter, out); }
            if let Some(e) = expr { collect_lambdas_from_expr(e, counter, out); }
        }
        Expr::Array(elems) | Expr::Tuple(elems) => {
            for e in elems { collect_lambdas_from_expr(e, counter, out); }
        }
        Expr::ConstructorCall { args, .. } => {
            for a in args { collect_lambdas_from_expr(a, counter, out); }
        }
        Expr::StructInit { fields, .. } => {
            for (_, e) in fields { collect_lambdas_from_expr(e, counter, out); }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ClassDef, ClassMethod, Expr, FieldDef, Param, Stmt, Visibility};

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
        ).unwrap();

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
        ).unwrap();

        assert_eq!(chir_func.params.len(), 2);
        assert_eq!(chir_func.params[0].name, "a");
        assert_eq!(chir_func.params[1].name, "b");
    }

    #[test]
    fn test_lower_program() {
        let program = make_program(vec![
            make_func("main", vec![], vec![Stmt::Return(Some(Expr::Integer(0)))])
        ]);

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
        class_offsets.insert("Counter".to_string(), [("n".to_string(), 8u32)].into_iter().collect());
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
        ).unwrap();

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
            functions: vec![make_func("main", vec![], vec![Stmt::Return(Some(Expr::Integer(0)))])],
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
}
