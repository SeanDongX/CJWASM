//! stdx.encoding.json 标准库实现
//!
//! 提供 CJson 宏生成代码所依赖的 JSON 值类型层次。
//! 在 cjwasm 中作为内建 AST 节点注册，编译时生成对应的 WASM 代码。

use crate::ast::*;
use std::collections::HashMap;

/// JSON 值类型标签（用于运行时类型判断）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonValueTag {
    Object = 0,
    Array = 1,
    StringVal = 2,
    Int = 3,
    Float = 4,
    Bool = 5,
    Null = 6,
}

/// 生成 stdx.encoding.json 标准库的 AST 定义
/// 返回需要注入到 Program 中的类型和函数
pub fn generate_json_stdlib() -> JsonStdlib {
    JsonStdlib {
        interfaces: vec![
            generate_json_value_interface(),
            generate_json_serializable_interface(),
        ],
        classes: vec![
            generate_json_object_class(),
            generate_json_array_class(),
            generate_json_string_class(),
            generate_json_int_class(),
            generate_json_float_class(),
            generate_json_bool_class(),
            generate_json_null_class(),
        ],
        functions: generate_json_helper_functions(),
    }
}

/// JSON 标准库的 AST 产出
pub struct JsonStdlib {
    pub interfaces: Vec<InterfaceDef>,
    pub classes: Vec<ClassDef>,
    pub functions: Vec<Function>,
}

impl JsonStdlib {
    /// 将 JSON 标准库注入到 Program 中
    pub fn inject_into(&self, program: &mut Program) {
        for iface in &self.interfaces {
            if !program.interfaces.iter().any(|i| i.name == iface.name) {
                program.interfaces.push(iface.clone());
            }
        }
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

/// interface JsonValue { func toString(): String }
fn generate_json_value_interface() -> InterfaceDef {
    InterfaceDef {
        visibility: Visibility::Public,
        name: "JsonValue".to_string(),
        type_params: vec![],
        constraints: vec![],
        parents: vec![],
        methods: vec![InterfaceMethod {
            name: "toString".to_string(),
            params: vec![],
            return_type: Some(Type::String),
            default_body: None,
        }],
        assoc_types: vec![],
    }
}

/// interface IJsonSerializable<T> {
///   func toJsonValue(): JsonObject
///   func toJson(): String
///   static func fromJson(jsonStr: String): T
/// }
fn generate_json_serializable_interface() -> InterfaceDef {
    InterfaceDef {
        visibility: Visibility::Public,
        name: "IJsonSerializable".to_string(),
        type_params: vec!["T".to_string()],
        constraints: vec![],
        parents: vec![],
        methods: vec![
            InterfaceMethod {
                name: "toJsonValue".to_string(),
                params: vec![],
                return_type: Some(Type::Struct("JsonObject".to_string(), vec![])),
                default_body: None,
            },
            InterfaceMethod {
                name: "toJson".to_string(),
                params: vec![],
                return_type: Some(Type::String),
                default_body: None,
            },
            InterfaceMethod {
                name: "fromJson".to_string(),
                params: vec![Param {
                    name: "jsonStr".to_string(),
                    ty: Type::String,
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                }],
                return_type: Some(Type::TypeParam("T".to_string())),
                default_body: None,
            },
        ],
        assoc_types: vec![],
    }
}

fn make_json_class(name: &str, extends: &str, fields: Vec<FieldDef>) -> ClassDef {
    ClassDef {
        visibility: Visibility::Public,
        name: name.to_string(),
        type_params: vec![],
        constraints: vec![],
        is_abstract: false,
        is_sealed: false,
        is_open: false,
        extends: Some(extends.to_string()),
        implements: vec!["JsonValue".to_string()],
        fields,
        init: Some(InitDef {
            params: vec![],
            body: vec![],
        }),
        deinit: None,
        static_init: None,
        methods: vec![ClassMethod {
            override_: true,
            func: Function {
                visibility: Visibility::Public,
                name: "toString".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![],
                return_type: Some(Type::String),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("null".to_string())))],
                extern_import: None,
            },
        }],
        primary_ctor_params: vec![],
    }
}

