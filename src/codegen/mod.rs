use crate::ast::{AssignTarget, BinOp, ClassDef, EnumDef, EnumVariant, Expr, FieldDef, InitDef, InterfaceDef, InterfaceMethod, InterpolatePart, Literal, MatchArm, Param, Pattern, Program, Stmt, StructDef, Type, UnaryOp, Visibility};
use crate::ast::Function as FuncDef;
use crate::memory;
use std::collections::HashMap;
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType, ExportKind,
    ExportSection, Function as WasmFunc, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemorySection, MemoryType, Module, RefType, TableSection, TableType, TypeSection,
    ValType,
};

mod decl;
mod expr;
mod type_;

pub(crate) use decl::ClassInfo;

/// 内存布局常量
const HEAP_BASE: i32 = 1024;  // 堆起始地址
const PAGE_SIZE: u64 = 65536; // WASM 页大小 64KB
/// I/O 缓冲区保留大小（地址 0-127 用于 println 等 I/O 操作）
const IO_BUFFER_RESERVE: u32 = 128;
/// iovec 结构体的内存偏移（8 字节：buf_ptr + buf_len）
pub(crate) const IOVEC_OFFSET: i32 = 64;
/// fd_write 的 nwritten 输出指针偏移
pub(crate) const NWRITTEN_OFFSET: i32 = 72;
/// WASI 系统调用临时内存区域（80-127，48 字节可用）
const WASI_SCRATCH: i32 = 80;

/// 代码生成器
pub struct CodeGen {
    /// 函数类型索引映射
    func_types: HashMap<String, u32>,
    /// 函数索引映射
    func_indices: HashMap<String, u32>,
    /// 函数返回类型（用于 let x = foo() 等类型推断）
    func_return_types: HashMap<String, Type>,
    /// 结构体定义
    structs: HashMap<String, StructDef>,
    /// 枚举定义
    enums: HashMap<String, EnumDef>,
    /// 类信息（含继承布局和 vtable）
    classes: HashMap<String, ClassInfo>,
    /// 函数参数列表（含默认值），用于 Call 时补全缺失实参
    func_params: HashMap<String, Vec<Param>>,
    /// 每个名字对应的函数个数，用于决定是否用修饰名解析
    name_count: HashMap<String, usize>,
    /// 字符串常量池 (字符串内容 -> 内存偏移)
    string_pool: Vec<(String, u32)>,
    /// 当前数据段偏移
    data_offset: u32,
    /// vtable 条目列表（function indices, 用于 Element Section）
    vtable_entries: Vec<u32>,
    /// 接口定义（方法签名表）
    interfaces: HashMap<String, Vec<InterfaceMethod>>,
    /// Lambda 函数列表（编译阶段收集），存储生成的函数名
    lambda_functions: Vec<FuncDef>,
    /// Lambda 计数器（使用 Cell 以支持在 &self 方法中修改）
    lambda_counter: std::cell::Cell<u32>,
    /// P2.2: 类型别名映射 (alias_name -> actual_type)
    type_aliases: HashMap<String, Type>,
    /// P2.3: Lambda 函数的 Table 索引映射 (lambda_name -> table_index)
    lambda_table_indices: HashMap<String, u32>,
    /// P2.3: 函数类型签名到类型索引的映射 (用于 call_indirect)
    func_type_by_sig: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
    /// P3.4: 类 ID 计数器（用于 `is` 类型检查）
    next_class_id: u32,
}

impl CodeGen {
    pub fn new() -> Self {
        Self {
            func_types: HashMap::new(),
            func_indices: HashMap::new(),
            func_return_types: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            classes: HashMap::new(),
            func_params: HashMap::new(),
            name_count: HashMap::new(),
            string_pool: Vec::new(),
            data_offset: IO_BUFFER_RESERVE,
            vtable_entries: Vec::new(),
            interfaces: HashMap::new(),
            lambda_functions: Vec::new(),
            lambda_counter: std::cell::Cell::new(0),
            type_aliases: HashMap::new(),
            lambda_table_indices: HashMap::new(),
            func_type_by_sig: HashMap::new(),
            next_class_id: 0,
        }
    }

    /// 获取运行时函数的索引
    fn get_or_create_func_index(&self, name: &str) -> u32 {
        *self.func_indices.get(name).expect(&format!("运行时函数 {} 未注册", name))
    }

