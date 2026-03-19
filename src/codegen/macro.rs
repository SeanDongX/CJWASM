use crate::ast::Expr;
use crate::codegen::CodeGen;
use wasm_encoder::{BlockType, Function as WasmFunc, Instruction};

// LocalsBuilder 的简化类型别名
type LocalsBuilder = crate::codegen::LocalsBuilder;

impl CodeGen {
    /// 编译宏调用
    pub(crate) fn compile_macro_call(
        &self,
        name: &str,
        args: &[Expr],
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        match name {
            // @Assert(a, b) - 断言 a == b，否则 panic
            "Assert" => {
                if args.len() != 2 {
                    eprintln!("Warning: @Assert requires exactly 2 arguments");
                    return;
                }

                // 编译两个参数
                self.compile_expr(&args[0], locals, func, loop_ctx);
                self.compile_expr(&args[1], locals, func, loop_ctx);

                // 比较是否相等 (假设都是 i64)
                func.instruction(&Instruction::I64Ne);

                // if 不相等则 panic
                func.instruction(&Instruction::If(BlockType::Empty));

                // 调用 panic 函数
                if let Some(&panic_idx) = self.func_indices.get("__panic") {
                    // 传递错误消息 "Assertion failed"
                    let msg_offset = self
                        .string_pool
                        .iter()
                        .find(|(s, _)| s == "Assertion failed")
                        .map(|(_, off)| *off)
                        .unwrap_or(0);
                    func.instruction(&Instruction::I32Const(msg_offset as i32));
                    func.instruction(&Instruction::Call(panic_idx));
                } else {
                    // 如果没有 panic 函数，使用 unreachable
                    func.instruction(&Instruction::Unreachable);
                }

                func.instruction(&Instruction::End);
            }

            // @Expect(actual, expected) - 类似 Assert
            "Expect" => {
                if args.len() != 2 {
                    eprintln!("Warning: @Expect requires exactly 2 arguments");
                    return;
                }

                // 与 Assert 相同的实现
                self.compile_expr(&args[0], locals, func, loop_ctx);
                self.compile_expr(&args[1], locals, func, loop_ctx);

                func.instruction(&Instruction::I64Ne);
                func.instruction(&Instruction::If(BlockType::Empty));

                if let Some(&panic_idx) = self.func_indices.get("__panic") {
                    let msg_offset = self
                        .string_pool
                        .iter()
                        .find(|(s, _)| s == "Expectation failed")
                        .map(|(_, off)| *off)
                        .unwrap_or(0);
                    func.instruction(&Instruction::I32Const(msg_offset as i32));
                    func.instruction(&Instruction::Call(panic_idx));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }

                func.instruction(&Instruction::End);
            }

            // @Deprecated - 编译时警告
            "Deprecated" => {
                eprintln!("Warning: Using deprecated feature");
                // 不生成任何代码
            }

            // @sourceFile - 返回当前文件名
            "sourceFile" => {
                // 返回文件名字符串
                let filename = "unknown.cj"; // TODO: 从编译上下文获取
                let offset = self
                    .string_pool
                    .iter()
                    .find(|(s, _)| s == filename)
                    .map(|(_, off)| *off)
                    .unwrap_or(0);
                func.instruction(&Instruction::I32Const(offset as i32));
            }

            // @sourceLine - 返回当前行号
            "sourceLine" => {
                // TODO: 从 AST 节点获取行号信息
                func.instruction(&Instruction::I64Const(0));
            }

            // @sourcePackage - 返回当前包名
            "sourcePackage" => {
                let package = "main"; // TODO: 从编译上下文获取
                let offset = self
                    .string_pool
                    .iter()
                    .find(|(s, _)| s == package)
                    .map(|(_, off)| *off)
                    .unwrap_or(0);
                func.instruction(&Instruction::I32Const(offset as i32));
            }

            // 未知宏
            _ => {
                eprintln!("Warning: Unknown macro @{}", name);
                // 不生成任何代码
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Function, Program, Stmt, Type, Visibility};
    use crate::codegen::{CodeGen, LocalsBuilder};

    fn codegen_with_main() -> CodeGen {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
            functions: vec![Function {
                visibility: Visibility::default(),
                name: "main".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Integer(0)))],
                extern_import: None,
            }],
        };
        let mut codegen = CodeGen::new();
        let _ = codegen.compile(&program);
        codegen
    }

    #[test]
    fn test_macro_assert_two_args() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        codegen.compile_macro_call(
            "Assert",
            &[Expr::Integer(1), Expr::Integer(1)],
            &locals,
            &mut wasm_func,
            None,
        );
        // Should have generated instructions (I64Const x2, I64Ne, If, End)
        assert!(wasm_func.byte_len() > 0);
    }

    #[test]
    fn test_macro_assert_wrong_args() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        let before = wasm_func.byte_len();
        codegen.compile_macro_call("Assert", &[Expr::Integer(1)], &locals, &mut wasm_func, None);
        // Wrong arg count - returns early without compiling (no extra instructions)
        assert_eq!(wasm_func.byte_len(), before);
    }

    #[test]
    fn test_macro_expect_two_args() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        codegen.compile_macro_call(
            "Expect",
            &[Expr::Integer(42), Expr::Integer(42)],
            &locals,
            &mut wasm_func,
            None,
        );
        assert!(wasm_func.byte_len() > 0);
    }

    #[test]
    fn test_macro_expect_wrong_args() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        let before = wasm_func.byte_len();
        codegen.compile_macro_call("Expect", &[Expr::Integer(1)], &locals, &mut wasm_func, None);
        // Wrong arg count - early return without compiling
        assert_eq!(wasm_func.byte_len(), before);
    }

    #[test]
    fn test_macro_deprecated() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        let before = wasm_func.byte_len();
        codegen.compile_macro_call("Deprecated", &[], &locals, &mut wasm_func, None);
        assert_eq!(
            wasm_func.byte_len(),
            before,
            "Deprecated should not emit code"
        );
    }

    #[test]
    fn test_macro_source_file() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        codegen.compile_macro_call("sourceFile", &[], &locals, &mut wasm_func, None);
        assert!(wasm_func.byte_len() > 0);
    }

    #[test]
    fn test_macro_source_line() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        codegen.compile_macro_call("sourceLine", &[], &locals, &mut wasm_func, None);
        assert!(wasm_func.byte_len() > 0);
    }

    #[test]
    fn test_macro_source_package() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        codegen.compile_macro_call("sourcePackage", &[], &locals, &mut wasm_func, None);
        assert!(wasm_func.byte_len() > 0);
    }

    #[test]
    fn test_macro_unknown() {
        let codegen = codegen_with_main();
        let locals = LocalsBuilder::new();
        let mut wasm_func = WasmFunc::new(vec![]);
        let before = wasm_func.byte_len();
        codegen.compile_macro_call("UnknownMacro", &[], &locals, &mut wasm_func, None);
        assert_eq!(
            wasm_func.byte_len(),
            before,
            "Unknown macro should not emit code"
        );
    }
}