/// class JsonObject <: JsonValue
fn generate_json_object_class() -> ClassDef {
    let mut c = make_json_class("JsonObject", "JsonValue", vec![]);
    c.extends = None;
    c.fields = vec![FieldDef {
        name: "_entries".to_string(),
        ty: Type::Map(Box::new(Type::String), Box::new(Type::Struct("JsonValue".to_string(), vec![]))),
        default: None,
        is_static: false,
    }];
    c.methods = vec![
        ClassMethod {
            override_: false,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonObject.add".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param { name: "this".to_string(), ty: Type::Struct("JsonObject".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                    Param { name: "key".to_string(), ty: Type::String, default: None, variadic: false, is_named: false, is_inout: false },
                    Param { name: "value".to_string(), ty: Type::Struct("JsonValue".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                ],
                return_type: None,
                throws: None,
                body: vec![],
                extern_import: None,
            },
        },
        ClassMethod {
            override_: false,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonObject.get".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param { name: "this".to_string(), ty: Type::Struct("JsonObject".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                    Param { name: "key".to_string(), ty: Type::String, default: None, variadic: false, is_named: false, is_inout: false },
                ],
                return_type: Some(Type::Option(Box::new(Type::Struct("JsonValue".to_string(), vec![])))),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::None))],
                extern_import: None,
            },
        },
        ClassMethod {
            override_: true,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonObject.toString".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param { name: "this".to_string(), ty: Type::Struct("JsonObject".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                ],
                return_type: Some(Type::String),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("{}".to_string())))],
                extern_import: None,
            },
        },
    ];
    c
}

/// class JsonArray <: JsonValue
fn generate_json_array_class() -> ClassDef {
    let mut c = make_json_class("JsonArray", "JsonValue", vec![]);
    c.extends = None;
    c.fields = vec![FieldDef {
        name: "_items".to_string(),
        ty: Type::Array(Box::new(Type::Struct("JsonValue".to_string(), vec![]))),
        default: None,
        is_static: false,
    }];
    c.methods = vec![
        ClassMethod {
            override_: false,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonArray.add".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param { name: "this".to_string(), ty: Type::Struct("JsonArray".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                    Param { name: "value".to_string(), ty: Type::Struct("JsonValue".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                ],
                return_type: None,
                throws: None,
                body: vec![],
                extern_import: None,
            },
        },
        ClassMethod {
            override_: true,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonArray.toString".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![
                    Param { name: "this".to_string(), ty: Type::Struct("JsonArray".to_string(), vec![]), default: None, variadic: false, is_named: false, is_inout: false },
                ],
                return_type: Some(Type::String),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("[]".to_string())))],
                extern_import: None,
            },
        },
    ];
    c
}

fn make_json_scalar_class(name: &str, field_ty: Type, init_param_ty: Type) -> ClassDef {
    ClassDef {
        visibility: Visibility::Public,
        name: name.to_string(),
        type_params: vec![],
        constraints: vec![],
        is_abstract: false,
        is_sealed: false,
        is_open: false,
        extends: None,
        implements: vec!["JsonValue".to_string()],
        fields: vec![FieldDef {
            name: "value".to_string(),
            ty: field_ty,
            default: None,
            is_static: false,
        }],
        init: Some(InitDef {
            params: vec![Param {
                name: "value".to_string(),
                ty: init_param_ty,
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            body: vec![Stmt::Assign {
                target: AssignTarget::Field {
                    object: "this".to_string(),
                    field: "value".to_string(),
                },
                value: Expr::Var("value".to_string()),
            }],
        }),
        deinit: None,
        static_init: None,
        methods: vec![ClassMethod {
            override_: true,
            func: Function {
                visibility: Visibility::Public,
                name: format!("{}.toString", name),
                type_params: vec![],
                constraints: vec![],
                params: vec![Param {
                    name: "this".to_string(),
                    ty: Type::Struct(name.to_string(), vec![]),
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                }],
                return_type: Some(Type::String),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("<json>".to_string())))],
                extern_import: None,
            },
        }],
        primary_ctor_params: vec![],
    }
}

fn generate_json_string_class() -> ClassDef {
    make_json_scalar_class("JsonString", Type::String, Type::String)
}

fn generate_json_int_class() -> ClassDef {
    make_json_scalar_class("JsonInt", Type::Int64, Type::Int64)
}

fn generate_json_float_class() -> ClassDef {
    make_json_scalar_class("JsonFloat", Type::Float64, Type::Float64)
}

fn generate_json_bool_class() -> ClassDef {
    make_json_scalar_class("JsonBool", Type::Bool, Type::Bool)
}

fn generate_json_null_class() -> ClassDef {
    ClassDef {
        visibility: Visibility::Public,
        name: "JsonNull".to_string(),
        type_params: vec![],
        constraints: vec![],
        is_abstract: false,
        is_sealed: false,
        is_open: false,
        extends: None,
        implements: vec!["JsonValue".to_string()],
        fields: vec![],
        init: Some(InitDef {
            params: vec![],
            body: vec![],
        }),
        deinit: None,
        static_init: None,
        methods: vec![ClassMethod {
            override_: true,
            func: Function {
                visibility: Visibility::Public,
                name: "JsonNull.toString".to_string(),
                type_params: vec![],
                constraints: vec![],
                params: vec![Param {
                    name: "this".to_string(),
                    ty: Type::Struct("JsonNull".to_string(), vec![]),
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                }],
                return_type: Some(Type::String),
                throws: None,
                body: vec![Stmt::Return(Some(Expr::String("null".to_string())))],
                extern_import: None,
            },
        }],
        primary_ctor_params: vec![],
    }
}

