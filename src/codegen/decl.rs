//! 声明代码生成：Program/StructDef/ClassDef/InterfaceDef/EnumDef 到 WASM 的声明、vtable、init/deinit、字符串收集等。

use crate::ast::{ClassDef, Expr, FieldDef, InitDef, InterpolatePart, Param, Program, Stmt, StructDef, Type, Visibility};
use crate::ast::Function as FuncDef;
use std::collections::HashMap;

use super::CodeGen;

/// 类的运行时信息（包含继承布局和 vtable）
#[derive(Debug, Clone)]
pub(crate) struct ClassInfo {
    /// 类名
    pub name: String,
    /// 唯一类 ID（用于 `is` 类型检查）
    pub class_id: u32,
    /// 父类名
    pub parent: Option<String>,
    /// 完整字段列表（先父类后子类），不含 vtable_ptr
    pub all_fields: Vec<FieldDef>,
    /// 自身字段列表（不含继承的）
    pub own_fields: Vec<FieldDef>,
    /// vtable 方法名列表（按槽位顺序）
    pub vtable_methods: Vec<String>,
    /// 方法名 → vtable 槽位索引
    pub vtable_slot: HashMap<String, usize>,
    /// 该类 vtable 在 WASM Table 中的起始索引
    pub vtable_base: u32,
    /// 对象是否需要 vtable_ptr（有继承或被继承时为 true）
    pub has_vtable: bool,
    /// 是否是 abstract 类
    pub is_abstract: bool,
    /// 是否是 sealed 类
    pub is_sealed: bool,
    /// init 定义
    pub init: Option<InitDef>,
    /// deinit body
    pub deinit: Option<Vec<Stmt>>,
    /// 原始 ClassDef 引用的方法列表
    pub methods: Vec<(String, bool)>, // (fully_qualified_name, is_override)
}

impl ClassInfo {
    /// 对象总大小（包含 vtable_ptr + 所有字段）
    pub fn object_size(&self) -> u32 {
        let header = if self.has_vtable { 4 } else { 0 }; // vtable_ptr: i32
        header + self.all_fields.iter().map(|f| f.ty.size()).sum::<u32>()
    }

