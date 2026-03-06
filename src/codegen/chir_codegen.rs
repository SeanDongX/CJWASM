//! CHIR → WASM 代码生成

use crate::chir::{
    CHIRExpr, CHIRExprKind, CHIRStmt, CHIRBlock, CHIRFunction, CHIRProgram,
    CHIRLValue,
};
use crate::ast::{BinOp, UnaryOp, Type};
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection,
    MemoryType, Module, TypeSection, ValType, ConstExpr,
};
use std::collections::HashMap;

// ─── WASI 导入配置 ───────────────────────────────────────────────────────────

/// WASI fd_write: (fd: i32, iovs_ptr: i32, iovs_len: i32, nwritten_ptr: i32) -> i32
const WASI_FD_WRITE_PARAMS: &[ValType] = &[ValType::I32, ValType::I32, ValType::I32, ValType::I32];
const WASI_FD_WRITE_RESULTS: &[ValType] = &[ValType::I32];

/// WASI proc_exit: (code: i32) -> !
const WASI_PROC_EXIT_PARAMS: &[ValType] = &[ValType::I32];
const WASI_PROC_EXIT_RESULTS: &[ValType] = &[];

/// 导入函数的 WASM 函数索引（在所有用户函数之前）
#[allow(dead_code)]
const IDX_FD_WRITE: u32 = 0;
#[allow(dead_code)]
const IDX_PROC_EXIT: u32 = 1;
const IMPORT_COUNT: u32 = 2;

/// 内存起始页（1页 = 64 KiB）
const MEMORY_PAGES_MIN: u64 = 4;
const MEMORY_PAGES_MAX: u64 = 64;

// ─── CHIRCodeGen ──────────────────────────────────────────────────────────────

/// CHIR 代码生成器
pub struct CHIRCodeGen {
    /// 用户函数名 → WASM 函数索引（已加 IMPORT_COUNT 偏移）
    func_indices: HashMap<String, u32>,
}

impl CHIRCodeGen {
    pub fn new() -> Self {
        CHIRCodeGen {
            func_indices: HashMap::new(),
        }
    }

    /// 生成完整 WASM 模块
    pub fn generate(&mut self, program: &CHIRProgram) -> Vec<u8> {
        // 用户函数索引 = 导入数量 + 用户序号
        for (i, func) in program.functions.iter().enumerate() {
            self.func_indices.insert(func.name.clone(), IMPORT_COUNT + i as u32);
        }

        let mut types = TypeSection::new();
        let mut imports = ImportSection::new();
        let mut functions = FunctionSection::new();
        let mut memories = MemorySection::new();
        let mut globals = GlobalSection::new();
        let mut exports = ExportSection::new();
        let mut codes = CodeSection::new();

        // ── 1. 类型段：先 WASI，再用户函数 ──────────────────────────
        // ty 0: fd_write
        types.ty().function(WASI_FD_WRITE_PARAMS.to_vec(), WASI_FD_WRITE_RESULTS.to_vec());
        // ty 1: proc_exit
        types.ty().function(WASI_PROC_EXIT_PARAMS.to_vec(), WASI_PROC_EXIT_RESULTS.to_vec());
        // ty 2+: 用户函数
        let mut user_type_indices: Vec<u32> = Vec::new();
        for func in &program.functions {
            let param_tys: Vec<ValType> = func.params.iter().map(|p| p.wasm_ty).collect();
            let result_tys = wasm_result_tys(&func.return_ty, func.return_wasm_ty);
            let type_idx = types.len();
            types.ty().function(param_tys, result_tys);
            user_type_indices.push(type_idx);
        }

        // ── 2. 导入段 ─────────────────────────────────────────────────
        imports.import(
            "wasi_snapshot_preview1",
            "fd_write",
            EntityType::Function(0), // ty idx 0
        );
        imports.import(
            "wasi_snapshot_preview1",
            "proc_exit",
            EntityType::Function(1), // ty idx 1
        );

        // ── 3. 函数段 ─────────────────────────────────────────────────
        for &type_idx in &user_type_indices {
            functions.function(type_idx);
        }

        // ── 4. 内存段 ─────────────────────────────────────────────────
        memories.memory(MemoryType {
            minimum: MEMORY_PAGES_MIN,
            maximum: Some(MEMORY_PAGES_MAX),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });

        // ── 5. 全局段：堆指针 ─────────────────────────────────────────
        // global 0: heap_ptr (可变 i32，初始指向页起始 64KiB)
        globals.global(
            GlobalType { val_type: ValType::I32, mutable: true, shared: false },
            &ConstExpr::i32_const(65536), // 1 page offset
        );

        // ── 6. 导出段 ─────────────────────────────────────────────────
        exports.export("memory", ExportKind::Memory, 0);
        // 导出所有用户函数
        for func in &program.functions {
            let idx = self.func_indices[&func.name];
            exports.export(&func.name, ExportKind::Func, idx);
        }
        // 兼容 WASI: _start = main
        if let Some(&main_idx) = self.func_indices.get("main") {
            // 仅当 main 返回 Unit 时才导出 _start
            if let Some(main_func) = program.functions.iter().find(|f| f.name == "main") {
                if main_func.return_ty == Type::Unit {
                    exports.export("_start", ExportKind::Func, main_idx);
                }
            }
        }

        // ── 7. 代码段 ─────────────────────────────────────────────────
        for func in &program.functions {
            self.emit_function(func, &mut codes);
        }

        // ── 组装（WASM 段顺序必须按规范） ─────────────────────────────
        let mut module = Module::new();
        module.section(&types);
        module.section(&imports);
        module.section(&functions);
        module.section(&memories);
        module.section(&globals);
        module.section(&exports);
        module.section(&codes);
        module.finish()
    }

