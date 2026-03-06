//! AST → CHIR 完整降低（Lowering）

use crate::ast::{Function, Program, Type};
use crate::chir::{CHIRFunction, CHIRProgram, CHIRParam};
use crate::chir::type_inference::TypeInferenceContext;
use crate::chir::lower_expr::LoweringContext;
use std::collections::HashMap;

/// 降低函数
pub fn lower_function(
    func: &Function,
    base_type_ctx: &TypeInferenceContext,
    func_indices: &HashMap<String, u32>,
    struct_field_offsets: &HashMap<String, HashMap<String, u32>>,
    class_field_offsets: &HashMap<String, HashMap<String, u32>>,
) -> Result<CHIRFunction, String> {
    // 为每个函数创建局部类型推断上下文（包含参数和局部变量）
    let mut type_ctx = base_type_ctx.clone();
    for param in &func.params {
        type_ctx.add_local(param.name.clone(), param.ty.clone());
    }
    // 预扫描函数体中的 let/var 声明类型
    type_ctx.collect_locals_from_function(func);

    let mut ctx = LoweringContext::new(
        &type_ctx,
        func_indices,
        struct_field_offsets,
        class_field_offsets,
    );

    // 处理参数（分配局部变量索引）
    let mut params = Vec::new();
    for param in &func.params {
        let local_idx = ctx.alloc_local(param.name.clone());
        params.push(CHIRParam {
            name: param.name.clone(),
            ty: param.ty.clone(),
            wasm_ty: param.ty.to_wasm(),
            local_idx,
        });
    }

    // 转换函数体
    let body = ctx.lower_stmts_to_block(&func.body)?;

    // 返回类型
    let return_ty = func.return_type.clone().unwrap_or(Type::Unit);
    let return_wasm_ty = return_ty.to_wasm();

    Ok(CHIRFunction {
        name: func.name.clone(),
        params,
        return_ty,
        return_wasm_ty,
        locals: Vec::new(), // 局部变量在 lower 过程中已分配
        body,
    })
}

/// 降低程序
pub fn lower_program(program: &Program) -> Result<CHIRProgram, String> {
    // 构建类型推断上下文
    let type_ctx = TypeInferenceContext::from_program(program);

    // 构建函数索引表（偏移 2 跳过 WASI 导入：fd_write=0, proc_exit=1）
    let import_offset: u32 = 2;
    let mut func_indices = HashMap::new();
    for (i, func) in program.functions.iter().enumerate() {
        func_indices.insert(func.name.clone(), import_offset + i as u32);
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

    // 构建类字段偏移表
    let mut class_field_offsets = HashMap::new();
    for class_def in &program.classes {
        let mut offsets = HashMap::new();
        let mut offset = 8u32; // vtable 指针占 8 字节
        for field in &class_def.fields {
            offsets.insert(field.name.clone(), offset);
            offset += field.ty.size() as u32;
        }
        class_field_offsets.insert(class_def.name.clone(), offsets);
    }

    // 转换所有函数
    let mut chir_functions = Vec::new();
    for func in &program.functions {
        let chir_func = lower_function(
            func,
            &type_ctx,
            &func_indices,
            &struct_field_offsets,
            &class_field_offsets,
        )?;
        chir_functions.push(chir_func);
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
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
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
        let struct_offsets = HashMap::new();
        let class_offsets = HashMap::new();

        let chir_func = lower_function(
            &func,
            &type_ctx,
            &func_indices,
            &struct_offsets,
            &class_offsets,
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
