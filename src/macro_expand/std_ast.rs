//! std.ast API 的 Rust 实现
//!
//! 为 CJson 宏提供编译期 AST 操作能力。
//! 实现 CJson 使用的 ~15 个类型和 ~30 个方法的子集。

use crate::ast::{
    ClassDef, Expr, FieldDef, Function, Param, Stmt, StructDef, Type, Visibility,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Token 类型（编译期 token 表示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroToken {
    pub kind: TokenKind,
    pub value: String,
}

impl MacroToken {
    pub fn new(kind: TokenKind, value: String) -> Self {
        Self { kind, value }
    }

    pub fn identifier(name: &str) -> Self {
        Self {
            kind: TokenKind::Identifier,
            value: name.to_string(),
        }
    }

    pub fn string_literal(s: &str) -> Self {
        Self {
            kind: TokenKind::StringLiteral,
            value: s.to_string(),
        }
    }
}

impl fmt::Display for MacroToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

/// TokenKind 枚举（CJson 使用的子集）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Identifier,
    StringLiteral,
    IntegerLiteral,
    FloatLiteral,
    // 关键字
    Class,
    Struct,
    Func,
    Var,
    Let,
    Return,
    If,
    Else,
    For,
    While,
    Match,
    Public,
    Private,
    Protected,
    Internal,
    Open,
    Abstract,
    Sealed,
    Override,
    Static,
    Init,
    Import,
    Extend,
    Interface,
    // 分隔符和运算符
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Dot,
    Arrow,
    FatArrow,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    SubType,
    At,
    Dollar,
    Semicolon,
    // 特殊
    Eof,
}

/// Tokens 集合（编译期 token 流）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub tokens: Vec<MacroToken>,
}

impl Tokens {
    pub fn new() -> Self {
        Self { tokens: Vec::new() }
    }

    pub fn from_tokens(tokens: Vec<MacroToken>) -> Self {
        Self { tokens }
    }

    pub fn to_string_repr(&self) -> String {
        self.tokens
            .iter()
            .map(|t| t.value.clone())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn to_list(&self) -> &[MacroToken] {
        &self.tokens
    }

    pub fn concat(&self, other: &Tokens) -> Tokens {
        let mut result = self.tokens.clone();
        result.extend(other.tokens.clone());
        Tokens { tokens: result }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

impl Default for Tokens {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_repr())
    }
}

/// ClassDecl — CJson 宏中操作的类声明 AST 节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroClassDecl {
    pub modifiers: Vec<String>,
    pub keyword: String,
    pub identifier: Tokens,
    pub super_types: Vec<Tokens>,
    pub body: MacroClassBody,
}

impl MacroClassDecl {
    pub fn from_class_def(def: &ClassDef) -> Self {
        let mut modifiers = Vec::new();
        match def.visibility {
            Visibility::Public => modifiers.push("public".to_string()),
            Visibility::Private => modifiers.push("private".to_string()),
            Visibility::Protected => modifiers.push("protected".to_string()),
            Visibility::Internal => {}
        }
        if def.is_abstract {
            modifiers.push("abstract".to_string());
        }
        if def.is_sealed {
            modifiers.push("sealed".to_string());
        }
        if def.is_open {
            modifiers.push("open".to_string());
        }

        let keyword = "class".to_string();
        let identifier = Tokens::from_tokens(vec![MacroToken::identifier(&def.name)]);

        let mut super_types = Vec::new();
        if let Some(ref ext) = def.extends {
            super_types.push(Tokens::from_tokens(vec![MacroToken::identifier(ext)]));
        }
        for iface in &def.implements {
            super_types.push(Tokens::from_tokens(vec![MacroToken::identifier(iface)]));
        }

        let body = MacroClassBody::from_class_def(def);

        MacroClassDecl {
            modifiers,
            keyword,
            identifier,
            super_types,
            body,
        }
    }
}

/// StructDecl — 结构体声明 AST 节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroStructDecl {
    pub modifiers: Vec<String>,
    pub keyword: String,
    pub identifier: Tokens,
    pub body: MacroClassBody,
}

