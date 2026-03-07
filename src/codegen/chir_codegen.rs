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

// ─── I/O 内存布局 ─────────────────────────────────────────────────────────────

/// iovec 结构体偏移（8 字节：buf_ptr[0..4] + buf_len[4..8]）
const IOVEC_OFFSET: i32 = 64;
/// fd_write nwritten 输出指针偏移
const NWRITTEN_OFFSET: i32 = 72;
/// 数据段起始地址（前 128 字节保留给 I/O 缓冲区）
const DATA_SECTION_BASE: u32 = 128;

// ─── 运行时助手函数名 ─────────────────────────────────────────────────────────

const RT_PRINTLN_I64:   &str = "__rt_println_i64";
const RT_PRINT_I64:     &str = "__rt_print_i64";
const RT_PRINTLN_STR:   &str = "__rt_println_str";
const RT_PRINT_STR:     &str = "__rt_print_str";
const RT_PRINTLN_BOOL:  &str = "__rt_println_bool";
const RT_PRINT_BOOL:    &str = "__rt_print_bool";
const RT_PRINTLN_EMPTY: &str = "__rt_println_empty";
const RT_NAMES: &[&str] = &[
    RT_PRINTLN_I64, RT_PRINT_I64,
    RT_PRINTLN_STR, RT_PRINT_STR,
    RT_PRINTLN_BOOL, RT_PRINT_BOOL,
    RT_PRINTLN_EMPTY,
];

// ─── CHIRCodeGen ──────────────────────────────────────────────────────────────

/// CHIR 代码生成器
pub struct CHIRCodeGen {
    /// 用户函数名 → WASM 函数索引（已加 IMPORT_COUNT 偏移）
    func_indices: HashMap<String, u32>,
    /// 函数索引 → 是否为 void（无返回值）函数
    func_void_map: HashMap<u32, bool>,
    /// 当前循环上下文中 break 所需的 Br 深度
    /// 0 = 不在循环内；1 = 直接在 loop body 内；每进入一个 if/block +1
    loop_break_depth: std::cell::Cell<u32>,
    /// 字符串常量池：(内容, 内存地址)
    string_data: Vec<(String, u32)>,
    /// 字符串内容 → 内存地址快速查找
    string_addresses: HashMap<String, u32>,
    /// 数据段当前写入位置
    data_offset: u32,
}

impl CHIRCodeGen {
    pub fn new() -> Self {
        CHIRCodeGen {
            func_indices: HashMap::new(),
            func_void_map: HashMap::new(),
            loop_break_depth: std::cell::Cell::new(0),
            string_data: Vec::new(),
            string_addresses: HashMap::new(),
            data_offset: DATA_SECTION_BASE,
        }
    }

