//! WasmGC 类型映射（G1：叶子类 → GC struct）
//!
//! 本模块是 lowering 与 codegen 共享的单一真源：只有登记进 [`GcTypeMap`]
//! 的类类型才会被映射为 `(ref $T)`，其余类型一律保持原有线性内存表示
//! （`Type::to_wasm()`）。这保证未迁移类型（String/集合/继承类等）行为不变。
//!
//! 类型索引分配：WASI 导入占用类型索引 0..3，GC rec group 紧随其后，从
//! [`GC_TYPE_BASE`] (= 4) 开始按 [`crate::ast::Program::classes`] 的声明顺序
//! 连续分配。lowering 与 codegen 用同一顺序构建映射，因此两端的索引一致。

use std::collections::{HashMap, HashSet};

use wasm_encoder::{HeapType, RefType, ValType};

use crate::ast::{ClassDef, Program, Type};

/// GC struct 类型索引基址：4 个 WASI 导入函数类型占据 0..3。
pub const GC_TYPE_BASE: u32 = 4;

/// G1：`--gc` 开关（沿用 `--checked` → `CJWASM_CHECKED` 的模式）。
pub fn gc_mode() -> bool {
    std::env::var("CJWASM_GC").is_ok()
}

/// 单个 GC struct 字段。
#[derive(Debug, Clone)]
pub struct GcField {
    pub name: String,
    /// 线性布局下的字节偏移（沿用现有 lowering 的字段偏移语义）
    pub offset: u32,
    pub ty: Type,
}

/// 单个叶子类/结构体的 GC 布局。
#[derive(Debug, Clone)]
pub struct GcStructLayout {
    pub type_index: u32,
    /// 按字节偏移升序（= 声明顺序）排列
    pub fields: Vec<GcField>,
    pub super_type: Option<String>,
}

/// 叶子类 → GC struct 类型表。
#[derive(Debug, Clone, Default)]
pub struct GcTypeMap {
    pub enabled: bool,
    pub structs: HashMap<String, GcStructLayout>,
}

/// 字段类型是否为可作 GC struct 字段的值类型（G1 只迁移值字段）。
fn is_value_field_ty(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int8
            | Type::Int16
            | Type::Int32
            | Type::Int64
            | Type::IntNative
            | Type::UInt8
            | Type::UInt16
            | Type::UInt32
            | Type::UInt64
            | Type::UIntNative
            | Type::Float16
            | Type::Float32
            | Type::Float64
            | Type::Rune
            | Type::Bool
    )
}

/// 收集「出现在非值容器类型内」的类名（Array/Option/Result/Tuple/...）。
/// G1 只迁移纯值字段叶子类；这些容器仍为线性表示，其中若承载类引用会违反
/// 「ref 不得进线性内存」，故对应类必须回退线性路径。
fn collect_container_structs(ty: &Type, out: &mut HashSet<String>) {
    match ty {
        Type::Struct(name, _) => {
            out.insert(name.clone());
        }
        Type::Array(t) | Type::VArray(t, _) | Type::Option(t) | Type::Slice(t) => {
            collect_container_structs(t, out)
        }
        Type::Result(a, b) | Type::Map(a, b) => {
            collect_container_structs(a, out);
            collect_container_structs(b, out);
        }
        Type::Tuple(ts) => {
            for t in ts {
                collect_container_structs(t, out);
            }
        }
        Type::Function { params, ret } => {
            for p in params {
                collect_container_structs(p, out);
            }
            if let Some(r) = ret.as_ref() {
                collect_container_structs(r, out);
            }
        }
        _ => {}
    }
}

/// 顶层类型注解：裸 `Struct(name)` 安全；容器内的类引用标记为 unsafe。
fn mark_unsafe_ty(ty: &Type, out: &mut HashSet<String>) {
    match ty {
        Type::Struct(_, args) if !args.is_empty() => {
            // 泛型实参中的类同样视为容器内引用
            for a in args {
                collect_container_structs(a, out);
            }
        }
        Type::Struct(..) => {}
        _ => collect_container_structs(ty, out),
    }
}

