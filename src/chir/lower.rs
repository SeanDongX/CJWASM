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
    // 当函数是类实例方法时，传入类名；否则传 None
    current_class_name: Option<&str>,
) -> Result<CHIRFunction, String> {
    // 为每个函数创建局部类型推断上下文（包含参数和局部变量）
    let mut type_ctx = base_type_ctx.clone();
    for param in &func.params {
        type_ctx.add_local(param.name.clone(), param.ty.clone());
    }
    // 类方法：将类字段作为局部变量注册到类型推断上下文
    // 使 infer_expr(Expr::Var("fieldName")) 能正确推断字段类型
    if let Some(class_name) = current_class_name {
        if let Some(fields) = class_field_info.get(class_name) {
            for (field_name, (_, field_ty)) in fields {
                // 不覆盖已有同名参数（如参数名与字段名冲突时以参数为准）
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
        if let Some(this_param) = params.first() {
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

    // 构建函数索引表（偏移 2 跳过 WASI 导入：fd_write=0, proc_exit=1）
    // 同名不同参数的函数（重载）使用 "name$arity" 修饰名，优先精确匹配
    let import_offset: u32 = 2;
    let mut func_indices = HashMap::new();
    let mut all_funcs: Vec<&Function> = program.functions.iter().collect();

    // 将类方法（含 struct/class 方法）提取为顶级函数，和 program.functions 一起注册
    // 方法名格式已是 "ClassName.methodName"，params[0] 为 "this"（非 static）
    let class_methods_owned: Vec<Function> = program.classes.iter()
        .flat_map(|c| c.methods.iter().map(|m| m.func.clone()))
        .collect();
    all_funcs.extend(class_methods_owned.iter());

    for (i, func) in all_funcs.iter().enumerate() {
        let idx = import_offset + i as u32;
        // 修饰名（按参数数量）：精确匹配重载版本
        let mangled = format!("{}${}", func.name, func.params.len());
        func_indices.insert(mangled, idx);
        // 原名：仅当尚未注册时插入（保留首个定义的覆盖规则；重载场景应走修饰名路径）
        func_indices.entry(func.name.clone()).or_insert(idx);
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

    // 构建类字段偏移表 + 完整字段信息（含类型）
    let mut class_field_offsets = HashMap::new();
    let mut class_field_info: HashMap<String, HashMap<String, (u32, Type)>> = HashMap::new();
    // 先记录每个类的直接字段
    for class_def in &program.classes {
        let mut offsets = HashMap::new();
        let mut info = HashMap::new();
        let mut offset = 8u32; // vtable 指针占 8 字节
        for field in &class_def.fields {
            offsets.insert(field.name.clone(), offset);
            info.insert(field.name.clone(), (offset, field.ty.clone()));
            offset += field.ty.size() as u32;
        }
        class_field_offsets.insert(class_def.name.clone(), offsets);
        class_field_info.insert(class_def.name.clone(), info);
    }
    // 继承：把父类字段合并到子类（不覆盖已有同名字段）
    // 多轮迭代处理多层继承
    let class_extends: HashMap<String, Option<String>> = program.classes.iter()
        .map(|c| (c.name.clone(), c.extends.clone()))
        .collect();
    for _ in 0..10 {
        let mut changed = false;
        for class_def in &program.classes {
            let mut parent = class_def.extends.clone();
            while let Some(ref parent_name) = parent {
                // 先 clone 父类数据，然后合并到子类
                let parent_info_snapshot = class_field_info.get(parent_name).cloned();
                let parent_offsets_snapshot = class_field_offsets.get(parent_name).cloned();
                if let Some(parent_info) = parent_info_snapshot {
                    let child_info = class_field_info.get_mut(&class_def.name).unwrap();
                    for (name, val) in parent_info {
                        if !child_info.contains_key(&name) {
                            child_info.insert(name, val);
                            changed = true;
                        }
                    }
                }
                if let Some(parent_offsets) = parent_offsets_snapshot {
                    let child_offsets = class_field_offsets.get_mut(&class_def.name).unwrap();
                    for (name, val) in parent_offsets {
                        child_offsets.entry(name).or_insert(val);
                    }
                }
                parent = class_extends.get(parent_name).and_then(|p| p.clone());
            }
        }
        if !changed { break; }
    }

    // 构建"方法名 → 类名"映射，用于 lower_function 时传入类上下文
    let mut method_class_map: HashMap<String, String> = HashMap::new();
    for class_def in &program.classes {
        for method in &class_def.methods {
            method_class_map.insert(method.func.name.clone(), class_def.name.clone());
        }
    }

    // 构建函数参数表（含修饰名和原名），用于命名参数默认值补全
    let mut func_params: HashMap<String, Vec<Param>> = HashMap::new();
    for func in &all_funcs {
        let params = func.params.clone();
        let mangled = format!("{}${}", func.name, func.params.len());
        func_params.insert(mangled, params.clone());
        func_params.entry(func.name.clone()).or_insert(params);
    }

    // 转换所有函数（包含类方法）
    let mut chir_functions = Vec::new();
    for func in &all_funcs {
        // 判断是否为类实例方法（params[0].name == "this"）
        let current_class_name = method_class_map.get(&func.name).map(|s| s.as_str());
        match lower_function(
            func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_field_offsets,
            &class_field_offsets,
            &class_field_info,
            current_class_name,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Stmt, Param};

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

        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
            None,
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

        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &func_params,
            &struct_offsets,
            &class_offsets,
            &class_field_info,
            None,
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
}
