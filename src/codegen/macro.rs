use crate::ast::Expr;
use crate::codegen::CodeGen;
use wasm_encoder::{Function as WasmFunc, Instruction, BlockType};

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
                    let msg_offset = self.string_pool
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
                    let msg_offset = self.string_pool
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
                let offset = self.string_pool
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
                let offset = self.string_pool
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
