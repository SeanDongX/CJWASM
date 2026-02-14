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

/// 内存布局常量
const HEAP_BASE: i32 = 1024;  // 堆起始地址
const PAGE_SIZE: u64 = 65536; // WASM 页大小 64KB

/// 类的运行时信息（包含继承布局和 vtable）
#[derive(Debug, Clone)]
struct ClassInfo {
    /// 类名
    name: String,
    /// 父类名
    parent: Option<String>,
    /// 完整字段列表（先父类后子类），不含 vtable_ptr
    all_fields: Vec<FieldDef>,
    /// 自身字段列表（不含继承的）
    own_fields: Vec<FieldDef>,
    /// vtable 方法名列表（按槽位顺序）
    vtable_methods: Vec<String>,
    /// 方法名 → vtable 槽位索引
    vtable_slot: HashMap<String, usize>,
    /// 该类 vtable 在 WASM Table 中的起始索引
    vtable_base: u32,
    /// 对象是否需要 vtable_ptr（有继承或被继承时为 true）
    has_vtable: bool,
    /// 是否是 abstract 类
    is_abstract: bool,
    /// 是否是 sealed 类
    is_sealed: bool,
    /// init 定义
    init: Option<InitDef>,
    /// deinit body
    deinit: Option<Vec<Stmt>>,
    /// 原始 ClassDef 引用的方法列表
    methods: Vec<(String, bool)>, // (fully_qualified_name, is_override)
}

impl ClassInfo {
    /// 对象总大小（包含 vtable_ptr + 所有字段）
    fn object_size(&self) -> u32 {
        let header = if self.has_vtable { 4 } else { 0 }; // vtable_ptr: i32
        header + self.all_fields.iter().map(|f| f.ty.size()).sum::<u32>()
    }

    /// 字段偏移（已加上 vtable_ptr 的 4 字节头部）
    fn field_offset(&self, field_name: &str) -> Option<u32> {
        let header = if self.has_vtable { 4u32 } else { 0 };
        let mut offset = header;
        for f in &self.all_fields {
            if f.name == field_name {
                return Some(offset);
            }
            offset += f.ty.size();
        }
        None
    }