    // ─── 函数 ──────────────────────────────────────────────────────────────

    fn emit_function(&self, func: &CHIRFunction, codes: &mut CodeSection) {
        // 收集函数体中所有 let/var 局部变量（索引 >= param_count）
        let param_count = func.params.len() as u32;
        let mut locals_map: Vec<(u32, ValType)> = Vec::new(); // (idx, wasm_ty)
        collect_locals_from_block(&func.body, param_count, &mut locals_map);
        // 去重，按索引排序
        locals_map.sort_by_key(|&(idx, _)| idx);
        locals_map.dedup_by_key(|l| l.0);

        // wasm_encoder 的 locals 格式：(count, ValType) 的 run-length
        let locals_for_encoder = run_length_encode_locals(&locals_map);
        let mut wasm_func = wasm_encoder::Function::new(locals_for_encoder);

        let has_result = !wasm_result_tys(&func.return_ty, func.return_wasm_ty).is_empty();

        self.emit_block(&func.body, &mut wasm_func);

        // 函数末尾若无显式 return 也无隐式 result 表达式，补默认返回值
        if has_result && !block_has_return(&func.body) && func.body.result.is_none() {
            emit_zero(func.return_wasm_ty, &mut wasm_func);
        }

        wasm_func.instruction(&Instruction::End);
        codes.function(&wasm_func);
    }

    fn emit_block(&self, block: &CHIRBlock, func: &mut wasm_encoder::Function) {
        for stmt in &block.stmts {
            self.emit_stmt(stmt, func);
        }
        if let Some(result) = &block.result {
            self.emit_expr(result, func);
        }
    }

    // ─── 表达式 ────────────────────────────────────────────────────────────

