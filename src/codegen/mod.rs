use crate::ast::{AssignTarget, BinOp, Expr, Literal, Pattern, Program, Stmt, StructDef, Type};
use crate::ast::Function as FuncDef;
use std::collections::HashMap;
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function as WasmFunc,
    FunctionSection, GlobalSection, GlobalType, Instruction, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};

/// 内存布局常量
const HEAP_BASE: i32 = 1024;  // 堆起始地址
const PAGE_SIZE: u64 = 65536; // WASM 页大小 64KB

/// 代码生成器
pub struct CodeGen {
    /// 函数类型索引映射
    func_types: HashMap<String, u32>,
    /// 函数索引映射
    func_indices: HashMap<String, u32>,
    /// 结构体定义
    structs: HashMap<String, StructDef>,
    /// 字符串常量池 (字符串内容 -> 内存偏移)
    string_pool: Vec<(String, u32)>,
    /// 当前数据段偏移
    data_offset: u32,
}

impl CodeGen {
    pub fn new() -> Self {
        Self {
            func_types: HashMap::new(),
            func_indices: HashMap::new(),
            structs: HashMap::new(),
            string_pool: Vec::new(),
            data_offset: 0,
        }
    }

    /// 编译程序生成 WASM 模块
    pub fn compile(&mut self, program: &Program) -> Vec<u8> {
        let mut module = Module::new();

        // 收集结构体定义
        for s in &program.structs {
            self.structs.insert(s.name.clone(), s.clone());
        }

        // 收集字符串常量
        self.collect_strings(program);

        // 1. 类型段 (Type Section)
        let mut types = TypeSection::new();
        for (i, func) in program.functions.iter().enumerate() {
            let params: Vec<ValType> = func.params.iter().map(|p| p.ty.to_wasm()).collect();
            let results: Vec<ValType> = func
                .return_type
                .as_ref()
                .map(|t| vec![t.to_wasm()])
                .unwrap_or_default();
            types.ty().function(params, results);
            self.func_types.insert(func.name.clone(), i as u32);
            self.func_indices.insert(func.name.clone(), i as u32);
        }
        module.section(&types);

        // 2. 函数段 (Function Section)
        let mut functions = FunctionSection::new();
        for i in 0..program.functions.len() {
            functions.function(i as u32);
        }
        module.section(&functions);

        // 3. 内存段 (Memory Section)
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: Some(16),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);