impl MacroStructDecl {
    pub fn from_struct_def(def: &StructDef) -> Self {
        let mut modifiers = Vec::new();
        match def.visibility {
            Visibility::Public => modifiers.push("public".to_string()),
            Visibility::Private => modifiers.push("private".to_string()),
            Visibility::Protected => modifiers.push("protected".to_string()),
            Visibility::Internal => {}
        }

        MacroStructDecl {
            modifiers,
            keyword: "struct".to_string(),
            identifier: Tokens::from_tokens(vec![MacroToken::identifier(&def.name)]),
            body: MacroClassBody {
                decls: def
                    .fields
                    .iter()
                    .map(|f| MacroDecl::Var(MacroVarDecl::from_field_def(f)))
                    .collect(),
            },
        }
    }
}

/// ClassBody 的声明列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroClassBody {
    pub decls: Vec<MacroDecl>,
}

impl MacroClassBody {
    pub fn from_class_def(def: &ClassDef) -> Self {
        let mut decls: Vec<MacroDecl> = def
            .fields
            .iter()
            .map(|f| MacroDecl::Var(MacroVarDecl::from_field_def(f)))
            .collect();

        for method in &def.methods {
            decls.push(MacroDecl::Func(MacroFuncDecl::from_function(&method.func)));
        }

        MacroClassBody { decls }
    }
}

/// 声明基类
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MacroDecl {
    Var(MacroVarDecl),
    Func(MacroFuncDecl),
    Class(MacroClassDecl),
    Struct(MacroStructDecl),
}

/// VarDecl — 变量/字段声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroVarDecl {
    pub identifier: Tokens,
    pub type_name: Option<Tokens>,
    pub expr: Option<Tokens>,
    pub is_static: bool,
}

impl MacroVarDecl {
    pub fn from_field_def(f: &FieldDef) -> Self {
        MacroVarDecl {
            identifier: Tokens::from_tokens(vec![MacroToken::identifier(&f.name)]),
            type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier(
                &type_to_string(&f.ty),
            )])),
            expr: None,
            is_static: f.is_static,
        }
    }

    pub fn to_tokens(&self) -> Tokens {
        let mut toks = Vec::new();
        if self.is_static {
            toks.push(MacroToken::new(TokenKind::Static, "static".to_string()));
        }
        toks.push(MacroToken::new(TokenKind::Var, "var".to_string()));
        toks.extend(self.identifier.tokens.clone());
        if let Some(ref ty) = self.type_name {
            toks.push(MacroToken::new(TokenKind::Colon, ":".to_string()));
            toks.extend(ty.tokens.clone());
        }
        Tokens::from_tokens(toks)
    }
}

/// FuncDecl — 函数声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroFuncDecl {
    pub identifier: Tokens,
    pub params: Vec<MacroVarDecl>,
    pub return_type: Option<Tokens>,
    pub body: Option<Vec<Stmt>>,
}

impl MacroFuncDecl {
    pub fn from_function(f: &Function) -> Self {
        MacroFuncDecl {
            identifier: Tokens::from_tokens(vec![MacroToken::identifier(&f.name)]),
            params: f
                .params
                .iter()
                .map(|p| MacroVarDecl {
                    identifier: Tokens::from_tokens(vec![MacroToken::identifier(&p.name)]),
                    type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier(
                        &type_to_string(&p.ty),
                    )])),
                    expr: p.default.as_ref().map(|_| {
                        Tokens::from_tokens(vec![MacroToken::identifier("<default>")])
                    }),
                    is_static: false,
                })
                .collect(),
            return_type: f.return_type.as_ref().map(|t| {
                Tokens::from_tokens(vec![MacroToken::identifier(&type_to_string(t))])
            }),
            body: Some(f.body.clone()),
        }
    }
}

/// Modifier 修饰符
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroModifier {
    pub keyword: String,
}

/// TypeNode 类型节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MacroTypeNode {
    RefType { name: String, type_args: Vec<MacroTypeNode> },
    PrimitiveType { name: String },
    PrefixType { inner: Box<MacroTypeNode> },
}