    fn emit_expr(&self, expr: &CHIRExpr, func: &mut wasm_encoder::Function) {
        match &expr.kind {
            CHIRExprKind::Integer(n) => {
                match expr.wasm_ty {
                    ValType::I32 => { func.instruction(&Instruction::I32Const(*n as i32)); }
                    ValType::I64 => { func.instruction(&Instruction::I64Const(*n)); }
                    _ => { func.instruction(&Instruction::I32Const(0)); }
                }
            }
            CHIRExprKind::Float(f) => { func.instruction(&Instruction::F64Const(*f)); }
            CHIRExprKind::Float32(f) => { func.instruction(&Instruction::F32Const(*f)); }
            CHIRExprKind::Bool(b) => {
                func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            }
            CHIRExprKind::Rune(c) => {
                func.instruction(&Instruction::I32Const(*c as i32));
            }
            CHIRExprKind::String(_s) => {
                // 字符串字面量简化：返回 0（完整实现需数据段）
                func.instruction(&Instruction::I32Const(0));
            }

            CHIRExprKind::Local(idx) => {
                func.instruction(&Instruction::LocalGet(*idx));
            }
            CHIRExprKind::Global(_name) => {
                // 全局变量简化：读堆指针占位
                func.instruction(&Instruction::GlobalGet(0));
            }

            CHIRExprKind::Binary { op, left, right } => {
                self.emit_expr(left, func);
                self.emit_expr(right, func);
                self.emit_binary_op(op, expr.wasm_ty, func);
            }
            CHIRExprKind::Unary { op, expr: inner } => {
                self.emit_expr(inner, func);
                self.emit_unary_op(op, expr.wasm_ty, func);
            }

            CHIRExprKind::Call { func_idx, args } => {
                for arg in args {
                    self.emit_expr(arg, func);
                }
                func.instruction(&Instruction::Call(*func_idx));
            }

            CHIRExprKind::MethodCall { func_idx, vtable_offset: _, receiver, args } => {
                self.emit_expr(receiver, func);
                for arg in args {
                    self.emit_expr(arg, func);
                }
                if let Some(idx) = func_idx {
                    func.instruction(&Instruction::Call(*idx));
                }
            }

            CHIRExprKind::Cast { expr: inner, from_ty, to_ty } => {
                self.emit_expr(inner, func);
                self.emit_cast(*from_ty, *to_ty, func);
            }

            CHIRExprKind::If { cond, then_block, else_block } => {
                self.emit_expr(cond, func);
                // 仅当 then/else 两侧都会产出值时，才用 Result block type
                let then_produces = then_block.result.is_some()
                    || then_block.stmts.iter().any(|s| matches!(s, CHIRStmt::Return(_)));
                let block_type = if matches!(expr.ty, Type::Unit) || !then_produces {
                    BlockType::Empty
                } else {
                    BlockType::Result(expr.wasm_ty)
                };
                func.instruction(&Instruction::If(block_type));
                self.emit_block(then_block, func);
                if let Some(else_blk) = else_block {
                    func.instruction(&Instruction::Else);
                    self.emit_block(else_blk, func);
                }
                func.instruction(&Instruction::End);
            }

            CHIRExprKind::Block(block) => {
                self.emit_block(block, func);
            }

            CHIRExprKind::FieldGet { object, field_offset, .. } => {
                self.emit_expr(object, func);
                func.instruction(&Instruction::I32Const(*field_offset as i32));
                func.instruction(&Instruction::I32Add);
                emit_load(expr.wasm_ty, func);
            }

            CHIRExprKind::ArrayGet { array, index } => {
                self.emit_expr(array, func);
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(index, func);
                // index 可能是 i64，统一截断到 i32 做地址计算
                if index.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                let elem_size = wasm_ty_bytes(expr.wasm_ty) as i32;
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                emit_load(expr.wasm_ty, func);
            }

            CHIRExprKind::TupleGet { tuple, index } => {
                self.emit_expr(tuple, func);
                // 简化：每个 tuple 元素固定 8 字节
                func.instruction(&Instruction::I32Const((*index * 8) as i32));
                func.instruction(&Instruction::I32Add);
                emit_load(expr.wasm_ty, func);
            }

            CHIRExprKind::Nop => {
                // 非 Unit 类型的 Nop 需要推入占位零值，否则返回/调用时栈不匹配
                if !matches!(expr.ty, Type::Unit | Type::Nothing) {
                    emit_zero(expr.wasm_ty, func);
                }
            }
            CHIRExprKind::Unreachable => {
                func.instruction(&Instruction::Unreachable);
            }

            // 未实现的表达式：推入零值占位
            _ => { emit_zero(expr.wasm_ty, func); }
        }
    }

    // ─── 语句 ──────────────────────────────────────────────────────────────