/// JSON 解析/序列化辅助函数
fn generate_json_helper_functions() -> Vec<Function> {
    vec![
        Function {
            visibility: Visibility::Public,
            name: "jsonParse".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![Param {
                name: "jsonStr".to_string(),
                ty: Type::String,
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            return_type: Some(Type::Struct("JsonValue".to_string(), vec![])),
            throws: None,
            body: vec![Stmt::Return(Some(Expr::ConstructorCall {
                name: "JsonNull".to_string(),
                type_args: None,
                args: vec![],
                named_args: vec![],
            }))],
            extern_import: None,
        },
        Function {
            visibility: Visibility::Public,
            name: "jsonStringify".to_string(),
            type_params: vec![],
            constraints: vec![],
            params: vec![Param {
                name: "value".to_string(),
                ty: Type::Struct("JsonValue".to_string(), vec![]),
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            }],
            return_type: Some(Type::String),
            throws: None,
            body: vec![Stmt::Return(Some(Expr::String("null".to_string())))],
            extern_import: None,
        },
    ]
}

/// C5.3: @OverflowWrapping 注解
/// 在 WASM 中溢出行为已由 WASM 语义保证，此注解为 no-op
pub fn is_overflow_wrapping_annotation(name: &str) -> bool {
    name == "OverflowWrapping"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_json_stdlib() {
        let stdlib = generate_json_stdlib();
        assert_eq!(stdlib.interfaces.len(), 2);
        assert_eq!(stdlib.classes.len(), 7);
        assert!(stdlib.functions.len() >= 2);
    }

    #[test]
    fn test_json_value_interface() {
        let iface = generate_json_value_interface();
        assert_eq!(iface.name, "JsonValue");
        assert_eq!(iface.methods.len(), 1);
        assert_eq!(iface.methods[0].name, "toString");
    }

    #[test]
    fn test_json_serializable_interface() {
        let iface = generate_json_serializable_interface();
        assert_eq!(iface.name, "IJsonSerializable");
        assert_eq!(iface.methods.len(), 3);
    }

    #[test]
    fn test_json_object_class() {
        let class = generate_json_object_class();
        assert_eq!(class.name, "JsonObject");
        assert!(!class.methods.is_empty());
    }

    #[test]
    fn test_json_scalar_classes() {
        let s = generate_json_string_class();
        assert_eq!(s.name, "JsonString");
        assert_eq!(s.fields.len(), 1);
        assert_eq!(s.fields[0].ty, Type::String);

        let i = generate_json_int_class();
        assert_eq!(i.name, "JsonInt");
        assert_eq!(i.fields[0].ty, Type::Int64);

        let f = generate_json_float_class();
        assert_eq!(f.name, "JsonFloat");
        assert_eq!(f.fields[0].ty, Type::Float64);

        let b = generate_json_bool_class();
        assert_eq!(b.name, "JsonBool");
        assert_eq!(b.fields[0].ty, Type::Bool);
    }

    #[test]
    fn test_json_null_class() {
        let n = generate_json_null_class();
        assert_eq!(n.name, "JsonNull");
        assert!(n.fields.is_empty());
    }

    #[test]
    fn test_overflow_wrapping() {
        assert!(is_overflow_wrapping_annotation("OverflowWrapping"));
        assert!(!is_overflow_wrapping_annotation("Other"));
    }

    #[test]
    fn test_inject_into_program() {
        let stdlib = generate_json_stdlib();
        let mut program = Program {
            package_name: None,
            is_macro_package: false,
            global_vars: vec![],
            imports: vec![],
            structs: vec![],
            interfaces: vec![],
            classes: vec![],
            enums: vec![],
            functions: vec![],
            extends: vec![],
            type_aliases: vec![],
            macros: vec![],
        };
        stdlib.inject_into(&mut program);
        assert_eq!(program.interfaces.len(), 2);
        assert_eq!(program.classes.len(), 7);
        assert!(program.functions.len() >= 2);

        // Injecting again should not duplicate
        stdlib.inject_into(&mut program);
        assert_eq!(program.interfaces.len(), 2);
        assert_eq!(program.classes.len(), 7);
    }

    #[test]
    fn test_json_value_tag() {
        assert_eq!(JsonValueTag::Object as u8, 0);
        assert_eq!(JsonValueTag::Null as u8, 6);
    }
}
