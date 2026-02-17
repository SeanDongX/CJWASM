//! WASM 宏执行运行时：使用 wasmtime 在编译期执行宏。
//!
//! 此模块仅在 `macro-system` feature 启用时编译。
//! 流程：
//! 1. 将 MacroDef 编译为独立 WASM 模块
//! 2. 通过 wasmtime 实例化模块
//! 3. 序列化宏参数为 JSON → 写入 WASM 线性内存
//! 4. 调用宏导出函数
//! 5. 从 WASM 线性内存读取结果 JSON → 反序列化为 AST

use crate::ast::{Expr, MacroDef, Stmt};
use super::MacroError;
use wasmtime::*;

/// 将 MacroDef 编译为独立 WASM 模块字节码
///
/// 宏函数被编译为一个简单的 WASM 模块，导出函数签名为：
/// `macro_<name>(json_ptr: i32, json_len: i32) -> i64`
/// 返回值高 32 位为结果指针，低 32 位为结果长度
pub fn compile_macro_to_wasm(macro_def: &MacroDef) -> Result<Vec<u8>, MacroError> {
    // 将宏体中的 quote 表达式序列化为 JSON 字符串常量
    // 宏本质上是 "接收 JSON 参数 → 返回 JSON AST" 的函数
    let quote_body = extract_quote_from_macro(macro_def)?;
    let json_template = serde_json::to_string(&quote_body)
        .map_err(|e| MacroError::SerdeError(e.to_string()))?;

    // 生成一个最小 WASM 模块：
    // - 导入 memory
    // - 将 JSON 模板写入 data section
    // - 导出函数返回 (data_ptr, data_len)
    let mut module = wasm_encoder::Module::new();

    // Type section: func type () -> i64
    let mut types = wasm_encoder::TypeSection::new();
    types.ty().function(vec![], vec![wasm_encoder::ValType::I64]);
    module.section(&types);

    // Function section
    let mut functions = wasm_encoder::FunctionSection::new();
    functions.function(0); // type index 0
    module.section(&functions);

    // Memory section
    let mut memories = wasm_encoder::MemorySection::new();
    memories.memory(wasm_encoder::MemoryType {
        minimum: 1,
        maximum: Some(16),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);

    // Export section
    let mut exports = wasm_encoder::ExportSection::new();
    exports.export(
        &format!("macro_{}", macro_def.name),
        wasm_encoder::ExportKind::Func,
        0,
    );
    exports.export("memory", wasm_encoder::ExportKind::Memory, 0);
    module.section(&exports);

    // Code section: 函数返回 (data_offset << 32) | data_len
    let json_bytes = json_template.as_bytes();
    let data_offset: i64 = 0;
    let data_len = json_bytes.len() as i64;
    let combined = (data_offset << 32) | data_len;

    let mut code = wasm_encoder::CodeSection::new();
    let mut func = wasm_encoder::Function::new(vec![]);
    func.instruction(&wasm_encoder::Instruction::I64Const(combined));
    func.instruction(&wasm_encoder::Instruction::End);
    code.function(&func);
    module.section(&code);

    // Data section: JSON 模板写入内存偏移 0
    let mut data = wasm_encoder::DataSection::new();
    data.active(0, &wasm_encoder::ConstExpr::i32_const(0), json_bytes.iter().copied());
    module.section(&data);

    Ok(module.finish())
}

/// 从 MacroDef 中提取 quote 体
fn extract_quote_from_macro(macro_def: &MacroDef) -> Result<Vec<Stmt>, MacroError> {
    for stmt in &macro_def.body {
        if let Stmt::Return(Some(Expr::Quote { body, .. })) = stmt {
            return Ok(body.clone());
        }
    }
    // 如果没有 quote，返回宏体本身（降级）
    Ok(macro_def.body.clone())
}

/// 使用 wasmtime 执行编译后的宏 WASM 模块
pub fn execute_macro(
    name: &str,
    wasm_bytes: &[u8],
    args: &[Expr],
) -> Result<Vec<Stmt>, MacroError> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytes)
        .map_err(|e| MacroError::RuntimeError(format!("WASM 模块加载失败: {}", e)))?;

    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|e| MacroError::RuntimeError(format!("WASM 实例化失败: {}", e)))?;

    // 调用导出的宏函数
    let export_name = format!("macro_{}", name);
    let macro_func = instance
        .get_typed_func::<(), i64>(&mut store, &export_name)
        .map_err(|e| MacroError::RuntimeError(format!("找不到导出函数 '{}': {}", export_name, e)))?;

    let result = macro_func
        .call(&mut store, ())
        .map_err(|e| MacroError::RuntimeError(format!("宏执行失败: {}", e)))?;

    // 解析返回值: 高 32 位 = ptr, 低 32 位 = len
    let ptr = ((result >> 32) & 0xFFFFFFFF) as u32;
    let len = (result & 0xFFFFFFFF) as u32;

    // 从 WASM 内存读取 JSON 字符串
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| MacroError::RuntimeError("WASM 模块缺少 memory 导出".to_string()))?;

    let data = memory.data(&store);
    if (ptr as usize + len as usize) > data.len() {
        return Err(MacroError::RuntimeError(format!(
            "宏返回的内存区域越界: ptr={}, len={}, memory_size={}",
            ptr, len, data.len()
        )));
    }

    let json_str = std::str::from_utf8(&data[ptr as usize..(ptr + len) as usize])
        .map_err(|e| MacroError::SerdeError(format!("JSON 解码失败: {}", e)))?;

    let stmts: Vec<Stmt> = serde_json::from_str(json_str)
        .map_err(|e| MacroError::SerdeError(format!("AST 反序列化失败: {}", e)))?;

    // 参数替换（将宏参数引用替换为实际值）
    let _ = args; // TODO: 实现参数替换

    Ok(stmts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    #[test]
    fn test_compile_macro_to_wasm() {
        let macro_def = MacroDef {
            visibility: Visibility::Public,
            name: "TestMacro".to_string(),
            params: vec![],
            body: vec![Stmt::Return(Some(Expr::Quote {
                body: vec![Stmt::Expr(Expr::Call {
                    name: "println".to_string(),
                    type_args: None,
                    args: vec![Expr::String("expanded!".to_string())],
                    named_args: vec![],
                })],
                splices: vec![],
            }))],
        };

        let wasm = compile_macro_to_wasm(&macro_def).unwrap();
        assert!(!wasm.is_empty());
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_execute_macro() {
        let macro_def = MacroDef {
            visibility: Visibility::Public,
            name: "TestMacro".to_string(),
            params: vec![],
            body: vec![Stmt::Return(Some(Expr::Quote {
                body: vec![Stmt::Expr(Expr::Call {
                    name: "println".to_string(),
                    type_args: None,
                    args: vec![Expr::String("expanded!".to_string())],
                    named_args: vec![],
                })],
                splices: vec![],
            }))],
        };

        let wasm = compile_macro_to_wasm(&macro_def).unwrap();
        let result = execute_macro("TestMacro", &wasm, &[]).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            Stmt::Expr(Expr::Call { name, .. }) => assert_eq!(name, "println"),
            other => panic!("Expected Call, got {:?}", other),
        }
    }
}