    /// 检查语句列表中是否包含未被 try-catch 包裹的 throw 表达式
    fn contains_unhandled_throw(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Expr(e) => {
                    if Self::expr_contains_unhandled_throw(e) { return true; }
                }
                Stmt::Return(Some(e)) => {
                    if Self::expr_contains_unhandled_throw(e) { return true; }
                }
                Stmt::Let { value, .. } => {
                    if Self::expr_contains_unhandled_throw(value) { return true; }
                }
                Stmt::Var { value: Some(value), .. } => {
                    if Self::expr_contains_unhandled_throw(value) { return true; }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body } | Stmt::DoWhile { body, .. } | Stmt::UnsafeBlock { body } => {
                    if Self::contains_unhandled_throw(body) { return true; }
                }
                Stmt::Const { value, .. } => {
                    if Self::expr_contains_unhandled_throw(value) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    fn expr_contains_unhandled_throw(expr: &Expr) -> bool {
        match expr {
            Expr::Throw(_) => true,
            // try-catch 包裹的 throw 不算未处理
            Expr::TryBlock { .. } => false,
            Expr::If { then_branch, else_branch, .. } => {
                Self::expr_contains_unhandled_throw(then_branch)
                    || else_branch.as_ref().map_or(false, |eb| Self::expr_contains_unhandled_throw(eb))
            }
            Expr::Block(stmts, _) => Self::contains_unhandled_throw(stmts),
            _ => false,
        }
    }

    /// 从 try body 语句列表中找到第一个 throw 表达式的内部表达式
    fn find_throw_inner_in_stmts<'a>(stmts: &'a [Stmt]) -> Option<&'a Expr> {
        for stmt in stmts {
            match stmt {
                Stmt::Expr(e) => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(e) {
                        return Some(inner);
                    }
                }
                Stmt::Return(Some(e)) => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(e) {
                        return Some(inner);
                    }
                }
                Stmt::Let { value, .. } => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(value) {
                        return Some(inner);
                    }
                }
                Stmt::Var { value: Some(value), .. } => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(value) {
                        return Some(inner);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body } | Stmt::DoWhile { body, .. } | Stmt::UnsafeBlock { body } => {
                    if let Some(inner) = Self::find_throw_inner_in_stmts(body) {
                        return Some(inner);
                    }
                }
                Stmt::Const { value, .. } => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(value) {
                        return Some(inner);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn find_throw_inner_in_expr<'a>(expr: &'a Expr) -> Option<&'a Expr> {
        match expr {
            Expr::Throw(inner) => Some(inner),
            Expr::TryBlock { .. } => None,
            Expr::If { then_branch, else_branch, .. } => {
                Self::find_throw_inner_in_expr(then_branch)
                    .or_else(|| else_branch.as_ref().and_then(|eb| Self::find_throw_inner_in_expr(eb)))
            }
            Expr::Block(stmts, _) => Self::find_throw_inner_in_stmts(stmts),
            _ => None,
        }
    }

    /// P2.9: 将位置参数和命名参数合并为按函数定义顺序排列的最终参数列表
    fn resolve_named_args(&self, func_name: &str, positional: &[Expr], named: &[(String, Expr)]) -> Vec<Expr> {
        // 查找函数参数定义
        let func_params = self.func_params.get(func_name);
        if func_params.is_none() {
            // 如果找不到函数定义，按位置参数 + 命名参数值的顺序追加
            let mut result: Vec<Expr> = positional.to_vec();
            for (_, val) in named {
                result.push(val.clone());
            }
            return result;
        }
        let params = func_params.unwrap();
        let mut result = Vec::new();
        // 先填入位置参数
        for (i, param) in params.iter().enumerate() {
            if i < positional.len() {
                result.push(positional[i].clone());
            } else if let Some((_, val)) = named.iter().find(|(n, _)| n == &param.name) {
                result.push(val.clone());
            } else if let Some(ref default) = param.default {
                result.push(default.clone());
            } else {
                // 参数缺失，使用默认的零值
                result.push(Expr::Integer(0));
            }
        }
        result
    }

    /// 编译程序生成 WASM 模块
    pub fn compile(&mut self, program: &Program) -> Vec<u8> {
        let mut module = Module::new();

        // P2.2: 注册类型别名
        for (name, ty) in &program.type_aliases {
            self.type_aliases.insert(name.clone(), ty.clone());
        }

        // 收集结构体定义（跳过未单态化的泛型结构体）
        for s in &program.structs {
            if s.type_params.is_empty() {
                self.structs.insert(s.name.clone(), s.clone());
            }
        }
        // 注册所有类（跳过未单态化的泛型类）
        let concrete_classes: Vec<_> = program.classes.iter()
            .filter(|c| c.type_params.is_empty())
            .cloned()
            .collect();
        self.register_classes(&concrete_classes);
        // 收集枚举定义（跳过未单态化的泛型枚举）
        for e in &program.enums {
            if e.type_params.is_empty() {
                self.enums.insert(e.name.clone(), e.clone());
            }
        }

        // 注册内建 Option / Result 枚举（若用户未自定义）
        if !self.enums.contains_key("Option") {
            self.enums.insert("Option".to_string(), EnumDef {
                visibility: Visibility::Public,
                name: "Option".to_string(),
                type_params: vec![],
                constraints: vec![],
                variants: vec![
                    EnumVariant { name: "None".to_string(), payload: None },
                    EnumVariant { name: "Some".to_string(), payload: Some(Type::Int64) },
                ],
            });
        }
        if !self.enums.contains_key("Result") {
            self.enums.insert("Result".to_string(), EnumDef {
                visibility: Visibility::Public,
                name: "Result".to_string(),
                type_params: vec![],
                constraints: vec![],
                variants: vec![
                    EnumVariant { name: "Ok".to_string(), payload: Some(Type::Int64) },
                    EnumVariant { name: "Err".to_string(), payload: Some(Type::String) },
                ],
            });
        }

        // --- 注册内建 Error 基类 (#37) ---
        // Error 类有一个 message: String 字段，是 open 类（可被继承）
        let mut builtin_error_init: Option<FuncDef> = None;
        if !self.classes.contains_key("Error") {
            let error_class = ClassDef {
                visibility: Visibility::Public,
                name: "Error".to_string(),
                type_params: vec![],
                constraints: vec![],
                is_abstract: false,
                is_sealed: false,
                is_open: true,
                extends: None,
                implements: vec![],
                fields: vec![FieldDef {
                    name: "message".to_string(),
                    ty: Type::String,
                    default: None,
                }],
                static_init: None,
                primary_ctor_params: vec![],
                init: Some(InitDef {
                    params: vec![Param {
                        name: "message".to_string(),
                        ty: Type::String,
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    }],
                    body: vec![Stmt::Assign {
                        target: AssignTarget::Field {
                            object: "this".to_string(),
                            field: "message".to_string(),
                        },
                        value: Expr::Var("message".to_string()),
                    }],
                }),
                deinit: None,
                methods: vec![],
            };
            self.register_classes(&[error_class.clone()]);
            if let Some(ref init_def) = error_class.init {
                builtin_error_init = Some(self.build_init_function(&error_class, init_def));
            }
        }

        // 收集字符串常量
        self.collect_strings(program);

        // --- 接口 codegen (#29, #30, #33) ---
        // 注册接口定义（含继承合并），生成默认实现函数
        let mut interface_methods: HashMap<String, Vec<InterfaceMethod>> = HashMap::new();
        // 先收集所有接口
        let mut all_interfaces: HashMap<String, &InterfaceDef> = HashMap::new();
        for iface in &program.interfaces {
            all_interfaces.insert(iface.name.clone(), iface);
        }
        // 接口继承合并 (#33)
        for iface in &program.interfaces {
            let mut methods = Vec::new();
            // 收集父接口方法
            for parent_name in &iface.parents {
                if let Some(parent) = all_interfaces.get(parent_name) {
                    for m in &parent.methods {
                        if !methods.iter().any(|em: &InterfaceMethod| em.name == m.name) {
                            methods.push(m.clone());
                        }
                    }
                }
            }
            // 添加自己的方法
            for m in &iface.methods {
                // 子接口方法覆盖父接口同名方法
                if let Some(pos) = methods.iter().position(|em| em.name == m.name) {
                    methods[pos] = m.clone();
                } else {
                    methods.push(m.clone());
                }
            }
            interface_methods.insert(iface.name.clone(), methods);
        }
        self.interfaces = interface_methods;

        // --- extends 处理 (#32) ---
        // 合并 extend 中的方法到 functions 列表
        let mut extend_functions: Vec<FuncDef> = Vec::new();
        let mut extend_interfaces: HashMap<String, Vec<String>> = HashMap::new(); // type -> [interface]
        for ext in &program.extends {
            for method in &ext.methods {
                extend_functions.push(method.clone());
            }
            if let Some(ref iface) = ext.interface {
                extend_interfaces
                    .entry(ext.target_type.clone())
                    .or_default()
                    .push(iface.clone());
            }
        }

        // 暂不编译泛型函数（单态化待实现），仅编译已单态化的函数
        let mut functions: Vec<_> = program
            .functions
            .iter()
            .filter(|f| f.type_params.is_empty() || f.extern_import.is_some())
            .cloned()
            .collect();
        // 添加所有类的方法（跳过未单态化的泛型类）
        for c in program.classes.iter().filter(|c| c.type_params.is_empty()) {
            for m in &c.methods {
                functions.push(m.func.clone());
            }
            // 为有 init 的类生成 __ClassName_init 函数和 __ClassName_init_body 函数
            if let Some(ref init_def) = c.init {
                let init_func = self.build_init_function(c, init_def);
                functions.push(init_func);
                // Bug B3: 生成 init_body 函数（用于 super() 调用）
                let init_body_func = self.build_init_body_function(c, init_def);
                functions.push(init_body_func);
            }
            // 为有 deinit 的类生成 __ClassName_deinit 函数
            if let Some(ref deinit_body) = c.deinit {
                let deinit_func = self.build_deinit_function(c, deinit_body);
                functions.push(deinit_func);
            }
        }
        // 添加 extend 中的方法
        functions.extend(extend_functions);
        // 添加内建 Error 类的 init 函数
        if let Some(init_func) = builtin_error_init {
            functions.push(init_func);
        }
        // 接口默认实现方法 (#30) → 生成为 InterfaceName.__default_method 函数
        for iface in &program.interfaces {
            for m in &iface.methods {
                if let Some(ref body) = m.default_body {
                    functions.push(FuncDef {
                        visibility: Visibility::Public,
                        name: format!("{}.__default_{}", iface.name, m.name),
                        type_params: vec![],
                        constraints: vec![],
                        params: m.params.clone(),
                        return_type: m.return_type.clone(),
                        throws: None,
                        body: body.clone(),
                        extern_import: None,
                    });
                }
            }
        }

        // 块内局部函数 (Stmt::LocalFunc) — 提前收集并加入 functions，以便分配索引
        let mut local_funcs = Vec::new();
        Self::collect_local_funcs_from_functions(&functions, &mut local_funcs);
        functions.extend(local_funcs);

        // Lambda 预扫描 (#35) — 收集所有 Lambda 表达式，生成匿名函数
        let mut lambda_counter = 0u32;
        let mut lambda_funcs = Vec::new();
        Self::collect_lambdas_from_functions(&functions, &mut lambda_counter, &mut lambda_funcs);
        functions.extend(lambda_funcs);
        self.lambda_counter.set(0); // 重置，编译阶段重新计数

        let name_count: HashMap<String, usize> = functions
            .iter()
            .map(|f| f.name.clone())
            .fold(HashMap::new(), |mut m, n| {
                *m.entry(n).or_default() += 1;
                m
            });
        self.name_count = name_count.clone();

        // 1. 类型段 (Type Section)，重载时按参数类型名字修饰
        let mut types = TypeSection::new();
        for (i, func) in functions.iter().enumerate() {
            // 可变参数类型转为 Array<T>
            let param_tys: Vec<Type> = func.params.iter().map(|p| {
                if p.variadic {
                    Type::Array(Box::new(p.ty.clone()))
                } else {
                    p.ty.clone()
                }
            }).collect();
            let params: Vec<ValType> = param_tys.iter().map(|p| p.to_wasm()).collect();
            let results: Vec<ValType> = func
                .return_type
                .as_ref()
                .and_then(|t| if *t == Type::Unit { None } else { Some(vec![t.to_wasm()]) })
                .unwrap_or_default();
            types.ty().function(params.clone(), results.clone());
            // P2.3: 记录类型签名映射
            self.func_type_by_sig.entry((params, results)).or_insert(i as u32);
            let key = if *name_count.get(&func.name).unwrap_or(&0) > 1 {
                Self::mangle_key(&func.name, &param_tys)
            } else {
                func.name.clone()
            };
            self.func_types.insert(key.clone(), i as u32);
            if let Some(ref ret) = func.return_type {
                if *ret != Type::Unit {
                    self.func_return_types.insert(key.clone(), ret.clone());
                }
            }
            self.func_params.insert(key, func.params.clone());
        }
        let num_user_imports = functions.iter().filter(|f| f.extern_import.is_some()).count() as u32;
        let num_builtin_imports = 13u32; // WASI: fd_write, fd_read, fd_close, args_sizes_get, args_get, clock_time_get, random_get, environ_sizes_get, environ_get, proc_exit, fd_prestat_get, path_open, fd_seek
        let num_imports = num_user_imports + num_builtin_imports;
        let num_non_extern = functions.len() as u32 - num_user_imports;
        let mut import_idx = 0u32;
        let mut non_extern_idx = 0u32;
        for (_i, func) in functions.iter().enumerate() {
            let param_tys: Vec<Type> = func.params.iter().map(|p| {
                if p.variadic { Type::Array(Box::new(p.ty.clone())) } else { p.ty.clone() }
            }).collect();
            let key = if *name_count.get(&func.name).unwrap_or(&0) > 1 {
                Self::mangle_key(&func.name, &param_tys)
            } else {
                func.name.clone()
            };
            let wasm_idx = if func.extern_import.is_some() {
                let idx = import_idx;
                import_idx += 1;
                idx
            } else {
                let idx = num_imports + non_extern_idx;
                non_extern_idx += 1;
                idx
            };
            self.func_indices.insert(key, wasm_idx);
        }
        // 运行时辅助函数类型（类型索引仍为 functions.len() + 0,1,...）
        let runtime_type_base = functions.len() as u32;
        let runtime_func_base = num_imports + num_non_extern;

        // __pow_i64(i64, i64) -> i64
        types.ty().function([ValType::I64, ValType::I64], [ValType::I64]);
        self.func_types.insert("__pow_i64".to_string(), runtime_type_base);
        self.func_indices.insert("__pow_i64".to_string(), runtime_func_base);

        // __str_concat(i32, i32) -> i32
        types.ty().function([ValType::I32, ValType::I32], [ValType::I32]);
        self.func_types.insert("__str_concat".to_string(), runtime_type_base + 1);
        self.func_indices.insert("__str_concat".to_string(), runtime_func_base + 1);

        // __i64_to_str(i64) -> i32
        types.ty().function([ValType::I64], [ValType::I32]);
        self.func_types.insert("__i64_to_str".to_string(), runtime_type_base + 2);
        self.func_indices.insert("__i64_to_str".to_string(), runtime_func_base + 2);

        // __i32_to_str(i32) -> i32
        types.ty().function([ValType::I32], [ValType::I32]);
        self.func_types.insert("__i32_to_str".to_string(), runtime_type_base + 3);
        self.func_indices.insert("__i32_to_str".to_string(), runtime_func_base + 3);

        // __f64_to_str(f64) -> i32
        types.ty().function([ValType::F64], [ValType::I32]);
        self.func_types.insert("__f64_to_str".to_string(), runtime_type_base + 4);
        self.func_indices.insert("__f64_to_str".to_string(), runtime_func_base + 4);

        // __f32_to_str(f32) -> i32
        types.ty().function([ValType::F32], [ValType::I32]);
        self.func_types.insert("__f32_to_str".to_string(), runtime_type_base + 5);
        self.func_indices.insert("__f32_to_str".to_string(), runtime_func_base + 5);

        // __bool_to_str(i32) -> i32
        types.ty().function([ValType::I32], [ValType::I32]);
        self.func_types.insert("__bool_to_str".to_string(), runtime_type_base + 6);
        self.func_indices.insert("__bool_to_str".to_string(), runtime_func_base + 6);

        // 标准库雏形：min/max/abs (Int64)
        types.ty().function([ValType::I64, ValType::I64], [ValType::I64]);
        self.func_types.insert("__min_i64".to_string(), runtime_type_base + 7);
        self.func_indices.insert("__min_i64".to_string(), runtime_func_base + 7);
        types.ty().function([ValType::I64, ValType::I64], [ValType::I64]);
        self.func_types.insert("__max_i64".to_string(), runtime_type_base + 8);
        self.func_indices.insert("__max_i64".to_string(), runtime_func_base + 8);
        types.ty().function([ValType::I64], [ValType::I64]);
        self.func_types.insert("__abs_i64".to_string(), runtime_type_base + 9);
        self.func_indices.insert("__abs_i64".to_string(), runtime_func_base + 9);

        // Phase 8: 内存管理运行时函数
        // __alloc(size: i32) -> i32
        types.ty().function([ValType::I32], [ValType::I32]);
        self.func_types.insert("__alloc".to_string(), runtime_type_base + 10);
        self.func_indices.insert("__alloc".to_string(), runtime_func_base + 10);

        // __free(ptr: i32)
        types.ty().function([ValType::I32], []);
        self.func_types.insert("__free".to_string(), runtime_type_base + 11);
        self.func_indices.insert("__free".to_string(), runtime_func_base + 11);

        // __rc_inc(ptr: i32)
        types.ty().function([ValType::I32], []);
        self.func_types.insert("__rc_inc".to_string(), runtime_type_base + 12);
        self.func_indices.insert("__rc_inc".to_string(), runtime_func_base + 12);

        // __rc_dec(ptr: i32)
        types.ty().function([ValType::I32], []);
        self.func_types.insert("__rc_dec".to_string(), runtime_type_base + 13);
        self.func_indices.insert("__rc_dec".to_string(), runtime_func_base + 13);

        // __gc_collect() -> i32
        types.ty().function([], [ValType::I32]);
        self.func_types.insert("__gc_collect".to_string(), runtime_type_base + 14);
        self.func_indices.insert("__gc_collect".to_string(), runtime_func_base + 14);

        // Phase 7: WASI I/O 支持 — print/println/eprint/eprintln/readln + math 运行时
        // WASI fd_write 类型: (i32, i32, i32, i32) -> i32
        let wasi_fd_write_type_idx = runtime_type_base + 15;
        types.ty().function(
            [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            [ValType::I32],
        );
        // fd_write 是内置导入，索引在用户导入之后
        self.func_indices.insert("__wasi_fd_write".to_string(), num_user_imports);
        // WASI fd_read 类型: (i32, i32, i32, i32) -> i32 (与 fd_write 相同签名)
        // fd_read 是第二个内置导入
        self.func_indices.insert("__wasi_fd_read".to_string(), num_user_imports + 1);

        // Phase 7.6: 新增 WASI 系统调用导入
        self.func_indices.insert("__wasi_fd_close".to_string(), num_user_imports + 2);
        self.func_indices.insert("__wasi_args_sizes_get".to_string(), num_user_imports + 3);
        self.func_indices.insert("__wasi_args_get".to_string(), num_user_imports + 4);
        self.func_indices.insert("__wasi_clock_time_get".to_string(), num_user_imports + 5);
        self.func_indices.insert("__wasi_random_get".to_string(), num_user_imports + 6);
        self.func_indices.insert("__wasi_environ_sizes_get".to_string(), num_user_imports + 7);
        self.func_indices.insert("__wasi_environ_get".to_string(), num_user_imports + 8);
        self.func_indices.insert("__wasi_proc_exit".to_string(), num_user_imports + 9);
        self.func_indices.insert("__wasi_fd_prestat_get".to_string(), num_user_imports + 10);
        self.func_indices.insert("__wasi_path_open".to_string(), num_user_imports + 11);
        self.func_indices.insert("__wasi_fd_seek".to_string(), num_user_imports + 12);

        // --- I/O 运行时函数类型 ---
        // (i64) -> () : 用于 *_i64 函数
        let ty_i64_void = runtime_type_base + 16;
        types.ty().function([ValType::I64], []);
        // (i32) -> () : 用于 *_str 和 *_bool 函数
        let ty_i32_void = runtime_type_base + 17;
        types.ty().function([ValType::I32], []);
        // (f64) -> f64 : 用于一元 math 函数 (sin, cos, tan, exp, log)
        let ty_f64_f64 = runtime_type_base + 18;
        types.ty().function([ValType::F64], [ValType::F64]);
        // (f64, f64) -> f64 : 用于二元 math 函数 (pow)
        let ty_f64f64_f64 = runtime_type_base + 19;
        types.ty().function([ValType::F64, ValType::F64], [ValType::F64]);

        let mut rt_idx = runtime_func_base + 15; // 从 15 开始（0-14 是已有运行时函数）

        // println 变体 (fd=1, newline=true) — 与之前兼容
        self.func_types.insert("__println_i64".to_string(), ty_i64_void);
        self.func_indices.insert("__println_i64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__println_str".to_string(), ty_i32_void);
        self.func_indices.insert("__println_str".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__println_bool".to_string(), ty_i32_void);
        self.func_indices.insert("__println_bool".to_string(), rt_idx); rt_idx += 1;

        // print 变体 (fd=1, newline=false)
        self.func_types.insert("__print_i64".to_string(), ty_i64_void);
        self.func_indices.insert("__print_i64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__print_str".to_string(), ty_i32_void);
        self.func_indices.insert("__print_str".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__print_bool".to_string(), ty_i32_void);
        self.func_indices.insert("__print_bool".to_string(), rt_idx); rt_idx += 1;

        // eprintln 变体 (fd=2, newline=true)
        self.func_types.insert("__eprintln_i64".to_string(), ty_i64_void);
        self.func_indices.insert("__eprintln_i64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__eprintln_str".to_string(), ty_i32_void);
        self.func_indices.insert("__eprintln_str".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__eprintln_bool".to_string(), ty_i32_void);
        self.func_indices.insert("__eprintln_bool".to_string(), rt_idx); rt_idx += 1;

        // eprint 变体 (fd=2, newline=false)
        self.func_types.insert("__eprint_i64".to_string(), ty_i64_void);
        self.func_indices.insert("__eprint_i64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__eprint_str".to_string(), ty_i32_void);
        self.func_indices.insert("__eprint_str".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__eprint_bool".to_string(), ty_i32_void);
        self.func_indices.insert("__eprint_bool".to_string(), rt_idx); rt_idx += 1;

        // math 运行时函数
        self.func_types.insert("__math_sin".to_string(), ty_f64_f64);
        self.func_indices.insert("__math_sin".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__math_cos".to_string(), ty_f64_f64);
        self.func_indices.insert("__math_cos".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__math_tan".to_string(), ty_f64_f64);
        self.func_indices.insert("__math_tan".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__math_exp".to_string(), ty_f64_f64);
        self.func_indices.insert("__math_exp".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__math_log".to_string(), ty_f64_f64);
        self.func_indices.insert("__math_log".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__math_pow".to_string(), ty_f64f64_f64);
        self.func_indices.insert("__math_pow".to_string(), rt_idx); rt_idx += 1;

        // Phase 7.1 #44: readln() -> i32 (返回字符串指针)
        // () -> i32
        let ty_void_i32 = runtime_type_base + 20;
        types.ty().function([], [ValType::I32]);
        self.func_types.insert("__readln".to_string(), ty_void_i32);
        self.func_indices.insert("__readln".to_string(), rt_idx);
        rt_idx += 1;

        // Phase 7.2: __str_to_i64(str_ptr: i32) -> i64
        // (i32) -> i64
        let ty_i32_i64 = runtime_type_base + 21;
        types.ty().function([ValType::I32], [ValType::I64]);
        self.func_types.insert("__str_to_i64".to_string(), ty_i32_i64);
        self.func_indices.insert("__str_to_i64".to_string(), rt_idx);
        rt_idx += 1;

        // Phase 7.2: __str_to_f64(str_ptr: i32) -> f64
        // (i32) -> f64
        let ty_i32_f64 = runtime_type_base + 22;
        types.ty().function([ValType::I32], [ValType::F64]);
        self.func_types.insert("__str_to_f64".to_string(), ty_i32_f64);
        self.func_indices.insert("__str_to_f64".to_string(), rt_idx);
        rt_idx += 1;

        // ============================================================
        // Phase 7.6: WASI 系统调用类型签名
        // ============================================================
        // (i32) -> i32 : fd_close
        let ty_wasi_i32_i32 = runtime_type_base + 23;
        types.ty().function([ValType::I32], [ValType::I32]);
        // (i32, i32) -> i32 : args_sizes_get, args_get, random_get, environ_sizes_get, environ_get, fd_prestat_get
        let ty_wasi_i32i32_i32 = runtime_type_base + 24;
        types.ty().function([ValType::I32, ValType::I32], [ValType::I32]);
        // (i32, i64, i32) -> i32 : clock_time_get
        let ty_wasi_clock = runtime_type_base + 25;
        types.ty().function([ValType::I32, ValType::I64, ValType::I32], [ValType::I32]);
        // (i32, i64, i32, i32) -> i32 : fd_seek
        let ty_wasi_fd_seek = runtime_type_base + 26;
        types.ty().function([ValType::I32, ValType::I64, ValType::I32, ValType::I32], [ValType::I32]);
        // (i32, i32, i32, i32, i32, i64, i64, i32, i32) -> i32 : path_open
        let ty_wasi_path_open = runtime_type_base + 27;
        types.ty().function(
            [ValType::I32, ValType::I32, ValType::I32, ValType::I32, ValType::I32,
             ValType::I64, ValType::I64, ValType::I32, ValType::I32],
            [ValType::I32],
        );
        // (i32) -> () : proc_exit (reuse ty_i32_void for runtime, but WASI needs separate)
        // 注意: ty_i32_void (runtime_type_base + 17) 已定义 (i32) -> ()，proc_exit 直接复用

        // ============================================================
        // Phase 7.7: 运行时包装函数类型
        // ============================================================
        // () -> i64 : get_time_ns, random_i64
        let ty_void_i64 = runtime_type_base + 28;
        types.ty().function([], [ValType::I64]);
        // () -> f64 : random_f64
        let ty_void_f64 = runtime_type_base + 29;
        types.ty().function([], [ValType::F64]);

        // ============================================================
        // Phase 7.4: 格式化函数类型
        // ============================================================
        // (i64, i32) -> i32 : i64_format(val, spec_ptr) -> str_ptr
        let ty_i64i32_i32 = runtime_type_base + 30;
        types.ty().function([ValType::I64, ValType::I32], [ValType::I32]);
        // (f64, i32) -> i32 : f64_format(val, spec_ptr) -> str_ptr
        let ty_f64i32_i32 = runtime_type_base + 31;
        types.ty().function([ValType::F64, ValType::I32], [ValType::I32]);

        // ============================================================
        // Phase 7.5: 集合类型运行时函数类型
        // ============================================================
        // (i32, i64) -> () : arraylist_append, linkedlist_append
        let ty_i32i64_void = runtime_type_base + 32;
        types.ty().function([ValType::I32, ValType::I64], []);
        // (i32, i64) -> i64 : arraylist_get, arraylist_remove, hashmap_get, hashmap_remove
        let ty_i32i64_i64 = runtime_type_base + 33;
        types.ty().function([ValType::I32, ValType::I64], [ValType::I64]);
        // (i32, i64, i64) -> () : arraylist_set, hashmap_put
        let ty_i32i64i64_void = runtime_type_base + 34;
        types.ty().function([ValType::I32, ValType::I64, ValType::I64], []);
        // (i32, i64) -> i32 : hashmap_contains
        let ty_i32i64_i32 = runtime_type_base + 35;
        types.ty().function([ValType::I32, ValType::I64], [ValType::I32]);

        // ============================================================
        // Phase 7.8: 字符串操作 / 排序函数类型
        // ============================================================
        // (i32, i32) -> i64 : str_index_of
        let ty_i32i32_i64 = runtime_type_base + 36;
        types.ty().function([ValType::I32, ValType::I32], [ValType::I64]);
        // (i32, i32, i32) -> i32 : str_replace(str, old, new)
        let ty_i32i32i32_i32 = runtime_type_base + 37;
        types.ty().function([ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);

        // ============================================================
        // Phase 7.7: 运行时包装函数索引注册
        // ============================================================
        self.func_types.insert("__get_time_ns".to_string(), ty_void_i64);
        self.func_indices.insert("__get_time_ns".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__random_i64".to_string(), ty_void_i64);
        self.func_indices.insert("__random_i64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__random_f64".to_string(), ty_void_f64);
        self.func_indices.insert("__random_f64".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__get_args".to_string(), ty_void_i32);
        self.func_indices.insert("__get_args".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__get_env".to_string(), ty_wasi_i32_i32);
        self.func_indices.insert("__get_env".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__exit".to_string(), ty_i32_void);
        self.func_indices.insert("__exit".to_string(), rt_idx); rt_idx += 1;

        // ============================================================
        // Phase 7.4: 格式化函数索引注册
        // ============================================================
        self.func_types.insert("__i64_format".to_string(), ty_i64i32_i32);
        self.func_indices.insert("__i64_format".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__f64_format".to_string(), ty_f64i32_i32);
        self.func_indices.insert("__f64_format".to_string(), rt_idx); rt_idx += 1;

        // ============================================================
        // Phase 7.5: 集合类型函数索引注册
        // ============================================================
        // ArrayList
        self.func_types.insert("__arraylist_new".to_string(), ty_void_i32);
        self.func_indices.insert("__arraylist_new".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__arraylist_append".to_string(), ty_i32i64_void);
        self.func_indices.insert("__arraylist_append".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__arraylist_get".to_string(), ty_i32i64_i64);
        self.func_indices.insert("__arraylist_get".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__arraylist_set".to_string(), ty_i32i64i64_void);
        self.func_indices.insert("__arraylist_set".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__arraylist_remove".to_string(), ty_i32i64_i64);
        self.func_indices.insert("__arraylist_remove".to_string(), rt_idx); rt_idx += 1;
        // HashMap
        self.func_types.insert("__hashmap_new".to_string(), ty_void_i32);
        self.func_indices.insert("__hashmap_new".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__hashmap_put".to_string(), ty_i32i64i64_void);
        self.func_indices.insert("__hashmap_put".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__hashmap_get".to_string(), ty_i32i64_i64);
        self.func_indices.insert("__hashmap_get".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__hashmap_contains".to_string(), ty_i32i64_i32);
        self.func_indices.insert("__hashmap_contains".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__hashmap_remove".to_string(), ty_i32i64_i64);
        self.func_indices.insert("__hashmap_remove".to_string(), rt_idx); rt_idx += 1;
        // LinkedList
        self.func_types.insert("__linkedlist_new".to_string(), ty_void_i32);
        self.func_indices.insert("__linkedlist_new".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__linkedlist_append".to_string(), ty_i32i64_void);
        self.func_indices.insert("__linkedlist_append".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__linkedlist_prepend".to_string(), ty_i32i64_void);
        self.func_indices.insert("__linkedlist_prepend".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__linkedlist_get".to_string(), ty_i32i64_i64);
        self.func_indices.insert("__linkedlist_get".to_string(), rt_idx); rt_idx += 1;

        // ============================================================
        // Phase 7.8: 字符串操作 / 排序函数索引注册
        // ============================================================
        self.func_types.insert("__str_contains".to_string(), ty_wasi_i32i32_i32);
        self.func_indices.insert("__str_contains".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_index_of".to_string(), ty_i32i32_i64);
        self.func_indices.insert("__str_index_of".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_replace".to_string(), ty_i32i32i32_i32);
        self.func_indices.insert("__str_replace".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_split".to_string(), ty_wasi_i32i32_i32);
        self.func_indices.insert("__str_split".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_to_rune_array".to_string(), ty_wasi_i32_i32);
        self.func_indices.insert("__str_to_rune_array".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__sort_array".to_string(), ty_i32_void);
        self.func_indices.insert("__sort_array".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_substring".to_string(), ty_i32i32i32_i32);
        self.func_indices.insert("__str_substring".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__hashcode_i64".to_string(), ty_i32_i64);
        self.func_indices.insert("__hashcode_i64".to_string(), rt_idx); rt_idx += 1;
        // P2.10: String 方法运行时函数
        self.func_types.insert("__str_trim".to_string(), ty_wasi_i32_i32);
        self.func_indices.insert("__str_trim".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_starts_with".to_string(), ty_wasi_i32i32_i32);
        self.func_indices.insert("__str_starts_with".to_string(), rt_idx); rt_idx += 1;
        self.func_types.insert("__str_ends_with".to_string(), ty_wasi_i32i32_i32);
        self.func_indices.insert("__str_ends_with".to_string(), rt_idx); rt_idx += 1;
        let _rt_idx_end = rt_idx;

        module.section(&types);

        // 构建 vtable（需在 func_indices 设置后）
        self.build_vtables();

        // 2. 导入段 (Import Section) — extern func + WASI 内置导入
        let mut imports = ImportSection::new();
        for (i, func) in functions.iter().enumerate() {
            if let Some(ref imp) = func.extern_import {
                imports.import(&imp.module, &imp.name, EntityType::Function(i as u32));
            }
        }
        // WASI fd_write 内置导入（用于 println 等 I/O 操作）
        imports.import(
            "wasi_snapshot_preview1",
            "fd_write",
            EntityType::Function(wasi_fd_write_type_idx),
        );
        // WASI fd_read 内置导入（用于 readln）— 与 fd_write 相同类型签名
        imports.import(
            "wasi_snapshot_preview1",
            "fd_read",
            EntityType::Function(wasi_fd_write_type_idx), // 同 (i32,i32,i32,i32)->i32
        );
        // Phase 7.6: 新增 WASI 系统调用导入
        imports.import("wasi_snapshot_preview1", "fd_close", EntityType::Function(ty_wasi_i32_i32));
        imports.import("wasi_snapshot_preview1", "args_sizes_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "args_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "clock_time_get", EntityType::Function(ty_wasi_clock));
        imports.import("wasi_snapshot_preview1", "random_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "environ_sizes_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "environ_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "proc_exit", EntityType::Function(ty_i32_void));
        imports.import("wasi_snapshot_preview1", "fd_prestat_get", EntityType::Function(ty_wasi_i32i32_i32));
        imports.import("wasi_snapshot_preview1", "path_open", EntityType::Function(ty_wasi_path_open));
        imports.import("wasi_snapshot_preview1", "fd_seek", EntityType::Function(ty_wasi_fd_seek));
        module.section(&imports);

        // 3. 函数段 (Function Section)：仅非 extern 的 type 索引
        let mut func_section = FunctionSection::new();
        for (i, func) in functions.iter().enumerate() {
            if func.extern_import.is_none() {
                func_section.function(i as u32);
            }
        }
        for r in 0..15u32 {
            func_section.function(runtime_type_base + r);
        }
        // Phase 7: I/O 运行时函数 (12 个: println/print/eprintln/eprint × i64/str/bool)
        for _ in 0..4 { // 4 variants: println, print, eprintln, eprint
            func_section.function(ty_i64_void);  // *_i64
            func_section.function(ty_i32_void);  // *_str
            func_section.function(ty_i32_void);  // *_bool
        }
        // Phase 7.3: math 运行时函数 (6 个)
        func_section.function(ty_f64_f64);    // __math_sin
        func_section.function(ty_f64_f64);    // __math_cos
        func_section.function(ty_f64_f64);    // __math_tan
        func_section.function(ty_f64_f64);    // __math_exp
        func_section.function(ty_f64_f64);    // __math_log
        func_section.function(ty_f64f64_f64); // __math_pow
        // Phase 7.1 #44: readln
        func_section.function(ty_void_i32);   // __readln
        // Phase 7.2: str_to_i64, str_to_f64
        func_section.function(ty_i32_i64);    // __str_to_i64
        func_section.function(ty_i32_f64);    // __str_to_f64
        // Phase 7.7: 运行时包装函数
        func_section.function(ty_void_i64);       // __get_time_ns
        func_section.function(ty_void_i64);       // __random_i64
        func_section.function(ty_void_f64);       // __random_f64
        func_section.function(ty_void_i32);       // __get_args
        func_section.function(ty_wasi_i32_i32);   // __get_env
        func_section.function(ty_i32_void);       // __exit
        // Phase 7.4: 格式化函数
        func_section.function(ty_i64i32_i32);     // __i64_format
        func_section.function(ty_f64i32_i32);     // __f64_format
        // Phase 7.5: 集合类型函数
        func_section.function(ty_void_i32);       // __arraylist_new
        func_section.function(ty_i32i64_void);    // __arraylist_append
        func_section.function(ty_i32i64_i64);     // __arraylist_get
        func_section.function(ty_i32i64i64_void); // __arraylist_set
        func_section.function(ty_i32i64_i64);     // __arraylist_remove
        func_section.function(ty_void_i32);       // __hashmap_new
        func_section.function(ty_i32i64i64_void); // __hashmap_put
        func_section.function(ty_i32i64_i64);     // __hashmap_get
        func_section.function(ty_i32i64_i32);     // __hashmap_contains
        func_section.function(ty_i32i64_i64);     // __hashmap_remove
        func_section.function(ty_void_i32);       // __linkedlist_new
        func_section.function(ty_i32i64_void);    // __linkedlist_append
        func_section.function(ty_i32i64_void);    // __linkedlist_prepend
        func_section.function(ty_i32i64_i64);     // __linkedlist_get
        // Phase 7.8: 字符串操作 / 排序
        func_section.function(ty_wasi_i32i32_i32);  // __str_contains
        func_section.function(ty_i32i32_i64);        // __str_index_of
        func_section.function(ty_i32i32i32_i32);     // __str_replace
        func_section.function(ty_wasi_i32i32_i32);   // __str_split
        func_section.function(ty_wasi_i32_i32);      // __str_to_rune_array
        func_section.function(ty_i32_void);           // __sort_array
        func_section.function(ty_i32i32i32_i32);     // __str_substring
        func_section.function(ty_i32_i64);            // __hashcode_i64
        // P2.10: String 方法
        func_section.function(ty_wasi_i32_i32);      // __str_trim
        func_section.function(ty_wasi_i32i32_i32);   // __str_starts_with
        func_section.function(ty_wasi_i32i32_i32);   // __str_ends_with
        module.section(&func_section);

        // P2.3: 收集 Lambda 函数的 Table 索引
        let vtable_base_len = self.vtable_entries.len() as u32;
        let mut lambda_func_indices_for_table: Vec<u32> = Vec::new();
        for i in 0..lambda_counter {
            let lambda_name = format!("__lambda_{}", i);
            if let Some(&func_idx) = self.func_indices.get(&lambda_name) {
                let table_idx = vtable_base_len + lambda_func_indices_for_table.len() as u32;
                self.lambda_table_indices.insert(lambda_name, table_idx);
                lambda_func_indices_for_table.push(func_idx);
            }
        }

        // 3a. Table 段 (Table Section) — 用于 vtable / call_indirect / lambda
        let total_table_size = (self.vtable_entries.len() + lambda_func_indices_for_table.len()) as u64;
        if total_table_size > 0 {
            let mut tables = TableSection::new();
            tables.table(TableType {
                element_type: RefType::FUNCREF,
                minimum: total_table_size,
                maximum: Some(total_table_size),
                table64: false,
                shared: false,
            });
            module.section(&tables);
        }

        // 3b. 内存段 (Memory Section)
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: Some(16),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);

        // 4. 全局变量段 (Global Section) - 堆指针 + 空闲链表头
        let heap_start = HEAP_BASE + self.data_offset as i32;
        let mut globals = GlobalSection::new();
        // Global 0: heap_ptr (bump allocator 指针)
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(heap_start),
        );
        // Global 1: free_list_head (空闲链表头指针，Phase 8)
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0), // 初始为 null
        );
        module.section(&globals);

        // 5. 导出段：仅导出非 extern 函数，单一定义用原名，重载用修饰名
        let mut exports = ExportSection::new();
        for func in &functions {
            if func.extern_import.is_some() {
                continue;
            }
            let param_tys: Vec<Type> = func.params.iter().map(|p| {
                if p.variadic { Type::Array(Box::new(p.ty.clone())) } else { p.ty.clone() }
            }).collect();
            let key = if *name_count.get(&func.name).unwrap_or(&0) > 1 {
                Self::mangle_key(&func.name, &param_tys)
            } else {
                func.name.clone()
            };
            let idx = *self.func_indices.get(&key).expect("导出时函数索引");
            exports.export(&key, ExportKind::Func, idx);
        }
        // WASI requires "_start" entry point (must be () -> ())
        // Only add _start if main is void return type
        if let Some(main_func) = functions.iter().find(|f| f.name == "main") {
            if main_func.return_type.is_none() || main_func.return_type == Some(Type::Unit) {
                if let Some(&main_idx) = self.func_indices.get("main") {
                    exports.export("_start", ExportKind::Func, main_idx);
                }
            }
        }
        exports.export("memory", ExportKind::Memory, 0);
        // Phase 8: 导出内存管理函数
        exports.export("__alloc", ExportKind::Func, self.func_indices["__alloc"]);
        exports.export("__free", ExportKind::Func, self.func_indices["__free"]);
        exports.export("__rc_inc", ExportKind::Func, self.func_indices["__rc_inc"]);
        exports.export("__rc_dec", ExportKind::Func, self.func_indices["__rc_dec"]);
        exports.export("__gc_collect", ExportKind::Func, self.func_indices["__gc_collect"]);
        module.section(&exports);

        // 6. 代码段 (Code Section)：仅非 extern 函数
        let mut codes = CodeSection::new();
        for func in &functions {
            if func.extern_import.is_some() {
                continue;
            }
            let wasm_func = self.compile_function(func);
            codes.function(&wasm_func);
        }
        // 运行时辅助函数
        codes.function(&self.emit_pow_i64());
        codes.function(&self.emit_str_concat());
        codes.function(&self.emit_i64_to_str());
        codes.function(&self.emit_i32_to_str());
        codes.function(&self.emit_f64_to_str());
        codes.function(&self.emit_f32_to_str());
        codes.function(&self.emit_bool_to_str());
        codes.function(&self.emit_min_i64());
        codes.function(&self.emit_max_i64());
        codes.function(&self.emit_abs_i64());

        // Phase 8: 内存管理运行时函数
        let free_func_idx = self.func_indices["__free"];
        codes.function(&memory::emit_alloc_func(heap_start));
        codes.function(&memory::emit_free_func());
        codes.function(&memory::emit_rc_inc_func(heap_start));
        codes.function(&memory::emit_rc_dec_func(heap_start, free_func_idx));
        codes.function(&memory::emit_gc_collect_func(heap_start, free_func_idx));

        // Phase 7: I/O 运行时函数 (4 variants × 3 types)
        // println (fd=1, newline=true)
        codes.function(&self.emit_output_i64(1, true));
        codes.function(&self.emit_output_str(1, true));
        codes.function(&self.emit_output_bool(1, true));
        // print (fd=1, newline=false)
        codes.function(&self.emit_output_i64(1, false));
        codes.function(&self.emit_output_str(1, false));
        codes.function(&self.emit_output_bool(1, false));
        // eprintln (fd=2, newline=true)
        codes.function(&self.emit_output_i64(2, true));
        codes.function(&self.emit_output_str(2, true));
        codes.function(&self.emit_output_bool(2, true));
        // eprint (fd=2, newline=false)
        codes.function(&self.emit_output_i64(2, false));
        codes.function(&self.emit_output_str(2, false));
        codes.function(&self.emit_output_bool(2, false));

        // Phase 7.3: math 运行时函数
        codes.function(&self.emit_math_sin());
        codes.function(&self.emit_math_cos());
        codes.function(&self.emit_math_tan());
        codes.function(&self.emit_math_exp());
        codes.function(&self.emit_math_log());
        codes.function(&self.emit_math_pow());

        // Phase 7.1 #44: readln 运行时函数
        codes.function(&self.emit_readln(heap_start));

        // Phase 7.2: str_to_i64, str_to_f64 运行时函数
        codes.function(&self.emit_str_to_i64());
        codes.function(&self.emit_str_to_f64());

        // Phase 7.7: 运行时包装函数
        codes.function(&self.emit_get_time_ns());
        codes.function(&self.emit_random_i64());
        codes.function(&self.emit_random_f64());
        codes.function(&self.emit_get_args());
        codes.function(&self.emit_get_env());
        codes.function(&self.emit_exit());
        // Phase 7.4: 格式化运行时函数
        codes.function(&self.emit_i64_format());
        codes.function(&self.emit_f64_format());
        // Phase 7.5: 集合类型运行时函数
        codes.function(&self.emit_arraylist_new());
        codes.function(&self.emit_arraylist_append());
        codes.function(&self.emit_arraylist_get());
        codes.function(&self.emit_arraylist_set());
        codes.function(&self.emit_arraylist_remove());
        codes.function(&self.emit_hashmap_new());
        codes.function(&self.emit_hashmap_put());
        codes.function(&self.emit_hashmap_get());
        codes.function(&self.emit_hashmap_contains());
        codes.function(&self.emit_hashmap_remove());
        codes.function(&self.emit_linkedlist_new());
        codes.function(&self.emit_linkedlist_append());
        codes.function(&self.emit_linkedlist_prepend());
        codes.function(&self.emit_linkedlist_get());
        // Phase 7.8: 字符串操作 / 排序
        codes.function(&self.emit_str_contains());
        codes.function(&self.emit_str_index_of());
        codes.function(&self.emit_str_replace());
        codes.function(&self.emit_str_split());
        codes.function(&self.emit_str_to_rune_array());
        codes.function(&self.emit_sort_array());
        codes.function(&self.emit_str_substring());
        codes.function(&self.emit_hashcode_i64());
        // P2.10: String 方法
        codes.function(&self.emit_str_trim());
        codes.function(&self.emit_str_starts_with());
        codes.function(&self.emit_str_ends_with());

        // 7. Element 段 (Element Section) — vtable + lambda 函数引用
        // 注意: WASM 规范要求 Element 在 Code 之前 (Type→Import→Function→Table→Memory→Global→Export→Element→Code→Data)
        {
            let mut all_table_entries: Vec<u32> = self.vtable_entries.clone();
            all_table_entries.extend_from_slice(&lambda_func_indices_for_table);
            if !all_table_entries.is_empty() {
                let mut elements = ElementSection::new();
                elements.active(
                    Some(0), // table index
                    &ConstExpr::i32_const(0),
                    Elements::Functions(std::borrow::Cow::Borrowed(&all_table_entries)),
                );
                module.section(&elements);
            }
        }

        module.section(&codes);

        // 8. 数据段 (Data Section) - 字符串常量
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

    /// P2.3: 简单推断 Lambda body 的返回类型
    fn infer_lambda_return_type(body: &Expr, params: &[(String, Type)]) -> Option<Type> {
        match body {
            Expr::Integer(_) => Some(Type::Int64),
            Expr::Float(_) => Some(Type::Float64),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::String(_) => Some(Type::String),
            Expr::Var(name) => {
                params.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone())
            }
            Expr::Binary { op, left, right } => {
                match op {
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq
                    | BinOp::LogicalAnd | BinOp::LogicalOr | BinOp::NotIn => Some(Type::Bool),
                    _ => {
                        Self::infer_lambda_return_type(left, params)
                            .or_else(|| Self::infer_lambda_return_type(right, params))
                    }
                }
            }
            Expr::If { then_branch, .. } => Self::infer_lambda_return_type(then_branch, params),
            Expr::Block(_, Some(tail)) => Self::infer_lambda_return_type(tail, params),
            _ => Some(Type::Int64), // 默认推断为 Int64
        }
    }

    /// Lambda 预扫描：递归遍历所有函数体，收集 Lambda 表达式并生成匿名函数
    fn collect_lambdas_from_functions(
        functions: &[FuncDef],
        counter: &mut u32,
        out: &mut Vec<FuncDef>,
    ) {
        for func in functions {
            for stmt in &func.body {
                Self::collect_lambdas_from_stmt(stmt, counter, out);
            }
        }
    }

    /// 从所有函数体中收集 Stmt::LocalFunc，以便提前加入 functions 列表并分配索引
    fn collect_local_funcs_from_functions(functions: &[FuncDef], out: &mut Vec<FuncDef>) {
        for func in functions {
            Self::collect_local_funcs_from_stmts(&func.body, out);
        }
    }

    fn collect_local_funcs_from_stmts(stmts: &[Stmt], out: &mut Vec<FuncDef>) {
        for stmt in stmts {
            if let Stmt::LocalFunc(ref f) = stmt {
                out.push(f.clone());
                Self::collect_local_funcs_from_stmts(&f.body, out);
            }
        }
    }

    fn collect_lambdas_from_stmt(stmt: &Stmt, counter: &mut u32, out: &mut Vec<FuncDef>) {
        match stmt {
            Stmt::Let { value, .. } => {
                Self::collect_lambdas_from_expr(value, counter, out);
            }
            Stmt::Var { value: Some(value), .. } => {
                Self::collect_lambdas_from_expr(value, counter, out);
            }
            Stmt::Assign { value, .. } => {
                Self::collect_lambdas_from_expr(value, counter, out);
            }
            Stmt::Expr(e) => Self::collect_lambdas_from_expr(e, counter, out),
            Stmt::Return(Some(e)) => Self::collect_lambdas_from_expr(e, counter, out),
            Stmt::While { cond, body, .. } => {
                Self::collect_lambdas_from_expr(cond, counter, out);
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            Stmt::DoWhile { body, cond } => {
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
                Self::collect_lambdas_from_expr(cond, counter, out);
            }
            Stmt::For { iterable, body, .. } => {
                Self::collect_lambdas_from_expr(iterable, counter, out);
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            Stmt::Loop { body, .. } => {
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            Stmt::UnsafeBlock { body } => {
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            Stmt::Const { value, .. } => {
                Self::collect_lambdas_from_expr(value, counter, out);
            }
            _ => {}
        }
    }

    fn collect_lambdas_from_expr(expr: &Expr, counter: &mut u32, out: &mut Vec<FuncDef>) {
        match expr {
            Expr::Lambda { params, return_type, body } => {
                let lambda_name = format!("__lambda_{}", *counter);
                *counter += 1;
                // P2.3: 推断 Lambda 返回类型（如果未显式标注）
                let inferred_ret = if return_type.is_some() {
                    return_type.clone()
                } else {
                    // 简单推断：从 body 表达式推断
                    Self::infer_lambda_return_type(body, params)
                };
                // 将 Lambda body 包装为 return 语句
                let body_stmt = vec![Stmt::Return(Some(body.as_ref().clone()))];
                out.push(FuncDef {
                    visibility: Visibility::Public,
                    name: lambda_name,
                    type_params: vec![],
                    constraints: vec![],
                    params: params.iter().map(|(name, ty)| Param {
                        name: name.clone(),
                        ty: ty.clone(),
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    }).collect(),
                    return_type: inferred_ret,
                    throws: None,
                    body: body_stmt,
                    extern_import: None,
                });
                // 递归处理 Lambda body
                Self::collect_lambdas_from_expr(body, counter, out);
            }
            Expr::Binary { left, right, .. } => {
                Self::collect_lambdas_from_expr(left, counter, out);
                Self::collect_lambdas_from_expr(right, counter, out);
            }
            Expr::Unary { expr, .. } => Self::collect_lambdas_from_expr(expr, counter, out),
            Expr::Call { args, .. } => {
                for a in args { Self::collect_lambdas_from_expr(a, counter, out); }
            }
            Expr::MethodCall { object, args, .. } => {
                Self::collect_lambdas_from_expr(object, counter, out);
                for a in args { Self::collect_lambdas_from_expr(a, counter, out); }
            }
            Expr::If { cond, then_branch, else_branch, .. } => {
                Self::collect_lambdas_from_expr(cond, counter, out);
                Self::collect_lambdas_from_expr(then_branch, counter, out);
                if let Some(e) = else_branch { Self::collect_lambdas_from_expr(e, counter, out); }
            }
            Expr::Block(stmts, expr) => {
                for s in stmts { Self::collect_lambdas_from_stmt(s, counter, out); }
                if let Some(e) = expr { Self::collect_lambdas_from_expr(e, counter, out); }
            }
            Expr::Array(elems) | Expr::Tuple(elems) => {
                for e in elems { Self::collect_lambdas_from_expr(e, counter, out); }
            }
            Expr::ConstructorCall { args, .. } => {
                for a in args { Self::collect_lambdas_from_expr(a, counter, out); }
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields { Self::collect_lambdas_from_expr(e, counter, out); }
            }
            _ => {}
        }
    }

    /// 编译函数
    fn compile_function(&self, func: &FuncDef) -> WasmFunc {
        // --- throws 声明验证 (#38) ---
        // 如果函数声明了 throws，但函数体中包含的 throw 表达式是合法的
        // 如果函数没有声明 throws 但包含 throw，发出警告
        if func.throws.is_none() && Self::contains_unhandled_throw(&func.body) {
            eprintln!("[warning] 函数 '{}' 包含 throw 但未声明 throws", func.name);
        }

        let mut locals = LocalsBuilder::new();

        // 检查是否为 init 函数（__ClassName_init）
        let is_init = func.name.starts_with("__") && func.name.ends_with("_init");
        let init_class_name = if is_init {
            Some(func.name.strip_prefix("__").unwrap().strip_suffix("_init").unwrap().to_string())
        } else {
            None
        };

        // 添加参数作为局部变量（含 AST 类型，便于字段访问计算偏移）
        for param in &func.params {
            // 可变参数类型转为 Array<T>
            let actual_ty = if param.variadic {
                Type::Array(Box::new(param.ty.clone()))
            } else {
                param.ty.clone()
            };
            locals.add(&param.name, actual_ty.to_wasm(), Some(actual_ty));
        }

        // init 函数额外添加 this 局部变量
        if let Some(ref class_name) = init_class_name {
            locals.add("this", ValType::I32, Some(Type::Struct(class_name.clone(), vec![])));
        }

        // 收集函数体中的局部变量
        for stmt in &func.body {
            self.collect_locals(stmt, &mut locals);
        }
        // 逻辑或短路求值用临时变量
        locals.add("__logical_tmp", ValType::I32, None);
        // ? 运算符临时指针
        locals.add("__try_ptr", ValType::I32, None);
        // Phase 8: 内存管理临时变量（__alloc 返回指针暂存）
        locals.add("__struct_alloc_ptr", ValType::I32, None);
        locals.add("__array_alloc_ptr", ValType::I32, None);
        locals.add("__tuple_alloc_ptr", ValType::I32, None);
        locals.add("__enum_alloc_ptr", ValType::I32, None);
        locals.add("__range_alloc_ptr", ValType::I32, None);

        // 创建 WASM 函数
        let local_types: Vec<(u32, ValType)> = locals
            .types
            .iter()
            .skip(func.params.len())
            .map(|t| (1, *t))
            .collect();

        let mut wasm_func = WasmFunc::new(local_types);

        // init 函数前序：分配内存 + 设置 vtable_ptr (Phase 8: 使用 __alloc)
        if let Some(ref class_name) = init_class_name {
            if let Some(class_info) = self.classes.get(class_name) {
                let obj_size = class_info.object_size();
                let alloc_idx = self.func_indices["__alloc"];
                // this = __alloc(obj_size)
                wasm_func.instruction(&Instruction::I32Const(obj_size as i32));
                wasm_func.instruction(&Instruction::Call(alloc_idx));
                let this_idx = locals.get("this").expect("this 局部变量");
                wasm_func.instruction(&Instruction::LocalSet(this_idx));
                // 设置 vtable_ptr（如果有 vtable）
                if class_info.has_vtable && !class_info.vtable_methods.is_empty() {
                    wasm_func.instruction(&Instruction::LocalGet(this_idx));
                    wasm_func.instruction(&Instruction::I32Const(class_info.vtable_base as i32));
                    wasm_func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                }
            }
        }

        // P3.11: 在 main 函数入口调用所有 static init
        if func.name == "main" {
            for (fname, idx) in &self.func_indices {
                if fname.ends_with(".__static_init") {
                    wasm_func.instruction(&Instruction::Call(*idx));
                }
            }
        }
        // 编译函数体（顶层无循环上下文）
        // 特殊处理：最后一条 Stmt::Expr 若产生值，则作为隐式返回值（不 drop）
        let body_len = func.body.len();
        let has_return_type = func.return_type.as_ref().map_or(false, |t| *t != Type::Unit);
        for (i, stmt) in func.body.iter().enumerate() {
            let is_last = i == body_len - 1;
            if is_last && has_return_type {
                if let Stmt::Expr(expr) = stmt {
                    if self.expr_produces_value(expr) {
                        // 最后一条表达式语句作为隐式返回
                        self.compile_expr(expr, &locals, &mut wasm_func, None);
                        wasm_func.instruction(&Instruction::Return);
                        continue;
                    }
                }
            }
            self.compile_stmt(stmt, &locals, &mut wasm_func, None);
        }

        // init 函数后序：返回 this 指针
        if init_class_name.is_some() {
            if let Some(this_idx) = locals.get("this") {
                wasm_func.instruction(&Instruction::LocalGet(this_idx));
                wasm_func.instruction(&Instruction::Return);
            }
        }

        // Phase 8: 函数退出前对所有堆类型局部变量执行 rc_dec
        // 注意：仅对非返回值的局部变量执行，init 函数的 this 除外
        if let Some(rc_dec_idx) = self.func_indices.get("__rc_dec").copied() {
            let return_var = func.body.last().and_then(|s| {
                if let Stmt::Return(Some(Expr::Var(name))) = s {
                    Some(name.as_str())
                } else {
                    None
                }
            });
            for (name, &idx) in &locals.names {
                // 跳过内部临时变量、参数、返回值
                if name.starts_with("__") { continue; }
                if init_class_name.is_some() && name == "this" { continue; }
                if Some(name.as_str()) == return_var { continue; }
                if let Some(ast_ty) = locals.get_type(name) {
                    if memory::is_heap_type(ast_ty) || memory::may_hold_heap_ptr(ast_ty) {
                        wasm_func.instruction(&Instruction::LocalGet(idx));
                        wasm_func.instruction(&Instruction::Call(rc_dec_idx));
                    }
                }
            }
        }

        // 如果函数有返回类型，在函数末尾添加 unreachable 指令
        // 这处理了所有路径都通过 return 退出的情况（如 match 所有分支都 return）
        // 没有 unreachable，WASM 验证器会报 "nothing on stack" 错误
        if func.return_type.as_ref().map_or(false, |t| *t != Type::Unit) && init_class_name.is_none() {
            wasm_func.instruction(&Instruction::Unreachable);
        }

        wasm_func.instruction(&Instruction::End);
        wasm_func
    }

    /// 生成 __pow_i64(base: i64, exp: i64) -> i64 辅助函数体（局部 0=base, 1=exp, 2=result）
    fn emit_pow_i64(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![(1, ValType::I64)]);
        // exp < 0 -> return 0
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // result = 1
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::LocalSet(2));
        // loop: if exp <= 0 break; result *= base; exp--
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LeS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
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

    /// 生成 __str_concat(ptr1: i32, ptr2: i32) -> i32 辅助函数
    /// 内存布局: [len: i32][bytes...]
    /// 逻辑: 读取两个字符串的长度，分配新空间，复制两部分，返回新指针
    fn emit_str_concat(&self) -> WasmFunc {
        // 局部变量: 0=ptr1, 1=ptr2, 2=len1, 3=len2, 4=total_len, 5=new_ptr
        let mut f = WasmFunc::new(vec![(4, ValType::I32)]);

        // len1 = mem[ptr1]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalSet(2));

        // len2 = mem[ptr2]
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalSet(3));

        // total_len = len1 + len2
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));

        // Phase 8: 使用 __alloc 分配新空间
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::LocalGet(4)); // total_len
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add); // total_len + 4
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(5)); // new_ptr

        // mem[new_ptr] = total_len
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

        // 复制第一个字符串 (memory.copy new_ptr+4, ptr1+4, len1)
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });

        // 复制第二个字符串 (memory.copy new_ptr+4+len1, ptr2+4, len2)
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });

        // return new_ptr
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __i64_to_str(val: i64) -> i32 辅助函数
    /// 将 i64 值转换为十进制字符串，返回堆上字符串指针 [len: i32][bytes...]
    /// 使用 I/O 缓冲区 (地址 0-63) 作为临时空间，然后 alloc + memory.copy
    fn emit_i64_to_str(&self) -> WasmFunc {
        let alloc_idx = self.func_indices["__alloc"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };
        // 参数: local 0 = val (i64)
        // 局部变量: local 1 = pos (i32), local 2 = is_neg (i32), local 3 = len (i32), local 4 = ptr (i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // pos
            (1, ValType::I32), // is_neg
            (1, ValType::I32), // len
            (1, ValType::I32), // ptr
        ]);

        // pos = 23 (缓冲区末尾)
        f.instruction(&Instruction::I32Const(23));
        f.instruction(&Instruction::LocalSet(1));

        // if value == 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // pos = 22, buf[22] = '0'
            f.instruction(&Instruction::I32Const(22));
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::I32Const(22));
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
        }
        f.instruction(&Instruction::Else);
        {
            // is_neg = (val < 0)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(2));

            // if is_neg: val = 0 - val
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(0));
            f.instruction(&Instruction::End);

            // while val > 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::BrIf(1));

            // pos--
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(1));

            // buf[pos] = (val %u 10) + '0'
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(10));
            f.instruction(&Instruction::I64RemU);
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Store8(mem(0, 0)));

            // val = val /u 10
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(10));
            f.instruction(&Instruction::I64DivU);
            f.instruction(&Instruction::LocalSet(0));

            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End); // end loop
            f.instruction(&Instruction::End); // end block

            // if is_neg: prepend '-'
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End); // end if val == 0

        // len = 23 - pos (数字字符串在 buf[pos..23) 中，位置 23 不含)
        f.instruction(&Instruction::I32Const(23));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(3));

        // ptr = __alloc(len + 4)
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(4));

        // mem[ptr] = len
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // memory.copy(ptr + 4, pos, len)  — 将 buf[pos..pos+len] 复制到 ptr+4
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);  // dst = ptr + 4
        f.instruction(&Instruction::LocalGet(1)); // src = pos
        f.instruction(&Instruction::LocalGet(3)); // len
        f.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });

        // return ptr
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __i32_to_str(val: i32) -> i32 辅助函数
    /// 将 i32 值扩展为 i64 后调用 __i64_to_str
    fn emit_i32_to_str(&self) -> WasmFunc {
        let i64_to_str_idx = self.func_indices["__i64_to_str"];
        let mut f = WasmFunc::new(vec![]);
        // i32 -> i64 (符号扩展) -> __i64_to_str
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::Call(i64_to_str_idx));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __f64_to_str(val: f64) -> i32 辅助函数
    /// 简化实现：输出整数部分 + "." + 小数部分（最多 6 位）
    /// 使用 I/O 缓冲区 (地址 32-63) 作为临时空间
    fn emit_f64_to_str(&self) -> WasmFunc {
        let alloc_idx = self.func_indices["__alloc"];
        let i64_to_str_idx = self.func_indices["__i64_to_str"];
        let str_concat_idx = self.func_indices["__str_concat"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };
        // 参数: local 0 = val (f64)
        // 局部变量: local 1 = int_str (i32), local 2 = frac_str (i32),
        //          local 3 = result (i32), local 4 = frac_val (i64),
        //          local 5 = dot_str (i32), local 6 = is_neg (i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
            (1, ValType::I64), (1, ValType::I32), (1, ValType::I32),
        ]);

        // 整数部分字符串: __i64_to_str(trunc(val))
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::Call(i64_to_str_idx));
        f.instruction(&Instruction::LocalSet(1)); // int_str

        // 创建 "." 字符串: alloc(5), len=1, byte='.'
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(5)); // dot_str
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(46)); // '.'
        f.instruction(&Instruction::I32Store8(mem(4, 0)));

        // 小数部分: frac = abs((val - trunc(val)) * 1000000) 取整后得到6位小数
        // is_neg = val < 0.0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(0.0));
        f.instruction(&Instruction::F64Lt);
        f.instruction(&Instruction::LocalSet(6));

        // frac_f = val - trunc(val), 然后取绝对值
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Trunc);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::F64Abs);
        f.instruction(&Instruction::F64Const(1000000.0));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::F64Nearest); // 四舍五入
        f.instruction(&Instruction::I64TruncF64U);
        f.instruction(&Instruction::LocalSet(4)); // frac_val

        // frac_str = __i64_to_str(frac_val)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::Call(i64_to_str_idx));
        f.instruction(&Instruction::LocalSet(2)); // frac_str

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

    /// 生成 __f32_to_str(val: f32) -> i32 辅助函数
    /// 提升为 f64 后调用 __f64_to_str
    fn emit_f32_to_str(&self) -> WasmFunc {
        let f64_to_str_idx = self.func_indices["__f64_to_str"];
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64PromoteF32);
        f.instruction(&Instruction::Call(f64_to_str_idx));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __bool_to_str(val: i32) -> i32 辅助函数
    fn emit_bool_to_str(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

        // if val == 0 return "false" else return "true"
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));

        // "false" (5 bytes) - Phase 8: 使用 __alloc
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::I32Const(9));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x736C6166)); // "fals"
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x65)); // "e"
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg { offset: 8, align: 0, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));

        f.instruction(&Instruction::Else);

        // "true" (4 bytes) - Phase 8: 使用 __alloc
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x65757274)); // "true"
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));

        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 标准库雏形：min(a, b) -> i64，局部 0=a, 1=b
    fn emit_min_i64(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 标准库雏形：max(a, b) -> i64
    fn emit_max_i64(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 标准库雏形：abs(x) -> i64
    fn emit_abs_i64(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    // =========================================================================
    // Phase 9: println 运行时函数实现（基于 WASI fd_write）
    // =========================================================================

    /// 生成 __println_i64(value: i64) -> () 函数
    /// 将 Int64 值转换为十进制字符串并输出到 stdout，末尾加换行符
    /// 使用内存地址 0-63 作为数字转换缓冲区，64-75 作为 iovec/nwritten
    /// 生成 output_i64(val: i64) 函数，参数化 fd 和 newline
    fn emit_output_i64(&self, fd: i32, newline: bool) -> WasmFunc {
        let fd_write_idx = self.func_indices["__wasi_fd_write"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };

        let end_pos: i32 = if newline { 23 } else { 22 }; // 23 留给 '\n'，22 为数字末尾
        let buf_len_base: i32 = if newline { 24 } else { 22 };

        let mut f = WasmFunc::new(vec![(1, ValType::I32), (1, ValType::I32)]);

        // pos = end_pos
        f.instruction(&Instruction::I32Const(end_pos));
        f.instruction(&Instruction::LocalSet(1));

        if newline {
            // mem[23] = '\n'
            f.instruction(&Instruction::I32Const(23));
            f.instruction(&Instruction::I32Const(10));
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
        }

        // if value == 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            let zero_pos = end_pos - 1;
            f.instruction(&Instruction::I32Const(zero_pos));
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::I32Const(zero_pos));
            f.instruction(&Instruction::I32Const(48)); // '0'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
        }
        f.instruction(&Instruction::Else);
        {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
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
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I64Const(10));
                f.instruction(&Instruction::I64DivU);
                f.instruction(&Instruction::LocalSet(0));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // iovec: buf = pos, len = buf_len_base - pos
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::I32Const(buf_len_base));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Store(mem(4, 2)));

        // fd_write(fd, IOVEC_OFFSET, 1, NWRITTEN_OFFSET)
        f.instruction(&Instruction::I32Const(fd));
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
        f.instruction(&Instruction::Call(fd_write_idx));
        f.instruction(&Instruction::Drop);

        f.instruction(&Instruction::End);
        f
    }

    /// 生成 output_str(ptr: i32) 函数，参数化 fd 和 newline
    fn emit_output_str(&self, fd: i32, newline: bool) -> WasmFunc {
        let fd_write_idx = self.func_indices["__wasi_fd_write"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };

        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

        // len = mem[ptr]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));

        // iovec.buf = ptr + 4
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // iovec.len = len
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(4, 2)));

        // fd_write(fd, ...)
        f.instruction(&Instruction::I32Const(fd));
        f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
        f.instruction(&Instruction::Call(fd_write_idx));
        f.instruction(&Instruction::Drop);

        if newline {
            // 输出换行符
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(10));
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::I32Const(fd));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
            f.instruction(&Instruction::Call(fd_write_idx));
            f.instruction(&Instruction::Drop);
        }

        f.instruction(&Instruction::End);
        f
    }

    /// 生成 output_bool(val: i32) 函数，参数化 fd 和 newline
    fn emit_output_bool(&self, fd: i32, newline: bool) -> WasmFunc {
        let fd_write_idx = self.func_indices["__wasi_fd_write"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };

        let mut f = WasmFunc::new(vec![]);

        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // "true" (+ optional '\n')
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(0x65757274_u32 as i32)); // "true" little-endian
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            let len = if newline {
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                5
            } else {
                4
            };
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(len));
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::I32Const(fd));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
            f.instruction(&Instruction::Call(fd_write_idx));
            f.instruction(&Instruction::Drop);
        }
        f.instruction(&Instruction::Else);
        {
            // "false" (+ optional '\n')
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(0x736C6166_u32 as i32)); // "fals" little-endian
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Const(101)); // 'e'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            let len = if newline {
                f.instruction(&Instruction::I32Const(5));
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                6
            } else {
                5
            };
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(len));
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::I32Const(fd));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
            f.instruction(&Instruction::Call(fd_write_idx));
            f.instruction(&Instruction::Drop);
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __math_sin(x: f64) -> f64 (泰勒级数, 17 项)
    fn emit_math_sin(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };
        let _ = mem; // 不需要内存操作
        // local 0 = x (f64), local 1 = term (f64), local 2 = sum (f64),
        // local 3 = x_sq (f64), local 4 = i (f64, 循环计数器)
        let mut f = WasmFunc::new(vec![
            (1, ValType::F64), (1, ValType::F64), (1, ValType::F64), (1, ValType::F64),
        ]);

        // 先将 x 归约到 [-π, π] 范围: x = x - round(x / (2*PI)) * (2*PI)
        // PI ≈ 3.141592653589793
        let two_pi = std::f64::consts::TAU;
        // x = x - trunc(x / TWO_PI) * TWO_PI (简化归约)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(two_pi));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::F64Nearest);
        f.instruction(&Instruction::F64Const(two_pi));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::LocalSet(0));

        // sum = x, term = x, x_sq = x * x
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2)); // sum = x
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(1)); // term = x
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::LocalSet(3)); // x_sq = x*x
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(4)); // i = 1

        // 12 iterations: term = -term * x_sq / ((2i)*(2i+1)), sum += term
        for _ in 0..12 {
            // term = -term * x_sq
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::F64Neg);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Mul);
            // / (2*i * (2*i + 1))
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalTee(1)); // temp = 2*i (reuse local 1 briefly)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add); // 2*i + 1
            f.instruction(&Instruction::F64Mul); // (2i) * (2i+1)
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::LocalSet(1)); // term = result
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

        f.instruction(&Instruction::LocalGet(2)); // return sum
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __math_cos(x: f64) -> f64 : cos(x) = sin(x + PI/2)
    fn emit_math_cos(&self) -> WasmFunc {
        let sin_idx = self.func_indices["__math_sin"];
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(std::f64::consts::FRAC_PI_2));
        f.instruction(&Instruction::F64Add);
        f.instruction(&Instruction::Call(sin_idx));
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __math_tan(x: f64) -> f64 : tan(x) = sin(x) / cos(x)
    fn emit_math_tan(&self) -> WasmFunc {
        let sin_idx = self.func_indices["__math_sin"];
        let cos_idx = self.func_indices["__math_cos"];
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(sin_idx));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(cos_idx));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __math_exp(x: f64) -> f64 (泰勒级数: e^x = 1 + x + x²/2! + x³/3! + ...)
    fn emit_math_exp(&self) -> WasmFunc {
        // local 0 = x, local 1 = term, local 2 = sum, local 3 = i
        let mut f = WasmFunc::new(vec![
            (1, ValType::F64), (1, ValType::F64), (1, ValType::F64),
        ]);

        // sum = 1, term = 1, i = 1
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(3));

        // 20 iterations: term = term * x / i; sum += term
        for _ in 0..20 {
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::LocalSet(1)); // term = term * x / i
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(2)); // sum += term
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(3)); // i += 1
        }

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __math_log(x: f64) -> f64 (自然对数, 使用 ln(x) = 2 * atanh((x-1)/(x+1)))
    fn emit_math_log(&self) -> WasmFunc {
        // local 0 = x, local 1 = y = (x-1)/(x+1), local 2 = y_sq, local 3 = term
        // local 4 = sum, local 5 = i
        let mut f = WasmFunc::new(vec![
            (1, ValType::F64), (1, ValType::F64), (1, ValType::F64),
            (1, ValType::F64), (1, ValType::F64),
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

        // 40 iterations: term *= y_sq; sum += term / (2*i + 1); i += 1
        for _ in 0..40 {
            // term *= y_sq
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalSet(3));
            // i += 1 (i 从 1 开始，第一次变成 2，对应分母 2*1+1=3... 不对)
            // 修正: 先计算分母 = 2*i + 1 (用当前 i)，再递增 i
            // 但 i 初始为 1，第一个 term 为 y^3，分母应为 3 = 2*1+1 ✓
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            // denominator = 2*i + 1
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::F64Const(2.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::F64Const(1.0));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::LocalSet(4)); // sum += term / (2i+1)
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

    /// 生成 __math_pow(base: f64, exp: f64) -> f64 : pow = exp(exp * log(base))
    fn emit_math_pow(&self) -> WasmFunc {
        let exp_idx = self.func_indices["__math_exp"];
        let log_idx = self.func_indices["__math_log"];
        // local 0 = base, local 1 = exponent
        let mut f = WasmFunc::new(vec![]);
        // exp(exponent * log(base))
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(log_idx));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::Call(exp_idx));
        f.instruction(&Instruction::End);
        f
    }

    /// Phase 7.1 #44: 生成 __readln() -> i32 (字符串指针)
    /// 从 stdin (fd=0) 逐字节读取直到 '\n' 或 EOF
    /// 使用线性内存临时缓冲区，最终分配字符串对象
    fn emit_readln(&self, _heap_start: i32) -> WasmFunc {
        let fd_read_idx = self.func_indices["__wasi_fd_read"];
        let alloc_idx = self.func_indices["__alloc"];
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };

        // 临时缓冲区: 使用内存低位区域 (偏移 128-1024，最多 896 字节输入)
        // 注意: IOVEC_OFFSET=64, NWRITTEN_OFFSET=72 已占用，需避开
        const BUF_START: i32 = 128;
        const BUF_MAX: i32 = 896;
        // iovec 结构在偏移 32-40

        // local 0 = pos (i32, 当前写入位置), local 1 = nread (i32), local 2 = byte (i32)
        // local 3 = result_ptr (i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
        ]);

        // pos = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(0));

        // loop: 逐字节读取
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // outer block for break
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));  // loop start
        {
            // 检查缓冲区溢出: if pos >= BUF_MAX then break
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(BUF_MAX));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1)); // break outer

            // 设置 iovec: buf = BUF_START + pos, len = 1 (读一个字节)
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(BUF_START));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Store(mem(4, 2)));

            // nread_offset = NWRITTEN_OFFSET (复用)
            // fd_read(0, IOVEC_OFFSET, 1, NWRITTEN_OFFSET)
            f.instruction(&Instruction::I32Const(0)); // fd = stdin
            f.instruction(&Instruction::I32Const(IOVEC_OFFSET));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
            f.instruction(&Instruction::Call(fd_read_idx));
            f.instruction(&Instruction::Drop); // 忽略 errno

            // nread = mem[NWRITTEN_OFFSET]
            f.instruction(&Instruction::I32Const(NWRITTEN_OFFSET));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::LocalSet(1));

            // if nread == 0 then EOF, break
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(1)); // break outer

            // byte = mem[BUF_START + pos]
            f.instruction(&Instruction::I32Const(BUF_START));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::LocalSet(2));

            // pos += 1
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(0));

            // if byte == '\n' then break
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(10)); // '\n'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::BrIf(1)); // break outer

            // continue loop
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block

        // 去掉尾部的 '\n' (如果有)
        // if pos > 0 && mem[BUF_START + pos - 1] == '\n': pos -= 1
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::I32Const(BUF_START));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::I32Const(10));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(0));
            f.instruction(&Instruction::End);
            // 也去掉 \r (Windows CRLF)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32GtS);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(BUF_START));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::I32Const(13)); // '\r'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // 分配字符串对象: alloc(pos + 4) — [len:i32][bytes...]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(3)); // result_ptr

        // 写入长度: mem[result_ptr] = pos
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // 复制字节: memory.copy(result_ptr + 4, BUF_START, pos)
        // WASM memory.copy 可能不被所有运行时支持，使用循环复制
        // i = 0; while i < pos: mem[result_ptr+4+i] = mem[BUF_START+i]; i++
        // 复用 local 1 作为 i
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(1)); // i = 0

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // if i >= pos, break
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1));

            // mem[result_ptr + 4 + i] = mem[BUF_START + i]
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Add); // dest addr

            f.instruction(&Instruction::I32Const(BUF_START));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Add); // src addr
            f.instruction(&Instruction::I32Load8U(mem(0, 0))); // load byte

            f.instruction(&Instruction::I32Store8(mem(0, 0))); // store byte

            // i++
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(1));

            f.instruction(&Instruction::Br(0)); // continue
        }
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block

        // 返回 result_ptr
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::End);
        f
    }

    // =========== Phase 7.7: WASI 运行时包装函数 ===========

    /// __get_time_ns() -> i64: 调用 clock_time_get(0=realtime, 1=precision, scratch) 返回纳秒
    fn emit_get_time_ns(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        // clock_time_get(clock_id=0(realtime), precision=1, buf=WASI_SCRATCH)
        f.instruction(&Instruction::I32Const(0));            // clock_id = realtime
        f.instruction(&Instruction::I64Const(1));            // precision = 1ns
        f.instruction(&Instruction::I32Const(WASI_SCRATCH)); // output buffer
        f.instruction(&Instruction::Call(self.func_indices["__wasi_clock_time_get"]));
        f.instruction(&Instruction::Drop);                   // drop errno
        // 读取 i64 时间戳
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// __random_i64() -> i64: 调用 random_get(buf, 8) 返回随机 i64
    fn emit_random_i64(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::Call(self.func_indices["__wasi_random_get"]));
        f.instruction(&Instruction::Drop);
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// __random_f64() -> f64: 随机 i64 转为 [0.0, 1.0) 的 f64
    fn emit_random_f64(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::Call(self.func_indices["__random_i64"]));
        // i64 → 无符号右移 11 位得到 53 位正整数 → f64
        f.instruction(&Instruction::I64Const(11));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::F64ConvertI64U);
        // 除以 2^53 归一化到 [0, 1)
        f.instruction(&Instruction::F64Const(9007199254740992.0)); // 2^53
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f
    }

    /// __get_args() -> i32: 调用 args_sizes_get + args_get, 返回 Array<String> 指针
    /// 数组布局: [len: i32][str_ptr0: i64][str_ptr1: i64]...
    fn emit_get_args(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0 无参数; 1=argc(i32), 2=buf_size(i32), 3=argv_ptrs(i32),
        //         4=argv_buf(i32), 5=result(i32), 6=i(i32), 7=str_ptr(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 1: argc
            (1, ValType::I32), // 2: buf_size
            (1, ValType::I32), // 3: argv_ptrs (指向 char* 数组)
            (1, ValType::I32), // 4: argv_buf  (字符缓冲区)
            (1, ValType::I32), // 5: result (输出数组)
            (1, ValType::I32), // 6: i
            (1, ValType::I32), // 7: str_ptr
            (1, ValType::I32), // 8: str_len
        ]);

        // 调用 args_sizes_get(&argc, &buf_size)
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));      // argc 存储位置
        f.instruction(&Instruction::I32Const(WASI_SCRATCH + 4));  // buf_size 存储位置
        f.instruction(&Instruction::Call(self.func_indices["__wasi_args_sizes_get"]));
        f.instruction(&Instruction::Drop);
        // 读取 argc 和 buf_size
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(0)); // argc
        f.instruction(&Instruction::I32Const(WASI_SCRATCH + 4));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1)); // buf_size

        // 分配 argv_ptrs = __alloc(argc * 4), argv_buf = __alloc(buf_size)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(2)); // argv_ptrs

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(3)); // argv_buf

        // 调用 args_get(argv_ptrs, argv_buf)
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(self.func_indices["__wasi_args_get"]));
        f.instruction(&Instruction::Drop);

        // 分配结果数组: [len: i32][ptr0: i64][ptr1: i64]...
        f.instruction(&Instruction::I32Const(4)); // len field
        f.instruction(&Instruction::LocalGet(0)); // argc
        f.instruction(&Instruction::I32Const(8)); // 每个元素 i64 = 8 字节
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(4)); // result

        // result[0] = argc (数组长度)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // 循环: 为每个 arg 创建字符串并存入数组
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5)); // i = 0
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // if i >= argc → break
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));

            // c_str = argv_ptrs[i * 4] (char* 指针)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::LocalSet(6)); // c_str

            // 计算 c 字符串长度: 扫描到 0
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(7)); // str_len = 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Eqz);
                f.instruction(&Instruction::BrIf(1)); // 遇到 0 跳出
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(7));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End); // block
            f.instruction(&Instruction::End); // loop

            // 分配字符串: __alloc(str_len + 4), 写入 [len][bytes]
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            // 保存到临时变量 — 复用 local 6 (不再需要 c_str 原值)
            // 但我们还需要 c_str 做 memory.copy，所以保存 str_ptr 到 WASI_SCRATCH
            f.instruction(&Instruction::LocalTee(6)); // 此时 local 6 = 新分配的 str_ptr

            // 写入长度
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7)); // str_len
            f.instruction(&Instruction::I32Store(mem(0, 2)));

            // 复制字节: 简单循环 (从 argv 缓冲区到新字符串)
            // 重新获取 c_str 从 argv_ptrs
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            // stack: c_str
            // memory.copy(str_ptr+4, c_str, str_len)
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            // stack: c_str, dest
            // 需要重排: dest, src, len
            // WASM MemoryCopy 的操作数顺序: dest, src, len (stack top = len)
            // 当前 stack: c_str, dest
            // 我们需要把它们交换然后加 len
            // 使用 WASI_SCRATCH 临时保存
            f.instruction(&Instruction::I32Store(mem(WASI_SCRATCH as u64 + 8, 2))); // 临时保存 dest
            // stack: c_str
            f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
            f.instruction(&Instruction::I32Load(mem(0, 2))); // dest
            // stack: c_str, dest → 不对。Let me restructure.
            // 实际上 memory.copy 需要 (dest, src, len)
            // 使用本地变量更简洁:
            // 用一个简单的循环复制字节代替 memory.copy
            f.instruction(&Instruction::Drop); // 丢弃 dest
            f.instruction(&Instruction::Drop); // 丢弃 c_str

            // 使用循环复制字节 (复用 WASI_SCRATCH+8 作为循环变量)
            // 直接获取所有需要的值
            // j = 0
            f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(mem(0, 2))); // scratch[8] = j = 0

            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                // if j >= str_len → break
                f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::LocalGet(7)); // str_len
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));

                // dest[j] = src[j]
                // dest = str_ptr + 4 + j
                f.instruction(&Instruction::LocalGet(6)); // str_ptr
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
                f.instruction(&Instruction::I32Load(mem(0, 2))); // j
                f.instruction(&Instruction::I32Add);

                // src = argv_ptrs[i*4] + j
                f.instruction(&Instruction::LocalGet(2)); // argv_ptrs
                f.instruction(&Instruction::LocalGet(5)); // i
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load(mem(0, 2))); // c_str
                f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
                f.instruction(&Instruction::I32Load(mem(0, 2))); // j
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));

                // j++
                f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
                f.instruction(&Instruction::I32Const(WASI_SCRATCH + 8));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End); // block
            f.instruction(&Instruction::End); // loop

            // result[4 + i * 8] = str_ptr as i64
            f.instruction(&Instruction::LocalGet(4)); // result
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(5)); // i
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(6)); // str_ptr
            f.instruction(&Instruction::I64ExtendI32S);
            f.instruction(&Instruction::I64Store(mem(0, 3)));

            // i++
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // block
        f.instruction(&Instruction::End); // loop

        // 返回结果数组指针
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::End);
        f
    }

    /// __get_env(key_ptr: i32) -> i32: 简化版——返回空字符串
    fn emit_get_env(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // 简化: 分配空字符串并返回
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalTee(1));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::End);
        f
    }

    /// __get_env 完整版(禁用)
    fn emit_get_env_DISABLED(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=key_ptr, 1=env_count(i32), 2=buf_size(i32), 3=env_ptrs(i32),
        //         4=env_buf(i32), 5=i(i32), 6=c_str(i32), 7=key_len(i32),
        //         8=j(i32), 9=matched(i32), 10=val_ptr(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 1: env_count
            (1, ValType::I32), // 2: buf_size
            (1, ValType::I32), // 3: env_ptrs
            (1, ValType::I32), // 4: env_buf
            (1, ValType::I32), // 5: i
            (1, ValType::I32), // 6: c_str
            (1, ValType::I32), // 7: key_len
            (1, ValType::I32), // 8: j
            (1, ValType::I32), // 9: matched/val_start
            (1, ValType::I32), // 10: result_ptr
        ]);

        // key_len = mem[key_ptr] (仓颉字符串的长度)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(7)); // key_len

        // 调用 environ_sizes_get
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(WASI_SCRATCH + 4));
        f.instruction(&Instruction::Call(self.func_indices["__wasi_environ_sizes_get"]));
        f.instruction(&Instruction::Drop);

        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1)); // env_count

        f.instruction(&Instruction::I32Const(WASI_SCRATCH + 4));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2)); // buf_size

        // 如果 env_count == 0，直接返回 0
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);

        // 分配内存
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(3)); // env_ptrs

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(4)); // env_buf

        // 调用 environ_get
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::Call(self.func_indices["__wasi_environ_get"]));
        f.instruction(&Instruction::Drop);

        // 遍历环境变量，查找 key=value
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5)); // i = 0

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(ValType::I32)));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Br(2)); // break with 0
            f.instruction(&Instruction::End);

            // c_str = env_ptrs[i * 4]
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::LocalSet(6)); // c_str

            // 比较 key: 检查 c_str 前 key_len 字节是否等于 key_ptr+4 的内容
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(9)); // matched = true

            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(8)); // j = 0

            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::LocalGet(7)); // key_len
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1)); // j >= key_len → done comparing

                // if c_str[j] != key[j+4] → not matched
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));

                f.instruction(&Instruction::LocalGet(0)); // key_ptr
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));

                f.instruction(&Instruction::I32Ne);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(9)); // matched = false
                f.instruction(&Instruction::Br(2)); // break inner loop
                f.instruction(&Instruction::End);

                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(8));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            // 检查 matched && c_str[key_len] == '='
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Const(61)); // '='
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    // 找到！提取 value: val_start = c_str + key_len + 1
                    f.instruction(&Instruction::LocalGet(6));
                    f.instruction(&Instruction::LocalGet(7));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::LocalSet(9)); // val_start

                    // 计算 val 长度: 扫描到 0
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalSet(8)); // val_len = 0
                    f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    {
                        f.instruction(&Instruction::LocalGet(9));
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                        f.instruction(&Instruction::I32Eqz);
                        f.instruction(&Instruction::BrIf(1));
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Const(1));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalSet(8));
                        f.instruction(&Instruction::Br(0));
                    }
                    f.instruction(&Instruction::End);
                    f.instruction(&Instruction::End);

                    // 分配字符串 [val_len][bytes]
                    f.instruction(&Instruction::LocalGet(8)); // val_len
                    f.instruction(&Instruction::I32Const(4));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
                    f.instruction(&Instruction::LocalSet(10)); // result_ptr

                    // 写入长度
                    f.instruction(&Instruction::LocalGet(10));
                    f.instruction(&Instruction::LocalGet(8));
                    f.instruction(&Instruction::I32Store(mem(0, 2)));

                    // 复制字节 (简单循环)
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalSet(8)); // 复用 j/val_len 为 copy index，重置
                    // 需要重新计算 val_len，先保存到 scratch
                    // 实际上我们已经存了 val_len 在 local 8，但刚刚重置了...
                    // 用 result_ptr 的长度字段读取
                    f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    {
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::LocalGet(10));
                        f.instruction(&Instruction::I32Load(mem(0, 2))); // val_len from result
                        f.instruction(&Instruction::I32GeU);
                        f.instruction(&Instruction::BrIf(1));

                        f.instruction(&Instruction::LocalGet(10));
                        f.instruction(&Instruction::I32Const(4));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalGet(9));
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                        f.instruction(&Instruction::I32Store8(mem(0, 0)));

                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Const(1));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalSet(8));
                        f.instruction(&Instruction::Br(0));
                    }
                    f.instruction(&Instruction::End);
                    f.instruction(&Instruction::End);

                    // 返回 result_ptr
                    f.instruction(&Instruction::LocalGet(10));
                    f.instruction(&Instruction::Br(4)); // break outer block with result
                }
                f.instruction(&Instruction::End);
            }
            f.instruction(&Instruction::End);

            // i++
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // loop
        f.instruction(&Instruction::End); // block

        f.instruction(&Instruction::End);
        f
    }

    /// __exit(code: i32) -> (): 调用 proc_exit
    fn emit_exit(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0)); // code
        f.instruction(&Instruction::Call(self.func_indices["__wasi_proc_exit"]));
        f.instruction(&Instruction::End);
        f
    }

    // =========== Phase 7.4: 格式化运行时函数 ===========

    /// __i64_format(val: i64, spec_ptr: i32) -> i32
    /// 简化实现: 委托给 __i64_to_str (忽略 spec，后续迭代添加进制支持)
    fn emit_i64_format(&self) -> WasmFunc {
        let _mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        // 暂时忽略 spec，直接用 __i64_to_str 转换
        f.instruction(&Instruction::LocalGet(0)); // val
        f.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
        f.instruction(&Instruction::End);
        f
    }

    /// __f64_format(val: f64, spec_ptr: i32) -> i32
    /// 简化实现: 委托给 __f64_to_str
    fn emit_f64_format(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0)); // val
        f.instruction(&Instruction::Call(self.func_indices["__f64_to_str"]));
        f.instruction(&Instruction::End);
        f
    }

    // (保留完整格式化实现计划，后续迭代支持进制/宽度/精度)
    #[allow(dead_code)]
    fn emit_i64_format_FULL_PLACEHOLDER(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=val(i64), 1=spec_ptr(i32), 2=base(i32), 3=spec_len(i32),
        //         4=abs_val(i64), 5=buf_pos(i32), 6=is_neg(i32), 7=digit(i32),
        //         8=result_len(i32), 9=result_ptr(i32), 10=i(i32), 11=last_char(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: spec_len
            (1, ValType::I32), // 3: base
            (1, ValType::I32), // 4: width
            (1, ValType::I32), // 5: is_neg
            (1, ValType::I64), // 6: abs_val
            (1, ValType::I32), // 7: buf_pos (使用地址 0-63 做缓冲区)
            (1, ValType::I32), // 8: digit
            (1, ValType::I32), // 9: result_ptr
            (1, ValType::I32), // 10: result_len
            (1, ValType::I32), // 11: pad_char
            (1, ValType::I32), // 12: i (spec parsing index)
        ]);

        // 读取 spec 长度
        f.instruction(&Instruction::LocalGet(1)); // spec_ptr
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2)); // spec_len

        // 默认: base=10, width=0, pad_char=' '
        f.instruction(&Instruction::I32Const(10));
        f.instruction(&Instruction::LocalSet(3)); // base
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4)); // width
        f.instruction(&Instruction::I32Const(32)); // ' '
        f.instruction(&Instruction::LocalSet(11)); // pad_char

        // 解析 spec: 遍历字节
        // 最后一个字节是 specifier (d/x/b/o), 前面的数字是 width
        // 如果第一个字符是 '0'，pad_char = '0'
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(12)); // i = 0

        // 检查 spec 是否非空
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // 读取最后一个字符作为 specifier
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::LocalSet(8)); // last char

            // 检查 specifier
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(120)); // 'x'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(16));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(98)); // 'b'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(111)); // 'o'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::End);

            // 解析 width (spec 中数字部分)
            // 如果 specifier 是字母，width 在 spec[0..len-1]
            // 如果 specifier 是数字，整个 spec 都是 width，base=10
            // 简化: 检查最后字符是否为字母
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(97)); // 'a'
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // 最后字符是字母 → width = parse digits from spec[0..len-1]
                // 检查第一个字符是否为 '0'
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32GtS);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    f.instruction(&Instruction::LocalGet(1));
                    f.instruction(&Instruction::I32Const(4));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                    f.instruction(&Instruction::I32Const(48)); // '0'
                    f.instruction(&Instruction::I32Eq);
                    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    f.instruction(&Instruction::I32Const(48)); // '0'
                    f.instruction(&Instruction::LocalSet(11)); // pad_char = '0'
                    f.instruction(&Instruction::End);

                    // 解析 width 数字
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalSet(4)); // width = 0
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalSet(12)); // i = 0
                    f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    {
                        f.instruction(&Instruction::LocalGet(12));
                        f.instruction(&Instruction::LocalGet(2));
                        f.instruction(&Instruction::I32Const(1));
                        f.instruction(&Instruction::I32Sub);
                        f.instruction(&Instruction::I32GeU);
                        f.instruction(&Instruction::BrIf(1));

                        f.instruction(&Instruction::LocalGet(1));
                        f.instruction(&Instruction::I32Const(4));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalGet(12));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                        f.instruction(&Instruction::LocalSet(8));

                        // if digit >= '0' && digit <= '9'
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Const(48));
                        f.instruction(&Instruction::I32GeU);
                        f.instruction(&Instruction::LocalGet(8));
                        f.instruction(&Instruction::I32Const(57));
                        f.instruction(&Instruction::I32LeU);
                        f.instruction(&Instruction::I32And);
                        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                        {
                            f.instruction(&Instruction::LocalGet(4));
                            f.instruction(&Instruction::I32Const(10));
                            f.instruction(&Instruction::I32Mul);
                            f.instruction(&Instruction::LocalGet(8));
                            f.instruction(&Instruction::I32Const(48));
                            f.instruction(&Instruction::I32Sub);
                            f.instruction(&Instruction::I32Add);
                            f.instruction(&Instruction::LocalSet(4));
                        }
                        f.instruction(&Instruction::End);

                        f.instruction(&Instruction::LocalGet(12));
                        f.instruction(&Instruction::I32Const(1));
                        f.instruction(&Instruction::I32Add);
                        f.instruction(&Instruction::LocalSet(12));
                        f.instruction(&Instruction::Br(0));
                    }
                    f.instruction(&Instruction::End);
                    f.instruction(&Instruction::End);
                }
                f.instruction(&Instruction::End);
            }
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // 处理负数
        f.instruction(&Instruction::LocalGet(0)); // val
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::LocalSet(5)); // is_neg

        // abs_val = is_neg ? -val : val
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::End);

        // 使用地址 0-63 作为缓冲区，从末尾向前写数字
        f.instruction(&Instruction::I32Const(63));
        f.instruction(&Instruction::LocalSet(7)); // buf_pos = 63

        // 特殊情况: val == 0
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(48)); // '0'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(7));
        }
        f.instruction(&Instruction::Else);
        {
            // 循环: abs_val > 0 时提取数字
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::BrIf(1));

                // digit = abs_val % base
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64ExtendI32U);
                f.instruction(&Instruction::I64RemU);
                f.instruction(&Instruction::I32WrapI64);
                f.instruction(&Instruction::LocalSet(8));

                // 转为字符
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32LtU);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Const(48)); // '0'
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Const(87)); // 'a' - 10
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::End);

                // buf[buf_pos] = char
                f.instruction(&Instruction::LocalGet(7));
                // stack: char, buf_pos → 需要交换
                // 用 I32Store8 的地址/值: 先地址后值
                // 这里 stack 是: char_val → 需要先算地址
                // 实际上 I32Store8 期望 (addr, val) 在栈上
                // 当前栈: char_val
                // 需要重新组织
                f.instruction(&Instruction::I32Store8(mem(0, 0))); // buf[buf_pos] = char

                // abs_val = abs_val / base
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64ExtendI32U);
                f.instruction(&Instruction::I64DivU);
                f.instruction(&Instruction::LocalSet(6));

                // buf_pos--
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::LocalSet(7));

                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // 如果是负数且 base==10，添加 '-' 前缀
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(10));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(7));
        }
        f.instruction(&Instruction::End);

        // result_len = 63 - buf_pos
        f.instruction(&Instruction::I32Const(63));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(10));

        // 如果 width > result_len，需要填充
        // total_len = max(width, result_len)
        f.instruction(&Instruction::LocalGet(4)); // width
        f.instruction(&Instruction::LocalGet(10)); // result_len
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // 分配 width + 4 字节的字符串
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalSet(9));
            // 写入长度
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            // 先填充 pad_char
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(12)); // i = 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(11)); // pad_char
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(12));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // 复制数字到后面
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(12));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // dest = result_ptr + 4 + (width - result_len) + i
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Add);
                // src = buf[buf_pos + 1 + i]
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(12));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::Else);
        {
            // 不需要填充，直接分配 result_len + 4
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            // 复制数字
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(12));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(12));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::End);
        f
    }

    #[allow(dead_code)]
    fn emit_f64_format_FULL_PLACEHOLDER(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=val(f64), 1=spec_ptr(i32),
        // 2=precision(i32), 3=spec_len(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: precision
            (1, ValType::I32), // 3: spec_len
            (1, ValType::I32), // 4: is_neg
            (1, ValType::I64), // 5: int_part
            (1, ValType::F64), // 6: frac
            (1, ValType::I32), // 7: int_str
            (1, ValType::I32), // 8: frac_str
            (1, ValType::I32), // 9: result
            (1, ValType::I32), // 10: i
            (1, ValType::F64), // 11: multiplier
        ]);

        // 默认 precision = 6
        f.instruction(&Instruction::I32Const(6));
        f.instruction(&Instruction::LocalSet(2));

        // 解析 spec: "Nf" → precision = N
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3)); // spec_len

        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // 解析数字直到遇到非数字
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(2)); // precision = 0
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10)); // i = 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));

                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                // 如果是数字 '0'-'9'
                f.instruction(&Instruction::I32Const(48));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::LocalTee(4)); // temp = byte - '0'
                f.instruction(&Instruction::I32Const(10));
                f.instruction(&Instruction::I32LtU);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    f.instruction(&Instruction::LocalGet(2));
                    f.instruction(&Instruction::I32Const(10));
                    f.instruction(&Instruction::I32Mul);
                    f.instruction(&Instruction::LocalGet(4));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::LocalSet(2));
                }
                f.instruction(&Instruction::End);

                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // 处理负数
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Const(0.0));
        f.instruction(&Instruction::F64Lt);
        f.instruction(&Instruction::LocalSet(4)); // is_neg

        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::F64Neg);
        f.instruction(&Instruction::LocalSet(0)); // val = -val
        f.instruction(&Instruction::End);

        // int_part = trunc(val) as i64
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::LocalSet(5));

        // frac = val - int_part_as_f64
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::F64ConvertI64S);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::LocalSet(6)); // frac

        // 将 frac 乘以 10^precision 并取整
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(11)); // multiplier = 1.0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(10)); // i = 0
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(2)); // precision
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::F64Const(10.0));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::LocalSet(11));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);

        // frac_int = round(frac * multiplier) as i64
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(11));
        f.instruction(&Instruction::F64Mul);
        f.instruction(&Instruction::F64Const(0.5));
        f.instruction(&Instruction::F64Add);
        f.instruction(&Instruction::F64Floor);
        f.instruction(&Instruction::I64TruncF64S);

        // 转为字符串 (用 __i64_to_str)
        // 但需要前置补零到 precision 位
        // 策略: 先转字符串, 再手动补零
        // 这里简化: 如果 is_neg，int_part 前加 '-'
        // int_str = __i64_to_str(int_part)
        f.instruction(&Instruction::LocalSet(5)); // frac_int 暂存到 local 5 (覆盖 int_part)
        // 重新计算 int_part
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        // 如果 is_neg，取负
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Drop); // 丢弃之前的结果

        // 使用已有的 __i64_to_str (会处理负号)
        // 但我们已经取了绝对值...需要重新添加负号
        // 简化方案: 使用 __f64_to_str 的基本逻辑，但控制精度
        // 更简单: 直接用 __i64_to_str 生成整数部分，手动拼接小数部分

        // int_str = __i64_to_str(is_neg ? -int_part : int_part)
        // 由于 val 已取绝对值, int_part 来自 abs(val)
        f.instruction(&Instruction::LocalGet(0)); // abs(val)
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::LocalGet(4)); // is_neg
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::End);

        // 但 __i64_to_str 会处理负数，所以直接传原始 int_part (带符号)
        f.instruction(&Instruction::Drop);
        f.instruction(&Instruction::Drop);
        // 重新获取带符号的整数部分
        f.instruction(&Instruction::LocalGet(4)); // is_neg
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::I64Mul);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64TruncF64S);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
        f.instruction(&Instruction::LocalSet(7)); // int_str

        // frac_str: 将 frac_int 转字符串并前置补零到 precision 位
        f.instruction(&Instruction::LocalGet(5)); // frac_int
        f.instruction(&Instruction::Call(self.func_indices["__i64_to_str"]));
        f.instruction(&Instruction::LocalSet(8)); // frac_str (可能位数不够)

        // 如果 precision == 0，返回 int_str
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);

        // 创建 "." 字符串
        f.instruction(&Instruction::I32Const(5)); // 4 + 1
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalTee(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(46)); // '.'
        f.instruction(&Instruction::I32Store8(mem(4, 0)));

        // 如果 frac_str 位数 < precision，需要补零
        // 获取 frac_str 长度
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(2)); // precision
        f.instruction(&Instruction::I32LtU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // 需要补零: 创建 precision - frac_len 个 '0' + frac_str
            f.instruction(&Instruction::LocalGet(2)); // precision
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalTee(10));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Store(mem(0, 2)));

            // 填零
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10)); // 复用为 loop i
            // 需要保存 alloc 结果...由于 LocalTee 后被覆盖了
            // 重新分配
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalSet(9)); // new padded frac str

            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Store(mem(0, 2)));

            // 填充前导零
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::LocalGet(2)); // precision
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Load(mem(0, 2))); // frac_len
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Const(48)); // '0'
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            // 复制 frac_str 的数字到后面
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Store8(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalSet(8)); // frac_str = padded version
        }
        f.instruction(&Instruction::End);

        // 拼接: int_str + "." + frac_str
        // "." 在 local 9 (之前分配的)
        // 但 local 9 可能已被覆盖... 重新创建
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalTee(9));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(46)); // '.'
        f.instruction(&Instruction::I32Store8(mem(4, 0)));

        // result = __str_concat(int_str, ".")
        f.instruction(&Instruction::LocalGet(7)); // int_str
        f.instruction(&Instruction::LocalGet(9)); // "."
        f.instruction(&Instruction::Call(self.func_indices["__str_concat"]));
        // result = __str_concat(result, frac_str)
        f.instruction(&Instruction::LocalGet(8)); // frac_str
        f.instruction(&Instruction::Call(self.func_indices["__str_concat"]));

        f.instruction(&Instruction::End);
        f
    }

    // =========== Phase 7.5: 集合类型运行时函数 ===========

    /// ArrayList 布局: [len: i32][cap: i32][data_ptr: i32] (12 bytes)
    /// data 区: [elem0: i64][elem1: i64]... (每个元素 8 bytes)

    /// __arraylist_new() -> i32: 创建空 ArrayList (初始容量 8)
    fn emit_arraylist_new(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 0: list_ptr
            (1, ValType::I32), // 1: data_ptr
        ]);
        // 分配 ArrayList 头: 12 bytes
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(0));
        // 分配 data: 8 * 8 = 64 bytes (初始容量 8)
        f.instruction(&Instruction::I32Const(64));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(1));
        // 初始化: len=0, cap=8, data_ptr
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // len = 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // cap = 8
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(8, 2))); // data_ptr
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    /// __arraylist_append(list: i32, elem: i64): 追加元素，必要时扩容
    fn emit_arraylist_append(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=list, 1=elem(i64), 2=len(i32), 3=cap(i32), 4=data(i32),
        //         5=new_data(i32), 6=new_cap(i32), 7=i(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: len
            (1, ValType::I32), // 3: cap
            (1, ValType::I32), // 4: data
            (1, ValType::I32), // 5: new_data
            (1, ValType::I32), // 6: new_cap
            (1, ValType::I32), // 7: i
        ]);
        // 读取 len, cap, data
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2)); // len
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(3)); // cap
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(4)); // data

        // if len >= cap → 扩容
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            // new_cap = cap * 2
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::LocalSet(6));
            // new_data = __alloc(new_cap * 8)
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalSet(5));
            // 复制旧数据
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(7)); // i = 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(2)); // len
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // new_data[i*8] = data[i*8]
                f.instruction(&Instruction::LocalGet(5));
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
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // 更新 list.cap 和 list.data
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Store(mem(4, 2)));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Store(mem(8, 2)));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalSet(4)); // data = new_data
        }
        f.instruction(&Instruction::End);

        // data[len * 8] = elem
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(1)); // elem
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        // len++
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::End);
        f
    }

    /// __arraylist_get(list: i32, index: i64) -> i64
    fn emit_arraylist_get(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        // data = list.data_ptr
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        // offset = index * 8
        f.instruction(&Instruction::LocalGet(1)); // index (i64)
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// __arraylist_set(list: i32, index: i64, elem: i64)
    fn emit_arraylist_set(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2))); // data
        f.instruction(&Instruction::LocalGet(1)); // index
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(2)); // elem
        f.instruction(&Instruction::I64Store(mem(0, 3)));
        f.instruction(&Instruction::End);
        f
    }

    /// __arraylist_remove(list: i32, index: i64) -> i64: 移除并返回元素，后面的元素前移
    fn emit_arraylist_remove(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=list, 1=index(i64), 2=result(i64), 3=data(i32), 4=len(i32), 5=i(i32), 6=idx_i32(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I64), // 2: result
            (1, ValType::I32), // 3: data
            (1, ValType::I32), // 4: len
            (1, ValType::I32), // 5: i
            (1, ValType::I32), // 6: idx_i32
        ]);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::LocalSet(6)); // idx_i32

        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3)); // data
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4)); // len

        // result = data[idx * 8]
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        f.instruction(&Instruction::LocalSet(2));

        // 将后续元素前移
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalSet(5)); // i = idx
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // data[i] = data[i+1]
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
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // len--
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// HashMap 布局: [size: i32][cap: i32][buckets_ptr: i32] (12 bytes)
    /// Bucket: [occupied: i32][key: i64][val: i64] (20 bytes per bucket)

    fn emit_hashmap_new(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 0: map_ptr
            (1, ValType::I32), // 1: buckets
        ]);
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(0));
        // 初始容量 16, 每个 bucket 20 bytes
        f.instruction(&Instruction::I32Const(320)); // 16 * 20
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(1));
        // 零初始化 buckets (occupied 字段全为 0)
        // 简单循环清零
        f.instruction(&Instruction::I32Const(WASI_SCRATCH));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // i = 0
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::I32Const(WASI_SCRATCH));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Const(320));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // buckets[i] = 0
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Const(WASI_SCRATCH));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            // i++
            f.instruction(&Instruction::I32Const(WASI_SCRATCH));
            f.instruction(&Instruction::I32Const(WASI_SCRATCH));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // 初始化 map 头
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // size = 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // cap = 16
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(8, 2))); // buckets
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    /// __hashmap_put(map: i32, key: i64, val: i64): 插入或更新键值对
    fn emit_hashmap_put(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=map, 1=key(i64), 2=val(i64), 3=cap(i32), 4=buckets(i32),
        //         5=hash(i32), 6=idx(i32), 7=bucket_addr(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 3: cap
            (1, ValType::I32), // 4: buckets
            (1, ValType::I32), // 5: hash
            (1, ValType::I32), // 6: idx
            (1, ValType::I32), // 7: bucket_addr
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(3)); // cap
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(4)); // buckets

        // hash = (key ^ (key >> 32)) & 0x7fffffff
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Const(0x7fffffff));
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::LocalSet(5));

        // idx = hash % cap
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32RemU);
        f.instruction(&Instruction::LocalSet(6));

        // 线性探测
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // bucket_addr = buckets + idx * 20
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(20));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(7));

            // if !occupied
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // 空桶 → 插入
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Store(mem(0, 2))); // occupied = 1
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(1)); // key
                f.instruction(&Instruction::I64Store(mem(4, 3)));
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(2)); // val
                f.instruction(&Instruction::I64Store(mem(12, 3)));
                // size++
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load(mem(0, 2)));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Store(mem(0, 2)));
                f.instruction(&Instruction::Br(2)); // break
            }
            f.instruction(&Instruction::End);

            // if occupied && key matches → 更新
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(2)); // val
                f.instruction(&Instruction::I64Store(mem(12, 3)));
                f.instruction(&Instruction::Br(2)); // break
            }
            f.instruction(&Instruction::End);

            // 继续探测: idx = (idx + 1) % cap
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

    /// __hashmap_get(map: i32, key: i64) -> i64
    fn emit_hashmap_get(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: cap
            (1, ValType::I32), // 3: buckets
            (1, ValType::I32), // 4: hash
            (1, ValType::I32), // 5: idx
            (1, ValType::I32), // 6: bucket_addr
            (1, ValType::I32), // 7: probes
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3));
        // hash
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
        f.instruction(&Instruction::LocalSet(5)); // idx

        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(7)); // probes = 0

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(ValType::I64)));
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

            // if !occupied → not found
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Load(mem(0, 2)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);

            // if key matches → return val
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Load(mem(12, 3)));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);

            // next
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
        f.instruction(&Instruction::End); // loop end
        f.instruction(&Instruction::Unreachable); // loop never falls through
        f.instruction(&Instruction::End); // block end
        f.instruction(&Instruction::End); // function end
        f
    }

    /// __hashmap_contains(map: i32, key: i64) -> i32 (0/1)
    fn emit_hashmap_contains(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
            (1, ValType::I32), (1, ValType::I32),
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2)); // cap
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3)); // buckets
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
        f.instruction(&Instruction::LocalSet(4)); // idx
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5)); // probes

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(ValType::I32)));
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
        f.instruction(&Instruction::End); // loop end
        f.instruction(&Instruction::Unreachable); // loop never falls through
        f.instruction(&Instruction::End); // block end
        f.instruction(&Instruction::End); // function end
        f
    }

    /// __hashmap_remove(map: i32, key: i64) -> i64
    fn emit_hashmap_remove(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
            (1, ValType::I32), (1, ValType::I32),
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

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(ValType::I64)));
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
                // 找到 → 读取 val, 标记 occupied=0, size--
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I64Load(mem(12, 3)));
                // mark deleted
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
        f.instruction(&Instruction::End); // loop end
        f.instruction(&Instruction::Unreachable); // loop never falls through
        f.instruction(&Instruction::End); // block end
        f.instruction(&Instruction::End); // function end
        f
    }

    /// LinkedList 布局: [size: i32][head: i32][tail: i32] (12 bytes)
    /// Node: [prev: i32][next: i32][val: i64] (16 bytes)

    fn emit_linkedlist_new(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // size = 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // head = null (0)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(8, 2))); // tail = null (0)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_linkedlist_append(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=list, 1=elem(i64), 2=node(i32), 3=tail(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: node
            (1, ValType::I32), // 3: tail
        ]);
        // 分配节点
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(2));
        // node.prev = tail, node.next = 0, node.val = elem
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(8, 2)));
        f.instruction(&Instruction::LocalSet(3)); // old tail

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // node.prev = old tail

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // node.next = 0

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Store(mem(8, 3))); // node.val = elem

        // if tail != 0: tail.next = node
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // old_tail.next = node
        f.instruction(&Instruction::Else);
        // list is empty, set head = node
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // head = node
        f.instruction(&Instruction::End);

        // tail = node, size++
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(8, 2))); // tail = node
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_linkedlist_prepend(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: node
            (1, ValType::I32), // 3: head
        ]);
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(3)); // old head

        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // node.prev = 0
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // node.next = old head
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Store(mem(8, 3))); // node.val = elem

        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(0, 2))); // old_head.prev = node
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(8, 2))); // tail = node
        f.instruction(&Instruction::End);

        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(mem(4, 2))); // head = node
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::End);
        f
    }

    fn emit_linkedlist_get(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=list, 1=index(i64), 2=cur(i32), 3=i(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: cur
            (1, ValType::I32), // 3: i
        ]);
        // cur = head
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(4, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));
        // 从头遍历到 index
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // cur = cur.next
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Load(mem(4, 2)));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // return cur.val
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I64Load(mem(8, 3)));
        f.instruction(&Instruction::End);
        f
    }

    // =========== Phase 7.8: 字符串操作 / 排序 ===========

    /// __str_contains(str: i32, sub: i32) -> i32 (0/1)
    fn emit_str_contains(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // 简单实现: indexOf >= 0
        let mut f = WasmFunc::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Call(self.func_indices["__str_index_of"]));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::I64Ne);
        let _ = mem;
        f.instruction(&Instruction::End);
        f
    }

    /// __str_index_of(str: i32, sub: i32) -> i64 (-1 if not found)
    fn emit_str_index_of(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=sub, 2=str_len(i32), 3=sub_len(i32), 4=i(i32),
        //         5=j(i32), 6=matched(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: str_len
            (1, ValType::I32), // 3: sub_len
            (1, ValType::I32), // 4: i
            (1, ValType::I32), // 5: j
            (1, ValType::I32), // 6: matched
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));

        // if sub_len == 0 → return 0
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);

        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4)); // i = 0

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // if i > str_len - sub_len → not found
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GtS);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::I64Const(-1));
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);

            // check match at position i
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(6)); // matched = true
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5)); // j = 0
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // str[i+j+4] vs sub[j+4]
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
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I64ExtendI32S);
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End); // loop end
        f.instruction(&Instruction::Unreachable); // loop never falls through
        f.instruction(&Instruction::End); // block end
        f.instruction(&Instruction::End); // function end
        f
    }

    /// __str_replace(str: i32, old: i32, new: i32) -> i32
    /// 找到第一个匹配并替换
    fn emit_str_replace(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=old, 2=new, 3=idx(i64), 4=left(i32), 5=right(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I64), // 3: idx
            (1, ValType::I32), // 4: left
            (1, ValType::I32), // 5: right
        ]);
        // idx = __str_index_of(str, old)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Call(self.func_indices["__str_index_of"]));
        f.instruction(&Instruction::LocalSet(3));

        // if idx == -1 → return str (no match)
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::I64Eq);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);

        // left = str_substring(str, 0, idx)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
        f.instruction(&Instruction::LocalSet(4));

        // right = str_substring(str, idx + old.len, str.len)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
        f.instruction(&Instruction::LocalSet(5));

        // result = left + new + right
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Call(self.func_indices["__str_concat"]));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::Call(self.func_indices["__str_concat"]));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_split(str: i32, sep: i32) -> i32 (Array<String>)
    /// 简化实现: 返回包含原字符串的单元素数组（完整版后续迭代）
    fn emit_str_split(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);
        // 分配单元素数组: [len=1][str_ptr as i64]
        f.instruction(&Instruction::I32Const(12)); // 4 + 8
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalTee(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::I64Store(mem(4, 3)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    #[allow(dead_code)]
    fn emit_str_split_FULL_PLACEHOLDER(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // 简化实现: 最多分 64 段
        // locals: 0=str, 1=sep, 2=str_len, 3=sep_len, 4=count, 5=pos,
        //         6=result, 7=idx, 8=start
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 2: str_len
            (1, ValType::I32), // 3: sep_len
            (1, ValType::I32), // 4: count
            (1, ValType::I32), // 5: pos
            (1, ValType::I32), // 6: result
            (1, ValType::I32), // 7: idx (for second pass)
            (1, ValType::I32), // 8: start
            (1, ValType::I32), // 9: matched
            (1, ValType::I32), // 10: j
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(3));

        // 先计算有多少段 (count passes)
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::LocalSet(4)); // count = 1
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5)); // pos = 0

        // 如果 sep_len == 0, 返回只含原字符串的数组
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
        {
            // 返回单元素数组
            f.instruction(&Instruction::I32Const(12)); // 4 + 8
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalTee(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64ExtendI32S);
            f.instruction(&Instruction::I64Store(mem(4, 3)));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::Return);
        }
        f.instruction(&Instruction::End);

        // 第一遍: 计数
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GtS);
            f.instruction(&Instruction::BrIf(1));

            // 检查 str[pos..pos+sep_len] == sep
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Ne);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(9));
                f.instruction(&Instruction::Br(2));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(4));
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(5));
            }
            f.instruction(&Instruction::Else);
            {
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(5));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);

        // 分配结果数组
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // 第二遍: 提取子串
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5)); // pos = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(7)); // idx = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(8)); // start = 0

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GtS);
            f.instruction(&Instruction::BrIf(1));

            // 检查匹配 (same as above)
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load8U(mem(0, 0)));
                f.instruction(&Instruction::I32Ne);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(9));
                f.instruction(&Instruction::Br(2));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // 提取 str[start..pos] 作为子串
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(8)); // start
                f.instruction(&Instruction::LocalGet(5)); // end = pos
                f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
                // 存入结果数组
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(8));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                // stack: substring, dest_addr → 需要交换
                // 用临时变量
                f.instruction(&Instruction::I32Store(mem(WASI_SCRATCH as u64, 2))); // 保存 dest_addr
                // stack: substring
                f.instruction(&Instruction::I64ExtendI32S);
                f.instruction(&Instruction::I32Const(WASI_SCRATCH));
                f.instruction(&Instruction::I32Load(mem(0, 2))); // dest_addr
                // stack: substring_i64, dest_addr → 需要交换
                // 使用另一种方式
                f.instruction(&Instruction::Drop); // drop dest_addr
                f.instruction(&Instruction::Drop); // drop substring

                // 重新计算 (simpler approach)
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I32Const(4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(8));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
                f.instruction(&Instruction::I64ExtendI32S);
                f.instruction(&Instruction::I64Store(mem(0, 3)));

                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(7));
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(8)); // start = pos + sep_len
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(5)); // pos += sep_len
            }
            f.instruction(&Instruction::Else);
            {
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(5));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);

        // 最后一段: str[start..str_len]
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
        f.instruction(&Instruction::I64ExtendI32S);
        f.instruction(&Instruction::I64Store(mem(0, 3)));

        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_to_rune_array(str: i32) -> i32 (Array<Int64> where each elem is a byte/rune)
    fn emit_str_to_rune_array(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 1: str_len
            (1, ValType::I32), // 2: result
            (1, ValType::I32), // 3: i
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));
        // result = __alloc(4 + str_len * 8)
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Mul);
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Store(mem(0, 2)));
        // 循环: result[4+i*8] = str[4+i] as i64
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::I64Store(mem(0, 3)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::End);
        f
    }

    /// __sort_array(arr: i32): 原地插入排序 (i64 元素)
    fn emit_sort_array(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // 数组布局: [len: i32][elem0: i64]...
        // locals: 0=arr, 1=len(i32), 2=i(i32), 3=key(i64), 4=j(i32), 5=data_base(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 1: len
            (1, ValType::I32), // 2: i
            (1, ValType::I64), // 3: key
            (1, ValType::I32), // 4: j
            (1, ValType::I32), // 5: data_base
        ]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(5)); // data_base

        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::LocalSet(2)); // i = 1

        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));

            // key = data[i]
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I64Load(mem(0, 3)));
            f.instruction(&Instruction::LocalSet(3));

            // j = i - 1
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(4));

            // while j >= 0 && data[j] > key
            f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
            f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
            {
                // if j < 0 → break
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::I32LtS);
                f.instruction(&Instruction::BrIf(1));

                // if data[j] <= key → break
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(8));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I64Load(mem(0, 3)));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64LeS);
                f.instruction(&Instruction::BrIf(1));

                // data[j+1] = data[j]
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Const(8));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(8));
                f.instruction(&Instruction::I32Mul);
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I64Load(mem(0, 3)));
                f.instruction(&Instruction::I64Store(mem(0, 3)));

                // j--
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Sub);
                f.instruction(&Instruction::LocalSet(4));
                f.instruction(&Instruction::Br(0));
            }
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);

            // data[j+1] = key
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Mul);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I64Store(mem(0, 3)));

            // i++
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f
    }

    /// __str_substring(str: i32, start: i32, end: i32) -> i32
    fn emit_str_substring(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=start, 2=end, 3=len(i32), 4=result(i32), 5=i(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I32), // 3: len
            (1, ValType::I32), // 4: result
            (1, ValType::I32), // 5: i
        ]);
        // len = end - start
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(3));

        // if len <= 0 → return empty string
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32LeS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
            f.instruction(&Instruction::LocalTee(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Store(mem(0, 2)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Return);
        }
        f.instruction(&Instruction::End);

        // result = __alloc(len + 4)
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Call(self.func_indices["__alloc"]));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Store(mem(0, 2)));

        // 复制字节
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::I32Store8(mem(0, 0)));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
        }
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::End);
        f
    }

    /// __hashcode_i64(val_ptr: i32) -> i64: 简单哈希（直接返回值本身）
    fn emit_hashcode_i64(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        let mut f = WasmFunc::new(vec![]);
        // val_ptr 指向 i32 类型的指针
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Load(mem(0, 3)));
        let _ = mem;
        f.instruction(&Instruction::End);
        f
    }

    // =========== P2.10: String 方法运行时函数 ===========

    /// __str_trim(str: i32) -> i32: 去除首尾空白字符
    fn emit_str_trim(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=len, 2=start, 3=end
        let mut f = WasmFunc::new(vec![(3, ValType::I32)]);
        let alloc_idx = self.func_indices["__alloc"];
        // len = str[0]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(1));
        // start = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(2));
        // 跳过前导空白: while start < len && str[4+start] <= 32
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
        // 跳过尾部空白: while end > start && str[4+end-1] <= 32
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
        // 调用 __str_substring(str, start, end)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(self.func_indices["__str_substring"]));
        f.instruction(&Instruction::End);
        f
    }

    /// __str_starts_with(str: i32, prefix: i32) -> i32 (0/1)
    fn emit_str_starts_with(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=prefix, 2=str_len, 3=pre_len, 4=i
        let mut f = WasmFunc::new(vec![(3, ValType::I32)]);
        // str_len = str[0]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        // pre_len = prefix[0]
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
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        // 逐字节比较
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1)); // done, all matched
        // str[4+i] != prefix[4+i] → return 0
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
        // i++
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
    fn emit_str_ends_with(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg { offset, align, memory_index: 0 };
        // locals: 0=str, 1=suffix, 2=str_len, 3=suf_len, 4=i, 5=offset
        let mut f = WasmFunc::new(vec![(4, ValType::I32)]);
        // str_len
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(2));
        // suf_len
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
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        // 逐字节比较
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // str[4+offset+i] != suffix[4+i] → return 0
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
        // i++
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

    // =========== Phase 7.2: 内建类型方法 ===========

    /// 编译内建类型方法调用，返回 true 表示已处理
    /// 支持:
    ///   Int64:   toString(), toFloat64(), abs(), compareTo(other)
    ///   Float64: toString(), toInt64()
    ///   Bool:    toString()
    ///   String:  isEmpty(), toInt64(), toFloat64()

    /// 生成 __str_to_i64(str_ptr: i32) -> i64
    /// 逐字节解析十进制字符串，支持负号前缀
    /// 字符串布局: [len: i32][bytes...]
    fn emit_str_to_i64(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };
        // locals: 0=str_ptr, 1=result(i64), 2=sign(i64), 3=i(i32), 4=len(i32), 5=byte(i32)
        let mut f = WasmFunc::new(vec![
            (1, ValType::I64),  // result
            (1, ValType::I64),  // sign
            (1, ValType::I32),  // i (index)
            (1, ValType::I32),  // len
            (1, ValType::I32),  // byte
        ]);

        // result = 0
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalSet(1));
        // sign = 1
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::LocalSet(2));
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));

        // len = mem[str_ptr]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4));

        // 检查负号: if len > 0 && mem[str_ptr + 4] == '-'
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Load8U(mem(4, 0))); // first byte
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::I64Const(-1));
                f.instruction(&Instruction::LocalSet(2)); // sign = -1
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::LocalSet(3)); // i = 1
            }
            f.instruction(&Instruction::Else);
            {
                // 检查正号 '+'
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load8U(mem(4, 0)));
                f.instruction(&Instruction::I32Const(43)); // '+'
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::LocalSet(3)); // i = 1
                }
                f.instruction(&Instruction::End);
            }
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // while i < len
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // block for break
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));  // loop
        {
            // if i >= len: break
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1)); // break block

            // byte = mem[str_ptr + 4 + i]
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::LocalSet(5));

            // if byte < '0' || byte > '9': break
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(48)); // '0'
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(57)); // '9'
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::BrIf(1)); // break block

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

            f.instruction(&Instruction::Br(0)); // continue loop
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

    /// 生成 __str_to_f64(str_ptr: i32) -> f64
    /// 逐字节解析浮点数字符串，支持负号、整数部分和小数部分
    /// 字符串布局: [len: i32][bytes...]
    fn emit_str_to_f64(&self) -> WasmFunc {
        let mem = |offset: u64, align: u32| wasm_encoder::MemArg {
            offset, align, memory_index: 0,
        };
        // locals: 0=str_ptr, 1=result(f64), 2=sign(f64), 3=i(i32), 4=len(i32),
        //         5=byte(i32), 6=frac_divisor(f64), 7=in_frac(i32), 8=digit(f64)
        let mut f = WasmFunc::new(vec![
            (1, ValType::F64),  // 1: result
            (1, ValType::F64),  // 2: sign
            (1, ValType::I32),  // 3: i
            (1, ValType::I32),  // 4: len
            (1, ValType::I32),  // 5: byte
            (1, ValType::F64),  // 6: frac_divisor
            (1, ValType::I32),  // 7: in_frac (0=integer part, 1=fractional part)
            (1, ValType::F64),  // 8: digit
        ]);

        // result = 0.0
        f.instruction(&Instruction::F64Const(0.0));
        f.instruction(&Instruction::LocalSet(1));
        // sign = 1.0
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(2));
        // i = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(3));
        // frac_divisor = 1.0
        f.instruction(&Instruction::F64Const(1.0));
        f.instruction(&Instruction::LocalSet(6));
        // in_frac = 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(7));

        // len = mem[str_ptr]
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Load(mem(0, 2)));
        f.instruction(&Instruction::LocalSet(4));

        // 检查负号/正号
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32GtS);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Load8U(mem(4, 0)));
            f.instruction(&Instruction::I32Const(45)); // '-'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::F64Const(-1.0));
                f.instruction(&Instruction::LocalSet(2));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::LocalSet(3));
            }
            f.instruction(&Instruction::Else);
            {
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::I32Load8U(mem(4, 0)));
                f.instruction(&Instruction::I32Const(43)); // '+'
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                {
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::LocalSet(3));
                }
                f.instruction(&Instruction::End);
            }
            f.instruction(&Instruction::End);
        }
        f.instruction(&Instruction::End);

        // main loop: while i < len
        f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        {
            // if i >= len: break
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::BrIf(1));

            // byte = mem[str_ptr + 4 + i]
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::I32Load8U(mem(0, 0)));
            f.instruction(&Instruction::LocalSet(5));

            // if byte == '.': set in_frac = 1, skip
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(46)); // '.'
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::LocalSet(7)); // in_frac = 1
                // i++
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(3));
                f.instruction(&Instruction::Br(1)); // continue loop
            }
            f.instruction(&Instruction::End);

            // if byte < '0' || byte > '9': break
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(57));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::BrIf(1));

            // digit = (byte - '0') as f64 → save to local 8
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(48));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::F64ConvertI32S);
            f.instruction(&Instruction::LocalSet(8)); // digit saved

            // if in_frac
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                // frac_divisor *= 10.0
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::F64Const(10.0));
                f.instruction(&Instruction::F64Mul);
                f.instruction(&Instruction::LocalSet(6));
                // result += digit / frac_divisor
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::LocalGet(8)); // digit
                f.instruction(&Instruction::LocalGet(6)); // frac_divisor
                f.instruction(&Instruction::F64Div);
                f.instruction(&Instruction::F64Add);
                f.instruction(&Instruction::LocalSet(1));
            }
            f.instruction(&Instruction::Else);
            {
                // result = result * 10 + digit
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::F64Const(10.0));
                f.instruction(&Instruction::F64Mul);
                f.instruction(&Instruction::LocalGet(8)); // digit
                f.instruction(&Instruction::F64Add);
                f.instruction(&Instruction::LocalSet(1));
            }
            f.instruction(&Instruction::End);

            // i++
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(3));

            f.instruction(&Instruction::Br(0)); // continue
        }
        f.instruction(&Instruction::End); // end loop
        f.instruction(&Instruction::End); // end block

        // return result * sign
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::F64Mul);

        f.instruction(&Instruction::End);
        f
    }
}