        // 4. 全局变量段 (Global Section) - 堆指针
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(HEAP_BASE + self.data_offset as i32),
        );
        module.section(&globals);

        // 5. 导出段 (Export Section)
        let mut exports = ExportSection::new();
        for (i, func) in program.functions.iter().enumerate() {
            exports.export(&func.name, ExportKind::Func, i as u32);
        }
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);

        // 6. 代码段 (Code Section)
        let mut codes = CodeSection::new();
        for func in &program.functions {
            let wasm_func = self.compile_function(func);
            codes.function(&wasm_func);
        }
        module.section(&codes);

        // 7. 数据段 (Data Section) - 字符串常量
        if !self.string_pool.is_empty() {
            let mut data = DataSection::new();
            for (s, offset) in &self.string_pool {
                // 存储格式: [length: i32][bytes...]
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(s.len() as i32).to_le_bytes());
                bytes.extend_from_slice(s.as_bytes());
                data.active(0, &ConstExpr::i32_const(*offset as i32), bytes);
            }
            module.section(&data);
        }

        module.finish()
    }

    /// 收集所有字符串常量
    fn collect_strings(&mut self, program: &Program) {
        for func in &program.functions {
            for stmt in &func.body {
                self.collect_strings_in_stmt(stmt);
            }
        }
    }

    fn collect_strings_in_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
                self.collect_strings_in_expr(value);
            }
            Stmt::Assign { value, .. } => {
                self.collect_strings_in_expr(value);
            }
            Stmt::Expr(expr) | Stmt::Return(Some(expr)) => {
                self.collect_strings_in_expr(expr);
            }
            Stmt::While { cond, body } => {
                self.collect_strings_in_expr(cond);
                for s in body {
                    self.collect_strings_in_stmt(s);
                }
            }
            _ => {}
        }
    }

    fn collect_strings_in_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::String(s) => {
                if !self.string_pool.iter().any(|(str, _)| str == s) {
                    let offset = self.data_offset;
                    self.data_offset += 4 + s.len() as u32; // length + bytes
                    self.string_pool.push((s.clone(), offset));
                }
            }
            Expr::Binary { left, right, .. } => {
                self.collect_strings_in_expr(left);
                self.collect_strings_in_expr(right);
            }
            Expr::Call { args, .. } | Expr::MethodCall { args, .. } => {
                for arg in args {
                    self.collect_strings_in_expr(arg);
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_strings_in_expr(cond);
                self.collect_strings_in_expr(then_branch);
                if let Some(e) = else_branch {
                    self.collect_strings_in_expr(e);
                }
            }
            Expr::Array(elements) => {
                for e in elements {
                    self.collect_strings_in_expr(e);
                }
            }
            Expr::Index { array, index } => {
                self.collect_strings_in_expr(array);
                self.collect_strings_in_expr(index);
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.collect_strings_in_expr(e);
                }
            }
            Expr::Field { object, .. } => {
                self.collect_strings_in_expr(object);
            }
            _ => {}
        }
    }

    /// 编译函数
    fn compile_function(&self, func: &FuncDef) -> WasmFunc {
        let mut locals = LocalsBuilder::new();

        // 添加参数作为局部变量
        for param in &func.params {
            locals.add(&param.name, param.ty.to_wasm());
        }

        // 收集函数体中的局部变量
        for stmt in &func.body {
            self.collect_locals(stmt, &mut locals);
        }

        // 创建 WASM 函数
        let local_types: Vec<(u32, ValType)> = locals
            .types
            .iter()
            .skip(func.params.len())
            .map(|t| (1, *t))
            .collect();

        let mut wasm_func = WasmFunc::new(local_types);

        // 编译函数体
        for stmt in &func.body {
            self.compile_stmt(stmt, &locals, &mut wasm_func);
        }

        wasm_func.instruction(&Instruction::End);
        wasm_func
    }

    /// 收集局部变量
    fn collect_locals(&self, stmt: &Stmt, locals: &mut LocalsBuilder) {
        match stmt {
            Stmt::Let { name, ty, value } | Stmt::Var { name, ty, value } => {
                let val_type = ty
                    .as_ref()
                    .map(|t| t.to_wasm())
                    .unwrap_or_else(|| self.infer_type(value));
                locals.add(name, val_type);
            }
            Stmt::While { body, .. } => {
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::For { var, iterable, body } => {
                locals.add(var, ValType::I64); // 循环变量默认 i64
                // 如果是数组迭代，需要隐藏的索引变量
                if !matches!(iterable, Expr::Range { .. }) {
                    locals.add(&format!("__{}_idx", var), ValType::I64);
                    locals.add(&format!("__{}_len", var), ValType::I64);
                    locals.add(&format!("__{}_arr", var), ValType::I32);
                }
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            _ => {}
        }
    }

    /// 简单的类型推断
    fn infer_type(&self, expr: &Expr) -> ValType {
        match expr {
            Expr::Integer(_) => ValType::I64,
            Expr::Float(_) => ValType::F64,
            Expr::Bool(_) => ValType::I32,
            Expr::String(_) => ValType::I32,
            Expr::Array(_) => ValType::I32,
            Expr::StructInit { .. } => ValType::I32,
            Expr::Binary { left, .. } => self.infer_type(left),
            Expr::Index { .. } => ValType::I64, // 默认数组元素类型
            Expr::Field { .. } => ValType::I64, // 默认字段类型
            _ => ValType::I64,
        }
    }

    /// 编译语句
    fn compile_stmt(&self, stmt: &Stmt, locals: &LocalsBuilder, func: &mut WasmFunc) {
        match stmt {
            Stmt::Let { name, value, .. } | Stmt::Var { name, value, .. } => {
                self.compile_expr(value, locals, func);
                let idx = locals.get(name).expect("局部变量未找到");
                func.instruction(&Instruction::LocalSet(idx));
            }
            Stmt::Assign { target, value } => {
                match target {
                    AssignTarget::Var(name) => {
                        self.compile_expr(value, locals, func);
                        let idx = locals.get(name).expect("变量未找到");
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                    AssignTarget::Index { array, index } => {
                        // arr[i] = value
                        // 计算地址: arr + i * 8
                        let arr_idx = locals.get(array).expect("数组未找到");
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        self.compile_expr(index, locals, func);
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(8)); // 元素大小
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        // 存储值
                        self.compile_expr(value, locals, func);
                        func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                    AssignTarget::Field { object, field } => {
                        // obj.field = value
                        let obj_idx = locals.get(object).expect("对象未找到");
                        func.instruction(&Instruction::LocalGet(obj_idx));
                        // TODO: 需要类型信息来计算偏移
                        func.instruction(&Instruction::I32Const(0)); // 临时偏移
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func);
                        func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                        let _ = field; // 抑制警告
                    }
                }
            }
            Stmt::Return(Some(expr)) => {
                self.compile_expr(expr, locals, func);
                func.instruction(&Instruction::Return);
            }
            Stmt::Return(None) => {
                func.instruction(&Instruction::Return);
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr, locals, func);
                func.instruction(&Instruction::Drop);
            }
            Stmt::While { cond, body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                self.compile_expr(cond, locals, func);
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));

                for s in body {
                    self.compile_stmt(s, locals, func);
                }

                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::For { var, iterable, body } => {
                // for i in 0..10 { ... } 编译为:
                // let i = start
                // while i < end { ...; i = i + 1 }
                let var_idx = locals.get(var).expect("循环变量未找到");

                match iterable {
                    Expr::Range { start, end, inclusive } => {
                        // 初始化循环变量
                        self.compile_expr(start, locals, func);
                        func.instruction(&Instruction::LocalSet(var_idx));

                        // block { loop { ... } }
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件检查: i < end (或 i <= end)
                        func.instruction(&Instruction::LocalGet(var_idx));
                        self.compile_expr(end, locals, func);
                        if *inclusive {
                            func.instruction(&Instruction::I64GtS); // i > end
                        } else {
                            func.instruction(&Instruction::I64GeS); // i >= end
                        }
                        func.instruction(&Instruction::BrIf(1)); // 退出

                        // 循环体
                        for s in body {
                            self.compile_stmt(s, locals, func);
                        }

                        // 递增循环变量
                        func.instruction(&Instruction::LocalGet(var_idx));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(var_idx));

                        func.instruction(&Instruction::Br(0)); // 继续循环
                        func.instruction(&Instruction::End); // loop end
                        func.instruction(&Instruction::End); // block end
                    }
                    _ => {
                        // 数组迭代: for item in arr { ... }
                        // 编译为:
                        //   let __arr = arr
                        //   let __len = arr[0]  (数组长度在偏移0)
                        //   let __idx = 0
                        //   while __idx < __len {
                        //     let item = arr[4 + __idx * 8]
                        //     ...
                        //     __idx += 1
                        //   }

                        let idx_var = format!("__{}_idx", var);
                        let len_var = format!("__{}_len", var);
                        let arr_var = format!("__{}_arr", var);

                        let idx_idx = locals.get(&idx_var).expect("索引变量未找到");
                        let len_idx = locals.get(&len_var).expect("长度变量未找到");
                        let arr_idx = locals.get(&arr_var).expect("数组变量未找到");

                        // 计算数组地址并保存
                        self.compile_expr(iterable, locals, func);
                        func.instruction(&Instruction::LocalSet(arr_idx));

                        // 获取数组长度 (在偏移0处)
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I64ExtendI32S);
                        func.instruction(&Instruction::LocalSet(len_idx));

                        // 初始化索引为 0
                        func.instruction(&Instruction::I64Const(0));
                        func.instruction(&Instruction::LocalSet(idx_idx));

                        // block { loop { ... } }
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件检查: __idx >= __len 则退出
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::LocalGet(len_idx));
                        func.instruction(&Instruction::I64GeS);
                        func.instruction(&Instruction::BrIf(1));

                        // 获取当前元素: arr[4 + idx * 8]
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(8)); // 元素大小
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::LocalSet(var_idx));

                        // 循环体
                        for s in body {
                            self.compile_stmt(s, locals, func);
                        }

                        // 递增索引
                        func.instruction(&Instruction::LocalGet(idx_idx));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(idx_idx));

                        func.instruction(&Instruction::Br(0)); // 继续循环
                        func.instruction(&Instruction::End); // loop end
                        func.instruction(&Instruction::End); // block end
                    }
                }
            }
        }
    }

    /// 编译表达式
    fn compile_expr(&self, expr: &Expr, locals: &LocalsBuilder, func: &mut WasmFunc) {
        match expr {
            Expr::Integer(n) => {
                func.instruction(&Instruction::I64Const(*n));
            }
            Expr::Float(f) => {
                func.instruction(&Instruction::F64Const(*f));
            }
            Expr::Bool(b) => {
                func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            }
            Expr::String(s) => {
                // 返回字符串在数据段中的地址
                let offset = self
                    .string_pool
                    .iter()
                    .find(|(str, _)| str == s)
                    .map(|(_, off)| *off)
                    .unwrap_or(0);
                func.instruction(&Instruction::I32Const(offset as i32));
            }
            Expr::Var(name) => {
                let idx = locals.get(name).expect("变量未找到");
                func.instruction(&Instruction::LocalGet(idx));
            }
            Expr::Binary { op, left, right } => {
                self.compile_expr(left, locals, func);
                self.compile_expr(right, locals, func);

                let val_type = self.infer_type(left);
                let instr = match (op, val_type) {
                    (BinOp::Add, ValType::I64) => Instruction::I64Add,
                    (BinOp::Sub, ValType::I64) => Instruction::I64Sub,
                    (BinOp::Mul, ValType::I64) => Instruction::I64Mul,
                    (BinOp::Div, ValType::I64) => Instruction::I64DivS,
                    (BinOp::Mod, ValType::I64) => Instruction::I64RemS,
                    (BinOp::Lt, ValType::I64) => Instruction::I64LtS,
                    (BinOp::Gt, ValType::I64) => Instruction::I64GtS,
                    (BinOp::LtEq, ValType::I64) => Instruction::I64LeS,
                    (BinOp::GtEq, ValType::I64) => Instruction::I64GeS,
                    (BinOp::Eq, ValType::I64) => Instruction::I64Eq,
                    (BinOp::NotEq, ValType::I64) => Instruction::I64Ne,

                    (BinOp::Add, ValType::I32) => Instruction::I32Add,
                    (BinOp::Sub, ValType::I32) => Instruction::I32Sub,
                    (BinOp::Mul, ValType::I32) => Instruction::I32Mul,
                    (BinOp::Div, ValType::I32) => Instruction::I32DivS,
                    (BinOp::Mod, ValType::I32) => Instruction::I32RemS,
                    (BinOp::Lt, ValType::I32) => Instruction::I32LtS,
                    (BinOp::Gt, ValType::I32) => Instruction::I32GtS,
                    (BinOp::LtEq, ValType::I32) => Instruction::I32LeS,
                    (BinOp::GtEq, ValType::I32) => Instruction::I32GeS,
                    (BinOp::Eq, ValType::I32) => Instruction::I32Eq,
                    (BinOp::NotEq, ValType::I32) => Instruction::I32Ne,

                    (BinOp::Add, ValType::F64) => Instruction::F64Add,
                    (BinOp::Sub, ValType::F64) => Instruction::F64Sub,
                    (BinOp::Mul, ValType::F64) => Instruction::F64Mul,
                    (BinOp::Div, ValType::F64) => Instruction::F64Div,
                    (BinOp::Lt, ValType::F64) => Instruction::F64Lt,
                    (BinOp::Gt, ValType::F64) => Instruction::F64Gt,
                    (BinOp::LtEq, ValType::F64) => Instruction::F64Le,
                    (BinOp::GtEq, ValType::F64) => Instruction::F64Ge,
                    (BinOp::Eq, ValType::F64) => Instruction::F64Eq,
                    (BinOp::NotEq, ValType::F64) => Instruction::F64Ne,

                    _ => panic!("不支持的运算: {:?} for {:?}", op, val_type),
                };
                func.instruction(&instr);
            }
            Expr::Call { name, args } => {
                for arg in args {
                    self.compile_expr(arg, locals, func);
                }
                let idx = *self.func_indices.get(name).expect("函数未找到");
                func.instruction(&Instruction::Call(idx));
            }
            Expr::MethodCall { object, method, args } => {
                // 简单实现：将方法调用转换为普通函数调用
                self.compile_expr(object, locals, func);
                for arg in args {
                    self.compile_expr(arg, locals, func);
                }
                // 尝试找到方法对应的函数
                if let Some(&idx) = self.func_indices.get(method) {
                    func.instruction(&Instruction::Call(idx));
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.compile_expr(cond, locals, func);
                func.instruction(&Instruction::I32WrapI64);

                let result_type = wasm_encoder::BlockType::Result(self.infer_type(then_branch));
                func.instruction(&Instruction::If(result_type));
                self.compile_expr(then_branch, locals, func);

                if let Some(else_expr) = else_branch {
                    func.instruction(&Instruction::Else);
                    self.compile_expr(else_expr, locals, func);
                }

                func.instruction(&Instruction::End);
            }
            Expr::Array(elements) => {
                // 分配内存: global[0] 是堆指针
                let elem_size = 8; // i64 大小
                let total_size = 4 + elements.len() as i32 * elem_size; // length + elements

                // 获取当前堆指针
                func.instruction(&Instruction::GlobalGet(0));

                // 保存数组起始地址到栈上
                func.instruction(&Instruction::GlobalGet(0));

                // 写入数组长度
                func.instruction(&Instruction::I32Const(elements.len() as i32));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 写入每个元素
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::GlobalGet(0));
                    func.instruction(&Instruction::I32Const(4 + i as i32 * elem_size));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(elem, locals, func);
                    func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                }

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));

                // 栈上已经有数组起始地址了
            }
            Expr::Index { array, index } => {
                // arr[i] -> load from (arr + 4 + i * 8)
                self.compile_expr(array, locals, func);
                func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                func.instruction(&Instruction::I32Add);
                self.compile_expr(index, locals, func);
                func.instruction(&Instruction::I32WrapI64);
                func.instruction(&Instruction::I32Const(8));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Expr::StructInit { name, fields } => {
                let struct_def = self.structs.get(name).expect("结构体未定义");
                let struct_size = struct_def.size();

                // 获取当前堆指针作为结构体地址
                func.instruction(&Instruction::GlobalGet(0));

                // 写入每个字段
                for (field_name, value) in fields {
                    let offset = struct_def
                        .field_offset(field_name)
                        .expect("字段未定义");

                    func.instruction(&Instruction::GlobalGet(0));
                    func.instruction(&Instruction::I32Const(offset as i32));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(value, locals, func);
                    func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                }

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(struct_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));

                // 返回结构体地址 (已在栈上)
            }
            Expr::Field { object, field } => {
                self.compile_expr(object, locals, func);
                // TODO: 需要类型信息来计算偏移
                // 临时实现：假设第一个字段
                func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                let _ = field;
            }
            Expr::Block(stmts, result) => {
                for stmt in stmts {
                    self.compile_stmt(stmt, locals, func);
                }
                if let Some(expr) = result {
                    self.compile_expr(expr, locals, func);
                }
            }
            Expr::Range { start, end, inclusive } => {
                // Range 作为值时，返回一个包含 (start, end, inclusive) 的结构
                // 简化实现：只返回 start (用于 for 循环的初始化)
                self.compile_expr(start, locals, func);
                let _ = (end, inclusive);
            }
            Expr::Match { expr, arms } => {
                // match expr {
                //     pattern1 if guard1 => body1,
                //     pattern2 => body2,
                //     _ => default
                // }

                self.compile_expr(expr, locals, func);

                let result_type = if arms.is_empty() {
                    wasm_encoder::BlockType::Empty
                } else {
                    wasm_encoder::BlockType::Result(self.infer_type(&arms[0].body))
                };

                func.instruction(&Instruction::Block(result_type));

                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i == arms.len() - 1;
                    let has_guard = arm.guard.is_some();

                    match &arm.pattern {
                        Pattern::Wildcard => {
                            func.instruction(&Instruction::Drop);
                            if has_guard {
                                // _ if cond => body
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func);
                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                self.compile_expr(&arm.body, locals, func);
                                func.instruction(&Instruction::Br(0));
                            }
                        }
                        Pattern::Literal(lit) => {
                            match lit {
                                Literal::Integer(n) => {
                                    func.instruction(&Instruction::I64Const(*n));
                                    func.instruction(&Instruction::I64Eq);
                                }
                                Literal::Bool(b) => {
                                    func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
                                    func.instruction(&Instruction::I32Eq);
                                }
                                _ => {}
                            }

                            // 如果有 guard，需要额外检查
                            if has_guard {
                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func);
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(result_type));
                            self.compile_expr(&arm.body, locals, func);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::Else);
                            if is_last {
                                func.instruction(&Instruction::I64Const(0));
                            } else {
                                self.compile_expr(expr, locals, func);
                            }
                            func.instruction(&Instruction::End);
                        }
                        Pattern::Binding(name) => {
                            if let Some(idx) = locals.get(name) {
                                func.instruction(&Instruction::LocalSet(idx));
                            }
                            if has_guard {
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func);
                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                self.compile_expr(&arm.body, locals, func);
                                func.instruction(&Instruction::Br(0));
                            }
                        }
                        Pattern::Range { start, end, inclusive } => {
                            if let (Literal::Integer(s), Literal::Integer(e)) = (start, end) {
                                func.instruction(&Instruction::I64Const(*s));
                                func.instruction(&Instruction::I64GeS);

                                self.compile_expr(expr, locals, func);
                                func.instruction(&Instruction::I64Const(*e));
                                if *inclusive {
                                    func.instruction(&Instruction::I64LeS);
                                } else {
                                    func.instruction(&Instruction::I64LtS);
                                }

                                func.instruction(&Instruction::I32And);

                                if has_guard {
                                    func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                                    self.compile_expr(arm.guard.as_ref().unwrap(), locals, func);
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0));
                                    func.instruction(&Instruction::End);
                                }

                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func);
                                }
                                func.instruction(&Instruction::End);
                            }
                        }
                        Pattern::Or(patterns) => {
                            for (j, pat) in patterns.iter().enumerate() {
                                if let Pattern::Literal(Literal::Integer(n)) = pat {
                                    if j > 0 {
                                        self.compile_expr(expr, locals, func);
                                    }
                                    func.instruction(&Instruction::I64Const(*n));
                                    func.instruction(&Instruction::I64Eq);
                                    if j > 0 {
                                        func.instruction(&Instruction::I32Or);
                                    }
                                }
                            }

                            if has_guard {
                                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func);
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(result_type));
                            self.compile_expr(&arm.body, locals, func);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::Else);
                            if is_last {
                                func.instruction(&Instruction::I64Const(0));
                            } else {
                                self.compile_expr(expr, locals, func);
                            }
                            func.instruction(&Instruction::End);
                        }
                        _ => {
                            func.instruction(&Instruction::Drop);
                            self.compile_expr(&arm.body, locals, func);
                        }
                    }
                }

                func.instruction(&Instruction::End);
            }
        }
    }
}