fn walk_stmt_types(stmts: &[crate::ast::Stmt], out: &mut HashSet<String>) {
    use crate::ast::Stmt;
    for st in stmts {
        match st {
            Stmt::Let { ty, .. } | Stmt::Var { ty, .. } | Stmt::Const { ty, .. } => {
                if let Some(t) = ty {
                    mark_unsafe_ty(t, out);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Loop { body }
            | Stmt::UnsafeBlock { body } => walk_stmt_types(body, out),
            Stmt::WhileLet { body, .. } => walk_stmt_types(body, out),
            Stmt::LocalFunc(f) => {
                for p in &f.params {
                    mark_unsafe_ty(&p.ty, out);
                }
                if let Some(r) = &f.return_type {
                    mark_unsafe_ty(r, out);
                }
                walk_stmt_types(&f.body, out);
            }
            _ => {}
        }
    }
}

/// 与 `monomorph::type_mangle_suffix` 对齐的类型后缀（用于把泛型实参类型
/// 还原为单态化后的类名，如 `Stack<Int64>` → `Stack$Int64`）。
fn type_mangle_suffix(ty: &Type) -> String {
    match ty {
        Type::Int8 => "Int8".into(),
        Type::Int16 => "Int16".into(),
        Type::Int32 => "Int32".into(),
        Type::Int64 => "Int64".into(),
        Type::IntNative => "IntNative".into(),
        Type::UInt8 => "UInt8".into(),
        Type::UInt16 => "UInt16".into(),
        Type::UInt32 => "UInt32".into(),
        Type::UInt64 => "UInt64".into(),
        Type::UIntNative => "UIntNative".into(),
        Type::Float16 => "Float16".into(),
        Type::Float32 => "Float32".into(),
        Type::Float64 => "Float64".into(),
        Type::Rune => "Rune".into(),
        Type::Bool => "Bool".into(),
        Type::Unit => "Unit".into(),
        Type::Nothing => "Nothing".into(),
        Type::String => "String".into(),
        Type::Struct(n, args) if args.is_empty() => n.clone(),
        Type::Struct(n, args) => format!(
            "{}_{}",
            n,
            args.iter().map(type_mangle_suffix).collect::<Vec<_>>().join("_")
        ),
        Type::Array(t) => format!("Array_{}", type_mangle_suffix(t)),
        Type::Option(t) => format!("Option_{}", type_mangle_suffix(t)),
        _ => "Any".into(),
    }
}

impl GcTypeMap {
    /// 按 `--gc` 开关与类定义构建映射。未开启 `--gc` 时返回空表。
    pub fn build(program: &Program) -> Self {
        let enabled = gc_mode();
        let mut unsafe_names: HashSet<String> = HashSet::new();
        // 函数参数/返回值
        for f in &program.functions {
            for p in &f.params {
                mark_unsafe_ty(&p.ty, &mut unsafe_names);
            }
            if let Some(r) = &f.return_type {
                mark_unsafe_ty(r, &mut unsafe_names);
            }
            walk_stmt_types(&f.body, &mut unsafe_names);
        }
        // 类/结构体字段与构造函数参数
        for c in &program.classes {
            for fld in &c.fields {
                mark_unsafe_ty(&fld.ty, &mut unsafe_names);
            }
            for init in &c.inits {
                for p in &init.params {
                    mark_unsafe_ty(&p.ty, &mut unsafe_names);
                }
                walk_stmt_types(&init.body, &mut unsafe_names);
            }
            for m in &c.methods {
                for p in &m.func.params {
                    mark_unsafe_ty(&p.ty, &mut unsafe_names);
                }
                if let Some(r) = &m.func.return_type {
                    mark_unsafe_ty(r, &mut unsafe_names);
                }
                walk_stmt_types(&m.func.body, &mut unsafe_names);
            }
        }
        for sd in &program.structs {
            for fld in &sd.fields {
                mark_unsafe_ty(&fld.ty, &mut unsafe_names);
            }
            for init in &sd.inits {
                for p in &init.params {
                    mark_unsafe_ty(&p.ty, &mut unsafe_names);
                }
                walk_stmt_types(&init.body, &mut unsafe_names);
            }
        }
        Self::build_from_classes_with(&program.classes, enabled, &unsafe_names)
    }

    /// 从类定义构建映射（不做容器分析；仅供测试/无 unsafe 信息时使用）。
    pub fn build_from_classes(classes: &[ClassDef], enabled: bool) -> Self {
        Self::build_from_classes_with(classes, enabled, &HashSet::new())
    }

    fn build_from_classes_with(
        classes: &[ClassDef],
        enabled: bool,
        unsafe_names: &HashSet<String>,
    ) -> Self {
        let mut map = GcTypeMap {
            enabled,
            structs: HashMap::new(),
        };
        if !enabled {
            return map;
        }
        // 有子类的类不可迁移（继承留待 G5）
        let mut has_children: HashSet<String> = HashSet::new();
        for cd in classes {
            if let Some(ref parent) = cd.extends {
                has_children.insert(parent.clone());
            }
        }
        let mut next_index = GC_TYPE_BASE;
        for cd in classes {
            // G1 迁移条件：叶子类（无 extends / 无子类 / 无接口）、非泛型、非抽象、
            // 且字段均为值类型。
            if cd.extends.is_some()
                || has_children.contains(&cd.name)
                || !cd.implements.is_empty()
                || !cd.type_params.is_empty()
                || cd.is_abstract
                || unsafe_names.contains(&cd.name)
            {
                continue;
            }
            if !cd.fields.iter().all(|f| is_value_field_ty(&f.ty)) {
                continue;
            }
            // 声明顺序累加字节偏移（叶子类无 vtable，与 lower.rs 的类布局一致）
            let mut offset = 0u32;
            let mut fields: Vec<GcField> = Vec::with_capacity(cd.fields.len());
            for f in &cd.fields {
                fields.push(GcField {
                    name: f.name.clone(),
                    offset,
                    ty: f.ty.clone(),
                });
                offset += f.ty.size();
            }
            if std::env::var("CJWASM_GC_DEBUG").is_ok() {
                eprintln!("[gc] register {} idx={} fields={}", cd.name, next_index, fields.len());
            }
            map.structs.insert(
                cd.name.clone(),
                GcStructLayout {
                    type_index: next_index,
                    fields,
                    super_type: None,
                },
            );
            next_index += 1;
        }
        map
    }

    /// 该 AST 类型是否映射为 GC typed ref；是则返回其 struct 类型索引。
    pub fn type_of(&self, ty: &Type) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        self.resolve_struct_name(ty)
            .and_then(|n| self.structs.get(n))
            .map(|l| l.type_index)
    }

    /// 解析 AST 类型到映射表中的类名：先按原名，再按单态化名（`Name$T1$T2`）。
    pub fn resolve_struct_name(&self, ty: &Type) -> Option<&str> {
        let Type::Struct(name, args) = ty else { return None };
        if let Some((k, _)) = self.structs.get_key_value(name) {
            return Some(k.as_str());
        }
        if !args.is_empty() {
            let cand = format!(
                "{}${}",
                name,
                args.iter().map(type_mangle_suffix).collect::<Vec<_>>().join("$")
            );
            if let Some((k, _)) = self.structs.get_key_value(&cand) {
                return Some(k.as_str());
            }
        }
        None
    }

    /// 类型映射：登记类型 → 非空 typed ref；其余沿用 `to_wasm()`。
    pub fn gc_wasm_ty(&self, ty: &Type) -> ValType {
        match ty {
            Type::Unit | Type::Nothing => ValType::I32,
            _ => match self.type_of(ty) {
                Some(idx) => ValType::Ref(RefType {
                    nullable: false,
                    heap_type: HeapType::Concrete(idx),
                }),
                None => ty.to_wasm(),
            },
        }
    }

    pub fn layout(&self, name: &str) -> Option<&GcStructLayout> {
        self.structs.get(name)
    }

    /// 按字节偏移反查字段下标。
    pub fn field_index(&self, class: &str, offset: u32) -> Option<u32> {
        self.structs
            .get(class)?
            .fields
            .iter()
            .position(|f| f.offset == offset)
            .map(|i| i as u32)
    }

    /// 从 AST 类型取类/结构体名（仅 `Struct(name)`，不做单态化解析）。
    pub fn struct_name_of(ty: &Type) -> Option<&str> {
        match ty {
            Type::Struct(name, _) => Some(name.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FieldDef, Visibility};
    use wasm_encoder::HeapType;

    fn field(name: &str, _offset: u32, ty: Type) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            default: None,
            is_mut: true,
            is_static: false,
            decl_kw: true,
            is_const: false,
            visibility: Visibility::Private,
        }
    }

    fn leaf(name: &str, fields: Vec<FieldDef>) -> ClassDef {
        ClassDef {
            visibility: Visibility::Public,
            name: name.to_string(),
            type_params: vec![],
            constraints: vec![],
            is_abstract: false,
            is_sealed: false,
            is_open: false,
            has_decl_attr: false,
            extends: None,
            implements: vec![],
            supertype_args: vec![],
            fields,
            init: None,
            inits: vec![],
            deinit: None,
            static_init: None,
            methods: vec![],
            primary_ctor_params: vec![],
            primary_ctor_member_flags: vec![],
        }
    }

    #[test]
    fn registers_leaf_value_class() {
        let classes = vec![leaf(
            "Cell",
            vec![field("a", 0, Type::Int64), field("b", 8, Type::Int64)],
        )];
        let map = GcTypeMap::build_from_classes(&classes, true);
        assert_eq!(map.type_of(&Type::Struct("Cell".into(), vec![])), Some(4));
        let ref_ty = map.gc_wasm_ty(&Type::Struct("Cell".into(), vec![]));
        assert_eq!(
            ref_ty,
            ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(4)
            })
        );
        assert_eq!(map.field_index("Cell", 8), Some(1));
    }

    #[test]
    fn skips_non_value_and_inherited_classes() {
        let mut base = leaf("Base", vec![field("x", 0, Type::Int64)]);
        let mut child = leaf("Child", vec![field("y", 0, Type::Int64)]);
        child.extends = Some("Base".into());
        let stringy = leaf("S", vec![field("s", 0, Type::String)]);
        base.is_open = true;
        let classes = vec![base, child, stringy];
        let map = GcTypeMap::build_from_classes(&classes, true);
        assert!(map.structs.is_empty());
        assert_eq!(map.gc_wasm_ty(&Type::Struct("Child".into(), vec![])), ValType::I32);
    }

    #[test]
    fn disabled_when_gc_off() {
        let classes = vec![leaf("Cell", vec![field("a", 0, Type::Int64)])];
        let map = GcTypeMap::build_from_classes(&classes, false);
        assert!(map.structs.is_empty());
        assert_eq!(map.gc_wasm_ty(&Type::Struct("Cell".into(), vec![])), ValType::I32);
    }
}