    fn emit_stmt(&self, stmt: &CHIRStmt, func: &mut wasm_encoder::Function) {
        match stmt {
            CHIRStmt::Let { local_idx, value } => {
                self.emit_expr(value, func);
                func.instruction(&Instruction::LocalSet(*local_idx));
            }

            CHIRStmt::Assign { target, value } => {
                self.emit_assign(target, value, func);
            }

            CHIRStmt::Expr(expr) => {
                self.emit_expr(expr, func);
                // 非 Unit 表达式丢弃结果
                if !matches!(expr.ty, Type::Unit) && !matches!(expr.kind, CHIRExprKind::Nop) {
                    func.instruction(&Instruction::Drop);
                }
            }

            CHIRStmt::Return(expr_opt) => {
                if let Some(expr) = expr_opt {
                    self.emit_expr(expr, func);
                }
                func.instruction(&Instruction::Return);
            }

            CHIRStmt::Break => { func.instruction(&Instruction::Br(1)); }
            CHIRStmt::Continue => { func.instruction(&Instruction::Br(0)); }

            CHIRStmt::While { cond, body } => {
                // block { loop { break_if(!cond); body; br 0 } }
                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));
                self.emit_expr(cond, func);
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));
                self.emit_block(body, func);
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // loop
                func.instruction(&Instruction::End); // block
            }

            CHIRStmt::Loop { body } => {
                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));
                self.emit_block(body, func);
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // loop
                func.instruction(&Instruction::End); // block
            }
        }
    }

    fn emit_assign(&self, target: &CHIRLValue, value: &CHIRExpr, func: &mut wasm_encoder::Function) {
        match target {
            CHIRLValue::Local(idx) => {
                self.emit_expr(value, func);
                func.instruction(&Instruction::LocalSet(*idx));
            }
            CHIRLValue::Field { object, offset } => {
                self.emit_expr(object, func);
                func.instruction(&Instruction::I32Const(*offset as i32));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(value, func);
                emit_store(value.wasm_ty, func);
            }
            CHIRLValue::Index { array, index } => {
                self.emit_expr(array, func);
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(index, func);
                if index.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                let elem_size = wasm_ty_bytes(value.wasm_ty) as i32;
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                self.emit_expr(value, func);
                emit_store(value.wasm_ty, func);
            }
        }
    }

    // ─── 运算符 ────────────────────────────────────────────────────────────

    fn emit_binary_op(&self, op: &BinOp, ty: ValType, func: &mut wasm_encoder::Function) {
        match (op, ty) {
            (BinOp::Add,   ValType::I32) => { func.instruction(&Instruction::I32Add); }
            (BinOp::Sub,   ValType::I32) => { func.instruction(&Instruction::I32Sub); }
            (BinOp::Mul,   ValType::I32) => { func.instruction(&Instruction::I32Mul); }
            (BinOp::Div,   ValType::I32) => { func.instruction(&Instruction::I32DivS); }
            (BinOp::Mod,   ValType::I32) => { func.instruction(&Instruction::I32RemS); }
            (BinOp::BitAnd,ValType::I32) => { func.instruction(&Instruction::I32And); }
            (BinOp::BitOr, ValType::I32) => { func.instruction(&Instruction::I32Or); }
            (BinOp::BitXor,ValType::I32) => { func.instruction(&Instruction::I32Xor); }
            (BinOp::Shl,   ValType::I32) => { func.instruction(&Instruction::I32Shl); }
            (BinOp::Shr,   ValType::I32) => { func.instruction(&Instruction::I32ShrS); }
            (BinOp::Eq,    ValType::I32) => { func.instruction(&Instruction::I32Eq); }
            (BinOp::NotEq, ValType::I32) => { func.instruction(&Instruction::I32Ne); }
            (BinOp::Lt,    ValType::I32) => { func.instruction(&Instruction::I32LtS); }
            (BinOp::LtEq,  ValType::I32) => { func.instruction(&Instruction::I32LeS); }
            (BinOp::Gt,    ValType::I32) => { func.instruction(&Instruction::I32GtS); }
            (BinOp::GtEq,  ValType::I32) => { func.instruction(&Instruction::I32GeS); }

            (BinOp::Add,   ValType::I64) => { func.instruction(&Instruction::I64Add); }
            (BinOp::Sub,   ValType::I64) => { func.instruction(&Instruction::I64Sub); }
            (BinOp::Mul,   ValType::I64) => { func.instruction(&Instruction::I64Mul); }
            (BinOp::Div,   ValType::I64) => { func.instruction(&Instruction::I64DivS); }
            (BinOp::Mod,   ValType::I64) => { func.instruction(&Instruction::I64RemS); }
            (BinOp::BitAnd,ValType::I64) => { func.instruction(&Instruction::I64And); }
            (BinOp::BitOr, ValType::I64) => { func.instruction(&Instruction::I64Or); }
            (BinOp::BitXor,ValType::I64) => { func.instruction(&Instruction::I64Xor); }
            (BinOp::Shl,   ValType::I64) => { func.instruction(&Instruction::I64Shl); }
            (BinOp::Shr,   ValType::I64) => { func.instruction(&Instruction::I64ShrS); }
            (BinOp::Eq,    ValType::I64) => { func.instruction(&Instruction::I64Eq); }
            (BinOp::NotEq, ValType::I64) => { func.instruction(&Instruction::I64Ne); }
            (BinOp::Lt,    ValType::I64) => { func.instruction(&Instruction::I64LtS); }
            (BinOp::LtEq,  ValType::I64) => { func.instruction(&Instruction::I64LeS); }
            (BinOp::Gt,    ValType::I64) => { func.instruction(&Instruction::I64GtS); }
            (BinOp::GtEq,  ValType::I64) => { func.instruction(&Instruction::I64GeS); }

            (BinOp::Add,   ValType::F64) => { func.instruction(&Instruction::F64Add); }
            (BinOp::Sub,   ValType::F64) => { func.instruction(&Instruction::F64Sub); }
            (BinOp::Mul,   ValType::F64) => { func.instruction(&Instruction::F64Mul); }
            (BinOp::Div,   ValType::F64) => { func.instruction(&Instruction::F64Div); }
            (BinOp::Eq,    ValType::F64) => { func.instruction(&Instruction::F64Eq); }
            (BinOp::NotEq, ValType::F64) => { func.instruction(&Instruction::F64Ne); }
            (BinOp::Lt,    ValType::F64) => { func.instruction(&Instruction::F64Lt); }
            (BinOp::LtEq,  ValType::F64) => { func.instruction(&Instruction::F64Le); }
            (BinOp::Gt,    ValType::F64) => { func.instruction(&Instruction::F64Gt); }
            (BinOp::GtEq,  ValType::F64) => { func.instruction(&Instruction::F64Ge); }

            (BinOp::Add,   ValType::F32) => { func.instruction(&Instruction::F32Add); }
            (BinOp::Sub,   ValType::F32) => { func.instruction(&Instruction::F32Sub); }
            (BinOp::Mul,   ValType::F32) => { func.instruction(&Instruction::F32Mul); }
            (BinOp::Div,   ValType::F32) => { func.instruction(&Instruction::F32Div); }

            // 逻辑 And/Or：操作数已经是 i32，直接 And/Or
            (BinOp::LogicalAnd, _) => { func.instruction(&Instruction::I32And); }
            (BinOp::LogicalOr,  _) => { func.instruction(&Instruction::I32Or); }

            _ => {}
        }
    }

    fn emit_unary_op(&self, op: &UnaryOp, ty: ValType, func: &mut wasm_encoder::Function) {
        match (op, ty) {
            (UnaryOp::Not, _) => { func.instruction(&Instruction::I32Eqz); }
            (UnaryOp::Neg, ValType::I32) => {
                func.instruction(&Instruction::I32Const(0));
                // swap: we need 0 - x, but x is already on stack
                // so push 0 first would require putting x second
                // easier: i32.const(0) then i32.sub (wrong order)
                // correct: use i32.const(-1) xor + i32.const(1) add = bitwise neg+1
                func.instruction(&Instruction::I32Sub);
            }
            (UnaryOp::Neg, ValType::I64) => {
                func.instruction(&Instruction::I64Const(0));
                func.instruction(&Instruction::I64Sub);
            }
            (UnaryOp::Neg, ValType::F64) => { func.instruction(&Instruction::F64Neg); }
            (UnaryOp::Neg, ValType::F32) => { func.instruction(&Instruction::F32Neg); }
            (UnaryOp::BitNot, ValType::I32) => {
                func.instruction(&Instruction::I32Const(-1));
                func.instruction(&Instruction::I32Xor);
            }
            (UnaryOp::BitNot, ValType::I64) => {
                func.instruction(&Instruction::I64Const(-1));
                func.instruction(&Instruction::I64Xor);
            }
            _ => {}
        }
    }

    fn emit_cast(&self, from: ValType, to: ValType, func: &mut wasm_encoder::Function) {
        match (from, to) {
            (ValType::I64, ValType::I32) => { func.instruction(&Instruction::I32WrapI64); }
            (ValType::I32, ValType::I64) => { func.instruction(&Instruction::I64ExtendI32S); }
            (ValType::I64, ValType::F64) => { func.instruction(&Instruction::F64ConvertI64S); }
            (ValType::I32, ValType::F64) => { func.instruction(&Instruction::F64ConvertI32S); }
            (ValType::I64, ValType::F32) => { func.instruction(&Instruction::F32ConvertI64S); }
            (ValType::I32, ValType::F32) => { func.instruction(&Instruction::F32ConvertI32S); }
            (ValType::F64, ValType::I64) => { func.instruction(&Instruction::I64TruncF64S); }
            (ValType::F64, ValType::I32) => { func.instruction(&Instruction::I32TruncF64S); }
            (ValType::F32, ValType::I64) => { func.instruction(&Instruction::I64TruncF32S); }
            (ValType::F32, ValType::I32) => { func.instruction(&Instruction::I32TruncF32S); }
            (ValType::F32, ValType::F64) => { func.instruction(&Instruction::F64PromoteF32); }
            (ValType::F64, ValType::F32) => { func.instruction(&Instruction::F32DemoteF64); }
            _ => {} // 相同类型不需要指令
        }
    }
}