    /// 注册字符串到字符串池，返回内存地址（格式：[len:i32][bytes...]）
    fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&addr) = self.string_addresses.get(s) {
            return addr;
        }
        let addr = self.data_offset;
        self.string_addresses.insert(s.to_string(), addr);
        self.string_data.push((s.to_string(), addr));
        // 格式：[length: 4 bytes][utf8 bytes]，8字节对齐
        let total = 4 + s.len() as u32;
        let aligned = (total + 7) & !7; // 对齐到 8 字节
        self.data_offset += aligned;
        addr
    }

    /// 遍历 CHIR 程序，预先收集所有字符串字面量
    fn collect_strings_from_program(&mut self, program: &CHIRProgram) {
        for func in &program.functions {
            self.collect_strings_from_block(&func.body);
        }
    }

    fn collect_strings_from_block(&mut self, block: &CHIRBlock) {
        for stmt in &block.stmts {
            self.collect_strings_from_stmt(stmt);
        }
        if let Some(r) = &block.result {
            self.collect_strings_from_expr(r);
        }
    }

    fn collect_strings_from_stmt(&mut self, stmt: &CHIRStmt) {
        match stmt {
            CHIRStmt::Let { value, .. } => self.collect_strings_from_expr(value),
            CHIRStmt::Assign { value, .. } => self.collect_strings_from_expr(value),
            CHIRStmt::Expr(e) => self.collect_strings_from_expr(e),
            CHIRStmt::Return(Some(e)) => self.collect_strings_from_expr(e),
            CHIRStmt::While { cond, body } => {
                self.collect_strings_from_expr(cond);
                self.collect_strings_from_block(body);
            }
            CHIRStmt::Loop { body } => self.collect_strings_from_block(body),
            _ => {}
        }
    }

    fn collect_strings_from_expr(&mut self, expr: &CHIRExpr) {
        match &expr.kind {
            CHIRExprKind::String(s) => { self.intern_string(s); }
            CHIRExprKind::Print { arg, .. } => {
                if let Some(a) = arg { self.collect_strings_from_expr(a); }
            }
            CHIRExprKind::Binary { left, right, .. } => {
                self.collect_strings_from_expr(left);
                self.collect_strings_from_expr(right);
            }
            CHIRExprKind::Unary { expr: inner, .. } => self.collect_strings_from_expr(inner),
            CHIRExprKind::Call { args, .. } => {
                for a in args { self.collect_strings_from_expr(a); }
            }
            CHIRExprKind::MethodCall { receiver, args, .. } => {
                self.collect_strings_from_expr(receiver);
                for a in args { self.collect_strings_from_expr(a); }
            }
            CHIRExprKind::If { cond, then_block, else_block } => {
                self.collect_strings_from_expr(cond);
                self.collect_strings_from_block(then_block);
                if let Some(b) = else_block { self.collect_strings_from_block(b); }
            }
            CHIRExprKind::Block(b) => self.collect_strings_from_block(b),
            CHIRExprKind::Cast { expr: inner, .. } => self.collect_strings_from_expr(inner),
            CHIRExprKind::FieldGet { object, .. } => self.collect_strings_from_expr(object),
            CHIRExprKind::FieldSet { object, value, .. } => {
                self.collect_strings_from_expr(object);
                self.collect_strings_from_expr(value);
            }
            CHIRExprKind::ArrayGet { array, index } => {
                self.collect_strings_from_expr(array);
                self.collect_strings_from_expr(index);
            }
            CHIRExprKind::ArraySet { array, index, value } => {
                self.collect_strings_from_expr(array);
                self.collect_strings_from_expr(index);
                self.collect_strings_from_expr(value);
            }
            CHIRExprKind::ArrayNew { len, init } => {
                self.collect_strings_from_expr(len);
                self.collect_strings_from_expr(init);
            }
            CHIRExprKind::TupleGet { tuple, .. } => self.collect_strings_from_expr(tuple),
            CHIRExprKind::TupleNew { elements } => {
                for e in elements { self.collect_strings_from_expr(e); }
            }
            CHIRExprKind::StructNew { fields, .. } => {
                for (_, v) in fields { self.collect_strings_from_expr(v); }
            }
            _ => {}
        }
    }

    /// 生成完整 WASM 模块
    pub fn generate(&mut self, program: &CHIRProgram) -> Vec<u8> {
        // ── 0. 预处理：收集字符串字面量，分配内存地址 ────────────────
        self.collect_strings_from_program(program);

        // ── 用户函数索引 = 导入数量 + 用户序号 ─────────────────────────
        // 支持重载：用 "name$arity" 修饰名精确匹配；原名作为 fallback
        for (i, func) in program.functions.iter().enumerate() {
            let idx = IMPORT_COUNT + i as u32;
            let mangled = format!("{}${}", func.name, func.params.len());
            self.func_indices.insert(mangled, idx);
            self.func_indices.entry(func.name.clone()).or_insert(idx);
            // 记录函数是否为 void（Unit 返回）
            let is_void = matches!(func.return_ty, Type::Unit | Type::Nothing);
            self.func_void_map.insert(idx, is_void);
        }

        // ── 预注册运行时助手函数索引（在用户函数之后）─────────────────
        let user_count = program.functions.len() as u32;
        for (i, name) in RT_NAMES.iter().enumerate() {
            self.func_indices.insert(name.to_string(), IMPORT_COUNT + user_count + i as u32);
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
        // 运行时助手函数的类型索引（在用户函数之后添加）
        let rt_type_indices = self.register_rt_func_types(&mut types, &mut functions);

        // ── 4. 内存段 ─────────────────────────────────────────────────
        memories.memory(MemoryType {
            minimum: MEMORY_PAGES_MIN,
            maximum: Some(MEMORY_PAGES_MAX),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });

        // ── 5. 全局段：堆指针 ─────────────────────────────────────────
        // global 0: heap_ptr (可变 i32，初始指向字符串数据之后)
        let heap_start = (self.data_offset + 7) & !7; // 对齐
        globals.global(
            GlobalType { val_type: ValType::I32, mutable: true, shared: false },
            &ConstExpr::i32_const(heap_start as i32),
        );

        // ── 6. 导出段 ─────────────────────────────────────────────────
        exports.export("memory", ExportKind::Memory, 0);
        // 导出所有用户函数（用 HashSet 去重，防止同名函数多次导出导致 duplicate export 错误）
        let mut exported_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for func in &program.functions {
            let idx = self.func_indices[&func.name];
            if exported_names.insert(func.name.clone()) {
                exports.export(&func.name, ExportKind::Func, idx);
            }
        }
        // 兼容 WASI: _start = main（仅在用户未定义 _start 时添加）
        if !exported_names.contains("_start") {
            if let Some(&main_idx) = self.func_indices.get("main") {
                if let Some(main_func) = program.functions.iter().find(|f| f.name == "main") {
                    if main_func.return_ty == Type::Unit {
                        exports.export("_start", ExportKind::Func, main_idx);
                    }
                }
            }
        }

        // ── 7. 代码段 ─────────────────────────────────────────────────
        for func in &program.functions {
            self.emit_function(func, &mut codes);
        }
        // 运行时助手函数代码
        self.emit_rt_functions(&rt_type_indices, &mut codes);

        // ── 8. 数据段：字符串常量 ─────────────────────────────────────
        let data_section = if !self.string_data.is_empty() {
            let mut data = wasm_encoder::DataSection::new();
            for (s, addr) in &self.string_data {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(s.len() as i32).to_le_bytes());
                bytes.extend_from_slice(s.as_bytes());
                data.active(0, &ConstExpr::i32_const(*addr as i32), bytes);
            }
            Some(data)
        } else {
            None
        };

        // ── 组装（WASM 段顺序必须按规范） ─────────────────────────────
        let mut module = Module::new();
        module.section(&types);
        module.section(&imports);
        module.section(&functions);
        module.section(&memories);
        module.section(&globals);
        module.section(&exports);
        module.section(&codes);
        if let Some(ds) = &data_section {
            module.section(ds);
        }
        module.finish()
    }

    // ─── 运行时助手函数注册 ────────────────────────────────────────────────

    /// 为运行时助手注册类型/函数段，返回每个助手的 (type_idx, wasm_func_idx) 对
    fn register_rt_func_types(
        &self,
        types: &mut TypeSection,
        functions: &mut FunctionSection,
    ) -> Vec<u32> {
        let mut rt_type_indices = Vec::new();
        // i64 → void: __rt_println_i64, __rt_print_i64
        let ty_i64_void = types.len();
        types.ty().function(vec![ValType::I64], vec![]);
        functions.function(ty_i64_void);
        rt_type_indices.push(ty_i64_void); // println_i64
        functions.function(ty_i64_void);
        rt_type_indices.push(ty_i64_void); // print_i64
        // i32 → void: __rt_println_str, __rt_print_str, __rt_println_bool, __rt_print_bool
        let ty_i32_void = types.len();
        types.ty().function(vec![ValType::I32], vec![]);
        functions.function(ty_i32_void);
        rt_type_indices.push(ty_i32_void); // println_str
        functions.function(ty_i32_void);
        rt_type_indices.push(ty_i32_void); // print_str
        functions.function(ty_i32_void);
        rt_type_indices.push(ty_i32_void); // println_bool
        functions.function(ty_i32_void);
        rt_type_indices.push(ty_i32_void); // print_bool
        // () → void: __rt_println_empty
        let ty_void_void = types.len();
        types.ty().function(vec![], vec![]);
        functions.function(ty_void_void);
        rt_type_indices.push(ty_void_void); // println_empty
        rt_type_indices
    }

    /// 生成所有运行时助手函数的 WASM 代码
    fn emit_rt_functions(&self, _rt_type_indices: &[u32], codes: &mut CodeSection) {
        // 0: __rt_println_i64
        codes.function(&self.build_rt_print_i64(true, 1));
        // 1: __rt_print_i64
        codes.function(&self.build_rt_print_i64(false, 1));
        // 2: __rt_println_str
        codes.function(&self.build_rt_print_str(true, 1));
        // 3: __rt_print_str
        codes.function(&self.build_rt_print_str(false, 1));
        // 4: __rt_println_bool
        codes.function(&self.build_rt_print_bool(true, 1));
        // 5: __rt_print_bool
        codes.function(&self.build_rt_print_bool(false, 1));
        // 6: __rt_println_empty
        codes.function(&self.build_rt_println_empty(1));
    }

    /// 生成 I/O 辅助 MemArg
    fn mem(offset: u64, align: u32) -> MemArg {
        MemArg { offset, align, memory_index: 0 }
    }

    /// 生成 fd_write 调用（iovec 已设置好，直接调用）
    fn emit_fd_write_call(fd: i32, f: &mut wasm_encoder::Function) {
        f.instruction(&Instruction::I32Const(fd));
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
        f.instruction(&Instruction::Call(IDX_FD_WRITE));
        f.instruction(&Instruction::Drop);
    }

    /// 设置 iovec 并调用 fd_write 输出指定内存区域
    fn emit_write_buf(buf_ptr_expr: impl Fn(&mut wasm_encoder::Function), buf_len_expr: impl Fn(&mut wasm_encoder::Function), fd: i32, f: &mut wasm_encoder::Function) {
        // iovec.buf_ptr = buf_ptr
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        buf_ptr_expr(f);
        f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
        // iovec.buf_len = buf_len
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        buf_len_expr(f);
        f.instruction(&Instruction::I32Store(Self::mem(4, 2)));
        Self::emit_fd_write_call(fd, f);
    }

    /// 生成换行符输出指令（直接用 mem[0] 暂存 '\n'）
    fn emit_newline(fd: i32, f: &mut wasm_encoder::Function) {
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Const(10)); // '\n'
        f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
        Self::emit_write_buf(
            |f| { f.instruction(&Instruction::I32Const(0)); },
            |f| { f.instruction(&Instruction::I32Const(1)); },
            fd, f
        );
    }

    /// 生成 __rt_println_i64 / __rt_print_i64 函数体
    /// 参数 local 0: i64 (val)，局部 local 1: i32 (pos), local 2: i32 (is_neg)
    fn build_rt_print_i64(&self, newline: bool, fd: i32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![(2, ValType::I32)]);
        // end_pos / buf_len_base
        let end_pos: i32 = if newline { 23 } else { 22 };
        let buf_len_base: i32 = if newline { 24 } else { 22 };

        // pos = end_pos
        f.instruction(&Instruction::I32Const(end_pos));
        f.instruction(&Instruction::LocalSet(1));

        if newline {
            // mem[23] = '\n'
            f.instruction(&Instruction::I32Const(23));
            f.instruction(&Instruction::I32Const(10));
            f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
        }

        // if val == 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        {
            let zero_pos = end_pos - 1;
            f.instruction(&Instruction::I32Const(zero_pos));
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::I32Const(zero_pos));
            f.instruction(&Instruction::I32Const(48)); // '0'
            f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
        }
        f.instruction(&Instruction::Else);
        {
            // is_neg = val < 0
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(2));
            // if is_neg: val = -val
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(0));
            f.instruction(&Instruction::End);
            // loop: digits
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::LocalSet(1));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I64Const(10));
                f.instruction(&Instruction::I64RemU);
                f.instruction(&Instruction::I32WrapI64);
                f.instruction(&Instruction::I32Const(48));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I64Const(10));
                f.instruction(&Instruction::I64DivU);
                f.instruction(&Instruction::LocalSet(0));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // if is_neg: prepend '-'
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // iovec.buf = pos, iovec.len = buf_len_base - pos
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::I32Const(buf_len_base));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Store(Self::mem(4, 2)));
        Self::emit_fd_write_call(fd, &mut f);

        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __rt_println_str / __rt_print_str 函数体
    /// 参数 local 0: i32 (string ptr, 格式: [len:i32][bytes...])
    /// 局部变量 local 1: i32 (len)
    fn build_rt_print_str(&self, newline: bool, fd: i32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![(1, ValType::I32)]);

        // len = mem[ptr]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(Self::mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));

        // iovec.buf = ptr + 4
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(Self::mem(0, 2)));

        // iovec.len = len
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(Self::mem(4, 2)));

        Self::emit_fd_write_call(fd, &mut f);

        if newline {
            Self::emit_newline(fd, &mut f);
        }

        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __rt_println_bool / __rt_print_bool 函数体
    /// 参数 local 0: i32 (bool value, 0=false, 非0=true)
    fn build_rt_print_bool(&self, newline: bool, fd: i32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![]);

        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::If(BlockType::Empty));
        {
            // "true"
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(0x65757274_u32 as i32)); // "true" LE
            f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
            let len = if newline {
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
                5
            } else { 4 };
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(len));
            f.instruction(&Instruction::I32Store(Self::mem(4, 2)));
            Self::emit_fd_write_call(fd, &mut f);
        }
        f.instruction(&Instruction::Else);
        {
            // "false"
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(0x736C6166_u32 as i32)); // "fals" LE
            f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Const(101)); // 'e'
            f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
            let len = if newline {
                f.instruction(&Instruction::I32Const(5));
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32Store8(Self::mem(0, 0)));
                6
            } else { 5 };
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(Self::mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(len));
            f.instruction(&Instruction::I32Store(Self::mem(4, 2)));
            Self::emit_fd_write_call(fd, &mut f);
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __rt_println_empty() 函数体：仅输出换行符
    fn build_rt_println_empty(&self, fd: i32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![]);
        Self::emit_newline(fd, &mut f);
        f.instruction(&Instruction::End);
        f
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
        let locals_for_encoder = run_length_encode_locals(&locals_map, param_count);
        let mut wasm_func = wasm_encoder::Function::new(locals_for_encoder);

        let has_result = !wasm_result_tys(&func.return_ty, func.return_wasm_ty).is_empty();

        if has_result && !block_has_return(&func.body) {
            // 有返回值的函数：用 emit_block_with_ty 确保函数末尾推入正确类型的值
            self.emit_block_with_ty(&func.body, func.return_wasm_ty, &mut wasm_func);
        } else if !has_result {
            // Unit 函数：用 emit_block_void 确保不会在栈上留下残余值
            self.emit_block_void(&func.body, &mut wasm_func);
        } else {
            // 函数有显式 return（block_has_return=true）：直接 emit，return 已处理返回值
            self.emit_block(&func.body, &mut wasm_func);
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

    /// 判断 CHIRExpr 是否会在 WASM 栈上留下一个值
    /// 与 emit_expr 的实际行为保持一致，避免错误的 drop/local.set
    fn expr_produces_wasm_value_ctx(&self, expr: &CHIRExpr) -> bool {
        if matches!(expr.ty, Type::Unit | Type::Nothing) {
            return false;
        }
        match &expr.kind {
            // If 表达式：检查 then 分支的 result 是否真正产出值
            CHIRExprKind::If { then_block, .. } => {
                if let Some(result) = &then_block.result {
                    self.expr_produces_wasm_value_ctx(result)
                } else {
                    then_block.stmts.iter().any(|s| matches!(s, CHIRStmt::Return(_)))
                }
            }
            // Block：检查 result 是否真正产出值
            CHIRExprKind::Block(block) => {
                if let Some(result) = &block.result {
                    self.expr_produces_wasm_value_ctx(result)
                } else {
                    false
                }
            }
            // Call：检查被调用函数的实际返回类型
            CHIRExprKind::Call { func_idx, .. } => {
                if let Some(&is_void) = self.func_void_map.get(func_idx) {
                    !is_void
                } else {
                    true
                }
            }
            // Cast：内层表达式产出值时才产出值
            CHIRExprKind::Cast { expr: inner, .. } => {
                matches!(inner.ty, Type::Unit | Type::Nothing) || self.expr_produces_wasm_value_ctx(inner)
            }
            CHIRExprKind::Print { .. } => false,
            _ => true,
        }
    }

    /// 静态版本（不需要 self 的场景，兼容旧调用）
    fn expr_produces_wasm_value(expr: &CHIRExpr) -> bool {
        if matches!(expr.ty, Type::Unit | Type::Nothing) {
            return false;
        }
        match &expr.kind {
            CHIRExprKind::If { then_block, .. } => {
                then_block.result.is_some()
                    || then_block.stmts.iter().any(|s| matches!(s, CHIRStmt::Return(_)))
            }
            CHIRExprKind::Block(block) => block.result.is_some(),
            CHIRExprKind::Print { .. } => false,
            _ => true,
        }
    }

    /// void 上下文的块 emit（Unit If 分支），对产生的 result 主动 Drop
    fn emit_block_void(&self, block: &CHIRBlock, func: &mut wasm_encoder::Function) {
        for stmt in &block.stmts {
            self.emit_stmt(stmt, func);
        }
        if let Some(result) = &block.result {
            self.emit_expr(result, func);
            if self.expr_produces_wasm_value_ctx(result) {
                func.instruction(&Instruction::Drop);
            }
        }
    }

    /// 带期望返回类型的块 emit（用于 If 分支，确保类型一致）
    fn emit_block_with_ty(&self, block: &CHIRBlock, expected_ty: ValType, func: &mut wasm_encoder::Function) {
        for stmt in &block.stmts {
            self.emit_stmt(stmt, func);
        }
        if let Some(result) = &block.result {
            if !self.expr_produces_wasm_value_ctx(result) {
                self.emit_expr(result, func);
                emit_zero(expected_ty, func);
            } else {
                self.emit_expr(result, func);
                if result.wasm_ty != expected_ty {
                    self.emit_cast(result.wasm_ty, expected_ty, func);
                }
            }
        } else {
            emit_zero(expected_ty, func);
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
            CHIRExprKind::String(s) => {
                // 字符串字面量：返回数据段中的内存地址
                let addr = self.string_addresses.get(s).copied().unwrap_or(0);
                func.instruction(&Instruction::I32Const(addr as i32));
            }

            CHIRExprKind::Local(idx) => {
                func.instruction(&Instruction::LocalGet(*idx));
            }
            CHIRExprKind::Global(_name) => {
                // 全局变量简化：读堆指针占位
                func.instruction(&Instruction::GlobalGet(0));
            }

            CHIRExprKind::Binary { op, left, right } => {
                // 确定操作所需的操作数类型：
                // - 算术/位操作：使用结果类型（expr.wasm_ty）
                // - 比较操作：使用操作数类型（left.wasm_ty），结果为 I32 (bool)
                let operand_ty = match op {
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq
                    | BinOp::Gt | BinOp::GtEq => left.wasm_ty,
                    _ => expr.wasm_ty,
                };
                self.emit_expr(left, func);
                // 若操作数为 void（Unit/Nothing），补零值作为默认值
                if !Self::expr_produces_wasm_value(left) {
                    emit_zero(operand_ty, func);
                } else if left.wasm_ty != operand_ty {
                    self.emit_cast(left.wasm_ty, operand_ty, func);
                }
                self.emit_expr(right, func);
                if !Self::expr_produces_wasm_value(right) {
                    emit_zero(operand_ty, func);
                } else if right.wasm_ty != operand_ty {
                    self.emit_cast(right.wasm_ty, operand_ty, func);
                }
                self.emit_binary_op(op, operand_ty, func);
            }
            CHIRExprKind::Unary { op, expr: inner } => {
                self.emit_expr(inner, func);
                if !Self::expr_produces_wasm_value(inner) {
                    // 内层为 void，补零值（对于 Not：!void = !0 = true）
                    emit_zero(ValType::I32, func);
                } else if matches!(op, UnaryOp::Not) && inner.wasm_ty == ValType::I64 {
                    // Not(!): eqz 期望 I32；若内层是 I64，先截断
                    func.instruction(&Instruction::I32WrapI64);
                }
                self.emit_unary_op(op, expr.wasm_ty, func);
            }

            CHIRExprKind::Call { func_idx, args } => {
                for arg in args {
                    self.emit_expr(arg, func);
                    if !self.expr_produces_wasm_value_ctx(arg) {
                        emit_zero(arg.wasm_ty, func);
                    }
                }
                func.instruction(&Instruction::Call(*func_idx));
            }

            CHIRExprKind::MethodCall { func_idx, vtable_offset: _, receiver, args } => {
                self.emit_expr(receiver, func);
                for arg in args {
                    self.emit_expr(arg, func);
                    if !self.expr_produces_wasm_value_ctx(arg) {
                        emit_zero(arg.wasm_ty, func);
                    }
                }
                if let Some(idx) = func_idx {
                    func.instruction(&Instruction::Call(*idx));
                }
            }

            CHIRExprKind::Cast { expr: inner, from_ty, to_ty } => {
                if matches!(inner.ty, Type::Unit | Type::Nothing) {
                    emit_zero(*to_ty, func);
                } else if !self.expr_produces_wasm_value_ctx(inner) {
                    // 内部表达式有副作用但不产生值（如 void Call）
                    self.emit_expr(inner, func);
                    emit_zero(*to_ty, func);
                } else {
                    self.emit_expr(inner, func);
                    if from_ty != to_ty {
                        self.emit_cast(*from_ty, *to_ty, func);
                    }
                }
            }

            CHIRExprKind::If { cond, then_block, else_block } => {
                self.emit_expr(cond, func);
                // 若条件为 void（Unit Call），WASM if 指令需要 i32，补 0（false）
                if !Self::expr_produces_wasm_value(cond) {
                    func.instruction(&Instruction::I32Const(0));
                } else if cond.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                // 仅当 then/else 两侧都会产出值时，才用 Result block type
                let then_produces = then_block.result.is_some()
                    || then_block.stmts.iter().any(|s| matches!(s, CHIRStmt::Return(_)));
                let block_type = if matches!(expr.ty, Type::Unit) || !then_produces {
                    BlockType::Empty
                } else {
                    BlockType::Result(expr.wasm_ty)
                };
                func.instruction(&Instruction::If(block_type));
                // if 指令创建新的 WASM 标签层级，break 深度 +1
                let prev_depth = self.loop_break_depth.get();
                if prev_depth > 0 {
                    self.loop_break_depth.set(prev_depth + 1);
                }
                if matches!(block_type, BlockType::Empty) {
                    // Unit If：分支不得产生值；用 emit_block_void 确保栈干净
                    self.emit_block_void(then_block, func);
                } else {
                    // 有期望类型的 If，用 emit_block_with_ty 确保分支结果类型一致
                    self.emit_block_with_ty(then_block, expr.wasm_ty, func);
                }
                if let Some(else_blk) = else_block {
                    func.instruction(&Instruction::Else);
                    if matches!(block_type, BlockType::Empty) {
                        self.emit_block_void(else_blk, func);
                    } else {
                        self.emit_block_with_ty(else_blk, expr.wasm_ty, func);
                    }
                } else if !matches!(block_type, BlockType::Empty) {
                    // 有返回类型但缺少 else 分支：补零值，保持栈平衡
                    func.instruction(&Instruction::Else);
                    emit_zero(expr.wasm_ty, func);
                }
                func.instruction(&Instruction::End);
                // 退出 if 块，恢复 break 深度
                self.loop_break_depth.set(prev_depth);
            }

            CHIRExprKind::Block(block) => {
                if matches!(expr.ty, Type::Unit | Type::Nothing) {
                    self.emit_block_void(block, func);
                } else {
                    self.emit_block_with_ty(block, expr.wasm_ty, func);
                }
            }

            CHIRExprKind::FieldGet { object, field_offset, .. } => {
                self.emit_expr(object, func);
                // 内存地址计算需要 i32，若 object 是 i64 则截断
                if object.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
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

            CHIRExprKind::Print { arg, newline, fd } => {
                if let Some(arg_expr) = arg {
                    // 若参数是 Nop（如未实现的字符串插值），跳过输出（但仍输出换行）
                    let is_nop = matches!(&arg_expr.kind, CHIRExprKind::Nop);
                    if is_nop {
                        if *newline {
                            if let Some(&idx) = self.func_indices.get(RT_PRINTLN_EMPTY) {
                                func.instruction(&Instruction::Call(idx));
                            }
                        }
                    } else {
                        self.emit_expr(arg_expr, func);
                        // 根据参数类型选择运行时助手函数
                        let rt_name = match (&arg_expr.ty, arg_expr.wasm_ty) {
                            (Type::Bool, _) => {
                                if *newline { RT_PRINTLN_BOOL } else { RT_PRINT_BOOL }
                            }
                            (Type::String, _) => {
                                if *newline { RT_PRINTLN_STR } else { RT_PRINT_STR }
                            }
                            (_, ValType::I64) => {
                                if *newline { RT_PRINTLN_I64 } else { RT_PRINT_I64 }
                            }
                            (_, ValType::F64) => {
                                // Float64：暂时用 i64 版本（截断取整后打印）
                                func.instruction(&Instruction::I64TruncF64S);
                                if *newline { RT_PRINTLN_I64 } else { RT_PRINT_I64 }
                            }
                            (_, ValType::F32) => {
                                // Float32：先提升到 f64，再截断到 i64
                                func.instruction(&Instruction::F64PromoteF32);
                                func.instruction(&Instruction::I64TruncF64S);
                                if *newline { RT_PRINTLN_I64 } else { RT_PRINT_I64 }
                            }
                            _ => {
                                // I32 整型：扩展到 I64 后使用 i64 版本
                                func.instruction(&Instruction::I64ExtendI32S);
                                if *newline { RT_PRINTLN_I64 } else { RT_PRINT_I64 }
                            }
                        };
                        if let Some(&idx) = self.func_indices.get(rt_name) {
                            func.instruction(&Instruction::Call(idx));
                        }
                    }
                } else if *newline {
                    // 空 println：仅输出换行符
                    if let Some(&idx) = self.func_indices.get(RT_PRINTLN_EMPTY) {
                        func.instruction(&Instruction::Call(idx));
                    }
                }
                // Print 是 Unit 类型，不产生 WASM 栈值
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
                // 若 value 为 void 表达式（Unit Call 等），补零值保证 local.set 有操作数
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                func.instruction(&Instruction::LocalSet(*local_idx));
            }

            CHIRStmt::Assign { target, value } => {
                self.emit_assign(target, value, func);
            }

            CHIRStmt::Expr(expr) => {
                self.emit_expr(expr, func);
                if self.expr_produces_wasm_value_ctx(expr) {
                    func.instruction(&Instruction::Drop);
                }
            }

            CHIRStmt::Return(expr_opt) => {
                if let Some(expr) = expr_opt {
                    self.emit_expr(expr, func);
                }
                func.instruction(&Instruction::Return);
            }

            CHIRStmt::Break => {
                // 使用当前 break 深度：直接在 loop body 内为 1，每嵌套一层 if/block +1
                let depth = self.loop_break_depth.get();
                func.instruction(&Instruction::Br(if depth > 0 { depth } else { 1 }));
            }
            CHIRStmt::Continue => { func.instruction(&Instruction::Br(0)); }

            CHIRStmt::While { cond, body } => {
                // block { loop { break_if(!cond); body; br 0 } }
                // break 直接在 loop body 内时深度为 1（退出 block）
                let prev_depth = self.loop_break_depth.get();
                self.loop_break_depth.set(1);
                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));
                self.emit_expr(cond, func);
                // 条件可能是 void（Unit Call），需补零值（false）避免 i32.eqz 空栈错误
                if !Self::expr_produces_wasm_value(cond) {
                    func.instruction(&Instruction::I32Const(0));
                } else if cond.wasm_ty == ValType::I64 {
                    // 条件可能是 I64，需先截断到 I32，再用 I32Eqz 实现 `!cond`
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));
                self.emit_block(body, func);
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // loop
                func.instruction(&Instruction::End); // block
                self.loop_break_depth.set(prev_depth);
            }

            CHIRStmt::Loop { body } => {
                let prev_depth = self.loop_break_depth.get();
                self.loop_break_depth.set(1);
                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));
                self.emit_block(body, func);
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // loop
                func.instruction(&Instruction::End); // block
                self.loop_break_depth.set(prev_depth);
            }
        }
    }

    fn emit_assign(&self, target: &CHIRLValue, value: &CHIRExpr, func: &mut wasm_encoder::Function) {
        match target {
            CHIRLValue::Local(idx) => {
                self.emit_expr(value, func);
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                func.instruction(&Instruction::LocalSet(*idx));
            }
            CHIRLValue::Field { object, offset } => {
                self.emit_expr(object, func);
                if object.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
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
    // 遍历 block.result，其中的嵌套 Block 可能含有 Let 语句
    if let Some(result) = &block.result {
        collect_locals_from_expr(result, param_count, out);
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
        CHIRExprKind::Print { arg, .. } => {
            if let Some(a) = arg {
                collect_locals_from_expr(a, param_count, out);
            }
        }
        _ => {}
    }
}

/// 将 (idx, wasm_ty) 列表压缩为 wasm_encoder 的 run-length 格式 (count, ValType)
/// 处理索引空洞：在不连续的索引间插入 I32 占位，保证 WASM locals 声明与实际使用的索引一致
/// param_count：函数参数数量，用于计算首个 local 之前的初始间隙
fn run_length_encode_locals(locals: &[(u32, ValType)], param_count: u32) -> Vec<(u32, ValType)> {
    if locals.is_empty() {
        return vec![];
    }
    let mut result: Vec<(u32, ValType)> = Vec::new();
    let mut prev_idx: Option<u32> = None;

    for &(idx, ty) in locals {
        // 计算与上一个索引之间的空洞大小
        // 首次：从 param_count 到首个 local 索引之间可能存在空洞（双重 lower 等情况）
        let gap = match prev_idx {
            Some(p) => idx.saturating_sub(p + 1),
            None => idx.saturating_sub(param_count),
        };
        // 用 I32 填充空洞，保证索引连续
        if gap > 0 {
            if let Some(last) = result.last_mut() {
                if last.1 == ValType::I32 {
                    last.0 += gap;
                } else {
                    result.push((gap, ValType::I32));
                }
            } else {
                result.push((gap, ValType::I32));
            }
        }
        // 追加当前局部变量（尝试合并相邻同类型）
        if let Some(last) = result.last_mut() {
            if last.1 == ty {
                last.0 += 1;
                prev_idx = Some(idx);
                continue;
            }
        }
        result.push((1, ty));
        prev_idx = Some(idx);
    }
    result
}