impl MacroTypeNode {
    pub fn from_type(ty: &Type) -> Self {
        match ty {
            Type::Int8 => MacroTypeNode::PrimitiveType { name: "Int8".to_string() },
            Type::Int16 => MacroTypeNode::PrimitiveType { name: "Int16".to_string() },
            Type::Int32 => MacroTypeNode::PrimitiveType { name: "Int32".to_string() },
            Type::Int64 => MacroTypeNode::PrimitiveType { name: "Int64".to_string() },
            Type::UInt8 => MacroTypeNode::PrimitiveType { name: "UInt8".to_string() },
            Type::UInt16 => MacroTypeNode::PrimitiveType { name: "UInt16".to_string() },
            Type::UInt32 => MacroTypeNode::PrimitiveType { name: "UInt32".to_string() },
            Type::UInt64 => MacroTypeNode::PrimitiveType { name: "UInt64".to_string() },
            Type::Float32 => MacroTypeNode::PrimitiveType { name: "Float32".to_string() },
            Type::Float64 => MacroTypeNode::PrimitiveType { name: "Float64".to_string() },
            Type::Bool => MacroTypeNode::PrimitiveType { name: "Bool".to_string() },
            Type::String => MacroTypeNode::PrimitiveType { name: "String".to_string() },
            Type::Struct(name, args) => MacroTypeNode::RefType {
                name: name.clone(),
                type_args: args.iter().map(|a| MacroTypeNode::from_type(a)).collect(),
            },
            Type::Array(inner) => MacroTypeNode::RefType {
                name: "Array".to_string(),
                type_args: vec![MacroTypeNode::from_type(inner)],
            },
            Type::Option(inner) => MacroTypeNode::PrefixType {
                inner: Box::new(MacroTypeNode::from_type(inner)),
            },
            _ => MacroTypeNode::PrimitiveType {
                name: type_to_string(ty),
            },
        }
    }

    pub fn to_tokens(&self) -> Tokens {
        match self {
            MacroTypeNode::PrimitiveType { name } => {
                Tokens::from_tokens(vec![MacroToken::identifier(name)])
            }
            MacroTypeNode::RefType { name, type_args } => {
                let mut toks = vec![MacroToken::identifier(name)];
                if !type_args.is_empty() {
                    toks.push(MacroToken::new(TokenKind::Lt, "<".to_string()));
                    for (i, arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            toks.push(MacroToken::new(TokenKind::Comma, ",".to_string()));
                        }
                        toks.extend(arg.to_tokens().tokens);
                    }
                    toks.push(MacroToken::new(TokenKind::Gt, ">".to_string()));
                }
                Tokens::from_tokens(toks)
            }
            MacroTypeNode::PrefixType { inner } => {
                let mut toks = vec![MacroToken::new(TokenKind::Identifier, "?".to_string())];
                toks.extend(inner.to_tokens().tokens);
                Tokens::from_tokens(toks)
            }
        }
    }
}

/// AST Node（Visitor 遍历的通用节点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AstNode {
    ClassDecl(MacroClassDecl),
    StructDecl(MacroStructDecl),
    VarDecl(MacroVarDecl),
    FuncDecl(MacroFuncDecl),
    Decl(MacroDecl),
}