/// 局部变量构建器
struct LocalsBuilder {
    names: HashMap<String, u32>,
    types: Vec<ValType>,
}

impl LocalsBuilder {
    fn new() -> Self {
        Self {
            names: HashMap::new(),
            types: Vec::new(),
        }
    }

    fn add(&mut self, name: &str, ty: ValType) {
        if !self.names.contains_key(name) {
            let idx = self.types.len() as u32;
            self.names.insert(name.to_string(), idx);
            self.types.push(ty);
        }
    }

    fn get(&self, name: &str) -> Option<u32> {
        self.names.get(name).copied()
    }
}

impl Default for CodeGen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FieldDef, Param};

    #[test]
    fn test_compile_simple_function() {
        let program = Program {
            structs: vec![],
            functions: vec![FuncDef {
                name: "answer".to_string(),
                params: vec![],
                return_type: Some(Type::Int64),
                body: vec![Stmt::Return(Some(Expr::Integer(42)))],
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_compile_struct() {
        let program = Program {
            structs: vec![StructDef {
                name: "Point".to_string(),
                fields: vec![
                    FieldDef {
                        name: "x".to_string(),
                        ty: Type::Int64,
                    },
                    FieldDef {
                        name: "y".to_string(),
                        ty: Type::Int64,
                    },
                ],
            }],
            functions: vec![FuncDef {
                name: "test".to_string(),
                params: vec![],
                return_type: Some(Type::Int32),
                body: vec![
                    Stmt::Let {
                        name: "p".to_string(),
                        ty: Some(Type::Struct("Point".to_string())),
                        value: Expr::StructInit {
                            name: "Point".to_string(),
                            fields: vec![
                                ("x".to_string(), Expr::Integer(10)),
                                ("y".to_string(), Expr::Integer(20)),
                            ],
                        },
                    },
                    Stmt::Return(Some(Expr::Field {
                        object: Box::new(Expr::Var("p".to_string())),
                        field: "x".to_string(),
                    })),
                ],
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }
}