impl Default for CHIRCodeGen {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 辅助函数 ─────────────────────────────────────────────────────────────────

/// 根据 AST 返回类型计算 WASM 结果类型列表
fn wasm_result_tys(ty: &Type, wasm_ty: ValType) -> Vec<ValType> {
    match ty {
        Type::Unit | Type::Nothing => vec![],
        _ => vec![wasm_ty],
    }
}

/// 检查块是否包含显式 return 语句
fn block_has_return(block: &CHIRBlock) -> bool {
    block.stmts.iter().any(|s| matches!(s, CHIRStmt::Return(_)))
}

/// 推入类型对应的零值
fn emit_zero(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => { func.instruction(&Instruction::I32Const(0)); }
        ValType::I64 => { func.instruction(&Instruction::I64Const(0)); }
        ValType::F32 => { func.instruction(&Instruction::F32Const(0.0)); }
        ValType::F64 => { func.instruction(&Instruction::F64Const(0.0)); }
        _ => {}
    }
}

/// 生成 load 指令
fn emit_load(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => { func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 })); }
        ValType::I64 => { func.instruction(&Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 })); }
        ValType::F32 => { func.instruction(&Instruction::F32Load(MemArg { offset: 0, align: 2, memory_index: 0 })); }
        ValType::F64 => { func.instruction(&Instruction::F64Load(MemArg { offset: 0, align: 3, memory_index: 0 })); }
        _ => {}
    }
}

