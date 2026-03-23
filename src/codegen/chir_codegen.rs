//! CHIR → WASM 代码生成

use crate::ast::{BinOp, Type, UnaryOp};
use crate::chir::{
    CHIRBlock, CHIRExpr, CHIRExprKind, CHIRFunction, CHIRLValue, CHIRProgram, CHIRStmt,
};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection, MemoryType,
    Module, TypeSection, ValType,
};

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
#[allow(dead_code)]
const IDX_CLOCK_TIME_GET: u32 = 2;
#[allow(dead_code)]
const IDX_RANDOM_GET: u32 = 3;
const IMPORT_COUNT: u32 = 4;

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

const RT_PRINTLN_I64: &str = "__rt_println_i64";
const RT_PRINT_I64: &str = "__rt_print_i64";
const RT_PRINTLN_STR: &str = "__rt_println_str";
const RT_PRINT_STR: &str = "__rt_print_str";
const RT_PRINTLN_BOOL: &str = "__rt_println_bool";
const RT_PRINT_BOOL: &str = "__rt_print_bool";
const RT_PRINTLN_EMPTY: &str = "__rt_println_empty";
const RT_ALLOC: &str = "__alloc";
const RT_MATH_SIN: &str = "sin";
const RT_MATH_COS: &str = "cos";
const RT_MATH_TAN: &str = "tan";
const RT_MATH_EXP: &str = "exp";
const RT_MATH_LOG: &str = "log";
const RT_MATH_POW: &str = "pow";
const RT_I64_TO_STR: &str = "__i64_to_str";
const RT_BOOL_TO_STR: &str = "__bool_to_str";
const RT_STR_TO_I64: &str = "__str_to_i64";
const RT_STR_CONCAT: &str = "__str_concat";
const RT_F64_TO_STR: &str = "__f64_to_str";
const RT_NOW: &str = "now";
const RT_RANDOM_INT64: &str = "randomInt64";
const RT_RANDOM_FLOAT64: &str = "randomFloat64";
const RT_STR_CONTAINS: &str = "__str_contains";
const RT_STR_STARTS_WITH: &str = "__str_starts_with";
const RT_STR_ENDS_WITH: &str = "__str_ends_with";
const RT_STR_TRIM: &str = "__str_trim";
const RT_STR_TO_ARRAY: &str = "__str_to_array";
const RT_STR_INDEX_OF: &str = "__str_index_of";
const RT_STR_REPLACE: &str = "__str_replace";
const RT_POW_I64: &str = "__pow_i64";
const RT_POW_F64: &str = "__pow_f64";
// Collection runtime functions
const RT_ARRAYLIST_NEW: &str = "__arraylist_new";
const RT_ARRAYLIST_APPEND: &str = "__arraylist_append";
const RT_ARRAYLIST_GET: &str = "__arraylist_get";
const RT_ARRAYLIST_SET: &str = "__arraylist_set";
const RT_ARRAYLIST_REMOVE: &str = "__arraylist_remove";
const RT_ARRAYLIST_SIZE: &str = "__arraylist_size";
const RT_HASHMAP_NEW: &str = "__hashmap_new";
const RT_HASHMAP_PUT: &str = "__hashmap_put";
const RT_HASHMAP_GET: &str = "__hashmap_get";
const RT_HASHMAP_CONTAINS: &str = "__hashmap_contains";
const RT_HASHMAP_REMOVE: &str = "__hashmap_remove";
const RT_HASHMAP_SIZE: &str = "__hashmap_size";
const RT_HASHSET_NEW: &str = "__hashset_new";
const RT_HASHSET_ADD: &str = "__hashset_add";
const RT_HASHSET_CONTAINS: &str = "__hashset_contains";
const RT_HASHSET_SIZE: &str = "__hashset_size";
/// WASI scratch area for clock_time_get / random_get
const WASI_SCRATCH: i32 = 80;
const RT_NAMES: &[&str] = &[
    RT_PRINTLN_I64,
    RT_PRINT_I64,
    RT_PRINTLN_STR,
    RT_PRINT_STR,
    RT_PRINTLN_BOOL,
    RT_PRINT_BOOL,
    RT_PRINTLN_EMPTY,
    RT_ALLOC,
    RT_MATH_SIN,
    RT_MATH_COS,
    RT_MATH_TAN,
    RT_MATH_EXP,
    RT_MATH_LOG,
    RT_MATH_POW,
    RT_I64_TO_STR,
    RT_BOOL_TO_STR,
    RT_STR_TO_I64,
    RT_STR_CONCAT,
    RT_F64_TO_STR,
    RT_NOW,
    RT_RANDOM_INT64,
    RT_RANDOM_FLOAT64,
    RT_STR_CONTAINS,
    RT_STR_STARTS_WITH,
    RT_STR_ENDS_WITH,
    RT_STR_TRIM,
    RT_STR_TO_ARRAY,
    RT_STR_INDEX_OF,
    RT_STR_REPLACE,
    // Collections
    RT_ARRAYLIST_NEW,
    RT_ARRAYLIST_APPEND,
    RT_ARRAYLIST_GET,
    RT_ARRAYLIST_SET,
    RT_ARRAYLIST_REMOVE,
    RT_ARRAYLIST_SIZE,
    RT_HASHMAP_NEW,
    RT_HASHMAP_PUT,
    RT_HASHMAP_GET,
    RT_HASHMAP_CONTAINS,
    RT_HASHMAP_REMOVE,
    RT_HASHMAP_SIZE,
    RT_HASHSET_NEW,
    RT_HASHSET_ADD,
    RT_HASHSET_CONTAINS,
    RT_HASHSET_SIZE,
    RT_POW_I64,
    RT_POW_F64,
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
    /// 结构体字段偏移表
    struct_field_offsets: HashMap<String, HashMap<String, (u32, Type)>>,
    /// 类字段偏移表
    class_field_offsets: HashMap<String, HashMap<String, (u32, Type)>>,
    /// 类对象大小（user data，不含 alloc header）
    class_object_sizes: HashMap<String, u32>,
    /// 类是否需要 vtable
    class_has_vtable: HashMap<String, bool>,
    /// 函数类型签名 → 类型索引缓存（用于 call_indirect）
    func_type_by_sig: std::cell::RefCell<HashMap<(Vec<ValType>, Vec<ValType>), u32>>,
    /// 当前函数的 local 索引 → 声明 WASM 类型（含参数和局部变量）
    current_local_types: std::cell::RefCell<HashMap<u32, ValType>>,
    /// 函数索引 → 参数 WASM 类型列表（用于 call 指令前的参数类型修正）
    func_param_types: HashMap<u32, Vec<ValType>>,
}

impl CHIRCodeGen {
    fn cond_needs_i32_wrap(cond: &CHIRExpr) -> bool {
        cond.wasm_ty == ValType::I64 && !matches!(cond.ty, Type::Bool)
    }

    pub fn new() -> Self {
        CHIRCodeGen {
            func_indices: HashMap::new(),
            func_void_map: HashMap::new(),
            loop_break_depth: std::cell::Cell::new(0),
            string_data: Vec::new(),
            string_addresses: HashMap::new(),
            data_offset: DATA_SECTION_BASE,
            struct_field_offsets: HashMap::new(),
            class_field_offsets: HashMap::new(),
            class_object_sizes: HashMap::new(),
            class_has_vtable: HashMap::new(),
            func_type_by_sig: std::cell::RefCell::new(HashMap::new()),
            current_local_types: std::cell::RefCell::new(HashMap::new()),
            func_param_types: HashMap::new(),
        }
    }

    fn find_or_create_func_type_idx(&self, params: &[ValType], results: &[ValType]) -> u32 {
        let key = (params.to_vec(), results.to_vec());
        let map = self.func_type_by_sig.borrow();
        if let Some(&idx) = map.get(&key) {
            return idx;
        }
        drop(map);
        0 // fallback; will be resolved during type section build
    }

    /// 构建结构体/类的字段偏移和对象大小信息
    fn build_type_layout_info(&mut self, program: &CHIRProgram) {
        // 结构体字段偏移 + 对象大小
        for sd in &program.structs {
            let mut fields = HashMap::new();
            let mut offset = 0u32;
            for f in &sd.fields {
                fields.insert(f.name.clone(), (offset, f.ty.clone()));
                offset += f.ty.size();
            }
            self.struct_field_offsets
                .insert(sd.name.clone(), fields.clone());
            self.class_object_sizes
                .insert(sd.name.clone(), offset.max(8));
            self.class_field_offsets.insert(sd.name.clone(), fields);
        }
        // 预计算哪些类需要 vtable（有继承关系）
        let mut has_children: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cd in &program.classes {
            if let Some(ref parent) = cd.extends {
                has_children.insert(parent.clone());
            }
        }
        for cd in &program.classes {
            let needs_vtable = cd.extends.is_some() || has_children.contains(&cd.name);
            self.class_has_vtable.insert(cd.name.clone(), needs_vtable);
        }
        // 类字段偏移（父类字段在前）
        let class_extends: HashMap<String, Option<String>> = program
            .classes
            .iter()
            .map(|c| (c.name.clone(), c.extends.clone()))
            .collect();
        let class_fields_raw: HashMap<String, Vec<crate::ast::FieldDef>> = program
            .classes
            .iter()
            .map(|c| (c.name.clone(), c.fields.clone()))
            .collect();
        for cd in &program.classes {
            let has_vtable = *self.class_has_vtable.get(&cd.name).unwrap_or(&false);
            let header = if has_vtable { 4u32 } else { 0 };
            // 收集继承链上所有父类字段
            let mut parent_fields: Vec<crate::ast::FieldDef> = Vec::new();
            let mut parent = cd.extends.clone();
            while let Some(ref pname) = parent {
                if let Some(pf) = class_fields_raw.get(pname) {
                    for f in pf.iter().rev() {
                        parent_fields.push(f.clone());
                    }
                }
                parent = class_extends.get(pname).and_then(|p| p.clone());
            }
            parent_fields.reverse();
            let mut fields = HashMap::new();
            let mut offset = header;
            for f in parent_fields.iter().chain(cd.fields.iter()) {
                fields
                    .entry(f.name.clone())
                    .or_insert((offset, f.ty.clone()));
                offset += f.ty.size();
            }
            self.class_object_sizes.insert(cd.name.clone(), offset);
            self.class_field_offsets.insert(cd.name.clone(), fields);
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
            CHIRExprKind::String(s) => {
                self.intern_string(s);
            }
            CHIRExprKind::Print { arg, .. } => {
                if let Some(a) = arg {
                    self.collect_strings_from_expr(a);
                }
            }
            CHIRExprKind::Binary { left, right, .. } => {
                self.collect_strings_from_expr(left);
                self.collect_strings_from_expr(right);
            }
            CHIRExprKind::Unary { expr: inner, .. } => self.collect_strings_from_expr(inner),
            CHIRExprKind::Call { args, .. } => {
                for a in args {
                    self.collect_strings_from_expr(a);
                }
            }
            CHIRExprKind::MethodCall { receiver, args, .. } => {
                self.collect_strings_from_expr(receiver);
                for a in args {
                    self.collect_strings_from_expr(a);
                }
            }
            CHIRExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.collect_strings_from_expr(cond);
                self.collect_strings_from_block(then_block);
                if let Some(b) = else_block {
                    self.collect_strings_from_block(b);
                }
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
            CHIRExprKind::ArraySet {
                array,
                index,
                value,
            } => {
                self.collect_strings_from_expr(array);
                self.collect_strings_from_expr(index);
                self.collect_strings_from_expr(value);
            }
            CHIRExprKind::ArrayNew { len, init } => {
                self.collect_strings_from_expr(len);
                self.collect_strings_from_expr(init);
            }
            CHIRExprKind::ArrayLiteral { elements } => {
                for e in elements {
                    self.collect_strings_from_expr(e);
                }
            }
            CHIRExprKind::TupleGet { tuple, .. } => self.collect_strings_from_expr(tuple),
            CHIRExprKind::TupleNew { elements } => {
                for e in elements {
                    self.collect_strings_from_expr(e);
                }
            }
            CHIRExprKind::StructNew { fields, .. } => {
                for (_, v) in fields {
                    self.collect_strings_from_expr(v);
                }
            }
            CHIRExprKind::Store { ptr, value, .. } => {
                self.collect_strings_from_expr(ptr);
                self.collect_strings_from_expr(value);
            }
            CHIRExprKind::Load { ptr, .. } => {
                self.collect_strings_from_expr(ptr);
            }
            _ => {}
        }
    }

    /// Pre-scan a block for CallIndirect nodes and register their type signatures
    fn collect_call_indirect_types(
        &self,
        block: &CHIRBlock,
        types: &mut wasm_encoder::TypeSection,
    ) {
        for stmt in &block.stmts {
            match stmt {
                CHIRStmt::Let { value, .. } => self.collect_call_indirect_types_expr(value, types),
                CHIRStmt::Assign { value, .. } => {
                    self.collect_call_indirect_types_expr(value, types)
                }
                CHIRStmt::Expr(e) => self.collect_call_indirect_types_expr(e, types),
                CHIRStmt::Return(Some(e)) => self.collect_call_indirect_types_expr(e, types),
                _ => {}
            }
        }
        if let Some(result) = &block.result {
            self.collect_call_indirect_types_expr(result, types);
        }
    }