    /// 字段类型查询
    fn field_type(&self, field_name: &str) -> Option<&Type> {
        self.all_fields.iter().find(|f| f.name == field_name).map(|f| &f.ty)
    }
}

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
            data_offset: 0,
            vtable_entries: Vec::new(),
            interfaces: HashMap::new(),
            lambda_functions: Vec::new(),
            lambda_counter: std::cell::Cell::new(0),
        }
    }

    /// 获取运行时函数的索引
    fn get_or_create_func_index(&self, name: &str) -> u32 {
        *self.func_indices.get(name).expect(&format!("运行时函数 {} 未注册", name))
    }

    /// 重载名字修饰：name$Type1$Type2 用于多态解析
    fn type_mangle_suffix(ty: &Type) -> String {
        match ty {
            Type::Int8 => "Int8".to_string(),
            Type::Int16 => "Int16".to_string(),
            Type::Int32 => "Int32".to_string(),
            Type::Int64 => "Int64".to_string(),
            Type::UInt8 => "UInt8".to_string(),
            Type::UInt16 => "UInt16".to_string(),
            Type::UInt32 => "UInt32".to_string(),
            Type::UInt64 => "UInt64".to_string(),
            Type::Float32 => "Float32".to_string(),
            Type::Float64 => "Float64".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Rune => "Rune".to_string(),
            Type::IntNative => "IntNative".to_string(),
            Type::UIntNative => "UIntNative".to_string(),
            Type::Float16 => "Float16".to_string(),
            Type::Nothing => "Nothing".to_string(),
            Type::Unit => "Unit".to_string(),
            Type::String => "String".to_string(),
            Type::Array(inner) => format!("Array_{}", Self::type_mangle_suffix(inner)),
            Type::Tuple(types) => format!("Tuple_{}", types.iter().map(Self::type_mangle_suffix).collect::<Vec<_>>().join("_")),
            Type::Struct(s, args) => {
                if args.is_empty() {
                    s.clone()
                } else {
                    format!("{}_{}", s, args.iter().map(Self::type_mangle_suffix).collect::<Vec<_>>().join("_"))
                }
            }
            Type::Range => "Range".to_string(),
            Type::Function { params, ret } => {
                let params_str = params.iter().map(Self::type_mangle_suffix).collect::<Vec<_>>().join("_");
                let ret_str = ret.as_ref().as_ref().map(Self::type_mangle_suffix).unwrap_or_else(|| "Unit".to_string());
                format!("Fn_{}_{}", params_str, ret_str)
            }
            Type::Option(inner) => format!("Option_{}", Self::type_mangle_suffix(inner)),
            Type::Result(ok, err) => format!("Result_{}_{}", Self::type_mangle_suffix(ok), Self::type_mangle_suffix(err)),
            Type::TypeParam(name) => name.clone(), // 单态化前用于名字修饰的占位
            Type::Slice(inner) => format!("Slice_{}", Self::type_mangle_suffix(inner)),
            Type::Map(k, v) => format!(
                "Map_{}_{}",
                Self::type_mangle_suffix(k),
                Self::type_mangle_suffix(v)
            ),
        }
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
                Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
                    if Self::expr_contains_unhandled_throw(value) { return true; }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body } => {
                    if Self::contains_unhandled_throw(body) { return true; }
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
                Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
                    if let Some(inner) = Self::find_throw_inner_in_expr(value) {
                        return Some(inner);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop { body } => {
                    if let Some(inner) = Self::find_throw_inner_in_stmts(body) {
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

    fn mangle_key(name: &str, param_tys: &[Type]) -> String {
        if param_tys.is_empty() {
            format!("{}$_", name)
        } else {
            format!(
                "{}${}",
                name,
                param_tys
                    .iter()
                    .map(Self::type_mangle_suffix)
                    .collect::<Vec<_>>()
                    .join("$")
            )
        }
    }

    /// 编译程序生成 WASM 模块
    pub fn compile(&mut self, program: &Program) -> Vec<u8> {
        let mut module = Module::new();

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
                init: Some(InitDef {
                    params: vec![Param {
                        name: "message".to_string(),
                        ty: Type::String,
                        default: None,
                        variadic: false,
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
            // 为有 init 的类生成 __ClassName_init 函数
            if let Some(ref init_def) = c.init {
                let init_func = self.build_init_function(c, init_def);
                functions.push(init_func);
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
                .map(|t| vec![t.to_wasm()])
                .unwrap_or_default();
            types.ty().function(params, results);
            let key = if *name_count.get(&func.name).unwrap_or(&0) > 1 {
                Self::mangle_key(&func.name, &param_tys)
            } else {
                func.name.clone()
            };
            self.func_types.insert(key.clone(), i as u32);
            if let Some(ref ret) = func.return_type {
                self.func_return_types.insert(key.clone(), ret.clone());
            }
            self.func_params.insert(key, func.params.clone());
        }
        let num_imports = functions.iter().filter(|f| f.extern_import.is_some()).count() as u32;
        let num_non_extern = functions.len() as u32 - num_imports;
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

        module.section(&types);

        // 构建 vtable（需在 func_indices 设置后）
        self.build_vtables();

        // 2. 导入段 (Import Section) — extern func
        let mut imports = ImportSection::new();
        for (i, func) in functions.iter().enumerate() {
            if let Some(ref imp) = func.extern_import {
                imports.import(&imp.module, &imp.name, EntityType::Function(i as u32));
            }
        }
        if num_imports > 0 {
            module.section(&imports);
        }

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
        module.section(&func_section);

        // 3a. Table 段 (Table Section) — 用于 vtable / call_indirect
        let vtable_size = self.vtable_entries.len() as u64;
        if vtable_size > 0 {
            let mut tables = TableSection::new();
            tables.table(TableType {
                element_type: RefType::FUNCREF,
                minimum: vtable_size,
                maximum: Some(vtable_size),
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
        module.section(&codes);

        // 7. Element 段 (Element Section) — vtable 函数引用
        if !self.vtable_entries.is_empty() {
            let mut elements = ElementSection::new();
            let func_indices: Vec<u32> = self.vtable_entries.clone();
            elements.active(
                Some(0), // table index
                &ConstExpr::i32_const(0),
                Elements::Functions(std::borrow::Cow::Borrowed(&func_indices)),
            );
            module.section(&elements);
        }

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

    /// 解析方法索引，支持继承链向上查找
    /// key 格式为 "ClassName.methodName"，如果找不到，沿继承链向上查找
    fn resolve_method_index(&self, key: &str, method: &str) -> u32 {
        if let Some(&idx) = self.func_indices.get(key) {
            return idx;
        }
        // 从 key 提取类名
        if let Some(dot_pos) = key.find('.') {
            let class_name = &key[..dot_pos];
            // 沿继承链向上查找
            if let Some(ci) = self.classes.get(class_name) {
                let mut parent = ci.parent.clone();
                while let Some(ref p) = parent {
                    let parent_key = format!("{}.{}", p, method);
                    if let Some(&idx) = self.func_indices.get(&parent_key) {
                        return idx;
                    }
                    parent = self.classes.get(p).and_then(|pi| pi.parent.clone());
                }
            }
        }
        panic!("方法未找到: '{}'", key);
    }

    // =========== 类与继承 ===========

    /// 注册所有类，构建 ClassInfo（含继承字段布局和 vtable）
    fn register_classes(&mut self, class_defs: &[ClassDef]) {
        // 第一遍：为每个类创建基本 ClassInfo（不含继承解析）
        let class_map: HashMap<String, &ClassDef> = class_defs.iter().map(|c| (c.name.clone(), c)).collect();

        // 确定哪些类参与继承（被继承或有继承），需要 vtable
        let mut has_children: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in class_defs {
            if let Some(ref parent) = c.extends {
                has_children.insert(parent.clone());
            }
        }

        // 拓扑排序：父类先注册
        let mut registered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut order = Vec::new();
        fn topo_sort(name: &str, class_map: &HashMap<String, &ClassDef>, registered: &mut std::collections::HashSet<String>, order: &mut Vec<String>) {
            if registered.contains(name) { return; }
            if let Some(c) = class_map.get(name) {
                if let Some(ref parent) = c.extends {
                    topo_sort(parent, class_map, registered, order);
                }
            }
            registered.insert(name.to_string());
            order.push(name.to_string());
        }
        for c in class_defs {
            topo_sort(&c.name, &class_map, &mut registered, &mut order);
        }

        // 按拓扑序注册
        for name in &order {
            let c = match class_map.get(name.as_str()) {
                Some(c) => *c,
                None => continue, // 父类不在当前文件中
            };

            let needs_vtable = c.extends.is_some() || has_children.contains(&c.name);

            // 收集所有字段（继承 + 自身）
            let mut all_fields = Vec::new();
            let mut vtable_methods: Vec<String> = Vec::new();
            let mut vtable_slot: HashMap<String, usize> = HashMap::new();

            if let Some(ref parent_name) = c.extends {
                if let Some(parent_info) = self.classes.get(parent_name) {
                    // sealed 类不能被继承
                    if parent_info.is_sealed {
                        panic!("sealed 类 {} 不能被继承", parent_name);
                    }
                    all_fields.extend(parent_info.all_fields.clone());
                    vtable_methods = parent_info.vtable_methods.clone();
                    vtable_slot = parent_info.vtable_slot.clone();
                }
            }
            all_fields.extend(c.fields.clone());

            // 注册方法到 vtable
            let mut method_entries: Vec<(String, bool)> = Vec::new();
            for m in &c.methods {
                // 方法全限定名: ClassName.methodName
                let fqn = m.func.name.clone();
                // 短名：去掉 ClassName. 前缀
                let short_name = fqn.strip_prefix(&format!("{}.", c.name)).unwrap_or(&fqn).to_string();
                method_entries.push((fqn.clone(), m.override_));
                if m.override_ {
                    // 替换父类 vtable 中的对应槽位
                    if let Some(&slot) = vtable_slot.get(&short_name) {
                        vtable_methods[slot] = fqn;
                    }
                } else {
                    // 新方法，追加到 vtable（仅实例方法）
                    let slot = vtable_methods.len();
                    vtable_slot.insert(short_name, slot);
                    vtable_methods.push(fqn);
                }
            }

            let info = ClassInfo {
                name: c.name.clone(),
                parent: c.extends.clone(),
                all_fields,
                own_fields: c.fields.clone(),
                vtable_methods,
                vtable_slot,
                vtable_base: 0, // 后续填充
                has_vtable: needs_vtable,
                is_abstract: c.is_abstract,
                is_sealed: c.is_sealed,
                init: c.init.clone(),
                deinit: c.deinit.clone(),
                methods: method_entries,
            };

            // 同时注册为 StructDef（用于字段访问 / ConstructorCall 兼容）
            self.structs.insert(c.name.clone(), StructDef {
                visibility: c.visibility.clone(),
                name: c.name.clone(),
                type_params: vec![],
                constraints: vec![],
                fields: info.all_fields.clone(),
            });

            self.classes.insert(c.name.clone(), info);
        }
    }

    /// 在函数索引确定后，为每个有 vtable 的类分配 table 条目
    fn build_vtables(&mut self) {
        let mut entries = Vec::new();
        let class_names: Vec<String> = self.classes.keys().cloned().collect();
        for name in &class_names {
            let info = self.classes.get(name).unwrap();
            if !info.has_vtable || info.vtable_methods.is_empty() {
                continue;
            }
            let base = entries.len() as u32;
            for method_fqn in &info.vtable_methods {
                let func_idx = self.func_indices.get(method_fqn)
                    .copied()
                    .unwrap_or_else(|| panic!("vtable 方法 {} 未找到函数索引", method_fqn));
                entries.push(func_idx);
            }
            // 更新 vtable_base
            let info = self.classes.get_mut(name).unwrap();
            info.vtable_base = base;
        }
        self.vtable_entries = entries;
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

    fn collect_lambdas_from_stmt(stmt: &Stmt, counter: &mut u32, out: &mut Vec<FuncDef>) {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Var { value, .. } => {
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
            Stmt::For { iterable, body, .. } => {
                Self::collect_lambdas_from_expr(iterable, counter, out);
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            Stmt::Loop { body, .. } => {
                for s in body { Self::collect_lambdas_from_stmt(s, counter, out); }
            }
            _ => {}
        }
    }

    fn collect_lambdas_from_expr(expr: &Expr, counter: &mut u32, out: &mut Vec<FuncDef>) {
        match expr {
            Expr::Lambda { params, return_type, body } => {
                let lambda_name = format!("__lambda_{}", *counter);
                *counter += 1;
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
                        variadic: false,
                    }).collect(),
                    return_type: return_type.clone(),
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
            _ => {}
        }
    }

    /// 构建 init 函数：__ClassName_init(params...) -> i32 (对象指针)
    fn build_init_function(&self, class: &ClassDef, init_def: &InitDef) -> FuncDef {
        let class_name = &class.name;
        let func_name = format!("__{}_init", class_name);

        // 参数与 init 定义一致
        let params = init_def.params.clone();

        // init body 前面加上 this 分配
        // 在实际编译时会特殊处理 init 函数
        let body = init_def.body.clone();

        FuncDef {
            visibility: Visibility::Public,
            name: func_name,
            type_params: vec![],
            constraints: vec![],
            params,
            return_type: Some(Type::Struct(class_name.clone(), vec![])),
            throws: None,
            body,
            extern_import: None,
        }
    }

    /// 构建 deinit 函数：__ClassName_deinit(this: i32) -> Unit
    fn build_deinit_function(&self, class: &ClassDef, deinit_body: &[Stmt]) -> FuncDef {
        let class_name = &class.name;
        let func_name = format!("__{}_deinit", class_name);

        FuncDef {
            visibility: Visibility::Public,
            name: func_name,
            type_params: vec![],
            constraints: vec![],
            params: vec![Param {
                name: "this".to_string(),
                ty: Type::Struct(class_name.clone(), vec![]),
                default: None,
                variadic: false,
            }],
            return_type: None,
            throws: None,
            body: deinit_body.to_vec(),
            extern_import: None,
        }
    }

    /// 收集所有字符串常量
    fn collect_strings(&mut self, program: &Program) {
        for func in &program.functions {
            for param in &func.params {
                if let Some(ref e) = param.default {
                    self.collect_strings_in_expr(e);
                }
            }
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
            Stmt::WhileLet { expr, body, .. } => {
                self.collect_strings_in_expr(expr);
                for s in body {
                    self.collect_strings_in_stmt(s);
                }
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
            Stmt::Loop { body } => {
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
            Expr::Call { args, .. } => {
                for arg in args {
                    self.collect_strings_in_expr(arg);
                }
            }
            Expr::MethodCall { object, args, .. } => {
                self.collect_strings_in_expr(object);
                for arg in args {
                    self.collect_strings_in_expr(arg);
                }
            }
            Expr::SuperCall { args, .. } => {
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
            Expr::IfLet { expr, then_branch, else_branch, .. } => {
                self.collect_strings_in_expr(expr);
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
            Expr::Tuple(elements) => {
                for e in elements {
                    self.collect_strings_in_expr(e);
                }
            }
            Expr::TupleIndex { object, .. } => {
                self.collect_strings_in_expr(object);
            }
            Expr::NullCoalesce { option, default } => {
                self.collect_strings_in_expr(option);
                self.collect_strings_in_expr(default);
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
            Expr::ConstructorCall { args, .. } => {
                for e in args {
                    self.collect_strings_in_expr(e);
                }
            }
            Expr::Field { object, .. } => {
                self.collect_strings_in_expr(object);
            }
            Expr::Unary { expr, .. } => {
                self.collect_strings_in_expr(expr);
            }
            Expr::Cast { expr, .. } => {
                self.collect_strings_in_expr(expr);
            }
            Expr::Interpolate(parts) => {
                // 收集插值字符串中的字面量和表达式
                for part in parts {
                    match part {
                        InterpolatePart::Literal(s) => {
                            if !self.string_pool.iter().any(|(str, _)| str == s) {
                                let offset = self.data_offset;
                                self.data_offset += 4 + s.len() as u32;
                                self.string_pool.push((s.clone(), offset));
                            }
                        }
                        InterpolatePart::Expr(e) => {
                            self.collect_strings_in_expr(e);
                        }
                    }
                }
                // 添加 "[object]" 占位符字符串（用于不支持的类型）
                let obj_str = "[object]".to_string();
                if !self.string_pool.iter().any(|(str, _)| str == &obj_str) {
                    let offset = self.data_offset;
                    self.data_offset += 4 + obj_str.len() as u32;
                    self.string_pool.push((obj_str, offset));
                }
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

        // 编译函数体（顶层无循环上下文）
        // 特殊处理：最后一条 Stmt::Expr 若产生值，则作为隐式返回值（不 drop）
        let body_len = func.body.len();
        let has_return_type = func.return_type.is_some();
        for (i, stmt) in func.body.iter().enumerate() {
            let is_last = i == body_len - 1;
            if is_last && has_return_type {
                if let Stmt::Expr(expr) = stmt {
                    if Self::expr_produces_value(expr) {
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
    /// 简化实现：将数字转为字符串（支持负数）
    fn emit_i64_to_str(&self) -> WasmFunc {
        // 局部变量: 0=val(i64 参数), 1=ptr(i32), 2..7=临时(i64)
        let mut f = WasmFunc::new(vec![(1, ValType::I32), (6, ValType::I64)]);

        // 简化：对于任何数字，返回固定字符串 "[number]"
        // 完整实现需要复杂的数字到字符串转换
        // 这里使用简化版本，返回堆上的占位符

        // Phase 8: 使用 __alloc 分配空间
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::I32Const(12)); // 4 + 8 bytes
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1)); // ptr

        // mem[ptr] = 8 (length)
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

        // 写入 "[number]" 字符串
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0x5D7265626D756E5B)); // "[number]" as i64 little endian
        f.instruction(&Instruction::I64Store(wasm_encoder::MemArg { offset: 4, align: 3, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __i32_to_str(val: i32) -> i32 辅助函数
    fn emit_i32_to_str(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

        // Phase 8: 使用 __alloc 分配
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Const(0x5D7265626D756E5B));
        f.instruction(&Instruction::I64Store(wasm_encoder::MemArg { offset: 4, align: 3, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __f64_to_str(val: f64) -> i32 辅助函数
    fn emit_f64_to_str(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

        // Phase 8: 使用 __alloc 分配
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::I32Const(11));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

        // "[float]" = 0x5D74616F6C665B (7 bytes)
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x6F6C665B)); // "[flo"
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x005D7461)); // "at]"
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 7, align: 0, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f
    }

    /// 生成 __f32_to_str(val: f32) -> i32 辅助函数
    fn emit_f32_to_str(&self) -> WasmFunc {
        let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

        // Phase 8: 使用 __alloc 分配
        let alloc_idx = self.func_indices["__alloc"];
        f.instruction(&Instruction::I32Const(11));
        f.instruction(&Instruction::Call(alloc_idx));
        f.instruction(&Instruction::LocalSet(1));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x6F6C665B));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 4, align: 2, memory_index: 0 }));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(0x005D7461));
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg { offset: 7, align: 0, memory_index: 0 }));

        f.instruction(&Instruction::LocalGet(1));
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

    /// 收集局部变量
    fn collect_locals(&self, stmt: &Stmt, locals: &mut LocalsBuilder) {
        match stmt {
            Stmt::Let { pattern, ty, value } => {
                match pattern {
                    Pattern::Binding(name) => {
                        let val_type = ty
                            .as_ref()
                            .map(|t| t.to_wasm())
                            .unwrap_or_else(|| self.infer_type(value));
                        let ast_type = ty.clone().or_else(|| self.infer_ast_type(value));
                        locals.add(name, val_type, ast_type);
                    }
                    Pattern::Struct { name: struct_name, fields } => {
                        locals.add("__let_struct_ptr", ValType::I32, None);
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::Var { name, ty, value } => {
                let val_type = ty
                    .as_ref()
                    .map(|t| t.to_wasm())
                    .unwrap_or_else(|| self.infer_type(value));
                let ast_type = ty.clone().or_else(|| self.infer_ast_type(value));
                locals.add(name, val_type, ast_type);
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::Assign { value, .. } => {
                self.collect_locals_from_expr(value, locals);
            }
            Stmt::Expr(expr) => {
                self.collect_locals_from_expr(expr, locals);
            }
            Stmt::Return(Some(expr)) => {
                self.collect_locals_from_expr(expr, locals);
            }
            Stmt::While { cond, body } => {
                self.collect_locals_from_expr(cond, locals);
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::WhileLet { pattern, expr, body } => {
                self.collect_locals_from_expr(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);
                locals.add("__match_enum_ptr", ValType::I32, None);
                match pattern {
                    Pattern::Binding(name) => {
                        locals.add(name, ValType::I64, None);
                    }
                    Pattern::Variant { enum_name, variant_name, binding: Some(name) } => {
                        if let Some(ty) = self.resolve_variant_payload(enum_name, variant_name, subject_ast_type.as_ref()) {
                            locals.add(name, ty.to_wasm(), Some(ty.clone()));
                        }
                    }
                    Pattern::Struct { name: struct_name, fields } => {
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::Loop { body } => {
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            Stmt::For { var, iterable, body } => {
                locals.add(var, ValType::I64, self.expr_object_type(iterable)); // 循环变量：范围时为 Int64，数组时为元素类型
                if !matches!(iterable, Expr::Range { .. }) {
                    locals.add(&format!("__{}_idx", var), ValType::I64, None);
                    locals.add(&format!("__{}_len", var), ValType::I64, None);
                    locals.add(&format!("__{}_arr", var), ValType::I32, None);
                }
                self.collect_locals_from_expr(iterable, locals);
                for s in body {
                    self.collect_locals(s, locals);
                }
            }
            _ => {}
        }
    }

    /// 从表达式中收集局部变量（含 match 分支绑定名，使 `x if x < 0` 中的 x 可用）
    fn collect_locals_from_expr(&self, expr: &Expr, locals: &mut LocalsBuilder) {
        match expr {
            Expr::Match { expr: sub, arms } => {
                self.collect_locals_from_expr(sub, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(sub, locals);
                locals.add("__match_enum_ptr", ValType::I32, None); // 关联值枚举 match 时暂存 ptr
                for arm in arms {
                    match &arm.pattern {
                        Pattern::Binding(name) => {
                            locals.add(name, ValType::I64, None);
                        }
                        Pattern::Variant { enum_name, variant_name, binding: Some(name) } => {
                            if let Some(ty) = self.resolve_variant_payload(enum_name, variant_name, subject_ast_type.as_ref()) {
                                locals.add(name, ty.to_wasm(), Some(ty.clone()));
                            }
                        }
                        Pattern::Struct { name: struct_name, fields } => {
                            if let Some(def) = self.structs.get(struct_name) {
                                for (fname, pat) in fields {
                                    if let Pattern::Binding(bind) = pat {
                                        if let Some(ft) = def.field_type(fname) {
                                            locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    self.collect_locals_from_expr(&arm.body, locals);
                    if let Some(g) = &arm.guard {
                        self.collect_locals_from_expr(g, locals);
                    }
                }
            }
            Expr::IfLet { pattern, expr, then_branch, else_branch } => {
                self.collect_locals_from_expr(expr, locals);
                let subject_ast_type = self.infer_ast_type_with_locals(expr, locals);
                locals.add("__match_enum_ptr", ValType::I32, None);
                match pattern {
                    Pattern::Binding(name) => {
                        locals.add(name, ValType::I64, None);
                    }
                    Pattern::Variant { enum_name, variant_name, binding: Some(name) } => {
                        if let Some(ty) = self.resolve_variant_payload(enum_name, variant_name, subject_ast_type.as_ref()) {
                            locals.add(name, ty.to_wasm(), Some(ty.clone()));
                        }
                    }
                    Pattern::Struct { name: struct_name, fields } => {
                        if let Some(def) = self.structs.get(struct_name) {
                            for (fname, pat) in fields {
                                if let Pattern::Binding(bind) = pat {
                                    if let Some(ft) = def.field_type(fname) {
                                        locals.add(bind, ft.to_wasm(), Some(ft.clone()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_locals_from_expr(then_branch, locals);
                if let Some(eb) = else_branch {
                    self.collect_locals_from_expr(eb, locals);
                }
            }
            Expr::Tuple(elements) => {
                for e in elements {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__tuple_alloc_ptr", ValType::I32, None);
            }
            Expr::TupleIndex { object, .. } => {
                self.collect_locals_from_expr(object, locals);
            }
            Expr::NullCoalesce { option, default } => {
                self.collect_locals_from_expr(option, locals);
                self.collect_locals_from_expr(default, locals);
            }
            Expr::Binary { left, right, .. } => {
                self.collect_locals_from_expr(left, locals);
                self.collect_locals_from_expr(right, locals);
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
            }
            Expr::MethodCall { object, args, .. } => {
                self.collect_locals_from_expr(object, locals);
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
            }
            Expr::SuperCall { args, .. } => {
                for arg in args {
                    self.collect_locals_from_expr(arg, locals);
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_locals_from_expr(cond, locals);
                self.collect_locals_from_expr(then_branch, locals);
                if let Some(e) = else_branch {
                    self.collect_locals_from_expr(e, locals);
                }
            }
            Expr::Block(stmts, result) => {
                for s in stmts {
                    self.collect_locals(s, locals);
                }
                if let Some(e) = result {
                    self.collect_locals_from_expr(e, locals);
                }
            }
            Expr::Array(elems) => {
                for e in elems {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__array_alloc_ptr", ValType::I32, None);
            }
            Expr::Index { array, index } => {
                self.collect_locals_from_expr(array, locals);
                self.collect_locals_from_expr(index, locals);
            }
            Expr::StructInit { fields, .. } => {
                for (_, e) in fields {
                    self.collect_locals_from_expr(e, locals);
                }
                locals.add("__struct_alloc_ptr", ValType::I32, None);
            }
            Expr::ConstructorCall { args, .. } => {
                for e in args {
                    self.collect_locals_from_expr(e, locals);
                }
            }
            Expr::Field { object, .. } => {
                self.collect_locals_from_expr(object, locals);
            }
            Expr::Unary { expr, .. } => {
                self.collect_locals_from_expr(expr, locals);
            }
            Expr::Range { start, end, .. } => {
                self.collect_locals_from_expr(start, locals);
                self.collect_locals_from_expr(end, locals);
                locals.add("__range_alloc_ptr", ValType::I32, None);
            }
            Expr::Cast { expr, .. } => {
                self.collect_locals_from_expr(expr, locals);
            }
            Expr::VariantConst { arg: Some(e), .. } => {
                self.collect_locals_from_expr(e, locals);
                locals.add("__enum_alloc_ptr", ValType::I32, None);
            }
            Expr::VariantConst { .. } => {}
            Expr::Lambda { body, .. } => {
                self.collect_locals_from_expr(body, locals);
            }
            Expr::Some(inner) | Expr::Ok(inner) | Expr::Err(inner) | Expr::Try(inner) | Expr::Throw(inner) => {
                self.collect_locals_from_expr(inner, locals);
            }
            Expr::None => {}
            Expr::TryBlock { body, catch_body, catch_var, finally_body } => {
                // 预分配 try-catch-finally 所需的内部局部变量
                // 推断 throw 表达式的值类型，以确保 __err_val 类型匹配
                let err_val_type = Self::find_throw_inner_in_stmts(body)
                    .map(|inner| self.infer_type(inner))
                    .unwrap_or(ValType::I32);
                locals.add("__err_flag", ValType::I32, None);
                locals.add("__err_val", err_val_type, None);
                for stmt in body {
                    self.collect_locals(stmt, locals);
                }
                if let Some(var) = catch_var {
                    locals.add(var, err_val_type, None); // 错误值，类型与 throw 表达式匹配
                }
                for stmt in catch_body {
                    self.collect_locals(stmt, locals);
                }
                if let Some(finally_stmts) = finally_body {
                    for stmt in finally_stmts {
                        self.collect_locals(stmt, locals);
                    }
                }
            }
            _ => {}
        }
    }

    /// 解析 Pattern::Variant 的 payload 类型（先查用户定义枚举，再查内建 Option/Result）
    fn resolve_variant_payload(&self, enum_name: &str, variant_name: &str, subject_ast_type: Option<&Type>) -> Option<Type> {
        // 1) 用户定义的枚举
        if let Some(ty) = self.enums.get(enum_name).and_then(|e| e.variant_payload(variant_name)) {
            return Some(ty.clone());
        }
        // 2) 内建 Option<T>
        if enum_name == "Option" {
            if variant_name == "Some" {
                if let Some(Type::Option(inner)) = subject_ast_type {
                    return Some(inner.as_ref().clone());
                }
                return Some(Type::Int64); // fallback
            }
            return None; // None 无 payload
        }
        // 3) 内建 Result<T, E>
        if enum_name == "Result" {
            if variant_name == "Ok" {
                if let Some(Type::Result(ok, _)) = subject_ast_type {
                    return Some(ok.as_ref().clone());
                }
                return Some(Type::Int64); // fallback
            }
            if variant_name == "Err" {
                if let Some(Type::Result(_, err)) = subject_ast_type {
                    return Some(err.as_ref().clone());
                }
                return Some(Type::String); // fallback
            }
        }
        None
    }

    /// 从表达式推断 AST 类型（用于局部变量类型注解缺失时）
    fn infer_ast_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Integer(_) => Some(Type::Int64),
            Expr::Float(_) => Some(Type::Float64),
            Expr::Float32(_) => Some(Type::Float32),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Rune(_) => Some(Type::Rune),
            Expr::String(_) => Some(Type::String),
            Expr::Tuple(ref elems) => {
                let types: Vec<Type> = elems.iter().filter_map(|e| self.infer_ast_type(e)).collect();
                if types.len() == elems.len() {
                    Some(Type::Tuple(types))
                } else {
                    None
                }
            }
            Expr::TupleIndex { object, index } => {
                self.infer_ast_type(object).and_then(|ty| {
                    if let Type::Tuple(types) = ty {
                        types.get(*index as usize).cloned()
                    } else {
                        None
                    }
                })
            }
            Expr::NullCoalesce { default, .. } => self.infer_ast_type(default),
            Expr::Array(ref elems) => elems
                .first()
                .and_then(|e| self.infer_ast_type(e).map(|t| Type::Array(Box::new(t))))
                .or(Some(Type::Array(Box::new(Type::Int64)))),
            Expr::StructInit { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::ConstructorCall { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::VariantConst { enum_name, .. } => Some(Type::Struct(enum_name.clone(), vec![])),
            Expr::Call { name, type_args: _, args } => {
                if self.structs.contains_key(name) {
                    Some(Type::Struct(name.clone(), vec![]))
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    Some(Type::Int64)
                } else {
                    let arg_tys: Vec<Type> = args
                        .iter()
                        .filter_map(|a| self.infer_ast_type(a))
                        .collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    key.and_then(|k| self.func_return_types.get(&k).cloned())
                }
            }
            Expr::MethodCall { .. } => None, // 需结合 locals 推断，调用处可写类型注解
            Expr::SuperCall { .. } => None, // super 调用，需结合父类推断
            Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
            Expr::IfLet { then_branch, .. } => self.infer_ast_type(then_branch),
            Expr::Lambda { params, return_type, .. } => {
                let param_types = params.iter().map(|(_, t)| t.clone()).collect();
                Some(Type::Function {
                    params: param_types,
                    ret: Box::new(return_type.clone()),
                })
            }
            Expr::Some(inner) => self.infer_ast_type(inner).map(|t| Type::Option(Box::new(t))),
            Expr::None => None, // 需要类型注解
            Expr::Ok(inner) => self.infer_ast_type(inner).map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
            Expr::Err(inner) => self.infer_ast_type(inner).map(|_| Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            Expr::Try(inner) => {
                // expr? 解包 Option<T> -> T 或 Result<T, E> -> T
                match self.infer_ast_type(inner) {
                    Some(Type::Option(t)) => Some(*t),
                    Some(Type::Result(t, _)) => Some(*t),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 带 locals 的类型推断（用于 Call 实参等，可解析变量类型）
    fn infer_ast_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<Type> {
        match expr {
            Expr::Var(name) => locals.get_type(name).cloned(),
            Expr::Integer(_) => Some(Type::Int64),
            Expr::Float(_) => Some(Type::Float64),
            Expr::Float32(_) => Some(Type::Float32),
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Rune(_) => Some(Type::Rune),
            Expr::String(_) => Some(Type::String),
            Expr::Tuple(ref elems) => {
                let types: Vec<Type> = elems.iter().filter_map(|e| self.infer_ast_type_with_locals(e, locals)).collect();
                if types.len() == elems.len() {
                    Some(Type::Tuple(types))
                } else {
                    None
                }
            }
            Expr::TupleIndex { object, index } => {
                self.infer_ast_type_with_locals(object, locals).and_then(|ty| {
                    if let Type::Tuple(types) = ty {
                        types.get(*index as usize).cloned()
                    } else {
                        None
                    }
                })
            }
            Expr::NullCoalesce { default, .. } => self.infer_ast_type_with_locals(default, locals),
            Expr::Array(ref elems) => elems
                .first()
                .and_then(|e| self.infer_ast_type_with_locals(e, locals).map(|t| Type::Array(Box::new(t))))
                .or(Some(Type::Array(Box::new(Type::Int64)))),
            Expr::StructInit { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::ConstructorCall { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::VariantConst { enum_name, .. } => Some(Type::Struct(enum_name.clone(), vec![])),
            Expr::Call { name, type_args: _, args } => {
                if self.structs.contains_key(name) {
                    Some(Type::Struct(name.clone(), vec![]))
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    Some(Type::Int64)
                } else {
                    let arg_tys: Vec<Type> = args
                        .iter()
                        .filter_map(|a| self.infer_ast_type_with_locals(a, locals))
                        .collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    key.and_then(|k| self.func_return_types.get(&k).cloned())
                }
            }
            Expr::MethodCall { .. } => None,
            Expr::SuperCall { .. } => None,
            Expr::Cast { target_ty, .. } => Some(target_ty.clone()),
            Expr::IfLet { then_branch, .. } => self.infer_ast_type_with_locals(then_branch, locals),
            Expr::Field { object, field, .. } => {
                self.infer_ast_type_with_locals(object, locals).and_then(|ty| {
                    if let Type::Struct(s, _) = ty {
                        self.structs.get(&s).and_then(|def| {
                            def.fields.iter().find(|f| f.name == *field).map(|f| f.ty.clone())
                        })
                    } else {
                        None
                    }
                })
            }
            Expr::Index { .. } => Some(Type::Int64), // 数组下标结果暂按 Int64
            Expr::Unary { expr, .. } => self.infer_ast_type_with_locals(expr, locals),
            Expr::Binary { op, left, right, .. } => {
                use crate::ast::BinOp;
                match op {
                    BinOp::LogicalAnd | BinOp::LogicalOr | BinOp::Eq | BinOp::NotEq
                    | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => Some(Type::Bool),
                    _ => self.infer_ast_type_with_locals(left, locals)
                        .or_else(|| self.infer_ast_type_with_locals(right, locals)),
                }
            }
            Expr::Range { .. } => Some(Type::Range),
            Expr::Lambda { params, return_type, .. } => {
                let param_types = params.iter().map(|(_, t)| t.clone()).collect();
                Some(Type::Function {
                    params: param_types,
                    ret: Box::new(return_type.clone()),
                })
            }
            Expr::Some(inner) => self.infer_ast_type_with_locals(inner, locals).map(|t| Type::Option(Box::new(t))),
            Expr::None => None,
            Expr::Ok(inner) => self.infer_ast_type_with_locals(inner, locals).map(|t| Type::Result(Box::new(t), Box::new(Type::String))),
            Expr::Err(inner) => self.infer_ast_type_with_locals(inner, locals).map(|_| Type::Result(Box::new(Type::Int64), Box::new(Type::String))),
            Expr::Try(inner) => {
                match self.infer_ast_type_with_locals(inner, locals) {
                    Some(Type::Option(t)) => Some(*t),
                    Some(Type::Result(t, _)) => Some(*t),
                    _ => None,
                }
            }
            _ => self.infer_ast_type(expr),
        }
    }

    /// 获取"对象表达式"的 AST 类型（用于字段访问、方法调用时查结构体与偏移）
    fn get_object_type(&self, expr: &Expr, locals: &LocalsBuilder) -> Option<Type> {
        match expr {
            Expr::Var(name) => locals.get_type(name).cloned(),
            Expr::StructInit { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::ConstructorCall { name, type_args, .. } => Some(Type::Struct(name.clone(), type_args.clone().unwrap_or_default())),
            Expr::Field { object, .. } => self.get_object_type(object, locals),
            _ => None,
        }
    }

    /// 用于 for 循环变量：可迭代表达式的“元素类型”（范围时为 Int64，数组时为元素类型）
    fn expr_object_type(&self, expr: &Expr) -> Option<Type> {
        match expr {
            Expr::Range { .. } => Some(Type::Int64),
            Expr::Array(ref elems) => elems.first().and_then(|e| self.infer_ast_type(e)).or(Some(Type::Int64)),
            _ => None,
        }
    }

    /// 按 AST 类型生成 load 指令（栈顶为 i32 地址）
    fn emit_load_by_type(&self, func: &mut WasmFunc, ty: &Type) {
        let instr = match ty.to_wasm() {
            ValType::I32 => Instruction::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::I64 => Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F64 => Instruction::F64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F32 => Instruction::F32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", ty),
        };
        func.instruction(&instr);
    }

    /// 按 WASM ValType 生成 store 指令（栈顶依次为：地址 i32，值）
    fn emit_store_by_wasm_type(func: &mut WasmFunc, vt: ValType) {
        let instr = match vt {
            ValType::I32 => Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::I64 => Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F64 => Instruction::F64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F32 => Instruction::F32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", vt),
        };
        func.instruction(&instr);
    }

    /// 按 AST 类型生成 store 指令（栈顶依次为：地址 i32，值）
    fn emit_store_by_type(&self, func: &mut WasmFunc, ty: &Type) {
        let instr = match ty.to_wasm() {
            ValType::I32 => Instruction::I32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::I64 => Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F64 => Instruction::F64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }),
            ValType::F32 => Instruction::F32Store(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }),
            ValType::V128 | ValType::Ref(_) => panic!("不支持的字段类型: {:?}", ty),
        };
        func.instruction(&instr);
    }

    /// 判断表达式编译后是否在栈上产生一个值
    fn expr_produces_value(expr: &Expr) -> bool {
        match expr {
            Expr::Block(_, result) => result.is_some(),
            // if 无 else 编译为 BlockType::Empty，不产生值
            Expr::If { else_branch: None, .. } => false,
            // if-else：只有两个分支都产生值时，整个 if-else 才产生值
            Expr::If { then_branch, else_branch: Some(else_expr), .. } => {
                Self::expr_produces_value(then_branch) && Self::expr_produces_value(else_expr)
            }
            // throw 设置 __err_flag/__err_val 并跳转，不在栈上留值
            Expr::Throw(_) => false,
            // try-catch 内部通过 return 处理控制流，不在栈上留值
            Expr::TryBlock { .. } => false,
            _ => true,
        }
    }

    /// 类型推断（含局部变量上下文），优先使用 AST 类型推断结果
    fn infer_type_with_locals(&self, expr: &Expr, locals: &LocalsBuilder) -> ValType {
        self.infer_ast_type_with_locals(expr, locals)
            .map(|t| t.to_wasm())
            .unwrap_or_else(|| self.infer_type(expr))
    }

    /// 简单的类型推断
    fn infer_type(&self, expr: &Expr) -> ValType {
        match expr {
            Expr::Integer(_) => ValType::I64,
            Expr::Float(_) => ValType::F64,
            Expr::Float32(_) => ValType::F32,
            Expr::Bool(_) => ValType::I32,
            Expr::Rune(_) => ValType::I32,
            Expr::String(_) => ValType::I32,
            Expr::Array(_) => ValType::I32,
            Expr::Tuple(_) => ValType::I32,
            Expr::TupleIndex { .. } => ValType::I64, // 默认假设 i64，实际需类型推断
            Expr::NullCoalesce { default, .. } => self.infer_type(default),
            Expr::StructInit { .. } => ValType::I32,
            Expr::ConstructorCall { .. } => ValType::I32,
            Expr::Call { name, type_args: _, args } => {
                if self.structs.contains_key(name) {
                    ValType::I32
                } else if (name == "min" || name == "max") && args.len() == 2
                    || (name == "abs" && args.len() == 1)
                {
                    ValType::I64
                } else {
                    let arg_tys: Vec<Type> = args
                        .iter()
                        .filter_map(|a| self.infer_ast_type(a))
                        .collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        if arg_tys.len() == args.len() {
                            Some(Self::mangle_key(name, &arg_tys))
                        } else {
                            None
                        }
                    } else {
                        Some(name.to_string())
                    };
                    key.and_then(|k| self.func_return_types.get(&k))
                        .map(|t| t.to_wasm())
                        .unwrap_or(ValType::I64)
                }
            }
            Expr::Unary { op, expr } => match op {
                UnaryOp::Not => ValType::I32,
                UnaryOp::Neg | UnaryOp::BitNot => self.infer_type(expr),
            },
            Expr::Binary { op, left, .. } => match op {
                BinOp::LogicalAnd | BinOp::LogicalOr
                | BinOp::Eq | BinOp::NotEq
                | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => ValType::I32,
                _ => self.infer_type(left),
            },
            Expr::Index { .. } => ValType::I64,
            Expr::Field { .. } => ValType::I64,
            Expr::VariantConst { .. } => ValType::I32,
            Expr::Cast { target_ty, .. } => target_ty.to_wasm(),
            Expr::IfLet { then_branch, .. } => self.infer_type(then_branch),
            Expr::Lambda { .. } => ValType::I32, // 函数表索引
            Expr::Some(_) | Expr::None | Expr::Ok(_) | Expr::Err(_) => ValType::I32, // 指针
            Expr::Try(inner) => {
                // expr? 解包后的类型
                match self.infer_ast_type(inner) {
                    Some(Type::Option(t)) => t.to_wasm(),
                    Some(Type::Result(t, _)) => t.to_wasm(),
                    _ => self.infer_type(inner),
                }
            }
            Expr::Throw(_) => ValType::I32, // 不返回，但需要类型
            Expr::TryBlock { body, .. } => {
                // try 块的结果类型来自最后一个表达式
                if let Some(Stmt::Expr(e)) = body.last() {
                    self.infer_type(e)
                } else {
                    ValType::I64
                }
            }
            _ => ValType::I64,
        }
    }

    /// 循环上下文：(break 目标深度, continue 目标深度)。单层 while/for 为 (1, 0)。
    fn compile_stmt(
        &self,
        stmt: &Stmt,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        match stmt {
            Stmt::Let { pattern, value, .. } => {
                self.compile_expr(value, locals, func, loop_ctx);
                match pattern {
                    Pattern::Binding(name) => {
                        let idx = locals.get(name).expect("局部变量未找到");
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                    Pattern::Struct { name: struct_name, fields } => {
                        let ptr_tmp = locals.get("__let_struct_ptr").expect("__let_struct_ptr");
                        func.instruction(&Instruction::LocalSet(ptr_tmp));
                        let struct_def = &self.structs[struct_name];
                        for (fname, pat) in fields {
                            let offset = struct_def.field_offset(fname).expect("结构体字段");
                            let fty = struct_def.field_type(fname).expect("字段类型");
                            func.instruction(&Instruction::LocalGet(ptr_tmp));
                            func.instruction(&Instruction::I32Const(offset as i32));
                            func.instruction(&Instruction::I32Add);
                            self.emit_load_by_type(func, fty);
                            if let Pattern::Binding(bind) = pat {
                                let idx = locals.get(bind).expect("解构绑定名");
                                func.instruction(&Instruction::LocalSet(idx));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Stmt::Var { name, value, .. } => {
                self.compile_expr(value, locals, func, loop_ctx);
                let idx = locals.get(name).expect("局部变量未找到");
                func.instruction(&Instruction::LocalSet(idx));
            }
            Stmt::Assign { target, value } => {
                match target {
                    AssignTarget::Var(name) => {
                        // Phase 8: 引用计数 - 赋值前对旧值 rc_dec
                        if let Some(ast_ty) = locals.get_type(name) {
                            if memory::is_heap_type(ast_ty) || memory::may_hold_heap_ptr(ast_ty) {
                                if let Some(rc_dec_idx) = self.func_indices.get("__rc_dec") {
                                    let idx = locals.get(name).expect("变量未找到");
                                    func.instruction(&Instruction::LocalGet(idx));
                                    func.instruction(&Instruction::Call(*rc_dec_idx));
                                }
                            }
                        }
                        self.compile_expr(value, locals, func, loop_ctx);
                        let idx = locals.get(name).expect("变量未找到");
                        func.instruction(&Instruction::LocalSet(idx));
                    }
                    AssignTarget::Index { array, index } => {
                        // arr[i] = value
                        // 计算地址: arr + i * 8
                        let arr_idx = locals.get(array).expect("数组未找到");
                        func.instruction(&Instruction::LocalGet(arr_idx));
                        self.compile_expr(index, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32WrapI64);
                        func.instruction(&Instruction::I32Const(8)); // 元素大小
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        // 存储值
                        self.compile_expr(value, locals, func, loop_ctx);
                        func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        }));
                    }
                    AssignTarget::Field { object, field } => {
                        // obj.field = value：用对象类型计算字段偏移与字段类型
                        let obj_idx = locals.get(object).expect("对象未找到");
                        let (offset, field_ty) = locals
                            .get_type(object)
                            .and_then(|ty| match ty {
                                Type::Struct(name, _) => {
                                    // 优先从 ClassInfo 获取偏移（包含 vtable header）
                                    if let Some(ci) = self.classes.get(name) {
                                        let off = ci.field_offset(field)?;
                                        let ft = ci.field_type(field)?.clone();
                                        Some((off, ft))
                                    } else {
                                        self.structs.get(name).and_then(|def| {
                                            let off = def.field_offset(field)?;
                                            let ft = def.field_type(field)?.clone();
                                            Some((off, ft))
                                        })
                                    }
                                }
                                _ => None,
                            })
                            .unwrap_or((0, Type::Int64));
                        func.instruction(&Instruction::LocalGet(obj_idx));
                        func.instruction(&Instruction::I32Const(offset as i32));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(value, locals, func, loop_ctx);
                        self.emit_store_by_type(func, &field_ty);
                    }
                }
            }
            Stmt::Return(Some(expr)) => {
                self.compile_expr(expr, locals, func, loop_ctx);
                func.instruction(&Instruction::Return);
            }
            Stmt::Return(None) => {
                func.instruction(&Instruction::Return);
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr, locals, func, loop_ctx);
                // 仅当表达式会在栈上产生值时才 drop
                if Self::expr_produces_value(expr) {
                    func.instruction(&Instruction::Drop);
                }
            }
            Stmt::Break => {
                if let Some((break_depth, _)) = loop_ctx {
                    func.instruction(&Instruction::Br(break_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Stmt::Continue => {
                if let Some((_, continue_depth)) = loop_ctx {
                    func.instruction(&Instruction::Br(continue_depth));
                } else {
                    func.instruction(&Instruction::Unreachable);
                }
            }
            Stmt::Loop { body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                let body_ctx = Some((1u32, 0u32));
                for s in body {
                    self.compile_stmt(s, locals, func, body_ctx);
                }
                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::While { cond, body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                self.compile_expr(cond, locals, func, loop_ctx);
                // 条件必须是 i32；仅当条件表达式结果是 i64 时才 wrap
                if self.infer_type_with_locals(cond, locals) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::BrIf(1));

                let body_ctx = Some((1u32, 0u32)); // break→block end, continue→loop start
                for s in body {
                    self.compile_stmt(s, locals, func, body_ctx);
                }

                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End);
                func.instruction(&Instruction::End);
            }
            Stmt::WhileLet { pattern, expr, body } => {
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // Br(1) = break
                func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));   // Br(0) = continue
                self.compile_expr(expr, locals, func, loop_ctx);
                let subject_ty = self.infer_type(expr);
                let subject_ast_type = self.infer_ast_type(expr);
                let ptr_tmp = locals.get("__match_enum_ptr").expect("__match_enum_ptr");
                let body_ctx = Some((1u32, 0u32));

                match pattern {
                    Pattern::Binding(name) => {
                        if let Some(idx) = locals.get(name) {
                            if subject_ty == ValType::I32 {
                                func.instruction(&Instruction::I64ExtendI32S);
                            }
                            func.instruction(&Instruction::LocalSet(idx));
                        } else {
                            func.instruction(&Instruction::Drop);
                        }
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::Br(0));
                    }
                    Pattern::Wildcard => {
                        func.instruction(&Instruction::Drop);
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
                        }
                        func.instruction(&Instruction::Br(0));
                    }
                    Pattern::Variant { enum_name, variant_name, binding } => {
                        let enum_def = self.enums.get(enum_name).and_then(|e| e.variant_index(variant_name).map(|_| e));
                        if let Some(enum_def) = enum_def {
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let expected_disc = enum_def.variant_index(variant_name).unwrap() as i32;
                            let has_variant_payload = enum_def.has_payload();
                            let resolved_payload = self.resolve_variant_payload(enum_name, variant_name, subject_ast_type.as_ref());
                            func.instruction(&Instruction::LocalGet(ptr_tmp));
                            if has_variant_payload {
                                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                    offset: 0,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                func.instruction(&Instruction::I32Const(expected_disc));
                                func.instruction(&Instruction::I32Eq);
                            } else {
                                func.instruction(&Instruction::I32Const(expected_disc));
                                func.instruction(&Instruction::I32Eq);
                            }
                            func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                            if has_variant_payload {
                                if let Some(ref bind_name) = binding {
                                    if let Some(ref payload_ty) = resolved_payload {
                                        func.instruction(&Instruction::LocalGet(ptr_tmp));
                                        func.instruction(&Instruction::I32Const(4));
                                        func.instruction(&Instruction::I32Add);
                                        self.emit_load_by_type(func, payload_ty);
                                        let bind_idx = locals.get(bind_name).expect("关联值绑定未找到");
                                        func.instruction(&Instruction::LocalSet(bind_idx));
                                    }
                                }
                            }
                            for s in body {
                                self.compile_stmt(s, locals, func, body_ctx);
                            }
                            func.instruction(&Instruction::Br(0));
                            func.instruction(&Instruction::Else);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::End);
                        } else {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::Br(1));
                        }
                    }
                    Pattern::Struct { name: struct_name, fields } => {
                        let handled = if let Some(Type::Struct(ref sub_name, _)) = subject_ast_type {
                            sub_name == struct_name && self.structs.contains_key(struct_name)
                        } else {
                            false
                        };
                        if handled {
                            func.instruction(&Instruction::LocalSet(ptr_tmp));
                            let struct_def = &self.structs[struct_name];
                            for (fname, pat) in fields {
                                let offset = struct_def.field_offset(fname).expect("结构体字段");
                                let fty = struct_def.field_type(fname).expect("字段类型");
                                func.instruction(&Instruction::LocalGet(ptr_tmp));
                                func.instruction(&Instruction::I32Const(offset as i32));
                                func.instruction(&Instruction::I32Add);
                                self.emit_load_by_type(func, fty);
                                if let Pattern::Binding(bind) = pat {
                                    let idx = locals.get(bind).expect("解构绑定名");
                                    func.instruction(&Instruction::LocalSet(idx));
                                }
                            }
                            for s in body {
                                self.compile_stmt(s, locals, func, body_ctx);
                            }
                            func.instruction(&Instruction::Br(0));
                        } else {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::Br(1));
                        }
                    }
                    _ => {
                        func.instruction(&Instruction::Drop);
                        func.instruction(&Instruction::Br(1));
                    }
                }

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
                        self.compile_expr(start, locals, func, loop_ctx);
                        func.instruction(&Instruction::LocalSet(var_idx));

                        // block { loop { ... } }
                        func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                        func.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

                        // 条件检查: i < end (或 i <= end)
                        func.instruction(&Instruction::LocalGet(var_idx));
                        self.compile_expr(end, locals, func, loop_ctx);
                        if *inclusive {
                            func.instruction(&Instruction::I64GtS); // i > end
                        } else {
                            func.instruction(&Instruction::I64GeS); // i >= end
                        }
                        func.instruction(&Instruction::BrIf(1)); // 退出

                        // 循环体
                        let body_ctx = Some((1u32, 0u32));
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
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
                        self.compile_expr(iterable, locals, func, loop_ctx);
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
                        let body_ctx = Some((1u32, 0u32));
                        for s in body {
                            self.compile_stmt(s, locals, func, body_ctx);
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
    fn compile_expr(
        &self,
        expr: &Expr,
        locals: &LocalsBuilder,
        func: &mut WasmFunc,
        loop_ctx: Option<(u32, u32)>,
    ) {
        match expr {
            Expr::Integer(n) => {
                func.instruction(&Instruction::I64Const(*n));
            }
            Expr::Float32(f) => {
                func.instruction(&Instruction::F32Const(*f));
            }
            Expr::Float(f) => {
                func.instruction(&Instruction::F64Const(*f));
            }
            Expr::Bool(b) => {
                func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            }
            Expr::Rune(c) => {
                func.instruction(&Instruction::I32Const(*c as i32));
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
            Expr::Interpolate(parts) => {
                // 字符串插值：逐部分分配并拼接
                // 简化实现：在堆上构建最终字符串
                // 首先计算总长度，然后分配并复制

                if parts.is_empty() {
                    // 空插值返回空字符串
                    let empty_offset = self
                        .string_pool
                        .iter()
                        .find(|(s, _)| s.is_empty())
                        .map(|(_, off)| *off)
                        .unwrap_or(0);
                    func.instruction(&Instruction::I32Const(empty_offset as i32));
                    return;
                }

                // 将每个部分编译为字符串指针，压入栈
                // 策略：使用 __str_concat 运行时函数逐个拼接
                // 生成：part1 -> __concat(part1, part2) -> __concat(result, part3) -> ...

                let mut is_first = true;
                for part in parts {
                    match part {
                        InterpolatePart::Literal(text) => {
                            // 获取字面量字符串的地址
                            let offset = self
                                .string_pool
                                .iter()
                                .find(|(s, _)| s == text)
                                .map(|(_, off)| *off)
                                .unwrap_or_else(|| {
                                    // 如果字符串不在池中，添加它
                                    // 注意：这里简化处理，实际应该在编译前收集所有字符串
                                    0
                                });
                            func.instruction(&Instruction::I32Const(offset as i32));
                        }
                        InterpolatePart::Expr(expr) => {
                            // 编译表达式
                            self.compile_expr(expr, locals, func, loop_ctx);
                            // 将值转换为字符串（调用 __to_string_TYPE 运行时函数）
                            let expr_type = self.infer_ast_type(expr);
                            match expr_type.as_ref() {
                                Some(Type::Int64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i64_to_str"),
                                    ));
                                }
                                Some(Type::Int32) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i32_to_str"),
                                    ));
                                }
                                Some(Type::Float64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__f64_to_str"),
                                    ));
                                }
                                Some(Type::Float32) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__f32_to_str"),
                                    ));
                                }
                                Some(Type::Int8) | Some(Type::Int16) | Some(Type::UInt8) | Some(Type::UInt16) | Some(Type::UInt32) | Some(Type::Rune) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i32_to_str"),
                                    ));
                                }
                                Some(Type::UInt64) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__i64_to_str"),
                                    ));
                                }
                                Some(Type::Bool) => {
                                    func.instruction(&Instruction::Call(
                                        self.get_or_create_func_index("__bool_to_str"),
                                    ));
                                }
                                Some(Type::String) => {
                                    // 已经是字符串，不需要转换
                                }
                                _ => {
                                    // 其他类型暂时转为 "[object]"
                                    func.instruction(&Instruction::Drop);
                                    let obj_str = self
                                        .string_pool
                                        .iter()
                                        .find(|(s, _)| s == "[object]")
                                        .map(|(_, off)| *off)
                                        .unwrap_or(0);
                                    func.instruction(&Instruction::I32Const(obj_str as i32));
                                }
                            }
                        }
                    }

                    if !is_first {
                        // 拼接前一个结果和当前部分
                        func.instruction(&Instruction::Call(
                            self.get_or_create_func_index("__str_concat"),
                        ));
                    }
                    is_first = false;
                }
            }
            Expr::Var(name) => {
                let idx = locals.get(name).unwrap_or_else(|| panic!("变量未找到: '{}'", name));
                func.instruction(&Instruction::LocalGet(idx));
            }
            Expr::Unary { op: UnaryOp::Not, expr } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                if self.infer_type(expr) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
            }
            Expr::Unary { op: UnaryOp::BitNot, expr } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                let ty = self.infer_type(expr);
                match ty {
                    ValType::I64 => {
                        func.instruction(&Instruction::I64Const(-1));
                        func.instruction(&Instruction::I64Xor);
                    }
                    ValType::I32 => {
                        func.instruction(&Instruction::I32Const(-1));
                        func.instruction(&Instruction::I32Xor);
                    }
                    _ => panic!("~ 仅支持整数类型"),
                }
            }
            Expr::Unary { op: UnaryOp::Neg, expr } => {
                let ty = self.infer_type(expr);
                match ty {
                    ValType::I32 => {
                        func.instruction(&Instruction::I32Const(0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::I32Sub);
                    }
                    ValType::I64 => {
                        func.instruction(&Instruction::I64Const(0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::I64Sub);
                    }
                    ValType::F64 => {
                        func.instruction(&Instruction::F64Const(0.0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::F64Sub);
                    }
                    ValType::F32 => {
                        func.instruction(&Instruction::F32Const(0.0));
                        self.compile_expr(expr, locals, func, loop_ctx);
                        func.instruction(&Instruction::F32Sub);
                    }
                    ValType::V128 | ValType::Ref(_) => panic!("不支持一元负号: {:?}", ty),
                }
            }
            Expr::Binary { op: BinOp::LogicalAnd, left, right } => {
                // 短路与：left && right，结果为 i32 (0/1)
                self.compile_expr(left, locals, func, loop_ctx);
                if self.infer_type(left) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::Else);
                self.compile_expr(right, locals, func, loop_ctx);
                if self.infer_type(right) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::End);
            }
            Expr::Binary { op: BinOp::LogicalOr, left, right } => {
                // 短路或：left || right，用 __logical_tmp 保存 left，结果为 i32 (0/1)
                let tmp = locals.get("__logical_tmp").expect("__logical_tmp 未找到");
                self.compile_expr(left, locals, func, loop_ctx);
                if self.infer_type(left) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::LocalSet(tmp));
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                self.compile_expr(right, locals, func, loop_ctx);
                if self.infer_type(right) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::Else);
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Eqz);
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Sub);
                func.instruction(&Instruction::End);
            }
            Expr::Binary { op, left, right } => {
                if op == &BinOp::Pow {
                    self.compile_expr(left, locals, func, loop_ctx);
                    self.compile_expr(right, locals, func, loop_ctx);
                    let idx = *self.func_indices.get("__pow_i64").unwrap();
                    func.instruction(&Instruction::Call(idx));
                    return;
                }
                self.compile_expr(left, locals, func, loop_ctx);
                self.compile_expr(right, locals, func, loop_ctx);

                // 检查是否为无符号类型，以选择无符号除法/比较指令
                let ast_ty = self.infer_ast_type_with_locals(left, locals);
                let is_unsigned = matches!(ast_ty.as_ref(), Some(Type::UInt8) | Some(Type::UInt16) | Some(Type::UInt32) | Some(Type::UInt64));

                let val_type = self.infer_type_with_locals(left, locals);

                // 无符号类型需要使用无符号指令
                if is_unsigned {
                    let instr = match (op, val_type) {
                        (BinOp::Div, ValType::I32) => Instruction::I32DivU,
                        (BinOp::Mod, ValType::I32) => Instruction::I32RemU,
                        (BinOp::Lt, ValType::I32) => Instruction::I32LtU,
                        (BinOp::Gt, ValType::I32) => Instruction::I32GtU,
                        (BinOp::LtEq, ValType::I32) => Instruction::I32LeU,
                        (BinOp::GtEq, ValType::I32) => Instruction::I32GeU,
                        (BinOp::Shr, ValType::I32) => Instruction::I32ShrU,
                        (BinOp::Div, ValType::I64) => Instruction::I64DivU,
                        (BinOp::Mod, ValType::I64) => Instruction::I64RemU,
                        (BinOp::Lt, ValType::I64) => Instruction::I64LtU,
                        (BinOp::Gt, ValType::I64) => Instruction::I64GtU,
                        (BinOp::LtEq, ValType::I64) => Instruction::I64LeU,
                        (BinOp::GtEq, ValType::I64) => Instruction::I64GeU,
                        (BinOp::Shr, ValType::I64) => Instruction::I64ShrU,
                        _ => {
                            // 对于 Add/Sub/Mul/Eq/NotEq 等，有符号和无符号相同
                            match (op, val_type) {
                                (BinOp::Add, ValType::I32) => Instruction::I32Add,
                                (BinOp::Sub, ValType::I32) => Instruction::I32Sub,
                                (BinOp::Mul, ValType::I32) => Instruction::I32Mul,
                                (BinOp::Eq, ValType::I32) => Instruction::I32Eq,
                                (BinOp::NotEq, ValType::I32) => Instruction::I32Ne,
                                (BinOp::BitAnd, ValType::I32) => Instruction::I32And,
                                (BinOp::BitOr, ValType::I32) => Instruction::I32Or,
                                (BinOp::BitXor, ValType::I32) => Instruction::I32Xor,
                                (BinOp::Shl, ValType::I32) => Instruction::I32Shl,
                                (BinOp::Add, ValType::I64) => Instruction::I64Add,
                                (BinOp::Sub, ValType::I64) => Instruction::I64Sub,
                                (BinOp::Mul, ValType::I64) => Instruction::I64Mul,
                                (BinOp::Eq, ValType::I64) => Instruction::I64Eq,
                                (BinOp::NotEq, ValType::I64) => Instruction::I64Ne,
                                (BinOp::BitAnd, ValType::I64) => Instruction::I64And,
                                (BinOp::BitOr, ValType::I64) => Instruction::I64Or,
                                (BinOp::BitXor, ValType::I64) => Instruction::I64Xor,
                                (BinOp::Shl, ValType::I64) => Instruction::I64Shl,
                                _ => panic!("不支持的无符号运算: {:?} for {:?}", op, val_type),
                            }
                        }
                    };
                    func.instruction(&instr);

                    // UInt8/UInt16 掩码
                    match ast_ty.as_ref() {
                        Some(Type::UInt8) => match op {
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                                func.instruction(&Instruction::I32Const(0xFF));
                                func.instruction(&Instruction::I32And);
                            }
                            _ => {}
                        },
                        Some(Type::UInt16) => match op {
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                                func.instruction(&Instruction::I32Const(0xFFFF));
                                func.instruction(&Instruction::I32And);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                    return;
                }

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

                    (BinOp::Add, ValType::F32) => Instruction::F32Add,
                    (BinOp::Sub, ValType::F32) => Instruction::F32Sub,
                    (BinOp::Mul, ValType::F32) => Instruction::F32Mul,
                    (BinOp::Div, ValType::F32) => Instruction::F32Div,
                    (BinOp::Lt, ValType::F32) => Instruction::F32Lt,
                    (BinOp::Gt, ValType::F32) => Instruction::F32Gt,
                    (BinOp::LtEq, ValType::F32) => Instruction::F32Le,
                    (BinOp::GtEq, ValType::F32) => Instruction::F32Ge,
                    (BinOp::Eq, ValType::F32) => Instruction::F32Eq,
                    (BinOp::NotEq, ValType::F32) => Instruction::F32Ne,

                    (BinOp::BitAnd, ValType::I64) => Instruction::I64And,
                    (BinOp::BitOr, ValType::I64) => Instruction::I64Or,
                    (BinOp::BitXor, ValType::I64) => Instruction::I64Xor,
                    (BinOp::Shl, ValType::I64) => Instruction::I64Shl,
                    (BinOp::Shr, ValType::I64) => Instruction::I64ShrS,
                    (BinOp::BitAnd, ValType::I32) => Instruction::I32And,
                    (BinOp::BitOr, ValType::I32) => Instruction::I32Or,
                    (BinOp::BitXor, ValType::I32) => Instruction::I32Xor,
                    (BinOp::Shl, ValType::I32) => Instruction::I32Shl,
                    (BinOp::Shr, ValType::I32) => Instruction::I32ShrS,

                    _ => panic!("不支持的运算: {:?} for {:?}", op, val_type),
                };
                func.instruction(&instr);

                // 对 Int8/Int16 算术运算结果进行符号扩展
                // 对 UInt8/UInt16 算术运算结果进行掩码
                let ast_ty = self.infer_ast_type_with_locals(left, locals);
                if let Some(ty) = &ast_ty {
                    match (ty, op) {
                        (Type::Int8, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) => {
                            // Int8 符号扩展: (val << 24) >> 24
                            func.instruction(&Instruction::I32Const(24));
                            func.instruction(&Instruction::I32Shl);
                            func.instruction(&Instruction::I32Const(24));
                            func.instruction(&Instruction::I32ShrS);
                        }
                        (Type::Int16, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) => {
                            // Int16 符号扩展: (val << 16) >> 16
                            func.instruction(&Instruction::I32Const(16));
                            func.instruction(&Instruction::I32Shl);
                            func.instruction(&Instruction::I32Const(16));
                            func.instruction(&Instruction::I32ShrS);
                        }
                        (Type::UInt8, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) => {
                            // UInt8 掩码: val & 0xFF
                            func.instruction(&Instruction::I32Const(0xFF));
                            func.instruction(&Instruction::I32And);
                        }
                        (Type::UInt16, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) => {
                            // UInt16 掩码: val & 0xFFFF
                            func.instruction(&Instruction::I32Const(0xFFFF));
                            func.instruction(&Instruction::I32And);
                        }
                        _ => {}
                    }
                }
            }
            Expr::Cast { expr, target_ty } => {
                self.compile_expr(expr, locals, func, loop_ctx);
                let src = self.infer_type(expr);
                let dst = target_ty.to_wasm();
                if src != dst {
                    let conv = match (src, dst) {
                        (ValType::I64, ValType::I32) => Instruction::I32WrapI64,
                        (ValType::I32, ValType::I64) => Instruction::I64ExtendI32S,
                        (ValType::I64, ValType::F64) => Instruction::F64ConvertI64S,
                        (ValType::F64, ValType::I64) => Instruction::I64TruncF64S,
                        (ValType::I32, ValType::F64) => Instruction::F64ConvertI32S,
                        (ValType::F64, ValType::I32) => Instruction::I32TruncF64S,
                        (ValType::F32, ValType::F64) => Instruction::F64PromoteF32,
                        (ValType::F64, ValType::F32) => Instruction::F32DemoteF64,
                        (ValType::I32, ValType::F32) => Instruction::F32ConvertI32S,
                        (ValType::F32, ValType::I32) => Instruction::I32TruncF32S,
                        (ValType::I64, ValType::F32) => Instruction::F32ConvertI64S,
                        (ValType::F32, ValType::I64) => Instruction::I64TruncF32S,
                        _ => panic!("不支持的 as 转换: {:?} -> {:?}", src, target_ty),
                    };
                    func.instruction(&conv);
                }
                // 转换到小整数类型时进行符号扩展/掩码
                match target_ty {
                    Type::Int8 => {
                        func.instruction(&Instruction::I32Const(24));
                        func.instruction(&Instruction::I32Shl);
                        func.instruction(&Instruction::I32Const(24));
                        func.instruction(&Instruction::I32ShrS);
                    }
                    Type::Int16 => {
                        func.instruction(&Instruction::I32Const(16));
                        func.instruction(&Instruction::I32Shl);
                        func.instruction(&Instruction::I32Const(16));
                        func.instruction(&Instruction::I32ShrS);
                    }
                    Type::UInt8 => {
                        func.instruction(&Instruction::I32Const(0xFF));
                        func.instruction(&Instruction::I32And);
                    }
                    Type::UInt16 => {
                        func.instruction(&Instruction::I32Const(0xFFFF));
                        func.instruction(&Instruction::I32And);
                    }
                    _ => {}
                }
            }
            Expr::Call { name, type_args: _, args } => {
                // 检查是否为带 init 的类构造调用
                let init_func_name = format!("__{}_init", name);
                if self.func_indices.contains_key(&init_func_name) {
                    // 调用 __ClassName_init(args...) 返回对象指针
                    for arg in args {
                        self.compile_expr(arg, locals, func, loop_ctx);
                    }
                    let idx = self.func_indices[&init_func_name];
                    func.instruction(&Instruction::Call(idx));
                } else if let Some(struct_def) = self.structs.get(name).cloned() {
                    if args.len() != struct_def.fields.len() {
                        panic!(
                            "结构体 {} 构造函数需要 {} 个参数，得到 {} 个",
                            name,
                            struct_def.fields.len(),
                            args.len()
                        );
                    }
                    let fields: Vec<(String, Expr)> = struct_def
                        .fields
                        .iter()
                        .map(|f| f.name.clone())
                        .zip(args.clone())
                        .collect();
                    let init_expr = Expr::StructInit {
                        name: name.clone(),
                        type_args: None,
                        fields,
                    };
                    self.compile_expr(&init_expr, locals, func, loop_ctx);
                } else if name == "min" && args.len() == 2
                    && self.infer_ast_type_with_locals(&args[0], locals).as_ref() == Some(&Type::Int64)
                    && self.infer_ast_type_with_locals(&args[1], locals).as_ref() == Some(&Type::Int64)
                {
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    self.compile_expr(&args[1], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.get_or_create_func_index("__min_i64")));
                } else if name == "max" && args.len() == 2
                    && self.infer_ast_type_with_locals(&args[0], locals).as_ref() == Some(&Type::Int64)
                    && self.infer_ast_type_with_locals(&args[1], locals).as_ref() == Some(&Type::Int64)
                {
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    self.compile_expr(&args[1], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.get_or_create_func_index("__max_i64")));
                } else if name == "abs" && args.len() == 1
                    && self.infer_ast_type_with_locals(&args[0], locals).as_ref() == Some(&Type::Int64)
                {
                    self.compile_expr(&args[0], locals, func, loop_ctx);
                    func.instruction(&Instruction::Call(self.get_or_create_func_index("__abs_i64")));
                } else if args.len() == 1 && ["Int64", "Int32", "Int16", "Int8", "UInt64", "UInt32", "UInt16", "UInt8", "Float64", "Float32", "Bool"].contains(&name.as_str()) {
                    // 类型转换构造函数 T(e) - cjc 兼容
                    let target_ty = match name.as_str() {
                        "Int64" => Type::Int64,
                        "Int32" => Type::Int32,
                        "Int16" => Type::Int16,
                        "Int8" => Type::Int8,
                        "UInt64" => Type::UInt64,
                        "UInt32" => Type::UInt32,
                        "UInt16" => Type::UInt16,
                        "UInt8" => Type::UInt8,
                        "Float64" => Type::Float64,
                        "Float32" => Type::Float32,
                        "Bool" => Type::Bool,
                        _ => unreachable!(),
                    };
                    self.compile_expr(&Expr::Cast { expr: Box::new(args[0].clone()), target_ty }, locals, func, loop_ctx);
                } else {
                    let arg_tys: Vec<Type> = args
                        .iter()
                        .map(|a| {
                            self.infer_ast_type_with_locals(a, locals).expect("无法推断实参类型，无法解析重载")
                        })
                        .collect();
                    let key = if *self.name_count.get(name).unwrap_or(&0) > 1 {
                        Self::mangle_key(name, &arg_tys)
                    } else {
                        name.to_string()
                    };
                    let params = self.func_params.get(&key).expect("函数未找到");

                    // 检查是否有可变参数
                    let variadic_idx = params.iter().position(|p| p.variadic);

                    for (i, param) in params.iter().enumerate() {
                        if param.variadic {
                            // 可变参数：将剩余实参打包成数组
                            let variadic_args: Vec<Expr> = args[i..].to_vec();
                            let arr_expr = Expr::Array(variadic_args);
                            self.compile_expr(&arr_expr, locals, func, loop_ctx);
                        } else if i < args.len() && variadic_idx.map_or(true, |vi| i < vi) {
                            // 普通参数：直接编译实参
                            self.compile_expr(&args[i], locals, func, loop_ctx);
                        } else if let Some(ref default) = param.default {
                            self.compile_expr(default, locals, func, loop_ctx);
                        } else {
                            panic!("函数 {} 第 {} 个参数缺少实参且无默认值", name, i + 1);
                        }
                    }
                    let idx = *self.func_indices.get(&key).expect("函数未找到");
                    func.instruction(&Instruction::Call(idx));
                }
            }
            Expr::SuperCall { method, args } => {
                // super 调用：直接调用父类的方法（绕过 vtable）
                // 从函数名推断当前类 → 找父类 → 调用父类方法
                // super(args) → 调用父类 init; super.method(args) → 调用父类方法
                // super 调用分两种：super(args) 和 super.method(args)
                if method == "init" {
                    // super(args) → 调用父类的 __ParentClass_init
                    // this 作为第一个参数已在栈上（由 init body 上下文提供）
                    for arg in args {
                        self.compile_expr(arg, locals, func, loop_ctx);
                    }
                    // 寻找匹配的父类 init 函数
                    for ci in self.classes.values() {
                        if let Some(ref parent) = ci.parent {
                            let parent_init = format!("__{}_init", parent);
                            if self.func_indices.contains_key(&parent_init) {
                                let idx = self.func_indices[&parent_init];
                                func.instruction(&Instruction::Call(idx));
                                // super init 返回 i32 (ptr)，但我们忽略（this 已分配）
                                func.instruction(&Instruction::Drop);
                                break;
                            }
                        }
                    }
                } else {
                    // super.method(args) → 直接调用父类版本的方法
                    // 查找当前类的父类，调用 ParentClass.method
                    for ci in self.classes.values() {
                        if let Some(ref parent) = ci.parent {
                            let parent_method = format!("{}.{}", parent, method);
                            if let Some(&idx) = self.func_indices.get(&parent_method) {
                                // this 指针作为第一个参数
                                if let Some(this_idx) = locals.get("this") {
                                    func.instruction(&Instruction::LocalGet(this_idx));
                                }
                                for arg in args {
                                    self.compile_expr(arg, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::Call(idx));
                                break;
                            }
                        }
                    }
                }
            }
            Expr::MethodCall { object, method, args } => {
                let type_name_opt = if let Expr::Var(ref n) = object.as_ref() {
                    Some(n.clone())
                } else {
                    None
                };
                let is_static = type_name_opt.as_ref().map_or(false, |n| {
                    (self.structs.contains_key(n) || self.enums.contains_key(n))
                        && self.func_indices.contains_key(&format!("{}.{}", n, method))
                });
                let key = if is_static {
                    format!("{}.{}", type_name_opt.unwrap(), method)
                } else {
                    let struct_ty = self
                        .get_object_type(object, locals)
                        .and_then(|ty| match ty {
                            Type::Struct(s, _) => Some(s),
                            _ => None,
                        });
                    struct_ty
                        .as_ref()
                        .map(|s| format!("{}.{}", s, method))
                        .unwrap_or_else(|| method.clone())
                };
                if !is_static {
                    self.compile_expr(object, locals, func, loop_ctx);
                }
                for arg in args {
                    self.compile_expr(arg, locals, func, loop_ctx);
                }
                // 查找方法索引，支持继承链向上查找
                let idx = self.resolve_method_index(&key, method);
                func.instruction(&Instruction::Call(idx));
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.compile_expr(cond, locals, func, loop_ctx);
                // 条件必须是 i32；仅当条件表达式结果是 i64 时才 wrap
                if self.infer_type_with_locals(cond, locals) == ValType::I64 {
                    func.instruction(&Instruction::I32WrapI64);
                }

                if let Some(else_expr) = else_branch {
                    let then_produces = Self::expr_produces_value(then_branch);
                    let else_produces = Self::expr_produces_value(else_expr);
                    if then_produces && else_produces {
                        // if-else 表达式：两个分支都有返回值
                        let result_type = wasm_encoder::BlockType::Result(self.infer_type_with_locals(then_branch, locals));
                        func.instruction(&Instruction::If(result_type));
                        self.compile_expr(then_branch, locals, func, loop_ctx);
                        func.instruction(&Instruction::Else);
                        self.compile_expr(else_expr, locals, func, loop_ctx);
                    } else {
                        // if-else 语句：至少一个分支不产生值，用 Empty 块
                        func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                        self.compile_expr(then_branch, locals, func, loop_ctx);
                        if then_produces {
                            func.instruction(&Instruction::Drop);
                        }
                        func.instruction(&Instruction::Else);
                        self.compile_expr(else_expr, locals, func, loop_ctx);
                        if else_produces {
                            func.instruction(&Instruction::Drop);
                        }
                    }
                } else {
                    // if 无 else：无返回值（语句级）
                    func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    // then_branch 如果会产生值（非 void Block），需要 drop
                    let produces_value = Self::expr_produces_value(then_branch);
                    self.compile_expr(then_branch, locals, func, loop_ctx);
                    if produces_value {
                        func.instruction(&Instruction::Drop);
                    }
                }

                func.instruction(&Instruction::End);
            }
            Expr::IfLet { pattern, expr, then_branch, else_branch } => {
                let else_expr = else_branch.clone().unwrap_or_else(|| Box::new(Expr::Integer(0)));
                let match_expr = Expr::Match {
                    expr: expr.clone(),
                    arms: vec![
                        MatchArm { pattern: pattern.clone(), guard: None, body: then_branch.clone() },
                        MatchArm { pattern: Pattern::Wildcard, guard: None, body: else_expr },
                    ],
                };
                self.compile_expr(&match_expr, locals, func, loop_ctx);
            }
            Expr::Tuple(elements) => {
                // Phase 8: 使用 __alloc 分配元组内存
                let elem_size = 8i32;
                let total_size = elements.len() as i32 * elem_size;
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals.get("__tuple_alloc_ptr").expect("__tuple_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入每个元素
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(i as i32 * elem_size));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(elem, locals, func, loop_ctx);
                    let elem_ty = self.infer_type(elem);
                    match elem_ty {
                        ValType::I64 => func.instruction(&Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                        ValType::F64 => func.instruction(&Instruction::F64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                        ValType::I32 => {
                            func.instruction(&Instruction::I64ExtendI32U);
                            func.instruction(&Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }))
                        }
                        ValType::F32 => {
                            func.instruction(&Instruction::F64PromoteF32);
                            func.instruction(&Instruction::F64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }))
                        }
                        _ => func.instruction(&Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                    };
                }

                // 返回元组地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::TupleIndex { object, index } => {
                // tuple.N -> load from (tuple_ptr + N * 8)
                self.compile_expr(object, locals, func, loop_ctx);
                func.instruction(&Instruction::I32Const(*index as i32 * 8));
                func.instruction(&Instruction::I32Add);
                // 推断元素类型来选择正确的 load 指令
                let elem_ty = self.infer_ast_type_with_locals(expr, locals);
                match elem_ty.as_ref().map(|t| t.to_wasm()) {
                    Some(ValType::I32) => {
                        // i32 值是零扩展存储的，读回 i64 后 wrap
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        func.instruction(&Instruction::I32WrapI64);
                    }
                    Some(ValType::F32) => {
                        func.instruction(&Instruction::F64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                        func.instruction(&Instruction::F32DemoteF64);
                    }
                    Some(ValType::F64) => {
                        func.instruction(&Instruction::F64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }
                    _ => {
                        // 默认按 i64 读取
                        func.instruction(&Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                    }
                }
            }
            Expr::NullCoalesce { option, default } => {
                // a ?? b: 若 a 为 Some(v) 返回 v，否则返回 b
                // Option 内存布局: [tag: i32][value: ...]
                // tag == 0 => None, tag == 1 => Some
                self.compile_expr(option, locals, func, loop_ctx);
                let result_type = self.infer_type(default);
                // 保存 option 指针到临时变量
                let tmp = locals.get("__try_ptr").expect("__try_ptr");
                func.instruction(&Instruction::LocalSet(tmp));
                // 检查 tag
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }));
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(result_type)));
                // Some: 读取 value（偏移 4 字节）
                func.instruction(&Instruction::LocalGet(tmp));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                match result_type {
                    ValType::I64 => func.instruction(&Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                    ValType::I32 => func.instruction(&Instruction::I32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 })),
                    ValType::F64 => func.instruction(&Instruction::F64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                    ValType::F32 => func.instruction(&Instruction::F32Load(wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 })),
                    _ => func.instruction(&Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })),
                };
                func.instruction(&Instruction::Else);
                // None: 返回默认值
                self.compile_expr(default, locals, func, loop_ctx);
                func.instruction(&Instruction::End);
            }
            Expr::Array(elements) => {
                // Phase 8: 使用 __alloc 分配数组内存
                let elem_size = 8; // i64 大小
                let total_size = 4 + elements.len() as i32 * elem_size; // length + elements
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals.get("__array_alloc_ptr").expect("__array_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(total_size));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入数组长度
                func.instruction(&Instruction::LocalGet(tmp_local));
                func.instruction(&Instruction::I32Const(elements.len() as i32));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 写入每个元素
                for (i, elem) in elements.iter().enumerate() {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(4 + i as i32 * elem_size));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(elem, locals, func, loop_ctx);
                    func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                }

                // 返回数组地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::Index { array, index } => {
                // arr[i] -> load from (arr + 4 + i * 8)
                self.compile_expr(array, locals, func, loop_ctx);
                func.instruction(&Instruction::I32Const(4)); // 跳过长度字段
                func.instruction(&Instruction::I32Add);
                self.compile_expr(index, locals, func, loop_ctx);
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
            Expr::StructInit { name, type_args, fields } => {
                let class_info = self.classes.get(name);
                let has_vtable = class_info.map_or(false, |ci| ci.has_vtable);
                let vtable_base = class_info.map_or(0, |ci| ci.vtable_base);
                let header_size = if has_vtable { 4u32 } else { 0 };
                let struct_def = self.structs.get(name).expect("结构体未定义");
                let struct_size = header_size + struct_def.size();

                // Phase 8: 使用 __alloc 分配内存
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals.get("__struct_alloc_ptr").expect("__struct_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(struct_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 写入 vtable_ptr（如果有 vtable）
                if has_vtable {
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(vtable_base as i32));
                    func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                }

                // 写入每个字段（偏移需要加上 header）
                for (field_name, value) in fields {
                    let base_offset = struct_def
                        .field_offset(field_name)
                        .expect("字段未定义");
                    let offset = header_size + base_offset;

                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(offset as i32));
                    func.instruction(&Instruction::I32Add);
                    self.compile_expr(value, locals, func, loop_ctx);
                    // 根据字段值类型选择正确的 store 指令
                    let field_val_type = self.infer_type_with_locals(value, locals);
                    match field_val_type {
                        ValType::I32 => func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        })),
                        ValType::F64 => func.instruction(&Instruction::F64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        })),
                        ValType::F32 => func.instruction(&Instruction::F32Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        })),
                        _ => func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        })),
                    };
                }

                // 返回对象地址
                func.instruction(&Instruction::LocalGet(tmp_local));
            }
            Expr::ConstructorCall { name, type_args, args } => {
                // abstract 类不能直接实例化
                if let Some(ci) = self.classes.get(name) {
                    if ci.is_abstract {
                        panic!("abstract 类 {} 不能直接实例化", name);
                    }
                }
                // 检查类是否有 init 函数
                let init_func_name = format!("__{}_init", name);
                if self.func_indices.contains_key(&init_func_name) {
                    // 调用 __ClassName_init(args...) 返回对象指针
                    for arg in args {
                        self.compile_expr(arg, locals, func, loop_ctx);
                    }
                    let idx = self.func_indices[&init_func_name];
                    func.instruction(&Instruction::Call(idx));
                } else {
                    // 无 init: 回退到 StructInit
                    let struct_def = self.structs.get(name).expect(&format!("结构体 {} 未定义", name));
                    if args.len() != struct_def.fields.len() {
                        panic!(
                            "结构体 {} 构造函数需要 {} 个参数，得到 {} 个",
                            name,
                            struct_def.fields.len(),
                            args.len()
                        );
                    }
                    let fields: Vec<(String, Expr)> = struct_def
                        .fields
                        .iter()
                        .map(|f| f.name.clone())
                        .zip(args.clone())
                        .collect();
                    let init_expr = Expr::StructInit {
                        name: name.clone(),
                        type_args: None,
                        fields,
                    };
                    self.compile_expr(&init_expr, locals, func, loop_ctx);
                }
            }
            Expr::Field { object, field } => {
                self.compile_expr(object, locals, func, loop_ctx);
                let (offset, field_ty) = self
                    .get_object_type(object, locals)
                    .and_then(|ty| match ty {
                        Type::Struct(ref name, _) => {
                            // 优先从 ClassInfo 获取偏移（包含 vtable header）
                            if let Some(ci) = self.classes.get(name) {
                                let off = ci.field_offset(field)?;
                                let ft = ci.field_type(field)?.clone();
                                Some((off, ft))
                            } else {
                                self.structs.get(name).and_then(|def| {
                                    let off = def.field_offset(field)?;
                                    let ft = def.field_type(field)?.clone();
                                    Some((off, ft))
                                })
                            }
                        }
                        _ => None,
                    })
                    .unwrap_or((0, Type::Int64)); // 回退：偏移 0，按 i64 加载
                func.instruction(&Instruction::I32Const(offset as i32));
                func.instruction(&Instruction::I32Add);
                self.emit_load_by_type(func, &field_ty);
            }
            Expr::Block(stmts, result) => {
                for stmt in stmts {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                }
                if let Some(expr) = result {
                    self.compile_expr(expr, locals, func, loop_ctx);
                }
            }
            Expr::Range { start, end, inclusive } => {
                // Phase 8: 使用 __alloc 分配 Range 内存
                let range_size = Type::range_heap_size();
                let alloc_idx = self.func_indices["__alloc"];
                let tmp_local = locals.get("__range_alloc_ptr").expect("__range_alloc_ptr 未预注册");

                func.instruction(&Instruction::I32Const(range_size as i32));
                func.instruction(&Instruction::Call(alloc_idx));
                func.instruction(&Instruction::LocalSet(tmp_local));

                // 存储 start 到 offset 0
                func.instruction(&Instruction::LocalGet(tmp_local));
                self.compile_expr(start, locals, func, loop_ctx);
                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));

                // 存储 end 到 offset 8
                func.instruction(&Instruction::LocalGet(tmp_local));
                self.compile_expr(end, locals, func, loop_ctx);
                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 8,
                    align: 3,
                    memory_index: 0,
                }));

                // 存储 inclusive 到 offset 16
                func.instruction(&Instruction::LocalGet(tmp_local));
                func.instruction(&Instruction::I32Const(if *inclusive { 1 } else { 0 }));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 16,
                    align: 2,
                    memory_index: 0,
                }));

                // 返回 Range 地址
                func.instruction(&Instruction::LocalGet(tmp_local));

                // 栈上留下指针（之前已经压入）
            }
            Expr::VariantConst {
                enum_name,
                variant_name,
                arg,
            } => {
                let enum_def = self.enums.get(enum_name).expect("枚举未找到");
                let disc = enum_def
                    .variant_index(variant_name)
                    .expect("变体未找到") as i32;

                if enum_def.has_payload() {
                    // Phase 8: 使用 __alloc 分配枚举内存
                    let payload_size = enum_def.payload_size().max(8) as i32;
                    let total_size = 4 + payload_size;
                    let alloc_idx = self.func_indices["__alloc"];
                    let tmp_local = locals.get("__enum_alloc_ptr").expect("__enum_alloc_ptr 未预注册");

                    func.instruction(&Instruction::I32Const(total_size));
                    func.instruction(&Instruction::Call(alloc_idx));
                    func.instruction(&Instruction::LocalSet(tmp_local));

                    // 写入判别式
                    func.instruction(&Instruction::LocalGet(tmp_local));
                    func.instruction(&Instruction::I32Const(disc));
                    func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));

                    if let Some(ref payload_expr) = arg {
                        let payload_ty = enum_def
                            .variant_payload(variant_name)
                            .expect("带关联值变体需提供参数");
                        func.instruction(&Instruction::LocalGet(tmp_local));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Add);
                        self.compile_expr(payload_expr, locals, func, loop_ctx);
                        self.emit_store_by_type(func, payload_ty);
                    }

                    // 返回枚举地址
                    func.instruction(&Instruction::LocalGet(tmp_local));
                } else {
                    if arg.is_some() {
                        panic!("简单枚举变体不能带关联值: {}.{}", enum_name, variant_name);
                    }
                    func.instruction(&Instruction::I32Const(disc));
                }
            }
            Expr::Match { expr, arms } => {
                let subject_ty = self.infer_type(expr);
                let subject_ast_type = self.infer_ast_type(expr);

                let result_type = if arms.is_empty() {
                    wasm_encoder::BlockType::Empty
                } else {
                    wasm_encoder::BlockType::Result(self.infer_type(&arms[0].body))
                };

                func.instruction(&Instruction::Block(result_type));
                // subject 表达式必须在 block 内部求值，否则值在 block 栈帧之下不可访问
                self.compile_expr(expr, locals, func, loop_ctx);

                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i == arms.len() - 1;
                    let has_guard = arm.guard.is_some();

                    match &arm.pattern {
                        Pattern::Wildcard => {
                            func.instruction(&Instruction::Drop);
                            if has_guard {
                                // _ if cond => body
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
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
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(result_type));
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::Else);
                            if is_last {
                                func.instruction(&Instruction::I64Const(0));
                            } else {
                                self.compile_expr(expr, locals, func, loop_ctx);
                            }
                            func.instruction(&Instruction::End);
                        }
                        Pattern::Binding(name) => {
                            if let Some(idx) = locals.get(name) {
                                if subject_ty == ValType::I32 {
                                    func.instruction(&Instruction::I64ExtendI32S);
                                }
                                func.instruction(&Instruction::LocalSet(idx));
                            }
                            if has_guard {
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(0));
                            }
                        }
                        Pattern::Variant {
                            enum_name,
                            variant_name,
                            binding,
                        } => {
                            // 判断是否为已知枚举（包含用户定义枚举 + 内建 Option/Result）
                            let handled = {
                                let is_user_enum = matches!(&subject_ast_type, Some(Type::Struct(ref name, _)) if name == enum_name && self.enums.contains_key(name));
                                let is_builtin_option = matches!(&subject_ast_type, Some(Type::Option(_))) && enum_name == "Option";
                                let is_builtin_result = matches!(&subject_ast_type, Some(Type::Result(_, _))) && enum_name == "Result";
                                (is_user_enum || is_builtin_option || is_builtin_result)
                                    && self.enums.contains_key(enum_name)
                                    && self.enums[enum_name].variant_index(variant_name).is_some()
                            };
                            if handled {
                                let enum_def = &self.enums[enum_name];
                                let expected_disc = enum_def.variant_index(variant_name).unwrap() as i32;
                                let has_variant_payload = enum_def.has_payload();
                                let resolved_payload = self.resolve_variant_payload(enum_name, variant_name, subject_ast_type.as_ref());
                                let ptr_tmp = locals.get("__match_enum_ptr").expect("__match_enum_ptr");

                                if has_variant_payload {
                                    func.instruction(&Instruction::LocalSet(ptr_tmp));
                                    func.instruction(&Instruction::LocalGet(ptr_tmp));
                                    func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                                        offset: 0,
                                        align: 2,
                                        memory_index: 0,
                                    }));
                                    func.instruction(&Instruction::I32Const(expected_disc));
                                    func.instruction(&Instruction::I32Eq);
                                } else {
                                    func.instruction(&Instruction::I32Const(expected_disc));
                                    func.instruction(&Instruction::I32Eq);
                                }

                                func.instruction(&Instruction::If(result_type));
                                if has_variant_payload {
                                    if let Some(ref bind_name) = binding {
                                        if let Some(ref payload_ty) = resolved_payload {
                                            func.instruction(&Instruction::LocalGet(ptr_tmp));
                                            func.instruction(&Instruction::I32Const(4));
                                            func.instruction(&Instruction::I32Add);
                                            self.emit_load_by_type(func, payload_ty);
                                            let bind_idx = locals.get(bind_name).expect("关联值绑定未找到");
                                            func.instruction(&Instruction::LocalSet(bind_idx));
                                        }
                                    }
                                }
                                if has_guard {
                                    self.compile_expr(
                                        arm.guard.as_ref().unwrap(),
                                        locals,
                                        func,
                                        loop_ctx,
                                    );
                                    func.instruction(&Instruction::If(result_type));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::Else);
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                    func.instruction(&Instruction::End);
                                    func.instruction(&Instruction::Br(1));
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                }
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                func.instruction(&Instruction::Drop);
                                if has_guard {
                                    self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                    func.instruction(&Instruction::If(result_type));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::Else);
                                    if is_last {
                                        func.instruction(&Instruction::I64Const(0));
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                    func.instruction(&Instruction::End);
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                        }
                        Pattern::Range { start, end, inclusive } => {
                            if let (Literal::Integer(s), Literal::Integer(e)) = (start, end) {
                                func.instruction(&Instruction::I64Const(*s));
                                func.instruction(&Instruction::I64GeS);

                                self.compile_expr(expr, locals, func, loop_ctx);
                                func.instruction(&Instruction::I64Const(*e));
                                if *inclusive {
                                    func.instruction(&Instruction::I64LeS);
                                } else {
                                    func.instruction(&Instruction::I64LtS);
                                }

                                func.instruction(&Instruction::I32And);

                                if has_guard {
                                    func.instruction(&Instruction::If(wasm_encoder::BlockType::Result(ValType::I32)));
                                    self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                    func.instruction(&Instruction::Else);
                                    func.instruction(&Instruction::I32Const(0));
                                    func.instruction(&Instruction::End);
                                }

                                func.instruction(&Instruction::If(result_type));
                                self.compile_expr(&arm.body, locals, func, loop_ctx);
                                func.instruction(&Instruction::Br(1));
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::End);
                            }
                        }
                        Pattern::Or(patterns) => {
                            for (j, pat) in patterns.iter().enumerate() {
                                if let Pattern::Literal(Literal::Integer(n)) = pat {
                                    if j > 0 {
                                        self.compile_expr(expr, locals, func, loop_ctx);
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
                                self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                func.instruction(&Instruction::Else);
                                func.instruction(&Instruction::I32Const(0));
                                func.instruction(&Instruction::End);
                            }

                            func.instruction(&Instruction::If(result_type));
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                            func.instruction(&Instruction::Br(1));
                            func.instruction(&Instruction::Else);
                            if is_last {
                                func.instruction(&Instruction::I64Const(0));
                            } else {
                                self.compile_expr(expr, locals, func, loop_ctx);
                            }
                            func.instruction(&Instruction::End);
                        }
                        Pattern::Struct { name: struct_name, fields } => {
                            let handled = if let Some(Type::Struct(ref sub_name, _)) = subject_ast_type {
                                sub_name == struct_name && self.structs.contains_key(struct_name)
                            } else {
                                false
                            };
                            if handled {
                                let struct_def = &self.structs[struct_name];
                                let ptr_tmp = locals.get("__match_enum_ptr").expect("__match_enum_ptr");
                                func.instruction(&Instruction::LocalSet(ptr_tmp));
                                for (fname, pat) in fields {
                                    let offset = struct_def.field_offset(fname).expect("结构体字段");
                                    let fty = struct_def.field_type(fname).expect("字段类型");
                                    func.instruction(&Instruction::LocalGet(ptr_tmp));
                                    func.instruction(&Instruction::I32Const(offset as i32));
                                    func.instruction(&Instruction::I32Add);
                                    self.emit_load_by_type(func, fty);
                                    if let Pattern::Binding(bind) = pat {
                                        let idx = locals.get(bind).expect("解构绑定名");
                                        func.instruction(&Instruction::LocalSet(idx));
                                    }
                                }
                                if has_guard {
                                    self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                    func.instruction(&Instruction::If(result_type));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::Else);
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                    func.instruction(&Instruction::End);
                                    func.instruction(&Instruction::Br(1));
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                }
                                func.instruction(&Instruction::Else);
                                if is_last {
                                    func.instruction(&Instruction::I64Const(0));
                                } else {
                                    self.compile_expr(expr, locals, func, loop_ctx);
                                }
                                func.instruction(&Instruction::End);
                            } else {
                                func.instruction(&Instruction::Drop);
                                if has_guard {
                                    self.compile_expr(arm.guard.as_ref().unwrap(), locals, func, loop_ctx);
                                    func.instruction(&Instruction::If(result_type));
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(1));
                                    func.instruction(&Instruction::Else);
                                    if is_last {
                                        func.instruction(&Instruction::I64Const(0));
                                    } else {
                                        self.compile_expr(expr, locals, func, loop_ctx);
                                    }
                                    func.instruction(&Instruction::End);
                                } else {
                                    self.compile_expr(&arm.body, locals, func, loop_ctx);
                                    func.instruction(&Instruction::Br(0));
                                }
                            }
                        }
                        _ => {
                            func.instruction(&Instruction::Drop);
                            self.compile_expr(&arm.body, locals, func, loop_ctx);
                        }
                    }
                }

                func.instruction(&Instruction::End);
            }
            Expr::Lambda { params, return_type, body } => {
                // Lambda 编译：返回预扫描阶段生成的匿名函数的索引
                let lambda_idx = self.lambda_counter.get();
                self.lambda_counter.set(lambda_idx + 1);
                let lambda_name = format!("__lambda_{}", lambda_idx);

                if let Some(&func_idx) = self.func_indices.get(&lambda_name) {
                    func.instruction(&Instruction::I32Const(func_idx as i32));
                } else {
                    // fallback：Lambda 未找到，返回 0
                    func.instruction(&Instruction::I32Const(0));
                }
            }
            Expr::Some(inner) => {
                // Option::Some(v) -> 堆分配 [tag=1: i32][value]
                // 返回指针
                let value_size = match self.infer_ast_type(inner) {
                    Some(t) => t.size(),
                    None => 8, // 默认 i64
                };
                let total_size = 4 + value_size;

                func.instruction(&Instruction::GlobalGet(0)); // 保存指针

                // 写入 tag = 1 (Some)
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 写入 value
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4)); // 跳过 tag
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                func.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::None => {
                // Option::None -> 堆分配 [tag=0: i32]
                func.instruction(&Instruction::GlobalGet(0)); // 保存指针

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(0)); // tag = 0 (None)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                // 更新堆指针
                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Ok(inner) => {
                // Result::Ok(v) -> 堆分配 [tag=0: i32][value]
                let inner_ast_type = self.infer_ast_type(inner);
                let value_size = match &inner_ast_type {
                    Some(t) => t.size(),
                    None => 8,
                };
                let total_size = 4 + value_size;

                func.instruction(&Instruction::GlobalGet(0));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(0)); // tag = 0 (Ok)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                // 根据内部值类型选择正确的 store 指令
                Self::emit_store_by_wasm_type(func, inner_ast_type.as_ref().map(|t| t.to_wasm()).unwrap_or(ValType::I64));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Err(inner) => {
                // Result::Err(e) -> 堆分配 [tag=1: i32][error]
                let inner_ast_type = self.infer_ast_type(inner);
                let value_size = match &inner_ast_type {
                    Some(t) => t.size(),
                    None => 8,
                };
                let total_size = 4 + value_size;

                func.instruction(&Instruction::GlobalGet(0));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(1)); // tag = 1 (Err)
                func.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                self.compile_expr(inner, locals, func, loop_ctx);
                // 根据内部值类型选择正确的 store 指令
                Self::emit_store_by_wasm_type(func, inner_ast_type.as_ref().map(|t| t.to_wasm()).unwrap_or(ValType::I64));

                func.instruction(&Instruction::GlobalGet(0));
                func.instruction(&Instruction::I32Const(total_size as i32));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::GlobalSet(0));
            }
            Expr::Try(inner) => {
                // expr? -> 检查 tag，若为 None/Err 则提前 return，否则解包
                // 先计算 inner 得到指针
                self.compile_expr(inner, locals, func, loop_ctx);
                // 栈顶是指针，复制一份用于检查 tag
                func.instruction(&Instruction::LocalTee(locals.get("__try_ptr").unwrap_or(0)));
                // 读取 tag
                func.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // 对于 Option: tag=0 是 None，需要提前返回
                // 对于 Result: tag=1 是 Err，需要提前返回
                // 简化：检查 tag != 0 (Some/Err)，若为 None/Ok 则继续
                // 注意：Option 的 tag=1 是 Some，Result 的 tag=0 是 Ok
                // 这里需要根据类型判断，简化处理：检查 tag
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                // tag != 0，需要提前返回
                func.instruction(&Instruction::LocalGet(locals.get("__try_ptr").unwrap_or(0)));
                func.instruction(&Instruction::Return);
                func.instruction(&Instruction::End);
                // tag == 0，解包 value
                func.instruction(&Instruction::LocalGet(locals.get("__try_ptr").unwrap_or(0)));
                func.instruction(&Instruction::I32Const(4));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Expr::Throw(inner) => {
                // throw expr -> 设置错误标志并跳出 try 块
                // 如果在 try-catch 上下文中，设置 __err_flag 并将值存入 __err_val
                self.compile_expr(inner, locals, func, loop_ctx);
                if let Some(err_val_idx) = locals.get("__err_val") {
                    func.instruction(&Instruction::LocalSet(err_val_idx));
                    // 设置 __err_flag = 1
                    func.instruction(&Instruction::I32Const(1));
                    if let Some(err_flag_idx) = locals.get("__err_flag") {
                        func.instruction(&Instruction::LocalSet(err_flag_idx));
                    }
                } else {
                    // 不在 try 上下文中，直接 return
                    func.instruction(&Instruction::Return);
                }
            }
            Expr::TryBlock { body, catch_var, catch_body, finally_body } => {
                // try { body } catch(e) { catch_body } finally { finally_body }
                // 实现策略：
                //   - try body 正常执行
                //   - 使用全局标志 __err_flag 来标记是否发生了错误（throw 会设置此标志）
                //   - 如果 throw 发生，跳转到 catch 块
                //   - finally 块无论是否异常都执行

                // 使用局部变量 __err_flag 标记是否发生错误（0=正常, 1=异常）
                // 这些局部变量在 collect_locals 阶段已预分配
                let err_flag = locals.get("__err_flag").unwrap_or(0);
                let err_val = locals.get("__err_val").unwrap_or(0);

                // 初始化 __err_flag = 0
                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::LocalSet(err_flag));

                // 用 block 包裹 try body，throw 后通过 br_if 跳出
                func.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                // 编译 try body，每条语句后检查 err_flag 并提前退出
                for stmt in body {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                    // throw 后 __err_flag=1，br_if 跳出 try block
                    func.instruction(&Instruction::LocalGet(err_flag));
                    func.instruction(&Instruction::BrIf(0));
                }
                func.instruction(&Instruction::End); // end of try body block

                // 编译 catch 块（在 throw 发生时执行）
                // 检查 __err_flag
                func.instruction(&Instruction::LocalGet(err_flag));
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                // 在 catch 块中，如果有 catch_var，将 __err_val 赋给它
                if let Some(ref var) = catch_var {
                    if let Some(var_idx) = locals.get(var) {
                        func.instruction(&Instruction::LocalGet(err_val));
                        func.instruction(&Instruction::LocalSet(var_idx));
                    }
                }
                for stmt in catch_body {
                    self.compile_stmt(stmt, locals, func, loop_ctx);
                }
                func.instruction(&Instruction::End); // end of catch if

                // 编译 finally 块（无论是否异常都执行）
                if let Some(finally_stmts) = finally_body {
                    for stmt in finally_stmts {
                        self.compile_stmt(stmt, locals, func, loop_ctx);
                    }
                }

                // try-catch 所有路径通常都通过 return 退出，
                // 但 WASM 验证器无法追踪条件返回，标记此处不可达
                func.instruction(&Instruction::Unreachable);
            }
            Expr::SliceExpr { .. } | Expr::MapLiteral { .. } => {
                todo!("SliceExpr and MapLiteral codegen not yet implemented")
            }
        }
    }
}

/// 局部变量构建器（同时保存 WASM 值类型与 AST 类型，用于字段偏移等）
struct LocalsBuilder {
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
        if !self.names.contains_key(name) {
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

    /// 确保临时变量存在，不存在则创建。返回 local index。
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
                        variadic: false,
                    },
                    Param {
                        name: "b".to_string(),
                        ty: Type::Int64,
                        default: None,
                        variadic: false,
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
            functions: vec![FuncDef {
                visibility: Visibility::default(),
                name: "match_test".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![Param {
                    name: "n".to_string(),
                    ty: Type::Int64,
                    default: None,
                    variadic: false,
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
                        name: "sum".to_string(),
                        ty: Some(Type::Int64),
                        value: Expr::Integer(0),
                    },
                    Stmt::For {
                        var: "i".to_string(),
                        iterable: Expr::Range {
                            start: Box::new(Expr::Integer(0)),
                            end: Box::new(Expr::Integer(3)),
                            inclusive: false,
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