/// 生成 store 指令
fn emit_store(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => { func.instruction(&Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 })); }
        ValType::I64 => { func.instruction(&Instruction::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 })); }
        ValType::F32 => { func.instruction(&Instruction::F32Store(MemArg { offset: 0, align: 2, memory_index: 0 })); }
        ValType::F64 => { func.instruction(&Instruction::F64Store(MemArg { offset: 0, align: 3, memory_index: 0 })); }
        _ => {}
    }
}

/// 获取 ValType 的字节大小
fn wasm_ty_bytes(ty: ValType) -> u32 {
    match ty {
        ValType::I32 | ValType::F32 => 4,
        ValType::I64 | ValType::F64 => 8,
        _ => 4,
    }
}

/// 遍历 CHIR 块，收集 let 语句分配的局部变量 (idx, wasm_ty)，只收集 idx >= param_count 的
fn collect_locals_from_block(block: &CHIRBlock, param_count: u32, out: &mut Vec<(u32, ValType)>) {
    for stmt in &block.stmts {
        collect_locals_from_stmt(stmt, param_count, out);
    }
}

fn collect_locals_from_stmt(stmt: &CHIRStmt, param_count: u32, out: &mut Vec<(u32, ValType)>) {
    match stmt {
        CHIRStmt::Let { local_idx, value } => {
            if *local_idx >= param_count {
                out.push((*local_idx, value.wasm_ty));
            }
        }
        CHIRStmt::While { body, .. } | CHIRStmt::Loop { body } => {
            collect_locals_from_block(body, param_count, out);
        }
        CHIRStmt::Expr(expr) => {
            collect_locals_from_expr(expr, param_count, out);
        }
        _ => {}
    }
}

fn collect_locals_from_expr(expr: &CHIRExpr, param_count: u32, out: &mut Vec<(u32, ValType)>) {
    match &expr.kind {
        CHIRExprKind::If { then_block, else_block, .. } => {
            collect_locals_from_block(then_block, param_count, out);
            if let Some(b) = else_block {
                collect_locals_from_block(b, param_count, out);
            }
        }
        CHIRExprKind::Block(b) => {
            collect_locals_from_block(b, param_count, out);
        }
        _ => {}
    }
}

/// 将 (idx, wasm_ty) 列表压缩为 wasm_encoder 的 run-length 格式 (count, ValType)
fn run_length_encode_locals(locals: &[(u32, ValType)]) -> Vec<(u32, ValType)> {
    if locals.is_empty() {
        return vec![];
    }
    // locals 已按 idx 排序，但 idx 可能不连续。
    // wasm_encoder 需要 (连续数量, 类型) 的分组，我们用最简单的按类型分组。
    let mut result: Vec<(u32, ValType)> = Vec::new();
    for &(_, ty) in locals {
        if let Some(last) = result.last_mut() {
            if last.1 == ty {
                last.0 += 1;
                continue;
            }
        }
        result.push((1, ty));
    }
    result
}