impl AstNode {
    pub fn traverse<V: AstVisitor>(&self, visitor: &mut V) {
        match self {
            AstNode::ClassDecl(cd) => {
                for decl in &cd.body.decls {
                    match decl {
                        MacroDecl::Var(vd) => {
                            if !visitor.visit_var_decl(vd) {
                                return;
                            }
                        }
                        MacroDecl::Func(fd) => {
                            if !visitor.visit_func_decl(fd) {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
            AstNode::StructDecl(sd) => {
                for decl in &sd.body.decls {
                    match decl {
                        MacroDecl::Var(vd) => {
                            if !visitor.visit_var_decl(vd) {
                                return;
                            }
                        }
                        MacroDecl::Func(fd) => {
                            if !visitor.visit_func_decl(fd) {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
            AstNode::VarDecl(vd) => {
                visitor.visit_var_decl(vd);
            }
            AstNode::FuncDecl(fd) => {
                visitor.visit_func_decl(fd);
            }
            AstNode::Decl(d) => match d {
                MacroDecl::Var(vd) => {
                    visitor.visit_var_decl(vd);
                }
                MacroDecl::Func(fd) => {
                    visitor.visit_func_decl(fd);
                }
                _ => {}
            },
        }
    }
}

/// Visitor 接口 (C4.2)
pub trait AstVisitor {
    /// 访问变量声明，返回 true 继续遍历，false 停止
    fn visit_var_decl(&mut self, _decl: &MacroVarDecl) -> bool {
        true
    }
    /// 访问函数声明，返回 true 继续遍历，false 停止
    fn visit_func_decl(&mut self, _decl: &MacroFuncDecl) -> bool {
        true
    }
    /// 中断遍历
    fn break_traverse(&mut self);
}

/// VarInfo — CJson 使用的变量信息结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarInfo {
    pub identifier: Tokens,
    pub type_tokens: Tokens,
    pub has_default: bool,
    pub default_expr: Option<Tokens>,
}

impl VarInfo {
    pub fn from_var_decl(vd: &MacroVarDecl) -> Self {
        VarInfo {
            identifier: vd.identifier.clone(),
            type_tokens: vd.type_name.clone().unwrap_or_else(Tokens::new),
            has_default: vd.expr.is_some(),
            default_expr: vd.expr.clone(),
        }
    }
}

/// parseDecl: 从 Tokens 解析为声明 (C4.4)
pub fn parse_decl(tokens: &Tokens) -> Option<MacroDecl> {
    let text = tokens.to_string_repr();
    match crate::pipeline::parse_source(&text) {
        Ok(program) => {
            if let Some(class) = program.classes.first() {
                return Some(MacroDecl::Class(MacroClassDecl::from_class_def(class)));
            }
            if let Some(st) = program.structs.first() {
                return Some(MacroDecl::Struct(MacroStructDecl::from_struct_def(st)));
            }
            if let Some(func) = program.functions.first() {
                return Some(MacroDecl::Func(MacroFuncDecl::from_function(func)));
            }
            None
        }
        Err(_) => None,
    }
}

/// 类型转字符串辅助函数
pub fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Int8 => "Int8".to_string(),
        Type::Int16 => "Int16".to_string(),
        Type::Int32 => "Int32".to_string(),
        Type::Int64 => "Int64".to_string(),
        Type::IntNative => "IntNative".to_string(),
        Type::UInt8 => "UInt8".to_string(),
        Type::UInt16 => "UInt16".to_string(),
        Type::UInt32 => "UInt32".to_string(),
        Type::UInt64 => "UInt64".to_string(),
        Type::UIntNative => "UIntNative".to_string(),
        Type::Float16 => "Float16".to_string(),
        Type::Float32 => "Float32".to_string(),
        Type::Float64 => "Float64".to_string(),
        Type::Rune => "Rune".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::String => "String".to_string(),
        Type::Nothing => "Nothing".to_string(),
        Type::Unit => "Unit".to_string(),
        Type::Tokens => "Tokens".to_string(),
        Type::Array(inner) => format!("Array<{}>", type_to_string(inner)),
        Type::Tuple(types) => {
            let inner: Vec<_> = types.iter().map(|t| type_to_string(t)).collect();
            format!("({})", inner.join(", "))
        }
        Type::Struct(name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<_> = args.iter().map(|t| type_to_string(t)).collect();
                format!("{}<{}>", name, inner.join(", "))
            }
        }
        Type::Range => "Range".to_string(),
        Type::Function { params, ret } => {
            let ps: Vec<_> = params.iter().map(|p| type_to_string(p)).collect();
            let r = match ret.as_ref() {
                Some(t) => type_to_string(t),
                None => "Unit".to_string(),
            };
            format!("({}) -> {}", ps.join(", "), r)
        }
        Type::Option(inner) => format!("?{}", type_to_string(inner)),
        Type::Result(ok, err) => format!("Result<{}, {}>", type_to_string(ok), type_to_string(err)),
        Type::Slice(inner) => format!("Slice<{}>", type_to_string(inner)),
        Type::Map(k, v) => format!("Map<{}, {}>", type_to_string(k), type_to_string(v)),
        Type::TypeParam(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens_new() {
        let t = Tokens::new();
        assert!(t.is_empty());
    }

    #[test]
    fn test_tokens_concat() {
        let a = Tokens::from_tokens(vec![MacroToken::identifier("hello")]);
        let b = Tokens::from_tokens(vec![MacroToken::identifier("world")]);
        let c = a.concat(&b);
        assert_eq!(c.tokens.len(), 2);
        assert_eq!(c.to_string_repr(), "hello world");
    }

    #[test]
    fn test_token_kind() {
        let t = MacroToken::new(TokenKind::Identifier, "foo".to_string());
        assert_eq!(t.kind, TokenKind::Identifier);
        assert_eq!(t.value, "foo");
    }

    #[test]
    fn test_macro_type_node_primitive() {
        let node = MacroTypeNode::from_type(&Type::Int64);
        match node {
            MacroTypeNode::PrimitiveType { name } => assert_eq!(name, "Int64"),
            _ => panic!("Expected PrimitiveType"),
        }
    }

    #[test]
    fn test_macro_type_node_ref() {
        let node = MacroTypeNode::from_type(&Type::Array(Box::new(Type::Int64)));
        match node {
            MacroTypeNode::RefType { name, type_args } => {
                assert_eq!(name, "Array");
                assert_eq!(type_args.len(), 1);
            }
            _ => panic!("Expected RefType"),
        }
    }

    #[test]
    fn test_type_to_string() {
        assert_eq!(type_to_string(&Type::Int64), "Int64");
        assert_eq!(type_to_string(&Type::String), "String");
        assert_eq!(
            type_to_string(&Type::Array(Box::new(Type::Int64))),
            "Array<Int64>"
        );
        assert_eq!(
            type_to_string(&Type::Struct("Point".to_string(), vec![Type::Int64])),
            "Point<Int64>"
        );
    }

    #[test]
    fn test_var_decl_to_tokens() {
        let vd = MacroVarDecl {
            identifier: Tokens::from_tokens(vec![MacroToken::identifier("x")]),
            type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier("Int64")])),
            expr: None,
            is_static: false,
        };
        let toks = vd.to_tokens();
        assert_eq!(toks.to_string_repr(), "var x : Int64");
    }

    #[test]
    fn test_var_decl_static() {
        let vd = MacroVarDecl {
            identifier: Tokens::from_tokens(vec![MacroToken::identifier("count")]),
            type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier("Int64")])),
            expr: None,
            is_static: true,
        };
        let toks = vd.to_tokens();
        assert_eq!(toks.to_string_repr(), "static var count : Int64");
    }

    #[test]
    fn test_var_info_from_var_decl() {
        let vd = MacroVarDecl {
            identifier: Tokens::from_tokens(vec![MacroToken::identifier("name")]),
            type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier("String")])),
            expr: None,
            is_static: false,
        };
        let info = VarInfo::from_var_decl(&vd);
        assert_eq!(info.identifier.to_string_repr(), "name");
        assert!(!info.has_default);
    }

    struct TestVisitor {
        var_names: Vec<String>,
        stopped: bool,
    }

    impl AstVisitor for TestVisitor {
        fn visit_var_decl(&mut self, decl: &MacroVarDecl) -> bool {
            self.var_names.push(decl.identifier.to_string_repr());
            !self.stopped
        }
        fn visit_func_decl(&mut self, _decl: &MacroFuncDecl) -> bool {
            true
        }
        fn break_traverse(&mut self) {
            self.stopped = true;
        }
    }

    #[test]
    fn test_visitor_traversal() {
        let body = MacroClassBody {
            decls: vec![
                MacroDecl::Var(MacroVarDecl {
                    identifier: Tokens::from_tokens(vec![MacroToken::identifier("x")]),
                    type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier("Int64")])),
                    expr: None,
                    is_static: false,
                }),
                MacroDecl::Var(MacroVarDecl {
                    identifier: Tokens::from_tokens(vec![MacroToken::identifier("y")]),
                    type_name: Some(Tokens::from_tokens(vec![MacroToken::identifier("Int64")])),
                    expr: None,
                    is_static: false,
                }),
            ],
        };
        let node = AstNode::StructDecl(MacroStructDecl {
            modifiers: vec![],
            keyword: "struct".to_string(),
            identifier: Tokens::from_tokens(vec![MacroToken::identifier("Point")]),
            body,
        });
        let mut visitor = TestVisitor {
            var_names: vec![],
            stopped: false,
        };
        node.traverse(&mut visitor);
        assert_eq!(visitor.var_names, vec!["x", "y"]);
    }
}