/// 局部变量构建器（同时保存 WASM 值类型与 AST 类型，用于字段偏移等）
pub(crate) struct LocalsBuilder {
    names: HashMap<String, u32>,
    types: Vec<ValType>,
    /// 变量名 -> AST 类型（结构体/数组等需用于计算字段偏移）
    ast_types: HashMap<String, Type>,
}

impl LocalsBuilder {
    fn new() -> Self {
        Self {
            names: HashMap::new(),
            types: Vec::new(),
            ast_types: HashMap::new(),
        }
    }

    fn add(&mut self, name: &str, ty: ValType, ast_type: Option<Type>) {
        if let Some(&idx) = self.names.get(name) {
            let existing = self.types[idx as usize];
            let should_upgrade = matches!(
                (existing, ty),
                (ValType::I32, ValType::I64) | (ValType::F32, ValType::F64)
            );
            if should_upgrade {
                self.types[idx as usize] = ty;
                if let Some(t) = ast_type {
                    self.ast_types.insert(name.to_string(), t);
                }
            }
        } else {
            let idx = self.types.len() as u32;
            self.names.insert(name.to_string(), idx);
            self.types.push(ty);
            if let Some(t) = ast_type {
                self.ast_types.insert(name.to_string(), t);
            }
        }
    }

    fn get(&self, name: &str) -> Option<u32> {
        self.names.get(name).copied()
    }