    /// 字段偏移（已加上 vtable_ptr 的 4 字节头部）
    pub fn field_offset(&self, field_name: &str) -> Option<u32> {
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
    pub fn field_type(&self, field_name: &str) -> Option<&Type> {
        self.all_fields.iter().find(|f| f.name == field_name).map(|f| &f.ty)
    }
}

impl CodeGen {
    /// 解析方法索引，支持继承链向上查找
    /// key 格式为 "ClassName.methodName"，如果找不到，沿继承链向上查找
    pub(crate) fn resolve_method_index(&self, key: &str, method: &str) -> u32 {
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

    /// 注册所有类，构建 ClassInfo（含继承字段布局和 vtable）
    pub(crate) fn register_classes(&mut self, class_defs: &[ClassDef]) {
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

            let cid = self.next_class_id;
            self.next_class_id += 1;
            let info = ClassInfo {
                name: c.name.clone(),
                class_id: cid,
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
    pub(crate) fn build_vtables(&mut self) {
        let mut entries = Vec::new();
        let class_names: Vec<String> = self.classes.keys().cloned().collect();
        for name in &class_names {
            let info = self.classes.get(name).unwrap();
            if !info.has_vtable || info.vtable_methods.is_empty() {
                continue;
            }
            let base = entries.len() as u32;
            for method_fqn in &info.vtable_methods {
                let func_idx = self.find_method_index(method_fqn)
                    .unwrap_or_else(|| {
                        eprintln!("错误: vtable 方法 '{}' 未找到函数索引", method_fqn);
                        eprintln!("搜索包含 '{}' 的函数:", method_fqn.split('.').next().unwrap_or(""));
                        let search_term = method_fqn.split('.').next().unwrap_or("");
                        for (key, idx) in self.func_indices.iter() {
                            if key.contains(search_term) {
                                eprintln!("  {} -> {}", key, idx);
                            }
                        }
                        panic!("vtable 方法 {} 未找到函数索引", method_fqn)
                    });
                entries.push(func_idx);
            }
            // 更新 vtable_base
            let info = self.classes.get_mut(name).unwrap();
            info.vtable_base = base;
        }
        self.vtable_entries = entries;
    }

    /// 查找方法索引，支持多种命名格式
    fn find_method_index(&self, method_fqn: &str) -> Option<u32> {
        // 1. 尝试精确匹配
        if let Some(&idx) = self.func_indices.get(method_fqn) {
            return Some(idx);
        }

        // 2. 尝试其他命名格式
        let candidates = vec![
            method_fqn.replace('.', "::"),                    // "Scope::lookup"
            method_fqn.replace('.', "_"),                     // "Scope_lookup"
        ];

        for candidate in candidates {
            if let Some(&idx) = self.func_indices.get(&candidate) {
                return Some(idx);
            }
        }

        // 3. 尝试前缀匹配（处理单态化后的函数名，如 Scope.lookup$Scope$Identifier）
        let prefix = format!("{}$", method_fqn);
        for (key, &idx) in self.func_indices.iter() {
            if key.starts_with(&prefix) {
                return Some(idx);
            }
        }

        None
    }

    /// 构建 init 函数：__ClassName_init(params...) -> i32 (对象指针)
    pub(crate) fn build_init_function(&self, class: &ClassDef, init_def: &InitDef) -> FuncDef {
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

    /// Bug B3 修复: 构建 init body 函数 __ClassName_init_body(this, params...) -> Unit
    /// 不分配内存，只执行 init body（用于 super() 调用）
    pub(crate) fn build_init_body_function(&self, class: &ClassDef, init_def: &InitDef) -> FuncDef {
        let class_name = &class.name;
        let func_name = format!("__{}_init_body", class_name);

        // 第一个参数是 this: ClassName
        let mut params = vec![Param {
            name: "this".to_string(),
            ty: Type::Struct(class_name.clone(), vec![]),
            default: None,
            variadic: false, is_named: false, is_inout: false,
        }];
        params.extend(init_def.params.iter().cloned());

        FuncDef {
            visibility: Visibility::Public,
            name: func_name,
            type_params: vec![],
            constraints: vec![],
            params,
            return_type: None, // 无返回值
            throws: None,
            body: init_def.body.clone(),
            extern_import: None,
        }
    }

    /// 构建 deinit 函数：__ClassName_deinit(this: i32) -> Unit
    pub(crate) fn build_deinit_function(&self, class: &ClassDef, deinit_body: &[Stmt]) -> FuncDef {
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
                variadic: false, is_named: false, is_inout: false,
            }],
            return_type: None,
            throws: None,
            body: deinit_body.to_vec(),
            extern_import: None,
        }
    }

    /// 收集所有字符串常量
    pub(crate) fn collect_strings(&mut self, program: &Program) {
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

    pub(crate) fn collect_strings_in_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { value, .. } => {
                self.collect_strings_in_expr(value);
            }
            Stmt::Var { value: Some(value), .. } => {
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
            Stmt::DoWhile { body, cond } => {
                for s in body {
                    self.collect_strings_in_stmt(s);
                }
                self.collect_strings_in_expr(cond);
            }
            Stmt::Loop { body } => {
                for s in body {
                    self.collect_strings_in_stmt(s);
                }
            }
            Stmt::UnsafeBlock { body } => {
                for s in body {
                    self.collect_strings_in_stmt(s);
                }
            }
            Stmt::Const { value, .. } => {
                self.collect_strings_in_expr(value);
            }
            _ => {}
        }
    }

    pub(crate) fn collect_strings_in_expr(&mut self, expr: &Expr) {
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
            Expr::SliceExpr { array, start, end } => {
                self.collect_strings_in_expr(array);
                self.collect_strings_in_expr(start);
                self.collect_strings_in_expr(end);
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
            Expr::Cast { expr, .. } | Expr::IsType { expr, .. } => {
                self.collect_strings_in_expr(expr);
            }
            Expr::PostfixIncr(inner) | Expr::PostfixDecr(inner) => {
                self.collect_strings_in_expr(inner);
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
}
