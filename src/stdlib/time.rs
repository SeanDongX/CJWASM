//! std.time 标准库实现
//!
//! 提供 CJson 等库依赖的 DateTime 类型（最小实现：now/parse/toString/addYears）。

use crate::ast::*;

/// std.time 标准库的 AST 产出（仅 DateTime）
pub struct TimeStdlib {
    pub classes: Vec<ClassDef>,
    pub functions: Vec<Function>,
}

/// 生成 std.time 标准库的 AST 定义
pub fn generate_time_stdlib() -> TimeStdlib {
    TimeStdlib {
        classes: vec![generate_datetime_class()],
        functions: vec![],
    }
}

/// class DateTime（最小：用于 CJson 的 time 字段）
/// 提供 static now(), static parse(s), toString(), addYears(n)
fn generate_datetime_class() -> ClassDef {
    ClassDef {
        visibility: Visibility::Public,
        name: "DateTime".to_string(),
        type_params: vec![],
        constraints: vec![],
        is_abstract: false,
        is_sealed: false,
        is_open: false,
        extends: None,
        implements: vec![],
        fields: vec![FieldDef {
            name: "value".to_string(),
            ty: Type::String,
            default: None,
            is_static: false,
        }],
        init: Some(InitDef {
            params: vec![Param {
                name: "value".to_string(),
                ty: Type::String,
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            body: vec![],
        }),
        deinit: None,
        static_init: None,
        methods: vec![
            ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "DateTime.now".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![],
                    return_type: Some(Type::Struct("DateTime".to_string(), vec![])),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            },
            ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "DateTime.parse".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![Param {
                        name: "s".to_string(),
                        ty: Type::String,
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    return_type: Some(Type::Struct("DateTime".to_string(), vec![])),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            },
            ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "DateTime.toString".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![Param {
                        name: "this".to_string(),
                        ty: Type::Struct("DateTime".to_string(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    }],
                    return_type: Some(Type::String),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            },
            ClassMethod {
                override_: false,
                func: Function {
                    visibility: Visibility::Public,
                    name: "DateTime.addYears".to_string(),
                    type_params: vec![],
                    constraints: vec![],
                    params: vec![
                        Param {
                            name: "this".to_string(),
                            ty: Type::Struct("DateTime".to_string(), vec![]),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                        Param {
                            name: "years".to_string(),
                            ty: Type::Int64,
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    ],
                    return_type: Some(Type::Struct("DateTime".to_string(), vec![])),
                    throws: None,
                    body: vec![],
                    extern_import: None,
                },
            },
        ],
        primary_ctor_params: vec![],
    }
}

impl TimeStdlib {
    /// 将 std.time 标准库注入到 Program 中
    pub fn inject_into(&self, program: &mut Program) {
        for class in &self.classes {
            if !program.classes.iter().any(|c| c.name == class.name) {
                program.classes.push(class.clone());
            }
        }
        for func in &self.functions {
            if !program.functions.iter().any(|f| f.name == func.name) {
                program.functions.push(func.clone());
            }
        }
    }
}