    fn get_type(&self, name: &str) -> Option<&Type> {
        self.ast_types.get(name)
    }

    fn get_valtype(&self, name: &str) -> Option<ValType> {
        self.names.get(name).map(|&idx| self.types[idx as usize])
    }

    fn get_ast_type(&self, name: &str) -> Option<Type> {
        self.ast_types.get(name).cloned()
    }

    fn ensure_temp(&mut self, name: &str, ty: ValType) -> u32 {
        if let Some(idx) = self.names.get(name) {
            *idx
        } else {
            let idx = self.types.len() as u32;
            self.names.insert(name.to_string(), idx);
            self.types.push(ty);
            idx
        }
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
    use crate::ast::{AssignTarget, FieldDef, Param, Visibility};

    #[test]
    fn test_compile_simple_function() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "answer".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Integer(42)))],
                extern_import: None,
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
            package_name: None,
            imports: vec![],
            structs: vec![StructDef {
                visibility: Visibility::default(),
                name: "Point".to_string(),
                type_params: vec![],
                constraints: vec![],
                fields: vec![
                    FieldDef {
                        name: "x".to_string(),
                        ty: Type::Int64,
                        default: None,
                    },
                    FieldDef {
                        name: "y".to_string(),
                        ty: Type::Int64,
                        default: None,
                    },
                ],
            }],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "test".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int32),
                throws: None,
                extern_import: None,
                body: vec![
                    Stmt::Let {
                        pattern: Pattern::Binding("p".to_string()),
                        ty: Some(Type::Struct("Point".to_string(), vec![])),
                        value: Expr::StructInit {
                            name: "Point".to_string(),
                            type_args: None,
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

    /// 验证多字段偏移：返回第二个字段 y（偏移 8）
    #[test]
    fn test_compile_struct_field_y() {
        let program = Program {
            package_name: None,
            imports: vec![],
            structs: vec![StructDef {
                visibility: Visibility::default(),
                name: "Point".to_string(),
                type_params: vec![],
                constraints: vec![],
                fields: vec![
                    FieldDef {
                        name: "x".to_string(),
                        ty: Type::Int64,
                        default: None,
                    },
                    FieldDef {
                        name: "y".to_string(),
                        ty: Type::Int64,
                        default: None,
                    },
                ],
            }],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            extends: vec![],
            type_aliases: vec![],
            constants: vec![],
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "get_y".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                extern_import: None,
                body: vec![
                    Stmt::Let {
                        pattern: Pattern::Binding("p".to_string()),
                        ty: Some(Type::Struct("Point".to_string(), vec![])),
                        value: Expr::StructInit {
                            name: "Point".to_string(),
                            type_args: None,
                            fields: vec![
                                ("x".to_string(), Expr::Integer(10)),
                                ("y".to_string(), Expr::Integer(20)),
                            ],
                        },
                    },
                    Stmt::Return(Some(Expr::Field {
                        object: Box::new(Expr::Var("p".to_string())),
                        field: "y".to_string(),
                    })),
                ],
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_binary_ops() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "compute".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(Expr::Integer(10)),
                    right: Box::new(Expr::Integer(32)),
                }))],
                extern_import: None,
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_compile_if_expr() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "max".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param {
                        name: "a".to_string(),
                        ty: Type::Int64,
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    },
                    Param {
                        name: "b".to_string(),
                        ty: Type::Int64,
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    },
                ],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::If {
                    cond: Box::new(Expr::Binary {
                        op: BinOp::Gt,
                        left: Box::new(Expr::Var("a".to_string())),
                        right: Box::new(Expr::Var("b".to_string())),
                    }),
                    then_branch: Box::new(Expr::Var("a".to_string())),
                    else_branch: Some(Box::new(Expr::Var("b".to_string()))),
                }))],
                extern_import: None,
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_array_literal_and_index() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "first".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                extern_import: None,
                body: vec![
                    Stmt::Let {
                        pattern: Pattern::Binding("arr".to_string()),
                        ty: Some(Type::Array(Box::new(Type::Int64))),
                        value: Expr::Array(vec![
                            Expr::Integer(10),
                            Expr::Integer(20),
                            Expr::Integer(30),
                        ]),
                    },
                    Stmt::Return(Some(Expr::Index {
                        array: Box::new(Expr::Var("arr".to_string())),
                        index: Box::new(Expr::Integer(0)),
                    })),
                ],
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_match_literal() {
        use crate::ast::{Literal, MatchArm, Pattern};

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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "match_test".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![Param {
                    name: "n".to_string(),
                    ty: Type::Int64,
                    default: None,
                    variadic: false, is_named: false, is_inout: false,
                }],
                return_type: Some(Type::Int64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Match {
                    expr: Box::new(Expr::Var("n".to_string())),
                    arms: vec![
                        MatchArm {
                            pattern: Pattern::Literal(Literal::Integer(0)),
                            guard: None,
                            body: Box::new(Expr::Integer(100)),
                        },
                        MatchArm {
                            pattern: Pattern::Wildcard,
                            guard: None,
                            body: Box::new(Expr::Integer(999)),
                        },
                    ],
                }))],
                extern_import: None,
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_for_range() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "sum_range".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Int64),
                throws: None,
                extern_import: None,
                body: vec![
                    Stmt::Var {
                        pattern: crate::ast::Pattern::Binding("sum".to_string()),
                        ty: Some(Type::Int64),
                        value: Some(Expr::Integer(0)),
                    },
                    Stmt::For {
                        var: "i".to_string(),
                        iterable: Expr::Range {
                            start: Box::new(Expr::Integer(0)),
                            end: Box::new(Expr::Integer(3)),
                            inclusive: false,
                            step: None,
                        },
                        body: vec![Stmt::Assign {
                            target: AssignTarget::Var("sum".to_string()),
                            value: Expr::Binary {
                                op: BinOp::Add,
                                left: Box::new(Expr::Var("sum".to_string())),
                                right: Box::new(Expr::Var("i".to_string())),
                            },
                        }],
                    },
                    Stmt::Return(Some(Expr::Var("sum".to_string()))),
                ],
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_float_ops() {
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "fadd".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::Float64),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(Expr::Float(1.5)),
                    right: Box::new(Expr::Float(2.5)),
                }))],
                extern_import: None,
            }],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_compile_multiple_functions() {
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
            functions: vec![
                FuncDef {
                    visibility: Visibility::default(),
                    name: "one".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![],
                    return_type: Some(Type::Int64),
                    throws: None,
                    body: vec![Stmt::Return(Some(Expr::Integer(1)))],
                    extern_import: None,
                },
                FuncDef {
                    visibility: Visibility::default(),
                    name: "main".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![],
                    return_type: Some(Type::Int64),
                    throws: None,
                    body: vec![Stmt::Return(Some(Expr::Call {
                        name: "one".to_string(),
                        type_args: None,
                        args: vec![],
                        named_args: vec![],
                    }))],
                    extern_import: None,
                },
            ],
        };

        let mut codegen = CodeGen::new();
        let wasm = codegen.compile(&program);
        assert!(!wasm.is_empty());
        assert_eq!(&wasm[0..4], b"\0asm");
    }

    #[test]
    fn test_codegen_default() {
        let codegen = CodeGen::default();
        // default() should be equivalent to new()
        let _ = codegen;
    }

    #[test]
    fn test_type_mangle_suffix_all() {
        // 测试所有类型的名字修饰
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Int8), "Int8");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Int16), "Int16");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Int32), "Int32");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Int64), "Int64");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::UInt8), "UInt8");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::UInt16), "UInt16");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::UInt32), "UInt32");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::UInt64), "UInt64");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Float32), "Float32");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Float64), "Float64");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Bool), "Bool");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Rune), "Rune");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Unit), "Unit");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::String), "String");
        assert_eq!(CodeGen::type_mangle_suffix(&Type::Range), "Range");
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Array(Box::new(Type::Int64))),
            "Array_Int64"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Tuple(vec![Type::Int64, Type::Float64])),
            "Tuple_Int64_Float64"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Struct("Point".to_string(), vec![])),
            "Point"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Struct("Pair".to_string(), vec![Type::Int64, Type::String])),
            "Pair_Int64_String"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Function {
                params: vec![Type::Int64],
                ret: Box::new(Some(Type::Bool)),
            }),
            "Fn_Int64_Bool"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Function {
                params: vec![],
                ret: Box::new(None),
            }),
            "Fn__Unit"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Option(Box::new(Type::Int64))),
            "Option_Int64"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            "Result_Int64_String"
        );
        assert_eq!(
            CodeGen::type_mangle_suffix(&Type::TypeParam("T".to_string())),
            "T"
        );
    }
}