    fn collect_call_indirect_types_expr(
        &self,
        expr: &CHIRExpr,
        types: &mut wasm_encoder::TypeSection,
    ) {
        match &expr.kind {
            CHIRExprKind::CallIndirect { args, .. } => {
                let params: Vec<ValType> = args.iter().map(|a| a.wasm_ty).collect();
                let results = if matches!(expr.ty, Type::Unit | Type::Nothing) {
                    vec![]
                } else {
                    vec![expr.wasm_ty]
                };
                let key = (params.clone(), results.clone());
                let mut map = self.func_type_by_sig.borrow_mut();
                if !map.contains_key(&key) {
                    let idx = types.len();
                    types.ty().function(params, results);
                    map.insert(key, idx);
                }
            }
            CHIRExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.collect_call_indirect_types_expr(cond, types);
                self.collect_call_indirect_types(then_block, types);
                if let Some(b) = else_block {
                    self.collect_call_indirect_types(b, types);
                }
            }
            CHIRExprKind::Block(b) => self.collect_call_indirect_types(b, types),
            CHIRExprKind::Match { subject, arms } => {
                self.collect_call_indirect_types_expr(subject, types);
                for arm in arms {
                    self.collect_call_indirect_types(&arm.body, types);
                }
            }
            CHIRExprKind::Binary { left, right, .. } => {
                self.collect_call_indirect_types_expr(left, types);
                self.collect_call_indirect_types_expr(right, types);
            }
            CHIRExprKind::Call { args, .. } => {
                for a in args {
                    self.collect_call_indirect_types_expr(a, types);
                }
            }
            CHIRExprKind::MethodCall { args, .. } => {
                for a in args {
                    self.collect_call_indirect_types_expr(a, types);
                }
            }
            CHIRExprKind::Cast { expr: inner, .. } => {
                self.collect_call_indirect_types_expr(inner, types)
            }
            CHIRExprKind::FieldGet { object, .. } => {
                self.collect_call_indirect_types_expr(object, types);
            }
            CHIRExprKind::FieldSet { object, value, .. } => {
                self.collect_call_indirect_types_expr(object, types);
                self.collect_call_indirect_types_expr(value, types);
            }
            CHIRExprKind::Store { ptr, value, .. } => {
                self.collect_call_indirect_types_expr(ptr, types);
                self.collect_call_indirect_types_expr(value, types);
            }
            CHIRExprKind::Load { ptr, .. } => {
                self.collect_call_indirect_types_expr(ptr, types);
            }
            _ => {}
        }
    }

    /// 生成完整 WASM 模块
    pub fn generate(&mut self, program: &CHIRProgram) -> Vec<u8> {
        // ── 0. 预处理：收集字符串字面量，分配内存地址 ────────────────
        self.collect_strings_from_program(program);

        // ── 0b. 构建结构体/类字段偏移表 ────────────────────────────────
        self.build_type_layout_info(program);

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
            // 记录函数参数类型（用于 call 指令前的参数类型修正）
            let param_tys: Vec<ValType> = func.params.iter().map(|p| p.wasm_ty).collect();
            self.func_param_types.insert(idx, param_tys);
        }

        // ── 预注册运行时助手函数索引（在用户函数之后）─────────────────
        let user_count = program.functions.len() as u32;
        // RT 函数参数类型（与 RT_NAMES 顺序一一对应）
        const RT_PARAM_TYPES: &[&[ValType]] = &[
            &[ValType::I64],                             // __rt_println_i64
            &[ValType::I64],                             // __rt_print_i64
            &[ValType::I32],                             // __rt_println_str
            &[ValType::I32],                             // __rt_print_str
            &[ValType::I32],                             // __rt_println_bool
            &[ValType::I32],                             // __rt_print_bool
            &[],                                         // __rt_println_empty
            &[ValType::I32],                             // __alloc
            &[ValType::F64],                             // sin
            &[ValType::F64],                             // cos
            &[ValType::F64],                             // tan
            &[ValType::F64],                             // exp
            &[ValType::F64],                             // log
            &[ValType::F64, ValType::F64],               // pow
            &[ValType::I64],                             // __i64_to_str
            &[ValType::I32],                             // __bool_to_str
            &[ValType::I32],                             // __str_to_i64
            &[ValType::I32, ValType::I32],               // __str_concat
            &[ValType::F64],                             // __f64_to_str
            &[],                                         // now
            &[],                                         // randomInt64
            &[],                                         // randomFloat64
            &[ValType::I32, ValType::I32],               // __str_contains
            &[ValType::I32, ValType::I32],               // __str_starts_with
            &[ValType::I32, ValType::I32],               // __str_ends_with
            &[ValType::I32],                             // __str_trim
            &[ValType::I32],                             // __str_to_array
            &[ValType::I32, ValType::I32],               // __str_index_of
            &[ValType::I32, ValType::I32, ValType::I32], // __str_replace
            &[],                                         // __arraylist_new
            &[ValType::I32, ValType::I64],               // __arraylist_append
            &[ValType::I32, ValType::I64],               // __arraylist_get
            &[ValType::I32, ValType::I64, ValType::I64], // __arraylist_set
            &[ValType::I32, ValType::I64],               // __arraylist_remove
            &[ValType::I32],                             // __arraylist_size
            &[],                                         // __hashmap_new
            &[ValType::I32, ValType::I64, ValType::I64], // __hashmap_put
            &[ValType::I32, ValType::I64],               // __hashmap_get
            &[ValType::I32, ValType::I64],               // __hashmap_contains
            &[ValType::I32, ValType::I64],               // __hashmap_remove
            &[ValType::I32],                             // __hashmap_size
            &[],                                         // __hashset_new
            &[ValType::I32, ValType::I64],               // __hashset_add
            &[ValType::I32, ValType::I64],               // __hashset_contains
            &[ValType::I32],                             // __hashset_size
            &[ValType::I64, ValType::I64],               // __pow_i64
            &[ValType::F64, ValType::F64],               // __pow_f64
        ];
        for (i, name) in RT_NAMES.iter().enumerate() {
            let idx = IMPORT_COUNT + user_count + i as u32;
            self.func_indices.insert(name.to_string(), idx);
            if let Some(params) = RT_PARAM_TYPES.get(i) {
                self.func_param_types.insert(idx, params.to_vec());
            }
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
        types.ty().function(
            WASI_FD_WRITE_PARAMS.to_vec(),
            WASI_FD_WRITE_RESULTS.to_vec(),
        );
        // ty 1: proc_exit
        types.ty().function(
            WASI_PROC_EXIT_PARAMS.to_vec(),
            WASI_PROC_EXIT_RESULTS.to_vec(),
        );
        // ty 2: clock_time_get (i32, i64, i32) -> i32
        let ty_clock = types.len();
        types.ty().function(
            vec![ValType::I32, ValType::I64, ValType::I32],
            vec![ValType::I32],
        );
        // ty 3: random_get (i32, i32) -> i32
        let ty_random = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
        // ty 4+: 用户函数
        let mut user_type_indices: Vec<u32> = Vec::new();
        for func in &program.functions {
            let param_tys: Vec<ValType> = func.params.iter().map(|p| p.wasm_ty).collect();
            let result_tys = wasm_result_tys(&func.return_ty, func.return_wasm_ty);
            let type_idx = types.len();
            types.ty().function(param_tys.clone(), result_tys.clone());
            user_type_indices.push(type_idx);
            self.func_type_by_sig
                .borrow_mut()
                .entry((param_tys, result_tys))
                .or_insert(type_idx);
        }

        // Pre-register CallIndirect type signatures so find_or_create_func_type_idx works
        for func in &program.functions {
            self.collect_call_indirect_types(&func.body, &mut types);
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
        imports.import(
            "wasi_snapshot_preview1",
            "clock_time_get",
            EntityType::Function(ty_clock),
        );
        imports.import(
            "wasi_snapshot_preview1",
            "random_get",
            EntityType::Function(ty_random),
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

        // ── 5. 全局段：堆指针 + free list ──────────────────────────────
        // global 0: heap_ptr (可变 i32，初始指向字符串数据之后)
        let heap_start = (self.data_offset + 7) & !7; // 对齐
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(heap_start as i32),
        );
        // global 1: free_list_head (可变 i32，空闲链表头，初始为 0)
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );

        // ── 6. 导出段 ─────────────────────────────────────────────────
        exports.export("memory", ExportKind::Memory, 0);
        // 导出所有用户函数（用 HashSet 去重，防止同名函数多次导出导致 duplicate export 错误）
        let mut exported_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
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
        for (fidx, func) in program.functions.iter().enumerate() {
            self.emit_function(func, &mut codes);
            if std::env::var("CJWASM_DEBUG_OFFSETS").is_ok() {
                let idx = IMPORT_COUNT + fidx as u32;
                let is_void = matches!(func.return_ty, Type::Unit | Type::Nothing);
                eprintln!(
                    "[codegen] func[{idx}] {name} void={is_void} params={params}",
                    name = func.name,
                    params = func.params.len()
                );
            }
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

        // ── 函数表 (Table + Element) ──────────────────────────────────
        // total WASM functions = imports + user funcs + RT funcs
        let total_funcs = (IMPORT_COUNT + user_count + RT_NAMES.len() as u32) as u64;
        let table_size = if total_funcs > 0 { total_funcs } else { 1 };
        let mut tables = wasm_encoder::TableSection::new();
        tables.table(wasm_encoder::TableType {
            element_type: wasm_encoder::RefType::FUNCREF,
            minimum: table_size,
            maximum: Some(table_size),
            table64: false,
            shared: false,
        });

        let all_func_indices: Vec<u32> = (0..total_funcs as u32).collect();
        let mut elements = wasm_encoder::ElementSection::new();
        elements.active(
            Some(0),
            &ConstExpr::i32_const(0),
            wasm_encoder::Elements::Functions(std::borrow::Cow::Owned(all_func_indices)),
        );

        // ── 组装（WASM 段顺序必须按规范） ─────────────────────────────
        let mut module = Module::new();
        module.section(&types);
        module.section(&imports);
        module.section(&functions);
        module.section(&tables);
        module.section(&memories);
        module.section(&globals);
        module.section(&exports);
        module.section(&elements);
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
                                            // i32 → i32: __alloc
        let ty_i32_i32 = types.len();
        types.ty().function(vec![ValType::I32], vec![ValType::I32]);
        functions.function(ty_i32_i32);
        rt_type_indices.push(ty_i32_i32); // __alloc
                                          // f64 → f64: sin, cos, tan, exp, log
        let ty_f64_f64 = types.len();
        types.ty().function(vec![ValType::F64], vec![ValType::F64]);
        for _ in 0..5 {
            functions.function(ty_f64_f64);
            rt_type_indices.push(ty_f64_f64);
        }
        // (f64, f64) → f64: pow
        let ty_f64f64_f64 = types.len();
        types
            .ty()
            .function(vec![ValType::F64, ValType::F64], vec![ValType::F64]);
        functions.function(ty_f64f64_f64);
        rt_type_indices.push(ty_f64f64_f64);
        // i64 → i32: __i64_to_str (returns string pointer)
        let ty_i64_i32 = types.len();
        types.ty().function(vec![ValType::I64], vec![ValType::I32]);
        functions.function(ty_i64_i32);
        rt_type_indices.push(ty_i64_i32);
        // i32 → i32: __bool_to_str (bool as i32 → string pointer)
        functions.function(ty_i32_i32);
        rt_type_indices.push(ty_i32_i32);
        // i32 → i64: __str_to_i64 (string pointer → i64)
        let ty_i32_i64 = types.len();
        types.ty().function(vec![ValType::I32], vec![ValType::I64]);
        functions.function(ty_i32_i64);
        rt_type_indices.push(ty_i32_i64);
        // (i32, i32) → i32: __str_concat (str_ptr, str_ptr → str_ptr)
        let ty_i32i32_i32 = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
        functions.function(ty_i32i32_i32);
        rt_type_indices.push(ty_i32i32_i32);
        // f64 → i32: __f64_to_str (float → string pointer)
        let ty_f64_i32 = types.len();
        types.ty().function(vec![ValType::F64], vec![ValType::I32]);
        functions.function(ty_f64_i32);
        rt_type_indices.push(ty_f64_i32);
        // () → i64: now (returns nanosecond timestamp)
        let ty_void_i64 = types.len();
        types.ty().function(vec![], vec![ValType::I64]);
        functions.function(ty_void_i64);
        rt_type_indices.push(ty_void_i64);
        // () → i64: randomInt64
        functions.function(ty_void_i64);
        rt_type_indices.push(ty_void_i64);
        // () → f64: randomFloat64
        let ty_void_f64 = types.len();
        types.ty().function(vec![], vec![ValType::F64]);
        functions.function(ty_void_f64);
        rt_type_indices.push(ty_void_f64);
        // (i32, i32) → i32: __str_contains, __str_starts_with, __str_ends_with
        functions.function(ty_i32i32_i32);
        rt_type_indices.push(ty_i32i32_i32);
        functions.function(ty_i32i32_i32);
        rt_type_indices.push(ty_i32i32_i32);
        functions.function(ty_i32i32_i32);
        rt_type_indices.push(ty_i32i32_i32);
        // i32 → i32: __str_trim
        functions.function(ty_i32_i32);
        rt_type_indices.push(ty_i32_i32);
        // i32 → i32: __str_to_array (string → array pointer)
        functions.function(ty_i32_i32);
        rt_type_indices.push(ty_i32_i32);
        // (i32, i32) → i64: __str_index_of (haystack, needle → index i64)
        let ty_i32i32_i64 = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![ValType::I64]);
        functions.function(ty_i32i32_i64 as u32);
        rt_type_indices.push(ty_i32i32_i64 as u32);
        // (i32, i32, i32) → i32: __str_replace (str, old, new → new_str)
        let ty_i32i32i32_i32 = types.len();
        types.ty().function(
            vec![ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
        );
        functions.function(ty_i32i32i32_i32 as u32);
        rt_type_indices.push(ty_i32i32i32_i32 as u32);
        // Collections (order must match RT_NAMES)
        // () → i32: __arraylist_new
        let ty_void_i32 = types.len();
        types.ty().function(vec![], vec![ValType::I32]);
        functions.function(ty_void_i32);
        rt_type_indices.push(ty_void_i32);
        // (i32, i64) → void: __arraylist_append
        let ty_i32i64_void = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I64], vec![]);
        functions.function(ty_i32i64_void);
        rt_type_indices.push(ty_i32i64_void);
        // (i32, i64) → i64: __arraylist_get
        let ty_i32i64_i64 = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]);
        functions.function(ty_i32i64_i64);
        rt_type_indices.push(ty_i32i64_i64);
        // (i32, i64, i64) → void: __arraylist_set
        let ty_i32i64i64_void = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I64, ValType::I64], vec![]);
        functions.function(ty_i32i64i64_void);
        rt_type_indices.push(ty_i32i64i64_void);
        // (i32, i64) → i64: __arraylist_remove
        functions.function(ty_i32i64_i64);
        rt_type_indices.push(ty_i32i64_i64);
        // (i32) → i64: __arraylist_size
        let ty_i32_i64 = types.len();
        types.ty().function(vec![ValType::I32], vec![ValType::I64]);
        functions.function(ty_i32_i64);
        rt_type_indices.push(ty_i32_i64);
        // () → i32: __hashmap_new
        functions.function(ty_void_i32);
        rt_type_indices.push(ty_void_i32);
        // (i32, i64, i64) → void: __hashmap_put
        functions.function(ty_i32i64i64_void);
        rt_type_indices.push(ty_i32i64i64_void);
        // (i32, i64) → i64: __hashmap_get
        functions.function(ty_i32i64_i64);
        rt_type_indices.push(ty_i32i64_i64);
        // (i32, i64) → i32: __hashmap_contains
        let ty_i32i64_i32 = types.len();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I64], vec![ValType::I32]);
        functions.function(ty_i32i64_i32);
        rt_type_indices.push(ty_i32i64_i32);
        // (i32, i64) → i64: __hashmap_remove
        functions.function(ty_i32i64_i64);
        rt_type_indices.push(ty_i32i64_i64);
        // (i32) → i64: __hashmap_size
        functions.function(ty_i32_i64);
        rt_type_indices.push(ty_i32_i64);
        // () → i32: __hashset_new
        functions.function(ty_void_i32);
        rt_type_indices.push(ty_void_i32);
        // (i32, i64) → void: __hashset_add
        functions.function(ty_i32i64_void);
        rt_type_indices.push(ty_i32i64_void);
        // (i32, i64) → i32: __hashset_contains
        functions.function(ty_i32i64_i32);
        rt_type_indices.push(ty_i32i64_i32);
        // (i32) → i64: __hashset_size
        functions.function(ty_i32_i64);
        rt_type_indices.push(ty_i32_i64);
        // (i64, i64) → i64: __pow_i64
        let ty_i64i64_i64 = types.len();
        types
            .ty()
            .function(vec![ValType::I64, ValType::I64], vec![ValType::I64]);
        functions.function(ty_i64i64_i64);
        rt_type_indices.push(ty_i64i64_i64);
        // (f64, f64) → f64: __pow_f64
        let ty_f64f64_f64 = types.len();
        types
            .ty()
            .function(vec![ValType::F64, ValType::F64], vec![ValType::F64]);
        functions.function(ty_f64f64_f64);
        rt_type_indices.push(ty_f64f64_f64);
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
        // 7: __alloc
        codes.function(&crate::memory::emit_alloc_func(0));
        // 8: sin (Taylor series, 12 iterations)
        codes.function(&Self::build_rt_math_sin());
        // 9: cos = sin(x + PI/2)
        let sin_idx = self.func_indices[RT_MATH_SIN];
        codes.function(&Self::build_rt_math_cos(sin_idx));
        // 10: tan = sin(x) / cos(x)
        let cos_idx = self.func_indices[RT_MATH_COS];
        codes.function(&Self::build_rt_math_tan(sin_idx, cos_idx));
        // 11: exp (Taylor series, 20 iterations)
        codes.function(&Self::build_rt_math_exp());
        // 12: log (atanh series, 40 iterations)
        codes.function(&Self::build_rt_math_log());
        // 13: pow = exp(y * log(x))
        let exp_idx = self.func_indices[RT_MATH_EXP];
        let log_idx = self.func_indices[RT_MATH_LOG];
        codes.function(&Self::build_rt_math_pow(exp_idx, log_idx));
        // 14: __i64_to_str
        let alloc_idx = self.func_indices[RT_ALLOC];
        codes.function(&Self::build_rt_i64_to_str(alloc_idx));
        // 15: __bool_to_str
        codes.function(&Self::build_rt_bool_to_str(alloc_idx));
        // 16: __str_to_i64
        codes.function(&Self::build_rt_str_to_i64());
        // 17: __str_concat
        codes.function(&Self::build_rt_str_concat(alloc_idx));
        // 18: __f64_to_str
        let i64_to_str_idx = self.func_indices[RT_I64_TO_STR];
        let str_concat_idx = self.func_indices[RT_STR_CONCAT];
        codes.function(&Self::build_rt_f64_to_str(
            alloc_idx,
            i64_to_str_idx,
            str_concat_idx,
        ));
        // 19: now
        codes.function(&Self::build_rt_now());
        // 20: randomInt64
        codes.function(&Self::build_rt_random_int64());
        // 21: randomFloat64
        codes.function(&Self::build_rt_random_float64());
        // 22: __str_contains
        codes.function(&Self::build_rt_str_contains());
        // 23: __str_starts_with
        codes.function(&Self::build_rt_str_starts_with());
        // 24: __str_ends_with
        codes.function(&Self::build_rt_str_ends_with());
        // 25: __str_trim
        codes.function(&Self::build_rt_str_trim(alloc_idx));
        // 26: __str_to_array
        codes.function(&Self::build_rt_str_to_array(alloc_idx));
        // 27: __str_index_of
        codes.function(&Self::build_rt_str_index_of());
        // 28: __str_replace
        codes.function(&Self::build_rt_str_replace(alloc_idx));
        // 29+: Collections
        codes.function(&self.emit_arraylist_new());
        codes.function(&self.emit_arraylist_append());
        codes.function(&self.emit_arraylist_get());
        codes.function(&self.emit_arraylist_set());
        codes.function(&self.emit_arraylist_remove());
        codes.function(&self.emit_arraylist_size());
        codes.function(&self.emit_hashmap_new());
        codes.function(&self.emit_hashmap_put());
        codes.function(&self.emit_hashmap_get());
        codes.function(&self.emit_hashmap_contains());
        codes.function(&self.emit_hashmap_remove());
        codes.function(&self.emit_hashmap_size());
        codes.function(&self.emit_hashset_new());
        codes.function(&self.emit_hashset_add());
        codes.function(&self.emit_hashset_contains());
        codes.function(&self.emit_hashset_size());
        // __pow_i64
        codes.function(&Self::build_rt_pow_i64());
        // __pow_f64
        let exp_idx2 = self.func_indices[RT_MATH_EXP];
        let log_idx2 = self.func_indices[RT_MATH_LOG];
        codes.function(&Self::build_rt_math_pow(exp_idx2, log_idx2));
    }

    /// 生成 I/O 辅助 MemArg
    fn mem(offset: u64, align: u32) -> MemArg {
        MemArg {
            offset,
            align,
            memory_index: 0,
        }
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
    fn emit_write_buf(
        buf_ptr_expr: impl Fn(&mut wasm_encoder::Function),
        buf_len_expr: impl Fn(&mut wasm_encoder::Function),
        fd: i32,
        f: &mut wasm_encoder::Function,
    ) {
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
            |f| {
                f.instruction(&Instruction::I32Const(0));
            },
            |f| {
                f.instruction(&Instruction::I32Const(1));
            },
            fd,
            f,
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
            } else {
                4
            };
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
            } else {
                5
            };
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

    // ─── 数学运行时函数 ─────────────────────────────────────────────────────

    /// sin(x) via Taylor series (12 iterations), with range reduction to [-π, π]
    fn build_rt_math_sin() -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::F64), // 1: term
            (1, ValType::F64), // 2: sum
            (1, ValType::F64), // 3: x_sq
            (1, ValType::F64), // 4: i (counter)
        ]);
        let two_pi = std::f64::consts::TAU;
        // x = x - nearest(x / 2π) * 2π
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(two_pi));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::F64Nearest);
        f.instruction(&Instruction::F64Const(two_pi));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::LocalSet(0));
        // sum = x, term = x
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(1));
        // x_sq = x * x
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(4));
        for _ in 0..12 {
            // term = -term * x_sq / ((2i)*(2i+1))
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::F64Neg);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalTee(1));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::LocalSet(1));
            // sum += term
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(2));
            // i += 1
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(4));
        }
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// cos(x) = sin(x + π/2)
    fn build_rt_math_cos(sin_idx: u32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(std::f64::consts::FRAC_PI_2));
        f.instruction(&Instruction::F64Add);
        f.instruction(&Instruction::Call(sin_idx));
        f.instruction(&Instruction::End);
        f
    }

    /// tan(x) = sin(x) / cos(x)
    fn build_rt_math_tan(sin_idx: u32, cos_idx: u32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(sin_idx));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(cos_idx));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f
    }

    /// exp(x) via Taylor series (20 iterations)
    fn build_rt_math_exp() -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::F64), // 1: term
            (1, ValType::F64), // 2: sum
            (1, ValType::F64), // 3: i
        ]);
        // sum = 1, term = 1, i = 1
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(3));
        for _ in 0..20 {
            // term = term * x / i
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::LocalSet(1));
            // sum += term
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(2));
            // i += 1
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(3));
        }
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// ln(x) via atanh series: 2 * atanh((x-1)/(x+1)), 40 iterations
    fn build_rt_math_log() -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::F64), // 1: y = (x-1)/(x+1)
            (1, ValType::F64), // 2: y_sq
            (1, ValType::F64), // 3: term
            (1, ValType::F64), // 4: sum
            (1, ValType::F64), // 5: i
        ]);
        // y = (x - 1) / (x + 1)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::F64Add);
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::LocalSet(1));
        // y_sq = y * y
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::LocalSet(2));
        // term = y, sum = y, i = 1
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(5));
        for _ in 0..40 {
            // term *= y_sq
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalSet(3));
            // sum += term / (2*i + 1)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(4));
            // i += 1
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(5));
        }
        // result = 2 * sum
        f.instruction(&Instruction::F64Const(2.0));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::End);
        f
    }

    /// pow(base, exp) = exp(exp * log(base))
    fn build_rt_math_pow(exp_idx: u32, log_idx: u32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(1)); // exp
        f.instruction(&Instruction::LocalGet(0)); // base
        f.instruction(&Instruction::Call(log_idx)); // log(base)
        f.instruction(&Instruction::F64Mul); // exp * log(base)
        f.instruction(&Instruction::Call(exp_idx)); // exp(exp * log(base))
        f.instruction(&Instruction::End);
        f
    }

    /// __pow_i64(base: i64, exp: i64) -> i64
    /// Integer exponentiation: base ** exp (returns 0 for negative exp)
    fn build_rt_pow_i64() -> wasm_encoder::Function {
        // locals: 0=base, 1=exp, 2=result
        let mut f = wasm_encoder::Function::new(vec![(1, ValType::I64)]);
        // exp < 0 -> return 0
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // result = 1
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::LocalSet(2));
        // loop: if exp <= 0 break; result *= base; exp--
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LeS);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// __i64_to_str(val: i64) -> i32 (string pointer)
    /// Converts an i64 to a decimal string, allocates memory: [len: i32][bytes...]
    fn build_rt_i64_to_str(alloc_idx: u32) -> wasm_encoder::Function {
        // locals: 0=val(i64), 1=buf_pos(i32), 2=is_neg(i32), 3=abs_val(i64),
        // 4=digit(i32), 5=str_len(i32), 6=result_ptr(i32)
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32), // 1: buf_pos
            (1, ValType::I32), // 2: is_neg
            (1, ValType::I64), // 3: abs_val
            (1, ValType::I32), // 4: digit
            (1, ValType::I32), // 5: str_len
            (1, ValType::I32), // 6: result_ptr
        ]);
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset,
            align,
            memory_index: 0,
        };

        // Use scratch area at address 0-23 as temp digit buffer (max 20 digits + sign)
        // buf_pos = 23 (write digits right to left)
        f.instruction(&Instruction::I32Const(23));
        f.instruction(&Instruction::LocalSet(1));

        // Handle zero case
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        {
            f.instruction(&Instruction::I32Const(23));
            f.instruction(&Instruction::I32Const(48)); // '0'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::I32Const(22));
            f.instruction(&Instruction::LocalSet(1));
        }
        f.instruction(&Instruction::Else);
        {
            // is_neg = val < 0
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(2));
            // abs_val = is_neg ? -val : val
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(3));

            // while abs_val > 0: digit = abs_val % 10; buf[buf_pos] = '0' + digit; abs_val /= 10; buf_pos--
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::BrIf(1));

                // digit = abs_val % 10
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64Const(10));
                f.instruction(&Instruction::I64RemU);
                f.instruction(&Instruction::I32WrapI64);
                f.instruction(&Instruction::LocalSet(4));
                // buf[buf_pos] = '0' + digit
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(48));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                // abs_val /= 10
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64Const(10));
                f.instruction(&Instruction::I64DivU);
                f.instruction(&Instruction::LocalSet(3));
                // buf_pos--
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::LocalSet(1));

                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End); // end loop
            f.instruction(&Instruction::End); // end block

            // if is_neg: buf[buf_pos] = '-'; buf_pos--
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End); // end if-else (zero case)

        // str_len = 23 - buf_pos
        f.instruction(&Instruction::I32Const(23));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(5));

        // Allocate string: result_ptr = __alloc(4 + str_len)
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(6));

        // result_ptr[0] = str_len (length header)
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // Copy digits from scratch buf to result string
        // Simple byte-by-byte copy loop
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4)); // reuse local 4 as loop counter
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1));

            // result_ptr[4 + i] = buf[buf_pos + 1 + i]
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Add);

            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));

            f.instruction(&Instruction::I32Store8(mem(0, 0)));

            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block

        // return result_ptr
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::End);
        f
    }

    /// __bool_to_str(val: i32) -> i32 (string pointer)
    fn build_rt_bool_to_str(alloc_idx: u32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32), // 1: result_ptr
        ]);
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset,
            align,
            memory_index: 0,
        };

        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        {
            // "true" = [4, 't', 'r', 'u', 'e']
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::Call(alloc_idx));
            f.instruction(&Instruction::LocalTee(1));
            f.instruction(&Instruction::I32Const(4)); // len
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(0x65757274)); // "true" in little-endian
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::LocalGet(1));
        }
        f.instruction(&Instruction::Else);
        {
            // "false" = [5, 'f', 'a', 'l', 's', 'e']
            f.instruction(&Instruction::I32Const(9));
            f.instruction(&Instruction::Call(alloc_idx));
            f.instruction(&Instruction::LocalTee(1));
            f.instruction(&Instruction::I32Const(5)); // len
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(0x736c6166)); // "fals" in little-endian
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(101)); // 'e'
            f.instruction(&Instruction::I32Store8(mem(8, 0)));
            f.instruction(&Instruction::LocalGet(1));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    /// __str_to_i64(str_ptr: i32) -> i64
    fn build_rt_str_to_i64() -> wasm_encoder::Function {
        // locals: 0=str_ptr, 1=result(i64), 2=sign(i64), 3=i(i32), 4=len(i32), 5=byte(i32)
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I64), // 1: result
            (1, ValType::I64), // 2: sign
            (1, ValType::I32), // 3: i
            (1, ValType::I32), // 4: len
            (1, ValType::I32), // 5: byte
        ]);
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset,
            align,
            memory_index: 0,
        };

        // result = 0, sign = 1, i = 0
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));

        // len = str_ptr[0]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4));

        // Check for '-' or '+'
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Load8U(mem(4, 0)));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I64Const(-1));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Load8U(mem(4, 0)));
            f.instruction(&Instruction::I32Const(43)); // '+'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // while i < len: result = result * 10 + (byte - '0')
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1));

            // byte = str_ptr[4 + i]
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::LocalSet(5));

            // if byte < '0' || byte > '9': break
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(57));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::BrIf(1));

            // result = result * 10 + (byte - '0')
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Const(10));
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I64ExtendI32S);
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(1));

            // i++
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(3));

            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block

        // return result * sign
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::End);
        f
    }

    /// __str_concat(ptr1: i32, ptr2: i32) -> i32
    fn build_rt_str_concat(alloc_idx: u32) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![(4, ValType::I32)]);
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // local 2=len1, 3=len2, 4=total_len, 5=new_ptr
        // len1 = mem[ptr1]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        // len2 = mem[ptr2]
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // total_len = len1 + len2
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        // new_ptr = alloc(total_len + 4)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(5));
        // mem[new_ptr] = total_len
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        // memory.copy(new_ptr+4, ptr1+4, len1)
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
        // memory.copy(new_ptr+4+len1, ptr2+4, len2)
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
        // return new_ptr
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// __f64_to_str(val: f64) -> i32
    fn build_rt_f64_to_str(
        alloc_idx: u32,
        i64_to_str_idx: u32,
        str_concat_idx: u32,
    ) -> wasm_encoder::Function {
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32), // 1: int_str
            (1, ValType::I32), // 2: frac_str
            (1, ValType::I32), // 3: result (unused)
            (1, ValType::I64), // 4: frac_val
            (1, ValType::I32), // 5: dot_str
            (1, ValType::I32), // 6: is_neg (unused)
        ]);
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // int_str = __i64_to_str(trunc(val))
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::Call(i64_to_str_idx));
        f.instruction(&Instruction::LocalSet(1));
        // dot_str = alloc(5), store len=1 and byte='.'
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(46)); // '.'
        f.instruction(&Instruction::I32Store8(mem(4, 0)));
        // frac = abs((val - trunc(val)) * 1000000)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Trunc);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::F64Abs);
        f.instruction(&Instruction::F64Const(1000000.0));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::F64Nearest);
        f.instruction(&Instruction::I64TruncF64U);
        f.instruction(&Instruction::LocalSet(4));
        // frac_str = __i64_to_str(frac_val)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::Call(i64_to_str_idx));
        f.instruction(&Instruction::LocalSet(2));
        // result = __str_concat(int_str, dot_str)
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::Call(str_concat_idx));
        // result = __str_concat(result, frac_str)
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Call(str_concat_idx));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// now() -> i64: call clock_time_get(0, 1, scratch), return i64 from scratch
    fn build_rt_now() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::I32Const(0)); // clock_id = realtime
        f.instruction(&Instruction::I64Const(1)); // precision = 1ns
        f.instruction(&Instruction::I32Const(WASI_SCRATCH)); // output buffer
        f.instruction(&Instruction::Call(IDX_CLOCK_TIME_GET));
        f.instruction(&Instruction::Drop); // drop errno
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// randomInt64() -> i64: call random_get(scratch, 8), return i64 from scratch
    fn build_rt_random_int64() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::Call(IDX_RANDOM_GET));
        f.instruction(&Instruction::Drop);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// randomFloat64() -> f64: randomInt64 as u64 / MAX_U64 (simplified)
    fn build_rt_random_float64() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::Call(IDX_RANDOM_GET));
        f.instruction(&Instruction::Drop);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        // Mask to positive (clear sign bit) then convert to f64 / MAX_I64
        f.instruction(&Instruction::I64Const(0x7FFFFFFFFFFFFFFF));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::F64ConvertI64U);
        f.instruction(&Instruction::F64Const(9223372036854775807.0)); // MAX_I63
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f
    }

    /// __str_contains(str: i32, sub: i32) -> i32 (0/1)
    /// Inline substring search — returns 1 if `sub` appears anywhere in `str`.
    fn build_rt_str_contains() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str, 1=sub, 2=str_len, 3=sub_len, 4=i, 5=j, 6=matched
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // sub_len == 0 → return 1
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // if i > str_len - sub_len → not found
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GtS);
            f.instruction(&Instruction::BrIf(1));
            // matched = 1; j = 0
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // str[4+i+j] vs sub[4+j]
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Ne);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Br(2));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(5));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End); // loop
            f.instruction(&Instruction::End); // block
                                              // if matched → return 1
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // i++
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // loop
        f.instruction(&Instruction::End); // block
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_starts_with(str: i32, prefix: i32) -> i32 (0/1)
    fn build_rt_str_starts_with() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str, 1=prefix, 2=str_len, 3=pre_len, 4=i
        let mut f = wasm_encoder::Function::new(vec![(3, ValType::I32)]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // if pre_len > str_len → return 0
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_ends_with(str: i32, suffix: i32) -> i32 (0/1)
    fn build_rt_str_ends_with() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str, 1=suffix, 2=str_len, 3=suf_len, 4=i, 5=offset
        let mut f = wasm_encoder::Function::new(vec![(4, ValType::I32)]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // if suf_len > str_len → return 0
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // offset = str_len - suf_len
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_trim(str: i32) -> i32: allocates a new trimmed string
    fn build_rt_str_trim(alloc_idx: u32) -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str, 1=len, 2=start, 3=end, 4=new_len, 5=new_ptr, 6=copy_i
        let mut f = wasm_encoder::Function::new(vec![(6, ValType::I32)]);
        // len = str[0]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));
        // start = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(2));
        // skip leading whitespace
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Const(32));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // end = len
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        // skip trailing whitespace
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32LeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Const(32));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // new_len = end - start
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(4));
        // allocate 4 + new_len
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(5));
        // write length
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        // copy bytes
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // new_ptr[4+i] = str[4+start+i]
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // return new_ptr
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_to_array(str_ptr: i32) -> i32: string → array of i64 (byte codes)
    fn build_rt_str_to_array(alloc_idx: u32) -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str_ptr, 1=len, 2=arr_ptr, 3=i
        let mut f = wasm_encoder::Function::new(vec![(3, ValType::I32)]);
        // len = load i32 at str_ptr
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));
        // arr_ptr = alloc(4 + len * 8)
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(2));
        // arr_ptr[0] = len (store size)
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));
        // loop: while i < len
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // arr_ptr + 4 + i*8 = (i64)byte at str_ptr + 4 + i
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        // load byte from str_ptr + 4 + i
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        // i++
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block
                                          // return arr_ptr
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_index_of(haystack: i32, needle: i32) -> i64: returns index or -1
    fn build_rt_str_index_of() -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=haystack, 1=needle, 2=h_len, 3=n_len, 4=i, 5=j, 6=match_flag
        let mut f = wasm_encoder::Function::new(vec![(5, ValType::I32)]);
        // h_len = haystack[0]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        // n_len = needle[0]
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // if n_len == 0, return 0
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // if n_len > h_len, return -1
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        // outer loop: while i <= h_len - n_len
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32GtS); // if i > h_len - n_len, break
        f.instruction(&Instruction::BrIf(1));
        // match_flag = 1
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::LocalSet(6));
        // j = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        // inner loop: while j < n_len
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // compare haystack[4 + i + j] vs needle[4 + j]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        // mismatch: match_flag = 0, break inner
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Br(2)); // break inner loop
        f.instruction(&Instruction::End);
        // j++
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // end inner loop
        f.instruction(&Instruction::End); // end inner block
                                          // if match_flag, return i
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // i++
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // end outer loop
        f.instruction(&Instruction::End); // end outer block
                                          // not found: return -1
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_replace(str: i32, old: i32, new: i32) -> i32: replace first occurrence
    fn build_rt_str_replace(alloc_idx: u32) -> wasm_encoder::Function {
        let mem = |offset: u64, align: u32| MemArg {
            offset,
            align,
            memory_index: 0,
        };
        // locals: 0=str, 1=old, 2=new, 3=str_len, 4=old_len, 5=new_len, 6=idx,
        //         7=result_len, 8=result_ptr, 9=copy_i
        let mut f = wasm_encoder::Function::new(vec![(7, ValType::I32)]);
        // str_len
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // old_len
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4));
        // new_len
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(5));
        // Find index of old in str (inline simple search)
        // idx = -1
        f.instruction(&Instruction::I32Const(-1));
        f.instruction(&Instruction::LocalSet(6));
        // search: i=0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        // if i > str_len - old_len, break
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::BrIf(1));
        // check if str[i..i+old_len] == old
        // j = 0, match_flag = 1
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::LocalSet(7)); // reuse 7 as match_flag temporarily
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(8)); // reuse 8 as j
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // compare bytes
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(7));
        f.instruction(&Instruction::Br(2));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(8));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // end inner loop
        f.instruction(&Instruction::End); // end inner block
                                          // if match_flag: idx = i, break
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Br(2)); // break outer
        f.instruction(&Instruction::End);
        // i++
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // end outer loop
        f.instruction(&Instruction::End); // end outer block
                                          // if idx == -1, return original string
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(-1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // result_len = str_len - old_len + new_len
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(7));
        // result_ptr = alloc(4 + result_len)
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(8));
        // store result_len
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        // copy prefix: str[4..4+idx] → result[4..4+idx]
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // copy new: new[4..4+new_len] → result[4+idx..4+idx+new_len]
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // copy suffix: str[4+idx+old_len..4+str_len] → result[4+idx+new_len..]
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // result[4 + idx + new_len + i] = str[4 + idx + old_len + i]
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(9));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // return result_ptr
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::End);
        f
    }

    // ─── 集合运行时函数 ─────────────────────────────────────────────────────
    // ArrayList 布局: [len: i32][cap: i32][data_ptr: i32] (12 bytes), data: [elem0: i64]...
    // HashMap 布局: [size: i32][cap: i32][buckets_ptr: i32] (12 bytes), bucket: [occupied: i32][key: i64][val: i64] (20 bytes)
    // HashSet 布局: [size: i32][cap: i32][entries_ptr: i32] (12 bytes), entry: [val: i64][occupied: i32] (12 bytes)

    fn emit_arraylist_new(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let alloc_idx = self.func_indices[RT_ALLOC];
        let mut f = wasm_encoder::Function::new(vec![(1, ValType::I32), (1, ValType::I32)]);
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::I32Const(64));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Store(mem(4, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(8, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_arraylist_append(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let alloc_idx = self.func_indices[RT_ALLOC];
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::Call(alloc_idx));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I64Load(mem(0, 3)));
            f.instruction(&Instruction::I64Store(mem(0, 3)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Store(mem(8, 2)));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalSet(4));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_arraylist_get(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_arraylist_set(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_arraylist_remove(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I64),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_arraylist_size(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_new(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let alloc_idx = self.func_indices[RT_ALLOC];
        let mut f = wasm_encoder::Function::new(vec![(1, ValType::I32), (1, ValType::I32)]);
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::I32Const(320));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(320));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::I32Store(mem(4, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(8, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_put(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(20));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I64Store(mem(4, 3)));
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::I64Store(mem(12, 3)));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::Br(2));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::I64Store(mem(12, 3)));
                f.instruction(&Instruction::Br(2));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_get(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(7));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
            ValType::I64,
        )));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(20));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(12, 3)));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Unreachable);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_contains(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
            ValType::I32,
        )));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(20));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Unreachable);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_remove(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
            ValType::I64,
        )));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(20));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I64Load(mem(12, 3)));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::Br(2));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Unreachable);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashmap_size(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashset_new(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let alloc_idx = self.func_indices[RT_ALLOC];
        let mut f = wasm_encoder::Function::new(vec![(1, ValType::I32), (1, ValType::I32)]);
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::I32Const(192));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(192));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store8(mem(0, 0)));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::I32Store(mem(4, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(8, 2)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashset_add(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(12));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(8, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I64Store(mem(0, 3)));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Store(mem(8, 2)));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::Br(2));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(0, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashset_contains(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
            (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
            ValType::I32,
        )));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(12));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(8, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(0, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32RemU);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Unreachable);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    fn emit_hashset_size(&self) -> wasm_encoder::Function {
        let mem = |o: u64, a: u32| MemArg {
            offset: o,
            align: a,
            memory_index: 0,
        };
        let mut f = wasm_encoder::Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::End);
        f
    }

    // ─── 函数 ──────────────────────────────────────────────────────────────

    /// 检查函数是否是 __ClassName_init，返回类名
    fn extract_init_class_name(func_name: &str) -> Option<String> {
        if func_name.starts_with("__")
            && func_name.ends_with("_init")
            && !func_name.ends_with("_init_body")
        {
            Some(func_name[2..func_name.len() - 5].to_string())
        } else {
            None
        }
    }

    fn emit_function(&self, func: &CHIRFunction, codes: &mut CodeSection) {
        let param_count = func.params.len() as u32;
        let is_init = Self::extract_init_class_name(&func.name).is_some();

        // 优先使用 CHIR lowering 阶段记录的 local 类型（精确），
        // fallback 到 collect_locals_from_block（向后兼容）
        let mut locals_map: Vec<(u32, ValType)> = if !func.local_wasm_types.is_empty() {
            func.local_wasm_types
                .iter()
                .filter(|(&idx, _)| idx >= param_count)
                .map(|(&idx, &ty)| (idx, ty))
                .collect()
        } else {
            let mut out = Vec::new();
            collect_locals_from_block(&func.body, param_count, &mut out);
            out
        };
        locals_map.sort_by_key(|&(idx, _)| idx);
        locals_map.dedup_by_key(|l| l.0);

        // 构建当前函数所有 local（含参数）的类型表，供 emit_stmt 中 local.set 类型修正
        {
            let mut lt = self.current_local_types.borrow_mut();
            lt.clear();
            for (i, p) in func.params.iter().enumerate() {
                lt.insert(i as u32, p.wasm_ty);
            }
            for &(idx, ty) in &locals_map {
                lt.insert(idx, ty);
            }
            // 无 debug
        }

        // wasm_encoder 的 locals 格式：(count, ValType) 的 run-length
        let locals_for_encoder = run_length_encode_locals(&locals_map, param_count);
        let mut wasm_func = wasm_encoder::Function::new(locals_for_encoder);

        // init 函数 prologue：分配对象，设 this local
        if let Some(class_name) = Self::extract_init_class_name(&func.name) {
            let obj_size = self
                .class_object_sizes
                .get(&class_name)
                .copied()
                .unwrap_or(16);
            let alloc_idx = self.func_indices.get(RT_ALLOC).copied().unwrap_or(0);
            // this local = param_count (第一个非参数 local，由 lowering 分配)
            let this_local = param_count;
            wasm_func.instruction(&Instruction::I32Const(obj_size as i32));
            wasm_func.instruction(&Instruction::Call(alloc_idx));
            wasm_func.instruction(&Instruction::LocalSet(this_local));
            // 如果有 vtable，在 offset 0 存储 vtable base（暂时存 0）
            if *self.class_has_vtable.get(&class_name).unwrap_or(&false) {
                wasm_func.instruction(&Instruction::LocalGet(this_local));
                wasm_func.instruction(&Instruction::I32Const(0)); // vtable base placeholder
                wasm_func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }
        }

        let has_result = !wasm_result_tys(&func.return_ty, func.return_wasm_ty).is_empty();

        if is_init {
            // init 函数：emit body，然后 epilogue 返回 this
            self.emit_block_void(&func.body, &mut wasm_func);
            let this_local = param_count;
            wasm_func.instruction(&Instruction::LocalGet(this_local));
            wasm_func.instruction(&Instruction::Return);
        } else if has_result {
            self.emit_block_with_ty(&func.body, func.return_wasm_ty, &mut wasm_func);
        } else {
            self.emit_block_void(&func.body, &mut wasm_func);
        }

        wasm_func.instruction(&Instruction::End);
        codes.function(&wasm_func);
    }

    fn emit_block(&self, block: &CHIRBlock, func: &mut wasm_encoder::Function) {
        for stmt in &block.stmts {
            self.emit_stmt_void(stmt, func);
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
                    then_block
                        .stmts
                        .iter()
                        .any(|s| matches!(s, CHIRStmt::Return(_)))
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
            CHIRExprKind::MethodCall { func_idx, .. } => {
                if let Some(idx) = func_idx {
                    if let Some(&is_void) = self.func_void_map.get(idx) {
                        return !is_void;
                    }
                }
                true
            }
            // Cast：emit_expr 总是产出值（void inner 时补零）
            CHIRExprKind::Cast { .. } => true,
            CHIRExprKind::Store { .. } => false,
            CHIRExprKind::FieldSet { .. } => false,
            CHIRExprKind::ArraySet { .. } => false,
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
                    || then_block
                        .stmts
                        .iter()
                        .any(|s| matches!(s, CHIRStmt::Return(_)))
            }
            CHIRExprKind::Block(block) => block.result.is_some(),
            CHIRExprKind::MethodCall { .. } => !matches!(expr.ty, Type::Unit | Type::Nothing),
            CHIRExprKind::Print { .. } => false,
            CHIRExprKind::Store { .. } => false,
            CHIRExprKind::FieldSet { .. } => false,
            CHIRExprKind::ArraySet { .. } => false,
            _ => true,
        }
    }

    /// 检查表达式类型是否总是在 WASM 栈上产生值（不依赖 ty 字段）
    /// 仅包含纯计算类表达式，不包含 Call/MethodCall（它们可能是 void）
    fn expr_kind_always_produces_value(kind: &CHIRExprKind) -> bool {
        matches!(
            kind,
            CHIRExprKind::Integer(_)
                | CHIRExprKind::Float(_)
                | CHIRExprKind::Float32(_)
                | CHIRExprKind::Bool(_)
                | CHIRExprKind::String(_)
                | CHIRExprKind::Rune(_)
                | CHIRExprKind::Local(_)
                | CHIRExprKind::Global(_)
                | CHIRExprKind::Binary { .. }
                | CHIRExprKind::Unary { .. }
                | CHIRExprKind::Cast { .. }
                | CHIRExprKind::FieldGet { .. }
                | CHIRExprKind::ArrayGet { .. }
                | CHIRExprKind::TupleGet { .. }
                | CHIRExprKind::TupleNew { .. }
                | CHIRExprKind::StructNew { .. }
                | CHIRExprKind::ArrayLiteral { .. }
                | CHIRExprKind::ArrayNew { .. }
                | CHIRExprKind::Match { .. }
                | CHIRExprKind::Load { .. }
                | CHIRExprKind::BuiltinAbs { .. }
                | CHIRExprKind::BuiltinCompareTo { .. }
                | CHIRExprKind::BuiltinStringIsEmpty { .. }
                | CHIRExprKind::MathUnary { .. }
                | CHIRExprKind::MathBinary { .. }
                | CHIRExprKind::CallIndirect { .. }
        )
    }

    /// 获取表达式实际产出的 WASM 类型。
    /// 对于指针/对象类型（Array, Struct, String 等），实际 WASM 类型始终是 I32，
    /// 即使 CHIR lowering 错误地标记为 I64。
    fn actual_wasm_ty(expr: &CHIRExpr) -> ValType {
        if matches!(expr.ty, Type::Unit | Type::Nothing) {
            return expr.wasm_ty;
        }
        // 对于指针/对象类型，使用 ty.to_wasm() 而非 wasm_ty
        match &expr.ty {
            Type::Array(_)
            | Type::Tuple(_)
            | Type::Struct(..)
            | Type::String
            | Type::Range
            | Type::Option(_)
            | Type::Result(_, _)
            | Type::Slice(_)
            | Type::Map(_, _)
            | Type::Function { .. }
            | Type::TypeParam(_)
            | Type::This
            | Type::Qualified(_) => ValType::I32,
            _ => expr.wasm_ty,
        }
    }

    /// void 上下文的块 emit（Unit If 分支），对产生的 result 主动 Drop
    fn emit_block_void(&self, block: &CHIRBlock, func: &mut wasm_encoder::Function) {
        for stmt in &block.stmts {
            self.emit_stmt_void(stmt, func);
        }
        if let Some(result) = &block.result {
            self.emit_expr_void(result, func);
        }
    }

    /// 带期望返回类型的块 emit（用于 If 分支，确保类型一致）
    fn emit_block_with_ty(
        &self,
        block: &CHIRBlock,
        expected_ty: ValType,
        func: &mut wasm_encoder::Function,
    ) {
        for stmt in &block.stmts {
            self.emit_stmt_void(stmt, func);
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
            CHIRExprKind::Integer(n) => match expr.wasm_ty {
                ValType::I32 => {
                    func.instruction(&Instruction::I32Const(*n as i32));
                }
                ValType::I64 => {
                    func.instruction(&Instruction::I64Const(*n));
                }
                _ => {
                    func.instruction(&Instruction::I32Const(0));
                }
            },
            CHIRExprKind::Float(f) => {
                func.instruction(&Instruction::F64Const(*f));
            }
            CHIRExprKind::Float32(f) => {
                func.instruction(&Instruction::F32Const(*f));
            }
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
                // If the actual WASM local type differs from expr.wasm_ty, cast
                if let Some(&actual_ty) = self.current_local_types.borrow().get(idx) {
                    if actual_ty != expr.wasm_ty {
                        self.emit_cast(actual_ty, expr.wasm_ty, func);
                    }
                }
            }
            CHIRExprKind::Global(_name) => {
                // 全局变量简化：读堆指针占位
                func.instruction(&Instruction::GlobalGet(0));
                // GlobalGet(0) always produces I32; cast if expr expects I64
                if expr.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I64ExtendI32S);
                }
            }

            CHIRExprKind::Binary { op, left, right } => {
                // 确定操作所需的操作数类型：
                // - 算术/位操作：使用结果类型（expr.wasm_ty）
                // - 比较操作：使用操作数类型（left.wasm_ty），结果为 I32 (bool)
                let operand_ty = match op {
                    BinOp::Eq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::LtEq
                    | BinOp::Gt
                    | BinOp::GtEq => left.wasm_ty,
                    // 逻辑 And/Or 始终使用 i32 操作数
                    BinOp::LogicalAnd | BinOp::LogicalOr => ValType::I32,
                    _ => expr.wasm_ty,
                };
                self.emit_expr(left, func);
                // 若操作数为 void（Unit/Nothing），补零值作为默认值
                if !self.expr_produces_wasm_value_ctx(left) {
                    emit_zero(operand_ty, func);
                } else if left.wasm_ty != operand_ty {
                    self.emit_cast(left.wasm_ty, operand_ty, func);
                }
                self.emit_expr(right, func);
                if !self.expr_produces_wasm_value_ctx(right) {
                    emit_zero(operand_ty, func);
                } else if right.wasm_ty != operand_ty {
                    self.emit_cast(right.wasm_ty, operand_ty, func);
                }
                self.emit_binary_op(op, operand_ty, func);
                // Binary op result type: comparisons/logical produce I32, arithmetic produces operand_ty
                let result_ty = match op {
                    BinOp::Eq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::LtEq
                    | BinOp::Gt
                    | BinOp::GtEq
                    | BinOp::LogicalAnd
                    | BinOp::LogicalOr => ValType::I32,
                    _ => operand_ty,
                };
                if result_ty != expr.wasm_ty {
                    self.emit_cast(result_ty, expr.wasm_ty, func);
                }
            }
            CHIRExprKind::Unary { op, expr: inner } => {
                if matches!(op, UnaryOp::Neg) {
                    // Neg: 需要 0 在栈底，inner 在栈顶，然后 sub
                    match expr.wasm_ty {
                        ValType::F64 => {
                            self.emit_expr(inner, func);
                            func.instruction(&Instruction::F64Neg);
                        }
                        ValType::F32 => {
                            self.emit_expr(inner, func);
                            func.instruction(&Instruction::F32Neg);
                        }
                        ValType::I64 => {
                            func.instruction(&Instruction::I64Const(0));
                            self.emit_expr(inner, func);
                            if !self.expr_produces_wasm_value_ctx(inner) {
                                emit_zero(ValType::I64, func);
                            }
                            func.instruction(&Instruction::I64Sub);
                        }
                        _ => {
                            func.instruction(&Instruction::I32Const(0));
                            self.emit_expr(inner, func);
                            if !self.expr_produces_wasm_value_ctx(inner) {
                                emit_zero(ValType::I32, func);
                            }
                            func.instruction(&Instruction::I32Sub);
                        }
                    }
                } else {
                    self.emit_expr(inner, func);
                    if !self.expr_produces_wasm_value_ctx(inner) {
                        emit_zero(ValType::I32, func);
                    } else if matches!(op, UnaryOp::Not) && inner.wasm_ty == ValType::I64 {
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    self.emit_unary_op(op, expr.wasm_ty, func);
                }
            }

            CHIRExprKind::Call { func_idx, args } => {
                let expected_param_tys = self.func_param_types.get(func_idx);
                // Only emit as many args as the function expects (truncate excess)
                let emit_count = if let Some(pts) = expected_param_tys {
                    args.len().min(pts.len())
                } else {
                    args.len()
                };
                for (i, arg) in args.iter().take(emit_count).enumerate() {
                    self.emit_expr(arg, func);
                    let produces = self.expr_produces_wasm_value_ctx(arg);
                    let arg_ty = if !produces {
                        let target = expected_param_tys
                            .and_then(|pts| pts.get(i).copied())
                            .unwrap_or(arg.wasm_ty);
                        emit_zero(target, func);
                        target
                    } else {
                        arg.wasm_ty
                    };
                    if let Some(expected_ty) =
                        expected_param_tys.and_then(|pts| pts.get(i).copied())
                    {
                        if arg_ty != expected_ty {
                            self.emit_cast(arg_ty, expected_ty, func);
                        }
                    }
                }
                // 补齐缺失的参数（重载解析 arity 不匹配时）
                if let Some(pts) = expected_param_tys {
                    for i in emit_count..pts.len() {
                        emit_zero(pts[i], func);
                    }
                }
                func.instruction(&Instruction::Call(*func_idx));
            }

            CHIRExprKind::MethodCall {
                func_idx,
                vtable_offset: _,
                receiver,
                args,
            } => {
                self.emit_expr(receiver, func);
                let expected_param_tys = func_idx.and_then(|idx| self.func_param_types.get(&idx));
                // Coerce receiver to expected param 0 type
                let recv_ty = if !self.expr_produces_wasm_value_ctx(receiver) {
                    let target = expected_param_tys
                        .and_then(|pts| pts.get(0).copied())
                        .unwrap_or(receiver.wasm_ty);
                    emit_zero(target, func);
                    target
                } else {
                    receiver.wasm_ty
                };
                if let Some(expected_ty) = expected_param_tys.and_then(|pts| pts.get(0).copied()) {
                    if recv_ty != expected_ty {
                        self.emit_cast(recv_ty, expected_ty, func);
                    }
                }
                for (i, arg) in args.iter().enumerate() {
                    let param_i = i + 1; // skip receiver (param 0 = this)
                    self.emit_expr(arg, func);
                    let arg_ty = if !self.expr_produces_wasm_value_ctx(arg) {
                        let target = expected_param_tys
                            .and_then(|pts| pts.get(param_i).copied())
                            .unwrap_or(arg.wasm_ty);
                        emit_zero(target, func);
                        target
                    } else {
                        arg.wasm_ty
                    };
                    if let Some(expected_ty) =
                        expected_param_tys.and_then(|pts| pts.get(param_i).copied())
                    {
                        if arg_ty != expected_ty {
                            self.emit_cast(arg_ty, expected_ty, func);
                        }
                    }
                }
                // 补齐缺失的参数（receiver 已占 param 0，args 从 param 1 开始）
                if let Some(pts) = expected_param_tys {
                    for i in (args.len() + 1)..pts.len() {
                        emit_zero(pts[i], func);
                    }
                }
                if let Some(idx) = func_idx {
                    func.instruction(&Instruction::Call(*idx));
                }
            }

            CHIRExprKind::CallIndirect {
                type_idx: _,
                args,
                callee,
            } => {
                for arg in args {
                    self.emit_expr(arg, func);
                    if !self.expr_produces_wasm_value_ctx(arg) {
                        emit_zero(arg.wasm_ty, func);
                    }
                }
                self.emit_expr(callee, func);
                if !self.expr_produces_wasm_value_ctx(callee) {
                    emit_zero(ValType::I32, func);
                } else if callee.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                // Build type from actual arg types
                let wasm_params: Vec<ValType> = args.iter().map(|a| a.wasm_ty).collect();
                let wasm_results = if matches!(expr.ty, Type::Unit | Type::Nothing) {
                    vec![]
                } else {
                    vec![expr.wasm_ty]
                };
                let type_idx = self.find_or_create_func_type_idx(&wasm_params, &wasm_results);
                func.instruction(&Instruction::CallIndirect {
                    type_index: type_idx,
                    table_index: 0,
                });
            }

            CHIRExprKind::Cast {
                expr: inner,
                from_ty,
                to_ty,
            } => {
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

            CHIRExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.emit_expr(cond, func);
                if !self.expr_produces_wasm_value_ctx(cond) {
                    func.instruction(&Instruction::I32Const(0));
                } else if Self::cond_needs_i32_wrap(cond) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                // 使用 expr_produces_wasm_value_ctx 统一判断，与 stmt 层一致
                let block_type = if self.expr_produces_wasm_value_ctx(expr) {
                    BlockType::Result(expr.wasm_ty)
                } else {
                    BlockType::Empty
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

            CHIRExprKind::FieldGet {
                object,
                field_offset,
                ..
            } => {
                self.emit_expr(object, func);
                if !self.expr_produces_wasm_value_ctx(object) {
                    emit_zero(ValType::I32, func);
                } else if object.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Const(*field_offset as i32));
                func.instruction(&Instruction::I32Add);
                emit_load(expr.wasm_ty, func);
            }

            CHIRExprKind::ArrayGet { array, index } => {
                self.emit_expr(array, func);
                if !self.expr_produces_wasm_value_ctx(array) {
                    emit_zero(ValType::I32, func);
                } else if array.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(index, func);
                if !self.expr_produces_wasm_value_ctx(index) {
                    emit_zero(ValType::I32, func);
                } else if index.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                let elem_size = wasm_ty_bytes(expr.wasm_ty) as i32;
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                emit_load(expr.wasm_ty, func);
            }

            CHIRExprKind::TupleNew { elements } => {
                let alloc_idx = self.func_indices.get(RT_ALLOC).copied().unwrap_or(0);
                let elem_count = elements.len();
                let total_size = (elem_count * 8) as i32;
                const TUPLE_SAVE: i32 = 48;
                func.instruction(&Instruction::I32Const(TUPLE_SAVE));
                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::I32Const(TUPLE_SAVE));
                    func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::I32Const((i * 8) as i32));
                    func.instruction(&Instruction::I32Add);
                    self.emit_expr(elem, func);
                    // Store as 8-byte value
                    match elem.wasm_ty {
                        ValType::I32 => {
                            func.instruction(&Instruction::I64ExtendI32S);
                            func.instruction(&Instruction::I64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        ValType::F64 => {
                            func.instruction(&Instruction::F64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        _ => {
                            func.instruction(&Instruction::I64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                    }
                }
                func.instruction(&Instruction::I32Const(TUPLE_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }

            CHIRExprKind::TupleGet { tuple, index } => {
                self.emit_expr(tuple, func);
                if !self.expr_produces_wasm_value_ctx(tuple) {
                    emit_zero(ValType::I32, func);
                } else if tuple.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
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
                                if *newline {
                                    RT_PRINTLN_BOOL
                                } else {
                                    RT_PRINT_BOOL
                                }
                            }
                            (Type::String, _) => {
                                if *newline {
                                    RT_PRINTLN_STR
                                } else {
                                    RT_PRINT_STR
                                }
                            }
                            (_, ValType::I64) => {
                                if *newline {
                                    RT_PRINTLN_I64
                                } else {
                                    RT_PRINT_I64
                                }
                            }
                            (_, ValType::F64) => {
                                // Float64：暂时用 i64 版本（截断取整后打印）
                                func.instruction(&Instruction::I64TruncF64S);
                                if *newline {
                                    RT_PRINTLN_I64
                                } else {
                                    RT_PRINT_I64
                                }
                            }
                            (_, ValType::F32) => {
                                // Float32：先提升到 f64，再截断到 i64
                                func.instruction(&Instruction::F64PromoteF32);
                                func.instruction(&Instruction::I64TruncF64S);
                                if *newline {
                                    RT_PRINTLN_I64
                                } else {
                                    RT_PRINT_I64
                                }
                            }
                            _ => {
                                // I32 整型：扩展到 I64 后使用 i64 版本
                                func.instruction(&Instruction::I64ExtendI32S);
                                if *newline {
                                    RT_PRINTLN_I64
                                } else {
                                    RT_PRINT_I64
                                }
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

            CHIRExprKind::MathUnary { op, arg } => {
                self.emit_expr(arg, func);
                match op.as_str() {
                    "sqrt" => {
                        func.instruction(&Instruction::F64Sqrt);
                    }
                    "floor" => {
                        func.instruction(&Instruction::F64Floor);
                    }
                    "ceil" => {
                        func.instruction(&Instruction::F64Ceil);
                    }
                    "trunc" => {
                        func.instruction(&Instruction::F64Trunc);
                    }
                    "nearest" => {
                        func.instruction(&Instruction::F64Nearest);
                    }
                    "abs" => {
                        func.instruction(&Instruction::F64Abs);
                    }
                    "sin" | "cos" | "tan" | "exp" | "log" => {
                        let idx = self.func_indices[op.as_str()];
                        func.instruction(&Instruction::Call(idx));
                    }
                    _ => {}
                }
            }
            CHIRExprKind::MathBinary { op, left, right } => match op.as_str() {
                "pow" => {
                    let idx = self.func_indices["pow"];
                    self.emit_expr(left, func);
                    self.emit_expr(right, func);
                    func.instruction(&Instruction::Call(idx));
                }
                _ => {
                    self.emit_expr(left, func);
                    self.emit_expr(right, func);
                    func.instruction(&Instruction::F64Add);
                }
            },

            CHIRExprKind::BuiltinAbs { val, tmp_local } => {
                // abs(x): local.set tmp; if (tmp < 0) { -tmp } else { tmp }
                self.emit_expr(val, func);
                func.instruction(&Instruction::LocalSet(*tmp_local));
                func.instruction(&Instruction::LocalGet(*tmp_local));
                func.instruction(&Instruction::I64Const(0));
                func.instruction(&Instruction::I64LtS);
                func.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
                func.instruction(&Instruction::I64Const(0));
                func.instruction(&Instruction::LocalGet(*tmp_local));
                func.instruction(&Instruction::I64Sub);
                func.instruction(&Instruction::Else);
                func.instruction(&Instruction::LocalGet(*tmp_local));
                func.instruction(&Instruction::End);
            }
            CHIRExprKind::BuiltinCompareTo { left, right } => {
                // compareTo: if left < right { -1 } elif left > right { 1 } else { 0 }
                self.emit_expr(left, func);
                self.emit_expr(right, func);
                func.instruction(&Instruction::I64LtS);
                func.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
                func.instruction(&Instruction::I64Const(-1));
                func.instruction(&Instruction::Else);
                self.emit_expr(left, func);
                self.emit_expr(right, func);
                func.instruction(&Instruction::I64GtS);
                func.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
                func.instruction(&Instruction::I64Const(1));
                func.instruction(&Instruction::Else);
                func.instruction(&Instruction::I64Const(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            CHIRExprKind::BuiltinStringIsEmpty { val } => {
                // isEmpty: load string length (at offset 0) == 0
                self.emit_expr(val, func);
                func.instruction(&Instruction::I32Load(Self::mem(0, 2)));
                func.instruction(&Instruction::I32Eqz);
            }

            CHIRExprKind::Store {
                ptr,
                value,
                offset,
                align,
            } => {
                self.emit_expr(ptr, func);
                if !self.expr_produces_wasm_value_ctx(ptr) {
                    emit_zero(ValType::I32, func);
                } else if ptr.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                if *offset > 0 {
                    func.instruction(&Instruction::I32Const(*offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                self.emit_expr(value, func);
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                match value.wasm_ty {
                    ValType::I64 => func.instruction(&Instruction::I64Store(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    ValType::F64 => func.instruction(&Instruction::F64Store(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    ValType::F32 => func.instruction(&Instruction::F32Store(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    _ => func.instruction(&Instruction::I32Store(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                };
            }

            CHIRExprKind::Load { ptr, offset, align } => {
                self.emit_expr(ptr, func);
                if !self.expr_produces_wasm_value_ctx(ptr) {
                    emit_zero(ValType::I32, func);
                } else if ptr.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                if *offset > 0 {
                    func.instruction(&Instruction::I32Const(*offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                match expr.wasm_ty {
                    ValType::I64 => func.instruction(&Instruction::I64Load(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    ValType::F64 => func.instruction(&Instruction::F64Load(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    ValType::F32 => func.instruction(&Instruction::F32Load(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                    _ => func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: *align,
                        memory_index: 0,
                    })),
                };
            }

            CHIRExprKind::Nop => {
                if !matches!(expr.ty, Type::Unit | Type::Nothing) {
                    emit_zero(expr.wasm_ty, func);
                }
            }
            CHIRExprKind::Unreachable => {
                func.instruction(&Instruction::Unreachable);
            }
            CHIRExprKind::StructNew {
                struct_name,
                fields,
            } => {
                let alloc_idx = self.func_indices.get(RT_ALLOC).copied().unwrap_or(0);
                // 计算对象大小
                let obj_size = self
                    .class_object_sizes
                    .get(struct_name)
                    .copied()
                    .unwrap_or_else(|| {
                        self.struct_field_offsets.get(struct_name).map_or(16, |fs| {
                            fs.values()
                                .map(|(off, ty)| off + ty.size())
                                .max()
                                .unwrap_or(0)
                        })
                    });
                let obj_size = std::cmp::max(obj_size, 8);
                // 使用 IO buffer 保留区的 56-59 字节暂存分配的指针
                const PTR_SAVE: i32 = 56;
                // alloc → 暂存到 mem[56]
                func.instruction(&Instruction::I32Const(PTR_SAVE));
                func.instruction(&Instruction::I32Const(obj_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // 写入每个字段
                for (fname, fexpr) in fields {
                    let (field_offset, field_ty) = if let Some(info) = self
                        .class_field_offsets
                        .get(struct_name)
                        .and_then(|m| m.get(fname))
                    {
                        info.clone()
                    } else if let Some(info) = self
                        .struct_field_offsets
                        .get(struct_name)
                        .and_then(|m| m.get(fname))
                    {
                        info.clone()
                    } else {
                        (0, Type::Int64)
                    };
                    // ptr + offset
                    func.instruction(&Instruction::I32Const(PTR_SAVE));
                    func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    if field_offset > 0 {
                        func.instruction(&Instruction::I32Const(field_offset as i32));
                        func.instruction(&Instruction::I32Add);
                    }
                    self.emit_expr(fexpr, func);
                    let field_wasm = field_ty.to_wasm();
                    if fexpr.wasm_ty != field_wasm {
                        self.emit_cast(fexpr.wasm_ty, field_wasm, func);
                    }
                    emit_store_by_type(&field_ty, func);
                }
                // 将指针推入栈作为结果
                func.instruction(&Instruction::I32Const(PTR_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }

            CHIRExprKind::Match { subject, arms } => {
                self.emit_match(subject, arms, expr.wasm_ty, func);
            }

            CHIRExprKind::ArrayLiteral { elements } => {
                let alloc_idx = self.func_indices.get(RT_ALLOC).copied().unwrap_or(0);
                let elem_count = elements.len();
                let elem_size = 8u32; // i64 elements
                let total_size = std::cmp::max(4 + elem_count as u32 * elem_size, 8);
                const ARR_PTR_SAVE: i32 = 48;
                // alloc → save ptr
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // store length at offset 0
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                func.instruction(&Instruction::I32Const(elem_count as i32));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // store each element at offset 4 + i * elem_size
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                    func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    let elem_offset = 4 + i as u32 * elem_size;
                    func.instruction(&Instruction::I32Const(elem_offset as i32));
                    func.instruction(&Instruction::I32Add);
                    self.emit_expr(elem, func);
                    if elem.wasm_ty == ValType::I32 {
                        func.instruction(&Instruction::I64ExtendI32S);
                    }
                    func.instruction(&Instruction::I64Store(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                }
                // push array pointer as result
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }

            CHIRExprKind::FieldSet {
                object,
                field_offset,
                value,
            } => {
                self.emit_expr(object, func);
                if object.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                if *field_offset > 0 {
                    func.instruction(&Instruction::I32Const(*field_offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                self.emit_expr(value, func);
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                emit_store(value.wasm_ty, func);
                // FieldSet is Unit — no value produced
            }

            CHIRExprKind::ArraySet {
                array,
                index,
                value,
            } => {
                self.emit_expr(array, func);
                if array.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
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
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                emit_store(value.wasm_ty, func);
                // ArraySet is Unit — no value produced
            }

            CHIRExprKind::ArrayNew { len, init: _ } => {
                let alloc_idx = self.func_indices.get(RT_ALLOC).copied().unwrap_or(0);
                const ARR_PTR_SAVE: i32 = 48;
                // alloc: 4 (length header) + len * 8
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                self.emit_expr(len, func);
                if len.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Const(8));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // store length at offset 0
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                self.emit_expr(len, func);
                if len.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // push array pointer
                func.instruction(&Instruction::I32Const(ARR_PTR_SAVE));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
            }

            // 未实现的表达式：推入零值占位
            _ => {
                emit_zero(expr.wasm_ty, func);
            }
        }
    }

    /// void 上下文表达式 emit：结果会被丢弃，If 始终用 Empty block type
    fn emit_expr_void(&self, expr: &CHIRExpr, func: &mut wasm_encoder::Function) {
        match &expr.kind {
            CHIRExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.emit_expr(cond, func);
                if !self.expr_produces_wasm_value_ctx(cond) {
                    func.instruction(&Instruction::I32Const(0));
                } else if Self::cond_needs_i32_wrap(cond) {
                    func.instruction(&Instruction::I32WrapI64);
                }
                // void 上下文：始终用 Empty，分支不得产出值
                func.instruction(&Instruction::If(BlockType::Empty));
                let prev_depth = self.loop_break_depth.get();
                if prev_depth > 0 {
                    self.loop_break_depth.set(prev_depth + 1);
                }
                self.emit_block_void(then_block, func);
                if let Some(else_blk) = else_block {
                    func.instruction(&Instruction::Else);
                    self.emit_block_void(else_blk, func);
                }
                func.instruction(&Instruction::End);
                self.loop_break_depth.set(prev_depth);
            }
            CHIRExprKind::Block(block) => {
                self.emit_block_void(block, func);
            }
            CHIRExprKind::Match { subject, arms } => {
                // void 上下文的 match：emit subject store，然后 emit 每个 arm body 为 void
                // 简化处理：emit_expr 产生值后 Drop
                self.emit_expr(expr, func);
                func.instruction(&Instruction::Drop);
            }
            _ => {
                self.emit_expr(expr, func);
                // expr_produces_wasm_value_ctx 对 Unit 类型返回 false，
                // 但某些表达式（Binary、Match 等）总是产生 WASM 值
                if self.expr_produces_wasm_value_ctx(expr)
                    || Self::expr_kind_always_produces_value(&expr.kind)
                {
                    func.instruction(&Instruction::Drop);
                }
            }
        }
    }

    /// void 上下文语句 emit：所有表达式结果都被丢弃
    fn emit_stmt_void(&self, stmt: &CHIRStmt, func: &mut wasm_encoder::Function) {
        match stmt {
            CHIRStmt::Expr(expr) => {
                self.emit_expr_void(expr, func);
            }
            _ => {
                self.emit_stmt(stmt, func);
            }
        }
    }

    // ─── 语句 ──────────────────────────────────────────────────────────────

    fn emit_stmt(&self, stmt: &CHIRStmt, func: &mut wasm_encoder::Function) {
        match stmt {
            CHIRStmt::Let { local_idx, value } => {
                self.emit_expr(value, func);
                let produces = self.expr_produces_wasm_value_ctx(value);
                let val_ty = if !produces {
                    let local_ty = self
                        .current_local_types
                        .borrow()
                        .get(local_idx)
                        .copied()
                        .unwrap_or(value.wasm_ty);
                    emit_zero(local_ty, func);
                    local_ty
                } else {
                    value.wasm_ty
                };
                let local_ty_opt = self.current_local_types.borrow().get(local_idx).copied();
                if let Some(local_ty) = local_ty_opt {
                    if val_ty != local_ty {
                        self.emit_cast(val_ty, local_ty, func);
                    }
                }
                func.instruction(&Instruction::LocalSet(*local_idx));
            }

            CHIRStmt::Assign { target, value } => {
                self.emit_assign(target, value, func);
            }

            CHIRStmt::Expr(expr) => {
                self.emit_expr(expr, func);
                if self.expr_produces_wasm_value_ctx(expr)
                    || Self::expr_kind_always_produces_value(&expr.kind)
                {
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
                // Clamp depth to avoid invalid br instructions
                // In most cases, depth should be 1 (loop body) or 2 (loop body + 1 if block)
                let clamped_depth = if depth > 2 { 2 } else { depth };
                // If depth >= 2, the target block might be an if i32 block that expects a value
                // Push a zero value to satisfy the validator
                if depth >= 2 {
                    func.instruction(&Instruction::I32Const(0));
                }
                func.instruction(&Instruction::Br(if clamped_depth > 0 {
                    clamped_depth
                } else {
                    1
                }));
            }
            CHIRStmt::Continue => {
                // continue 目标是 loop 标签，比 break 的 block 标签浅 1 级
                let depth = self.loop_break_depth.get();
                func.instruction(&Instruction::Br(if depth > 1 { depth - 1 } else { 0 }));
            }

            CHIRStmt::While { cond, body } => {
                // block { loop { break_if(!cond); body; br 0 } }
                // break 直接在 loop body 内时深度为 1（退出 block）
                let prev_depth = self.loop_break_depth.get();
                self.loop_break_depth.set(1);
                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));
                self.emit_expr(cond, func);
                // 条件可能是 void（Unit Call），需补零值（false）避免 i32.eqz 空栈错误
                if !self.expr_produces_wasm_value_ctx(cond) {
                    func.instruction(&Instruction::I32Const(0));
                } else if Self::cond_needs_i32_wrap(cond) {
                    // 条件可能是 I64，需先截断到 I32，再用 I32Eqz 实现 `!cond`
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));
                self.emit_block_void(body, func);
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
                self.emit_block_void(body, func);
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // loop
                func.instruction(&Instruction::End); // block
                self.loop_break_depth.set(prev_depth);
            }
        }
    }

    fn emit_assign(
        &self,
        target: &CHIRLValue,
        value: &CHIRExpr,
        func: &mut wasm_encoder::Function,
    ) {
        match target {
            CHIRLValue::Local(idx) => {
                self.emit_expr(value, func);
                let val_ty = if !self.expr_produces_wasm_value_ctx(value) {
                    let local_ty = self
                        .current_local_types
                        .borrow()
                        .get(idx)
                        .copied()
                        .unwrap_or(value.wasm_ty);
                    emit_zero(local_ty, func);
                    local_ty
                } else {
                    value.wasm_ty
                };
                if let Some(&local_ty) = self.current_local_types.borrow().get(idx) {
                    if val_ty != local_ty {
                        self.emit_cast(val_ty, local_ty, func);
                    }
                }
                func.instruction(&Instruction::LocalSet(*idx));
            }
            CHIRLValue::Field { object, offset } => {
                self.emit_expr(object, func);
                if !self.expr_produces_wasm_value_ctx(object) {
                    emit_zero(ValType::I32, func);
                } else if object.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Const(*offset as i32));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(value, func);
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                emit_store(value.wasm_ty, func);
            }
            CHIRLValue::Index { array, index } => {
                self.emit_expr(array, func);
                if !self.expr_produces_wasm_value_ctx(array) {
                    emit_zero(ValType::I32, func);
                } else if array.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.emit_expr(index, func);
                if !self.expr_produces_wasm_value_ctx(index) {
                    emit_zero(ValType::I32, func);
                } else if index.wasm_ty == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                let elem_size = wasm_ty_bytes(value.wasm_ty) as i32;
                func.instruction(&Instruction::I32Const(elem_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                self.emit_expr(value, func);
                if !self.expr_produces_wasm_value_ctx(value) {
                    emit_zero(value.wasm_ty, func);
                }
                emit_store(value.wasm_ty, func);
            }
        }
    }

    // ─── Match ──────────────────────────────────────────────────────────────

    fn emit_match(
        &self,
        subject: &CHIRExpr,
        arms: &[crate::chir::CHIRMatchArm],
        result_ty: ValType,
        func: &mut wasm_encoder::Function,
    ) {
        use crate::chir::{CHIRLiteral, CHIRPattern};

        // 暂存 subject 到 mem[60] (IO buffer 保留区)
        const MATCH_SAVE: i32 = 60;
        func.instruction(&Instruction::I32Const(MATCH_SAVE));
        self.emit_expr(subject, func);
        match subject.wasm_ty {
            ValType::I64 => func.instruction(&Instruction::I64Store(MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            })),
            _ => func.instruction(&Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            })),
        };

        // 生成 if-else 链
        let has_result = result_ty != ValType::I32 || !arms.is_empty();
        let block_ty = if has_result {
            BlockType::Result(result_ty)
        } else {
            BlockType::Empty
        };

        // 生成 if-else 链
        let arm_count = arms.len();
        let match_base_depth = self.loop_break_depth.get();
        // Increment loop_break_depth once for the entire match
        // Each match arm's if-block adds one level of nesting
        if match_base_depth > 0 {
            self.loop_break_depth.set(match_base_depth + 1);
        }
        for (i, arm) in arms.iter().enumerate() {
            let is_last = i == arm_count - 1;
            match &arm.pattern {
                CHIRPattern::Wildcard | CHIRPattern::Binding(_) => {
                    if let CHIRPattern::Binding(local_idx) = &arm.pattern {
                        func.instruction(&Instruction::I32Const(MATCH_SAVE));
                        let loaded_ty = match subject.wasm_ty {
                            ValType::I64 => {
                                func.instruction(&Instruction::I64Load(MemArg {
                                    offset: 0,
                                    align: 3,
                                    memory_index: 0,
                                }));
                                ValType::I64
                            }
                            _ => {
                                func.instruction(&Instruction::I32Load(MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                ValType::I32
                            }
                        };
                        if let Some(&local_ty) = self.current_local_types.borrow().get(local_idx) {
                            if loaded_ty != local_ty {
                                self.emit_cast(loaded_ty, local_ty, func);
                            }
                        }
                        func.instruction(&Instruction::LocalSet(*local_idx));
                    }
                    if let Some(guard_expr) = &arm.guard {
                        self.emit_expr(guard_expr, func);
                        if !self.expr_produces_wasm_value_ctx(guard_expr) {
                            func.instruction(&Instruction::I32Const(0));
                        } else if guard_expr.wasm_ty == ValType::I64 {
                            func.instruction(&Instruction::I32WrapI64);
                        }
                        func.instruction(&Instruction::If(block_ty));
                        self.emit_block_with_ty(&arm.body, result_ty, func);
                        func.instruction(&Instruction::Else);
                        if is_last {
                            emit_zero(result_ty, func);
                        }
                    } else {
                        // Wildcard with no guard: always matches, emit body and stop
                        self.emit_block_with_ty(&arm.body, result_ty, func);
                        // Close all if-blocks opened by previous arms
                        for prev_arm in arms[..i].iter() {
                            let needs_end = match &prev_arm.pattern {
                                CHIRPattern::Wildcard => prev_arm.guard.is_some(),
                                CHIRPattern::Binding(_) => prev_arm.guard.is_some(),
                                CHIRPattern::Struct { fields } => {
                                    use crate::chir::StructPatternField;
                                    let has_lit = fields.iter().any(|f| {
                                        matches!(
                                            f,
                                            StructPatternField::Literal { .. }
                                                | StructPatternField::NestedLiteral { .. }
                                        )
                                    });
                                    has_lit || prev_arm.guard.is_some()
                                }
                                _ => true,
                            };
                            if needs_end {
                                func.instruction(&Instruction::End);
                            }
                        }
                        return; // no subsequent arms can match
                    }
                }
                CHIRPattern::Literal(lit) => {
                    // subject == literal ?
                    func.instruction(&Instruction::I32Const(MATCH_SAVE));
                    match subject.wasm_ty {
                        ValType::I64 => {
                            func.instruction(&Instruction::I64Load(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            match lit {
                                CHIRLiteral::Integer(n) => {
                                    func.instruction(&Instruction::I64Const(*n))
                                }
                                CHIRLiteral::Bool(b) => {
                                    func.instruction(&Instruction::I64Const(if *b { 1 } else { 0 }))
                                }
                                _ => func.instruction(&Instruction::I64Const(0)),
                            };
                            func.instruction(&Instruction::I64Eq);
                        }
                        _ => {
                            func.instruction(&Instruction::I32Load(MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                            match lit {
                                CHIRLiteral::Integer(n) => {
                                    func.instruction(&Instruction::I32Const(*n as i32))
                                }
                                CHIRLiteral::Bool(b) => {
                                    func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }))
                                }
                                _ => func.instruction(&Instruction::I32Const(0)),
                            };
                            func.instruction(&Instruction::I32Eq);
                        }
                    };
                    func.instruction(&Instruction::If(block_ty));
                    self.emit_block_with_ty(&arm.body, result_ty, func);
                    func.instruction(&Instruction::Else);
                    if is_last {
                        emit_zero(result_ty, func);
                    }
                }
                CHIRPattern::Variant {
                    discriminant,
                    payload_binding,
                    enum_has_payload,
                } => {
                    if *enum_has_payload {
                        // 枚举含关联值：subject 是指针，discriminant 在 offset 0
                        func.instruction(&Instruction::I32Const(MATCH_SAVE));
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                    } else {
                        // 简单枚举：subject 直接是 discriminant
                        func.instruction(&Instruction::I32Const(MATCH_SAVE));
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                    }
                    func.instruction(&Instruction::I32Const(*discriminant));
                    func.instruction(&Instruction::I32Eq);
                    func.instruction(&Instruction::If(block_ty));
                    // Increment loop_break_depth for the if block
                    let prev_depth = self.loop_break_depth.get();
                    if prev_depth > 0 {
                        self.loop_break_depth.set(prev_depth + 1);
                    }
                    if let Some(bind_idx) = payload_binding {
                        // 加载 payload（位于 ptr + 4）
                        func.instruction(&Instruction::I32Const(MATCH_SAVE));
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::I64Load(MemArg {
                            offset: 4,
                            align: 3,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::LocalSet(*bind_idx));
                    }
                    self.emit_block_with_ty(&arm.body, result_ty, func);
                    // Restore loop_break_depth
                    self.loop_break_depth.set(prev_depth);
                    func.instruction(&Instruction::Else);
                    if is_last {
                        emit_zero(result_ty, func);
                    }
                }
                CHIRPattern::Struct { fields } => {
                    use crate::chir::StructPatternField;
                    let has_condition = fields.iter().any(|f| {
                        matches!(
                            f,
                            StructPatternField::Literal { .. }
                                | StructPatternField::NestedLiteral { .. }
                        )
                    });
                    if !has_condition {
                        // All bindings — bind fields first
                        for f in fields {
                            Self::emit_struct_field_load(f, MATCH_SAVE, func);
                        }
                        if let Some(guard_expr) = &arm.guard {
                            self.emit_expr(guard_expr, func);
                            if !self.expr_produces_wasm_value_ctx(guard_expr) {
                                func.instruction(&Instruction::I32Const(0));
                            } else if guard_expr.wasm_ty == ValType::I64 {
                                func.instruction(&Instruction::I32WrapI64);
                            }
                            func.instruction(&Instruction::If(block_ty));
                            // Increment loop_break_depth for the if block
                            let prev_depth = self.loop_break_depth.get();
                            if prev_depth > 0 {
                                self.loop_break_depth.set(prev_depth + 1);
                            }
                            self.emit_block_with_ty(&arm.body, result_ty, func);
                            // Restore loop_break_depth
                            self.loop_break_depth.set(prev_depth);
                            func.instruction(&Instruction::Else);
                            if is_last {
                                emit_zero(result_ty, func);
                            }
                        } else {
                            self.emit_block_with_ty(&arm.body, result_ty, func);
                        }
                    } else {
                        // Emit condition: AND of all literal field checks
                        let mut ci = 0;
                        for f in fields {
                            match f {
                                StructPatternField::Literal {
                                    offset,
                                    value,
                                    wasm_ty,
                                } => {
                                    func.instruction(&Instruction::I32Const(MATCH_SAVE));
                                    func.instruction(&Instruction::I32Load(MemArg {
                                        offset: 0,
                                        align: 2,
                                        memory_index: 0,
                                    }));
                                    if *offset > 0 {
                                        func.instruction(&Instruction::I32Const(*offset as i32));
                                        func.instruction(&Instruction::I32Add);
                                    }
                                    Self::emit_load_and_compare(*wasm_ty, *value, func);
                                    if ci > 0 {
                                        func.instruction(&Instruction::I32And);
                                    }
                                    ci += 1;
                                }
                                StructPatternField::NestedLiteral {
                                    outer_offset,
                                    inner_offset,
                                    value,
                                    wasm_ty,
                                } => {
                                    // Load pointer at outer_offset, then compare value at inner_offset
                                    func.instruction(&Instruction::I32Const(MATCH_SAVE));
                                    func.instruction(&Instruction::I32Load(MemArg {
                                        offset: 0,
                                        align: 2,
                                        memory_index: 0,
                                    }));
                                    if *outer_offset > 0 {
                                        func.instruction(&Instruction::I32Const(
                                            *outer_offset as i32,
                                        ));
                                        func.instruction(&Instruction::I32Add);
                                    }
                                    func.instruction(&Instruction::I32Load(MemArg {
                                        offset: 0,
                                        align: 2,
                                        memory_index: 0,
                                    }));
                                    if *inner_offset > 0 {
                                        func.instruction(&Instruction::I32Const(
                                            *inner_offset as i32,
                                        ));
                                        func.instruction(&Instruction::I32Add);
                                    }
                                    Self::emit_load_and_compare(*wasm_ty, *value, func);
                                    if ci > 0 {
                                        func.instruction(&Instruction::I32And);
                                    }
                                    ci += 1;
                                }
                                _ => {}
                            }
                        }
                        func.instruction(&Instruction::If(block_ty));
                        for f in fields {
                            Self::emit_struct_field_load(f, MATCH_SAVE, func);
                        }
                        self.emit_block_with_ty(&arm.body, result_ty, func);
                        func.instruction(&Instruction::Else);
                        if is_last {
                            emit_zero(result_ty, func);
                        }
                    }
                }
                CHIRPattern::Range {
                    start,
                    end,
                    inclusive,
                } => {
                    // subject >= start && subject < end (or <=)
                    func.instruction(&Instruction::I32Const(MATCH_SAVE));
                    match subject.wasm_ty {
                        ValType::I64 => {
                            func.instruction(&Instruction::I64Load(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::I64Const(*start));
                            func.instruction(&Instruction::I64GeS);
                            func.instruction(&Instruction::I32Const(MATCH_SAVE));
                            func.instruction(&Instruction::I64Load(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::I64Const(*end));
                            if *inclusive {
                                func.instruction(&Instruction::I64LeS);
                            } else {
                                func.instruction(&Instruction::I64LtS);
                            }
                        }
                        _ => {
                            func.instruction(&Instruction::I32Load(MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::I32Const(*start as i32));
                            func.instruction(&Instruction::I32GeS);
                            func.instruction(&Instruction::I32Const(MATCH_SAVE));
                            func.instruction(&Instruction::I32Load(MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                            func.instruction(&Instruction::I32Const(*end as i32));
                            if *inclusive {
                                func.instruction(&Instruction::I32LeS);
                            } else {
                                func.instruction(&Instruction::I32LtS);
                            }
                        }
                    };
                    func.instruction(&Instruction::I32And);
                    func.instruction(&Instruction::If(block_ty));
                    self.emit_block_with_ty(&arm.body, result_ty, func);
                    func.instruction(&Instruction::Else);
                    if is_last {
                        emit_zero(result_ty, func);
                    }
                }
            }
        }
        // Restore loop_break_depth after match
        if match_base_depth > 0 {
            self.loop_break_depth.set(match_base_depth);
        }
        // 关闭所有需要 End 的 if-else
        for arm in arms.iter() {
            let needs_end = match &arm.pattern {
                CHIRPattern::Wildcard => arm.guard.is_some(),
                CHIRPattern::Binding(_) => arm.guard.is_some(),
                CHIRPattern::Struct { fields } => {
                    use crate::chir::StructPatternField;
                    let has_lit = fields.iter().any(|f| {
                        matches!(
                            f,
                            StructPatternField::Literal { .. }
                                | StructPatternField::NestedLiteral { .. }
                        )
                    });
                    has_lit || arm.guard.is_some()
                }
                _ => true,
            };
            if needs_end {
                func.instruction(&Instruction::End);
            }
        }
    }

    // ─── 运算符 ────────────────────────────────────────────────────────────

    fn emit_load_and_compare(wasm_ty: ValType, value: i64, func: &mut wasm_encoder::Function) {
        match wasm_ty {
            ValType::I64 => {
                func.instruction(&Instruction::I64Load(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                func.instruction(&Instruction::I64Const(value));
                func.instruction(&Instruction::I64Eq);
            }
            _ => {
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                func.instruction(&Instruction::I32Const(value as i32));
                func.instruction(&Instruction::I32Eq);
            }
        };
    }

    fn emit_struct_field_load(
        f: &crate::chir::StructPatternField,
        save_addr: i32,
        func: &mut wasm_encoder::Function,
    ) {
        use crate::chir::StructPatternField;
        match f {
            StructPatternField::Binding {
                offset,
                local_idx,
                wasm_ty,
            } => {
                func.instruction(&Instruction::I32Const(save_addr));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                if *offset > 0 {
                    func.instruction(&Instruction::I32Const(*offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                match wasm_ty {
                    ValType::I64 => func.instruction(&Instruction::I64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => func.instruction(&Instruction::F64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    _ => func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    })),
                };
                func.instruction(&Instruction::LocalSet(*local_idx));
            }
            StructPatternField::NestedBinding {
                outer_offset,
                inner_offset,
                local_idx,
                wasm_ty,
            } => {
                func.instruction(&Instruction::I32Const(save_addr));
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                if *outer_offset > 0 {
                    func.instruction(&Instruction::I32Const(*outer_offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                func.instruction(&Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                if *inner_offset > 0 {
                    func.instruction(&Instruction::I32Const(*inner_offset as i32));
                    func.instruction(&Instruction::I32Add);
                }
                match wasm_ty {
                    ValType::I64 => func.instruction(&Instruction::I64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => func.instruction(&Instruction::F64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    })),
                    _ => func.instruction(&Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    })),
                };
                func.instruction(&Instruction::LocalSet(*local_idx));
            }
            _ => {} // Literal fields are only used in conditions, not bindings
        }
    }

    fn emit_binary_op(&self, op: &BinOp, ty: ValType, func: &mut wasm_encoder::Function) {
        match (op, ty) {
            (BinOp::Add, ValType::I32) => {
                func.instruction(&Instruction::I32Add);
            }
            (BinOp::Sub, ValType::I32) => {
                func.instruction(&Instruction::I32Sub);
            }
            (BinOp::Mul, ValType::I32) => {
                func.instruction(&Instruction::I32Mul);
            }
            (BinOp::Div, ValType::I32) => {
                func.instruction(&Instruction::I32DivS);
            }
            (BinOp::Mod, ValType::I32) => {
                func.instruction(&Instruction::I32RemS);
            }
            (BinOp::BitAnd, ValType::I32) => {
                func.instruction(&Instruction::I32And);
            }
            (BinOp::BitOr, ValType::I32) => {
                func.instruction(&Instruction::I32Or);
            }
            (BinOp::BitXor, ValType::I32) => {
                func.instruction(&Instruction::I32Xor);
            }
            (BinOp::Shl, ValType::I32) => {
                func.instruction(&Instruction::I32Shl);
            }
            (BinOp::Shr, ValType::I32) => {
                func.instruction(&Instruction::I32ShrS);
            }
            (BinOp::Eq, ValType::I32) => {
                func.instruction(&Instruction::I32Eq);
            }
            (BinOp::NotEq, ValType::I32) => {
                func.instruction(&Instruction::I32Ne);
            }
            (BinOp::Lt, ValType::I32) => {
                func.instruction(&Instruction::I32LtS);
            }
            (BinOp::LtEq, ValType::I32) => {
                func.instruction(&Instruction::I32LeS);
            }
            (BinOp::Gt, ValType::I32) => {
                func.instruction(&Instruction::I32GtS);
            }
            (BinOp::GtEq, ValType::I32) => {
                func.instruction(&Instruction::I32GeS);
            }

            (BinOp::Add, ValType::I64) => {
                func.instruction(&Instruction::I64Add);
            }
            (BinOp::Sub, ValType::I64) => {
                func.instruction(&Instruction::I64Sub);
            }
            (BinOp::Mul, ValType::I64) => {
                func.instruction(&Instruction::I64Mul);
            }
            (BinOp::Div, ValType::I64) => {
                func.instruction(&Instruction::I64DivS);
            }
            (BinOp::Mod, ValType::I64) => {
                func.instruction(&Instruction::I64RemS);
            }
            (BinOp::BitAnd, ValType::I64) => {
                func.instruction(&Instruction::I64And);
            }
            (BinOp::BitOr, ValType::I64) => {
                func.instruction(&Instruction::I64Or);
            }
            (BinOp::BitXor, ValType::I64) => {
                func.instruction(&Instruction::I64Xor);
            }
            (BinOp::Shl, ValType::I64) => {
                func.instruction(&Instruction::I64Shl);
            }
            (BinOp::Shr, ValType::I64) => {
                func.instruction(&Instruction::I64ShrS);
            }
            (BinOp::Eq, ValType::I64) => {
                func.instruction(&Instruction::I64Eq);
            }
            (BinOp::NotEq, ValType::I64) => {
                func.instruction(&Instruction::I64Ne);
            }
            (BinOp::Lt, ValType::I64) => {
                func.instruction(&Instruction::I64LtS);
            }
            (BinOp::LtEq, ValType::I64) => {
                func.instruction(&Instruction::I64LeS);
            }
            (BinOp::Gt, ValType::I64) => {
                func.instruction(&Instruction::I64GtS);
            }
            (BinOp::GtEq, ValType::I64) => {
                func.instruction(&Instruction::I64GeS);
            }

            (BinOp::Add, ValType::F64) => {
                func.instruction(&Instruction::F64Add);
            }
            (BinOp::Sub, ValType::F64) => {
                func.instruction(&Instruction::F64Sub);
            }
            (BinOp::Mul, ValType::F64) => {
                func.instruction(&Instruction::F64Mul);
            }
            (BinOp::Div, ValType::F64) => {
                func.instruction(&Instruction::F64Div);
            }
            (BinOp::Eq, ValType::F64) => {
                func.instruction(&Instruction::F64Eq);
            }
            (BinOp::NotEq, ValType::F64) => {
                func.instruction(&Instruction::F64Ne);
            }
            (BinOp::Lt, ValType::F64) => {
                func.instruction(&Instruction::F64Lt);
            }
            (BinOp::LtEq, ValType::F64) => {
                func.instruction(&Instruction::F64Le);
            }
            (BinOp::Gt, ValType::F64) => {
                func.instruction(&Instruction::F64Gt);
            }
            (BinOp::GtEq, ValType::F64) => {
                func.instruction(&Instruction::F64Ge);
            }

            (BinOp::Add, ValType::F32) => {
                func.instruction(&Instruction::F32Add);
            }
            (BinOp::Sub, ValType::F32) => {
                func.instruction(&Instruction::F32Sub);
            }
            (BinOp::Mul, ValType::F32) => {
                func.instruction(&Instruction::F32Mul);
            }
            (BinOp::Div, ValType::F32) => {
                func.instruction(&Instruction::F32Div);
            }

            // 逻辑 And/Or：操作数已经是 i32，直接 And/Or
            (BinOp::LogicalAnd, _) => {
                func.instruction(&Instruction::I32And);
            }
            (BinOp::LogicalOr, _) => {
                func.instruction(&Instruction::I32Or);
            }

            _ => {}
        }
    }

    fn emit_unary_op(&self, op: &UnaryOp, ty: ValType, func: &mut wasm_encoder::Function) {
        match (op, ty) {
            (UnaryOp::Not, _) => {
                func.instruction(&Instruction::I32Eqz);
            }
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
            (UnaryOp::Neg, ValType::F64) => {
                func.instruction(&Instruction::F64Neg);
            }
            (UnaryOp::Neg, ValType::F32) => {
                func.instruction(&Instruction::F32Neg);
            }
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
            (ValType::I64, ValType::I32) => {
                func.instruction(&Instruction::I32WrapI64);
            }
            (ValType::I32, ValType::I64) => {
                func.instruction(&Instruction::I64ExtendI32S);
            }
            (ValType::I64, ValType::F64) => {
                func.instruction(&Instruction::F64ConvertI64S);
            }
            (ValType::I32, ValType::F64) => {
                func.instruction(&Instruction::F64ConvertI32S);
            }
            (ValType::I64, ValType::F32) => {
                func.instruction(&Instruction::F32ConvertI64S);
            }
            (ValType::I32, ValType::F32) => {
                func.instruction(&Instruction::F32ConvertI32S);
            }
            (ValType::F64, ValType::I64) => {
                func.instruction(&Instruction::I64TruncF64S);
            }
            (ValType::F64, ValType::I32) => {
                func.instruction(&Instruction::I32TruncF64S);
            }
            (ValType::F32, ValType::I64) => {
                func.instruction(&Instruction::I64TruncF32S);
            }
            (ValType::F32, ValType::I32) => {
                func.instruction(&Instruction::I32TruncF32S);
            }
            (ValType::F32, ValType::F64) => {
                func.instruction(&Instruction::F64PromoteF32);
            }
            (ValType::F64, ValType::F32) => {
                func.instruction(&Instruction::F32DemoteF64);
            }
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

/// 检查块是否包含显式 return 语句（递归检查嵌套 Block/If）
fn block_has_return(block: &CHIRBlock) -> bool {
    for s in &block.stmts {
        if matches!(s, CHIRStmt::Return(_)) {
            return true;
        }
        if let CHIRStmt::Expr(expr) = s {
            if expr_has_return(expr) {
                return true;
            }
        }
    }
    if let Some(ref result) = block.result {
        if expr_has_return(result) {
            return true;
        }
    }
    false
}

fn expr_has_return(expr: &CHIRExpr) -> bool {
    match &expr.kind {
        CHIRExprKind::Block(b) => block_has_return(b),
        CHIRExprKind::If {
            then_block,
            else_block,
            ..
        } => block_has_return(then_block) || else_block.as_ref().map_or(false, block_has_return),
        _ => false,
    }
}

/// 推入类型对应的零值
fn emit_zero(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => {
            func.instruction(&Instruction::I32Const(0));
        }
        ValType::I64 => {
            func.instruction(&Instruction::I64Const(0));
        }
        ValType::F32 => {
            func.instruction(&Instruction::F32Const(0.0));
        }
        ValType::F64 => {
            func.instruction(&Instruction::F64Const(0.0));
        }
        _ => {}
    }
}

/// 生成 load 指令
fn emit_load(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => {
            func.instruction(&Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        }
        ValType::I64 => {
            func.instruction(&Instruction::I64Load(MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }));
        }
        ValType::F32 => {
            func.instruction(&Instruction::F32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        }
        ValType::F64 => {
            func.instruction(&Instruction::F64Load(MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }));
        }
        _ => {}
    }
}

/// 生成 store 指令
fn emit_store(ty: ValType, func: &mut wasm_encoder::Function) {
    match ty {
        ValType::I32 => {
            func.instruction(&Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        }
        ValType::I64 => {
            func.instruction(&Instruction::I64Store(MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }));
        }
        ValType::F32 => {
            func.instruction(&Instruction::F32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        }
        ValType::F64 => {
            func.instruction(&Instruction::F64Store(MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }));
        }
        _ => {}
    }
}

/// 按 AST 类型生成 store 指令
fn emit_store_by_type(ty: &Type, func: &mut wasm_encoder::Function) {
    emit_store(ty.to_wasm(), func);
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
        CHIRExprKind::If {
            then_block,
            else_block,
            ..
        } => {
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
        let gap = match prev_idx {
            Some(p) => idx.saturating_sub(p + 1),
            None => idx.saturating_sub(param_count),
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chir::{
        CHIRBlock, CHIRFunction, CHIRGlobal, CHIRLValue, CHIRMatchArm, CHIRParam, CHIRPattern,
        CHIRProgram,
    };

    fn make_func(
        name: &str,
        params: Vec<CHIRParam>,
        return_ty: Type,
        body: CHIRBlock,
    ) -> CHIRFunction {
        let return_wasm_ty = match &return_ty {
            Type::Unit | Type::Nothing => ValType::I32,
            t => t.to_wasm(),
        };
        CHIRFunction {
            name: name.into(),
            params,
            return_ty,
            return_wasm_ty,
            locals: vec![],
            body,
            local_wasm_types: HashMap::new(),
        }
    }

    fn make_program(functions: Vec<CHIRFunction>) -> CHIRProgram {
        CHIRProgram {
            functions,
            structs: vec![],
            classes: vec![],
            enums: vec![],
            globals: vec![],
        }
    }

    fn validate_wasm(bytes: &[u8]) -> bool {
        // WASM magic number: \0asm (0x00 0x61 0x73 0x6d)
        bytes.len() >= 8 && bytes[0..4] == [0x00, 0x61, 0x73, 0x6d]
    }

    // ─── run_length_encode_locals ───

    #[test]
    fn test_rle_empty() {
        assert!(run_length_encode_locals(&[], 0).is_empty());
    }

    #[test]
    fn test_rle_consecutive_same_type() {
        let locals = vec![(0, ValType::I32), (1, ValType::I32), (2, ValType::I32)];
        let result = run_length_encode_locals(&locals, 0);
        assert_eq!(result, vec![(3, ValType::I32)]);
    }

    #[test]
    fn test_rle_different_types() {
        let locals = vec![(0, ValType::I32), (1, ValType::I64), (2, ValType::F64)];
        let result = run_length_encode_locals(&locals, 0);
        assert_eq!(
            result,
            vec![(1, ValType::I32), (1, ValType::I64), (1, ValType::F64)]
        );
    }

    #[test]
    fn test_rle_with_gap() {
        let locals = vec![(0, ValType::I64), (3, ValType::F64)];
        let result = run_length_encode_locals(&locals, 0);
        assert_eq!(
            result,
            vec![(1, ValType::I64), (2, ValType::I32), (1, ValType::F64)]
        );
    }

    #[test]
    fn test_rle_with_param_offset() {
        let locals = vec![(2, ValType::I64)];
        let result = run_length_encode_locals(&locals, 2);
        assert_eq!(result, vec![(1, ValType::I64)]);
    }

    // ─── collect_locals ───

    #[test]
    fn test_collect_locals_let() {
        let block = CHIRBlock {
            stmts: vec![CHIRStmt::Let {
                local_idx: 1,
                value: CHIRExpr::int_const(42, Type::Int64),
            }],
            result: None,
        };
        let mut out = vec![];
        collect_locals_from_block(&block, 1, &mut out);
        assert_eq!(out, vec![(1, ValType::I64)]);
    }

    #[test]
    fn test_collect_locals_nested_while() {
        let block = CHIRBlock {
            stmts: vec![CHIRStmt::While {
                cond: CHIRExpr::bool_const(true),
                body: CHIRBlock {
                    stmts: vec![CHIRStmt::Let {
                        local_idx: 2,
                        value: CHIRExpr::new(CHIRExprKind::Float(1.0), Type::Float64, ValType::F64),
                    }],
                    result: None,
                },
            }],
            result: None,
        };
        let mut out = vec![];
        collect_locals_from_block(&block, 0, &mut out);
        assert!(out.iter().any(|&(idx, ty)| idx == 2 && ty == ValType::F64));
    }

    // ─── expr_produces_wasm_value ───

    #[test]
    fn test_expr_produces_value_integer() {
        assert!(CHIRCodeGen::expr_produces_wasm_value(&CHIRExpr::int_const(
            42,
            Type::Int64
        )));
    }

    #[test]
    fn test_expr_produces_value_unit() {
        let expr = CHIRExpr::new(CHIRExprKind::Nop, Type::Unit, ValType::I32);
        assert!(!CHIRCodeGen::expr_produces_wasm_value(&expr));
    }

    #[test]
    fn test_expr_produces_value_print() {
        let expr = CHIRExpr::new(
            CHIRExprKind::Print {
                arg: None,
                newline: true,
                fd: 1,
            },
            Type::Unit,
            ValType::I32,
        );
        assert!(!CHIRCodeGen::expr_produces_wasm_value(&expr));
    }

    // ─── expr_produces_wasm_value_ctx ───

    #[test]
    fn test_expr_produces_value_ctx_void_call() {
        let mut gen = CHIRCodeGen::new();
        gen.func_void_map.insert(5, true);

        let call = CHIRExpr::new(
            CHIRExprKind::Call {
                func_idx: 5,
                args: vec![],
            },
            Type::Unit,
            ValType::I32,
        );
        assert!(!gen.expr_produces_wasm_value_ctx(&call));
    }

    #[test]
    fn test_expr_produces_value_ctx_non_void_call() {
        let mut gen = CHIRCodeGen::new();
        gen.func_void_map.insert(6, false);

        let call = CHIRExpr::new(
            CHIRExprKind::Call {
                func_idx: 6,
                args: vec![],
            },
            Type::Int64,
            ValType::I64,
        );
        assert!(gen.expr_produces_wasm_value_ctx(&call));
    }

    #[test]
    fn test_expr_produces_value_ctx_if() {
        let mut gen = CHIRCodeGen::new();
        let if_expr = CHIRExpr::new(
            CHIRExprKind::If {
                cond: Box::new(CHIRExpr::bool_const(true)),
                then_block: CHIRBlock::from_expr(CHIRExpr::int_const(1, Type::Int64)),
                else_block: Some(CHIRBlock::from_expr(CHIRExpr::int_const(2, Type::Int64))),
            },
            Type::Int64,
            ValType::I64,
        );
        assert!(gen.expr_produces_wasm_value_ctx(&if_expr));
    }

    #[test]
    fn test_expr_produces_value_ctx_block() {
        let mut gen = CHIRCodeGen::new();
        let block_expr = CHIRExpr::new(
            CHIRExprKind::Block(CHIRBlock::from_expr(CHIRExpr::bool_const(true))),
            Type::Bool,
            ValType::I32,
        );
        assert!(gen.expr_produces_wasm_value_ctx(&block_expr));
    }

    #[test]
    fn test_expr_produces_value_ctx_empty_block() {
        let mut gen = CHIRCodeGen::new();
        let block_expr = CHIRExpr::new(
            CHIRExprKind::Block(CHIRBlock::empty()),
            Type::Bool,
            ValType::I32,
        );
        assert!(!gen.expr_produces_wasm_value_ctx(&block_expr));
    }

    // ─── 端到端生成 ───

    #[test]
    fn test_generate_empty_program() {
        let prog = make_program(vec![]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_simple_return() {
        let body = CHIRBlock {
            stmts: vec![CHIRStmt::Return(Some(CHIRExpr::int_const(42, Type::Int64)))],
            result: None,
        };
        let func = make_func("main", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_void_function() {
        let body = CHIRBlock {
            stmts: vec![CHIRStmt::Return(None)],
            result: None,
        };
        let func = make_func("doNothing", vec![], Type::Unit, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_with_locals() {
        let body = CHIRBlock {
            stmts: vec![
                CHIRStmt::Let {
                    local_idx: 0,
                    value: CHIRExpr::int_const(10, Type::Int64),
                },
                CHIRStmt::Return(Some(CHIRExpr::new(
                    CHIRExprKind::Local(0),
                    Type::Int64,
                    ValType::I64,
                ))),
            ],
            result: None,
        };
        let func = make_func("test", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_binary_expr() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(CHIRExpr::int_const(1, Type::Int64)),
                    right: Box::new(CHIRExpr::int_const(2, Type::Int64)),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("add", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_if_expr() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::If {
                    cond: Box::new(CHIRExpr::bool_const(true)),
                    then_block: CHIRBlock::from_expr(CHIRExpr::int_const(1, Type::Int64)),
                    else_block: Some(CHIRBlock::from_expr(CHIRExpr::int_const(2, Type::Int64))),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("test_if", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_void_if_in_void_func() {
        let body = CHIRBlock {
            stmts: vec![CHIRStmt::Expr(CHIRExpr::new(
                CHIRExprKind::If {
                    cond: Box::new(CHIRExpr::bool_const(true)),
                    then_block: CHIRBlock::from_expr(CHIRExpr::int_const(1, Type::Int64)),
                    else_block: Some(CHIRBlock::from_expr(CHIRExpr::int_const(2, Type::Int64))),
                },
                Type::Int64,
                ValType::I64,
            ))],
            result: None,
        };
        let func = make_func("test", vec![], Type::Unit, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_while_loop() {
        let body = CHIRBlock {
            stmts: vec![CHIRStmt::While {
                cond: CHIRExpr::bool_const(false),
                body: CHIRBlock {
                    stmts: vec![CHIRStmt::Break],
                    result: None,
                },
            }],
            result: None,
        };
        let func = make_func("test_while", vec![], Type::Unit, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_with_params() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Binary {
                    op: BinOp::Add,
                    left: Box::new(CHIRExpr::new(
                        CHIRExprKind::Local(0),
                        Type::Int64,
                        ValType::I64,
                    )),
                    right: Box::new(CHIRExpr::new(
                        CHIRExprKind::Local(1),
                        Type::Int64,
                        ValType::I64,
                    )),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func(
            "add",
            vec![
                CHIRParam {
                    name: "a".into(),
                    ty: Type::Int64,
                    wasm_ty: ValType::I64,
                    local_idx: 0,
                },
                CHIRParam {
                    name: "b".into(),
                    ty: Type::Int64,
                    wasm_ty: ValType::I64,
                    local_idx: 1,
                },
            ],
            Type::Int64,
            body,
        );
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_cast() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Cast {
                    expr: Box::new(CHIRExpr::int_const(42, Type::Int64)),
                    from_ty: ValType::I64,
                    to_ty: ValType::I32,
                },
                Type::Int32,
                ValType::I32,
            ))),
        };
        let func = make_func("cast_test", vec![], Type::Int32, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_unary() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(CHIRExpr::int_const(42, Type::Int64)),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("neg_test", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_multiple_funcs() {
        let f1 = make_func(
            "foo",
            vec![],
            Type::Int64,
            CHIRBlock {
                stmts: vec![],
                result: Some(Box::new(CHIRExpr::int_const(1, Type::Int64))),
            },
        );
        let f2_body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Call {
                    func_idx: 2, // foo is at IMPORT_COUNT + 0 = 2
                    args: vec![],
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let f2 = make_func("bar", vec![], Type::Int64, f2_body);
        let prog = make_program(vec![f1, f2]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_print() {
        let body = CHIRBlock {
            stmts: vec![CHIRStmt::Expr(CHIRExpr::new(
                CHIRExprKind::Print {
                    arg: Some(Box::new(CHIRExpr::int_const(42, Type::Int64))),
                    newline: true,
                    fd: 1,
                },
                Type::Unit,
                ValType::I32,
            ))],
            result: None,
        };
        let func = make_func("test_print", vec![], Type::Unit, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_intern_string() {
        let mut gen = CHIRCodeGen::new();
        let addr1 = gen.intern_string("hello");
        let addr2 = gen.intern_string("hello");
        let addr3 = gen.intern_string("world");
        assert_eq!(addr1, addr2);
        assert_ne!(addr1, addr3);
    }

    #[test]
    fn test_generate_assign_local() {
        let body = CHIRBlock {
            stmts: vec![
                CHIRStmt::Let {
                    local_idx: 0,
                    value: CHIRExpr::int_const(0, Type::Int64),
                },
                CHIRStmt::Assign {
                    target: CHIRLValue::Local(0),
                    value: CHIRExpr::int_const(42, Type::Int64),
                },
                CHIRStmt::Return(Some(CHIRExpr::new(
                    CHIRExprKind::Local(0),
                    Type::Int64,
                    ValType::I64,
                ))),
            ],
            result: None,
        };
        let func = make_func("test_assign", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_block_expr() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Block(CHIRBlock {
                    stmts: vec![],
                    result: Some(Box::new(CHIRExpr::int_const(99, Type::Int64))),
                }),
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("block_test", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_comparison_ops() {
        for op in &[
            BinOp::Lt,
            BinOp::Gt,
            BinOp::Eq,
            BinOp::NotEq,
            BinOp::LtEq,
            BinOp::GtEq,
        ] {
            let body = CHIRBlock {
                stmts: vec![],
                result: Some(Box::new(CHIRExpr::new(
                    CHIRExprKind::Binary {
                        op: op.clone(),
                        left: Box::new(CHIRExpr::int_const(1, Type::Int64)),
                        right: Box::new(CHIRExpr::int_const(2, Type::Int64)),
                    },
                    Type::Bool,
                    ValType::I32,
                ))),
            };
            let func = make_func("cmp", vec![], Type::Bool, body);
            let prog = make_program(vec![func]);
            let mut gen = CHIRCodeGen::new();
            let bytes = gen.generate(&prog);
            assert!(validate_wasm(&bytes), "Failed for op {:?}", op);
        }
    }

    #[test]
    fn test_generate_match_expr_multiple_arms() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Match {
                    subject: Box::new(CHIRExpr::int_const(1, Type::Int64)),
                    arms: vec![CHIRMatchArm {
                        pattern: CHIRPattern::Wildcard,
                        guard: None,
                        body: CHIRBlock {
                            stmts: vec![],
                            result: Some(Box::new(CHIRExpr::int_const(10, Type::Int64))),
                        },
                    }],
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("m", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_while_with_range_like_cond() {
        let body = CHIRBlock {
            stmts: vec![
                CHIRStmt::Let {
                    local_idx: 1,
                    value: CHIRExpr::int_const(0, Type::Int64),
                },
                CHIRStmt::While {
                    cond: CHIRExpr::new(
                        CHIRExprKind::Binary {
                            op: BinOp::Lt,
                            left: Box::new(CHIRExpr::new(
                                CHIRExprKind::Local(1),
                                Type::Int64,
                                ValType::I64,
                            )),
                            right: Box::new(CHIRExpr::int_const(5, Type::Int64)),
                        },
                        Type::Bool,
                        ValType::I32,
                    ),
                    body: CHIRBlock {
                        stmts: vec![CHIRStmt::Assign {
                            target: CHIRLValue::Local(1),
                            value: CHIRExpr::new(
                                CHIRExprKind::Binary {
                                    op: BinOp::Add,
                                    left: Box::new(CHIRExpr::new(
                                        CHIRExprKind::Local(1),
                                        Type::Int64,
                                        ValType::I64,
                                    )),
                                    right: Box::new(CHIRExpr::int_const(1, Type::Int64)),
                                },
                                Type::Int64,
                                ValType::I64,
                            ),
                        }],
                        result: None,
                    },
                },
            ],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Local(1),
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("loop_range", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_nested_if_else() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::If {
                    cond: Box::new(CHIRExpr::bool_const(true)),
                    then_block: CHIRBlock {
                        stmts: vec![],
                        result: Some(Box::new(CHIRExpr::new(
                            CHIRExprKind::If {
                                cond: Box::new(CHIRExpr::bool_const(false)),
                                then_block: CHIRBlock {
                                    stmts: vec![],
                                    result: Some(Box::new(CHIRExpr::int_const(1, Type::Int64))),
                                },
                                else_block: Some(CHIRBlock {
                                    stmts: vec![],
                                    result: Some(Box::new(CHIRExpr::int_const(2, Type::Int64))),
                                }),
                            },
                            Type::Int64,
                            ValType::I64,
                        ))),
                    },
                    else_block: Some(CHIRBlock {
                        stmts: vec![],
                        result: Some(Box::new(CHIRExpr::int_const(3, Type::Int64))),
                    }),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("nested_if", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_method_call() {
        let body = CHIRBlock {
            stmts: vec![],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::MethodCall {
                    func_idx: Some(1),
                    vtable_offset: None,
                    receiver: Box::new(CHIRExpr::new(
                        CHIRExprKind::Local(0),
                        Type::Int32,
                        ValType::I32,
                    )),
                    args: vec![CHIRExpr::int_const(10, Type::Int64)],
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func(
            "m",
            vec![CHIRParam {
                name: "this".into(),
                ty: Type::Int32,
                wasm_ty: ValType::I32,
                local_idx: 0,
            }],
            Type::Int64,
            body,
        );
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }

    #[test]
    fn test_generate_complex_block_locals_result() {
        let body = CHIRBlock {
            stmts: vec![
                CHIRStmt::Let {
                    local_idx: 1,
                    value: CHIRExpr::int_const(5, Type::Int64),
                },
                CHIRStmt::Let {
                    local_idx: 2,
                    value: CHIRExpr::int_const(10, Type::Int64),
                },
                CHIRStmt::Expr(CHIRExpr::new(
                    CHIRExprKind::Binary {
                        op: BinOp::Add,
                        left: Box::new(CHIRExpr::new(
                            CHIRExprKind::Local(1),
                            Type::Int64,
                            ValType::I64,
                        )),
                        right: Box::new(CHIRExpr::new(
                            CHIRExprKind::Local(2),
                            Type::Int64,
                            ValType::I64,
                        )),
                    },
                    Type::Int64,
                    ValType::I64,
                )),
            ],
            result: Some(Box::new(CHIRExpr::new(
                CHIRExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(CHIRExpr::new(
                        CHIRExprKind::Local(1),
                        Type::Int64,
                        ValType::I64,
                    )),
                    right: Box::new(CHIRExpr::new(
                        CHIRExprKind::Local(2),
                        Type::Int64,
                        ValType::I64,
                    )),
                },
                Type::Int64,
                ValType::I64,
            ))),
        };
        let func = make_func("complex", vec![], Type::Int64, body);
        let prog = make_program(vec![func]);
        let mut gen = CHIRCodeGen::new();
        let bytes = gen.generate(&prog);
        assert!(validate_wasm(&bytes));
    }
}
