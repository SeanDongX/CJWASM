use crate::ast::*;
use crate::lexer::{Token, StringOrInterpolated, StringPart};
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("意外的 token: {0:?}, 期望: {1}")]
    UnexpectedToken(Token, String),
    #[error("意外的输入结束")]
    UnexpectedEof,
    #[error("未知类型: {0}")]
    UnknownType(String),
}

/// 带字节偏移的解析错误，用于报告位置（可转换为行/列）
#[derive(Debug)]
pub struct ParseErrorAt {
    pub error: ParseError,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl fmt::Display for ParseErrorAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (字节偏移 {}-{})", self.error, self.byte_start, self.byte_end)
    }
}

impl std::error::Error for ParseErrorAt {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// 根据字节偏移和源码计算行号与列号（从 1 开始）
pub fn line_column_from_source(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1_usize;
    let mut col = 1_usize;
    for (i, c) in source.char_indices() {
        if i >= byte_offset {
            return (line, col);
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub struct Parser {
    tokens: Vec<(usize, Token, usize)>,
    pos: usize,
    /// 方法体内的 receiver 参数名（用于解析 this）
    receiver_name: Option<String>,
    /// 当前泛型作用域的类型参数名，用于将 Ident 解析为 TypeParam
    current_type_params: Vec<String>,
    /// struct/enum 内部方法，解析完成后合并到 functions
    pending_struct_methods: Vec<Function>,
    /// P2.2: 类型别名映射 (alias_name -> actual_type)
    type_aliases: std::collections::HashMap<String, Type>,
    /// 处理 >= 拆分时的 pending token（当 >= 被视为 > 后，= 存储在此）
    pending_token: Option<(usize, Token, usize)>,
}

impl Parser {
    pub fn new(tokens: Vec<(usize, Token, usize)>) -> Self {
        Self {
            tokens,
            pos: 0,
            receiver_name: None,
            current_type_params: Vec::new(),
            pending_struct_methods: Vec::new(),
            type_aliases: std::collections::HashMap::new(),
            pending_token: None,
        }
    }

    fn at(&self) -> (usize, usize) {
        self.tokens
            .get(self.pos)
            .map(|t| (t.0, t.2))
            .unwrap_or((0, 0))
    }

    fn at_prev(&self) -> (usize, usize) {
        if self.pos > 0 {
            let t = &self.tokens[self.pos - 1];
            (t.0, t.2)
        } else {
            (0, 0)
        }
    }

    fn bail<T>(&self, e: ParseError) -> Result<T, ParseErrorAt> {
        let (s, e_end) = self.at();
        Err(ParseErrorAt {
            error: e,
            byte_start: s,
            byte_end: e_end,
        })
    }

    fn bail_at<T>(&self, e: ParseError, (s, e_end): (usize, usize)) -> Result<T, ParseErrorAt> {
        Err(ParseErrorAt {
            error: e,
            byte_start: s,
            byte_end: e_end,
        })
    }

    fn peek(&self) -> Option<&Token> {
        // 如果有 pending token，返回它
        if let Some((_, ref tok, _)) = self.pending_token {
            return Some(tok);
        }
        self.tokens.get(self.pos).map(|(_, t, _)| t)
    }

    fn peek_next(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1).map(|(_, t, _)| t)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|(_, t, _)| t)
    }

    /// 将当前 token 作为标识符消费（允许部分关键字在标识符位置出现）
    /// cjc 中 main, type, where, is 等在某些上下文中可作为标识符
    fn advance_ident(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(_)) => {
                if let Some(Token::Ident(n)) = self.advance() { Some(n) } else { None }
            }
            Some(Token::Main) => { self.advance(); Some("main".to_string()) }
            Some(Token::Where) => { self.advance(); Some("where".to_string()) }
            Some(Token::TypeAlias) => { self.advance(); Some("type".to_string()) }
            Some(Token::Is) => { self.advance(); Some("is".to_string()) }
            Some(Token::Case) => { self.advance(); Some("case".to_string()) }
            Some(Token::With) => { self.advance(); Some("with".to_string()) }
            _ => None,
        }
    }

    fn advance(&mut self) -> Option<Token> {
        // 首先检查是否有 pending token（从 >= 拆分出来的 =）
        if let Some((_, tok, _)) = self.pending_token.take() {
            return Some(tok);
        }
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].1.clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    /// 尝试消费一个 `>` token。
    /// 特殊处理：
    /// - `>=` 的情况：视为 `>` 并将 `=` 保存为 pending token
    /// - `>>` 的情况：视为 `>` 并将另一个 `>` 保存为 pending token（用于嵌套泛型如 `ArrayList<ArrayList<T>>`）
    fn try_consume_gt(&mut self) -> bool {
        if self.check(&Token::Gt) {
            self.advance();
            return true;
        }
        if self.check(&Token::GtEq) {
            // 消费 >= 并将 = 存储为 pending token
            let (start, _, end) = self.tokens[self.pos].clone();
            self.pos += 1;
            self.pending_token = Some((start, Token::Assign, end));
            return true;
        }
        if self.check(&Token::Shr) {
            // 消费 >> 并将第二个 > 存储为 pending token（用于嵌套泛型）
            let (start, _, end) = self.tokens[self.pos].clone();
            self.pos += 1;
            self.pending_token = Some((start + 1, Token::Gt, end));
            return true;
        }
        if self.check(&Token::ShrEq) {
            // 消费 >>= 并将 >= 存储为 pending token
            let (start, _, end) = self.tokens[self.pos].clone();
            self.pos += 1;
            self.pending_token = Some((start + 1, Token::GtEq, end));
            return true;
        }
        false
    }

    fn expect(&mut self, expected: Token) -> Result<(), ParseErrorAt> {
        match self.advance() {
            Some(tok) if std::mem::discriminant(&tok) == std::mem::discriminant(&expected) => {
                Ok(())
            }
            Some(tok) => self.bail_at(
                ParseError::UnexpectedToken(tok, format!("{:?}", expected)),
                self.at_prev(),
            ),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    fn check(&self, expected: &Token) -> bool {
        self.peek()
            .map(|t| std::mem::discriminant(t) == std::mem::discriminant(expected))
            .unwrap_or(false)
    }

    /// 解析程序
    pub fn parse_program(&mut self) -> Result<Program, ParseErrorAt> {
        let mut package_name = None;
        let mut is_macro_package = false;
        let mut imports = Vec::new();
        let mut structs = Vec::new();
        let mut interfaces = Vec::new();
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        let mut enums = Vec::new();
        let mut extends = Vec::new();
        let mut type_aliases: Vec<(String, Type)> = Vec::new();
        let mut macros = Vec::new();
        let mut global_vars = Vec::new();

        // C2.1: 解析可选的 `macro package` 或 `package` 声明
        if self.check(&Token::Macro) && matches!(self.peek_next(), Some(Token::Package)) {
            self.advance(); // consume `macro`
            is_macro_package = true;
        }
        if self.check(&Token::Package) {
            self.advance();
            let mut name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                }
            };
            while self.check(&Token::Dot) {
                self.advance();
                let part = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                    }
                };
                name = format!("{}.{}", name, part);
            }
            package_name = Some(name);
        }

        // 解析 import 语句（支持 internal import / public import）
        loop {
            if self.check(&Token::Import) {
                imports.push(self.parse_import()?);
            } else if self.check(&Token::Internal) && matches!(self.peek_next(), Some(Token::Import)) {
                self.advance();
                imports.push(self.parse_import_with_visibility(Visibility::Internal)?);
            } else if self.check(&Token::Public) && matches!(self.peek_next(), Some(Token::Import)) {
                self.advance();
                imports.push(self.parse_import_with_visibility(Visibility::Public)?);
            } else if self.check(&Token::Private) && matches!(self.peek_next(), Some(Token::Import)) {
                self.advance();
                imports.push(self.parse_import_with_visibility(Visibility::Private)?);
            } else {
                break;
            }
        }

        while let Some(_tok) = self.peek() {
            // 首先解析顶层宏装饰符（如 @JsonSerializable, @Test 等）
            // 注意：这些装饰符会应用到紧随其后的声明
            while self.check(&Token::At) {
                // 检查是否是 @import（用于 extern func），这需要保留
                let next_is_import = match self.peek_at(1) {
                    Some(Token::Import) => true,
                    _ => false,
                };
                if next_is_import {
                    break; // 让后面的代码处理 @import
                }
                // 解析宏装饰符（目前忽略，将来用于宏展开）
                self.advance(); // consume @
                let _macro_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "宏名".to_string()));
                    }
                };
                // 跳过可选的宏参数 [args] 或 (args)
                if self.check(&Token::LBracket) {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LBracket) => depth += 1,
                            Some(Token::RBracket) => depth -= 1,
                            None => break,
                            _ => {}
                        }
                    }
                } else if self.check(&Token::LParen) {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LParen) => depth += 1,
                            Some(Token::RParen) => depth -= 1,
                            None => break,
                            _ => {}
                        }
                    }
                }
            }

            // 解析可见性修饰符
            let visibility = if self.check(&Token::Public) {
                self.advance();
                Visibility::Public
            } else if self.check(&Token::Private) {
                self.advance();
                Visibility::Private
            } else if self.check(&Token::Protected) {
                self.advance();
                Visibility::Protected
            } else if self.check(&Token::Internal) {
                self.advance();
                Visibility::Internal
            } else {
                Visibility::default()
            };

            // 解析 @import（用于 extern func）
            let extern_import = if self.check(&Token::At) {
                Some(self.parse_import_attr()?)
            } else {
                None
            };

            if self.check(&Token::Foreign) {
                self.advance();
                functions.push(self.parse_extern_func(visibility, extern_import)?);
            } else {
                match self.peek() {
                    Some(Token::Struct) => structs.push(self.parse_struct_with_visibility(visibility)?),
                    Some(Token::Interface) => interfaces.push(self.parse_interface_with_visibility(visibility)?),
                    Some(Token::Class) | Some(Token::Abstract) | Some(Token::Sealed) | Some(Token::Open)
                        => classes.push(self.parse_class_with_visibility(visibility)?),
                    Some(Token::Enum) => enums.push(self.parse_enum_with_visibility(visibility)?),
                    Some(Token::Extend) => extends.push(self.parse_extend()?),
                    // P2.2: type Name = Type
                    Some(Token::TypeAlias) => {
                        self.advance();
                        let alias_name = match self.advance() {
                            Some(Token::Ident(n)) => n,
                            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "类型别名名称".to_string())),
                            None => return self.bail(ParseError::UnexpectedEof),
                        };
                        self.expect(Token::Assign)?;
                        let target_ty = self.parse_type()?;
                        self.type_aliases.insert(alias_name.clone(), target_ty.clone());
                        type_aliases.push((alias_name, target_ty));
                    }
                    Some(Token::Func) => functions.push(self.parse_function_with_visibility(visibility)?),
                    // M2: macro func 声明
                    Some(Token::Macro) => macros.push(self.parse_macro_def(visibility)?),
                    // cjc: main() 无需 func 关键字 (main 是保留字)
                    Some(Token::Main) => {
                        functions.push(self.parse_main_function(visibility)?);
                    }
                    // E3: 包级 let/var/const 声明
                    Some(Token::Let) | Some(Token::Var) | Some(Token::Const) => {
                        let is_const = self.check(&Token::Const);
                        self.advance();
                        let f_name = match self.advance() {
                            Some(Token::Ident(n)) => n,
                            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "变量名".to_string())),
                            None => return self.bail(ParseError::UnexpectedEof),
                        };
                        let ty = if self.check(&Token::Colon) {
                            self.advance();
                            self.parse_type()?
                        } else {
                            Type::Unit
                        };
                        let default = if self.check(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        if self.check(&Token::Semicolon) {
                            self.advance();
                        }
                        global_vars.push(crate::ast::FieldDef {
                            name: f_name,
                            ty,
                            default,
                            is_static: is_const,
                        });
                    }
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(
                            tok.clone(),
                            "struct、interface、class、enum、extend、func、let、var 或 foreign func".to_string(),
                        ))
                    }
                    None => break,
                }
            }
        }
        functions.extend(self.pending_struct_methods.drain(..));
        Ok(Program {
            package_name,
            is_macro_package,
            imports,
            structs,
            interfaces,
            classes,
            enums,
            functions,
            extends,
            type_aliases,
            macros,
            global_vars,
        })
    }

    /// 解析 import 语句 (cjc: import / internal import / public import)
    fn parse_import(&mut self) -> Result<Import, ParseErrorAt> {
        self.parse_import_with_visibility(Visibility::default())
    }

    /// 解析带可见性的 import 语句
    fn parse_import_with_visibility(&mut self, visibility: Visibility) -> Result<Import, ParseErrorAt> {
        self.expect(Token::Import)?;

        let first = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };

        let mut module_path = vec![first];
        while self.check(&Token::Dot) {
            self.advance();
            if self.check(&Token::Star) {
                self.advance();
                return Ok(Import {
                    visibility: visibility.clone(),
                    module_path,
                    items: None,
                    alias: None,
                });
            }
            // E2: 选择性导入 import std.ast.{Token, TypeNode}
            if self.check(&Token::LBrace) {
                self.advance();
                let mut items = Vec::new();
                while !self.check(&Token::RBrace) {
                    match self.advance() {
                        Some(Token::Ident(n)) => items.push(n),
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "导入项名称".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    }
                }
                self.expect(Token::RBrace)?;
                return Ok(Import {
                    visibility,
                    module_path,
                    items: Some(items),
                    alias: None,
                });
            }
            let part = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            module_path.push(part);
        }
        let alias = if self.check(&Token::As) {
            self.advance();
            match self.advance() {
                Some(Token::Ident(n)) => Some(n),
                _ => return self.bail(ParseError::UnexpectedEof),
            }
        } else {
            None
        };
        Ok(Import {
            visibility,
            module_path,
            items: None,
            alias,
        })
    }

    /// 解析 @import("module", "name") 属性（用于 extern func 前）
    fn parse_import_attr(&mut self) -> Result<ExternImport, ParseErrorAt> {
        self.expect(Token::At)?;
        self.expect(Token::Import)?;
        self.expect(Token::LParen)?;
        let module = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字符串字面量 (模块名)".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::Comma)?;
        let name = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字符串字面量 (导入名)".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::RParen)?;
        Ok(ExternImport { module, name })
    }

    /// 解析 extern func 声明（无 body；可选 extern_import 来自前导 @import）
    fn parse_extern_func(&mut self, visibility: Visibility, extern_import: Option<ExternImport>) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "函数名".to_string()));
            }
        };
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;
        let return_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let import = extern_import.or_else(|| {
            Some(ExternImport { module: "env".to_string(), name: name.clone() })
        });
        Ok(Function {
            visibility,
            name,
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: None,
            body: vec![],
            extern_import: import,
        })
    }

    /// 解析结构体定义
    fn parse_struct(&mut self) -> Result<StructDef, ParseErrorAt> {
        self.parse_struct_with_visibility(Visibility::default())
    }

    /// 判断 token 是否为类型的有效起始（避免将 n < 10 的 < 误解析为类型实参）
    fn is_type_start(t: &Token) -> bool {
        matches!(t,
            Token::TypeInt8 | Token::TypeInt16 | Token::TypeInt32 | Token::TypeInt64
            | Token::TypeIntNative
            | Token::TypeUInt8 | Token::TypeUInt16 | Token::TypeUInt32 | Token::TypeUInt64
            | Token::TypeUIntNative
            | Token::TypeFloat16 | Token::TypeFloat32 | Token::TypeFloat64
            | Token::TypeRune | Token::TypeBool | Token::TypeNothing | Token::TypeUnit
            | Token::TypeVArray | Token::TypeThis
            | Token::TypeString | Token::TypeArray | Token::TypeTuple
            | Token::TypeRange | Token::TypeOption | Token::TypeResult
            | Token::TypeSlice | Token::TypeMap
            | Token::Ident(_)
        )
    }

    /// 解析可选类型实参 <Type1, Type2, ...>，用于调用与实例化
    fn parse_opt_type_args(&mut self) -> Result<Option<Vec<Type>>, ParseErrorAt> {
        if !self.check(&Token::Lt) {
            return Ok(None);
        }
        // 避免将 n < 10 的 < 误解析为类型实参：< 后必须是类型起始
        if !self.peek_next().map(Self::is_type_start).unwrap_or(false) {
            return Ok(None);
        }
        // 如果 < 后面是普通标识符（非类型关键字），再检查其后是否为 >, , 或 <（泛型上下文）
        // 例: i < end {  → end 后为 {，不是泛型
        //     Map<MyType> → MyType 后为 >，是泛型
        if let Some(Token::Ident(_)) = self.peek_next() {
            if !matches!(self.peek_at(2), Some(Token::Gt | Token::Comma | Token::Lt)) {
                return Ok(None);
            }
        }
        self.advance();
        let mut args = Vec::new();
        loop {
            args.push(self.parse_type()?);
            if self.check(&Token::Comma) {
                self.advance();
            } else if self.try_consume_gt() {
                break;
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Comma),
                    "`,` 或 `>`".to_string(),
                ));
            }
        }
        Ok(Some(args))
    }

    /// 解析泛型类型参数列表 <T, U, ...> 或 <T: Bound1 & Bound2, U: Bound3, ...>
    /// 返回 (类型参数名列表, 类型约束列表)
    fn parse_type_params_with_constraints(&mut self) -> Result<(Vec<String>, Vec<crate::ast::TypeConstraint>), ParseErrorAt> {
        if !self.check(&Token::Lt) {
            return Ok((Vec::new(), Vec::new()));
        }
        self.advance();
        let mut params = Vec::new();
        let mut constraints = Vec::new();
        loop {
            let p = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "类型参数名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // 检查是否有约束 T: Bound1 & Bound2 或 T <: Bound1 & Bound2 (cjc)
            if self.check(&Token::Colon) || self.check(&Token::SubType) {
                self.advance();
                let mut bounds = Vec::new();
                loop {
                    let bound = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    bounds.push(bound);
                    // 检查是否有多重约束 &
                    if self.check(&Token::And) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                constraints.push(crate::ast::TypeConstraint {
                    param: p.clone(),
                    bounds,
                });
            }
            params.push(p);
            if self.check(&Token::Comma) {
                self.advance();
            } else if self.try_consume_gt() {
                break;
            } else {
                return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Comma), "`,` 或 `>`".to_string()));
            }
        }
        Ok((params, constraints))
    }

    /// 解析泛型类型参数列表 <T, U, ...>（向后兼容，不解析约束）
    fn parse_type_params(&mut self) -> Result<Vec<String>, ParseErrorAt> {
        let (params, _) = self.parse_type_params_with_constraints()?;
        Ok(params)
    }

    /// 解析 where 子句：where T: Bound1 & Bound2, U: Bound3
    fn parse_where_clause(&mut self) -> Result<Vec<crate::ast::TypeConstraint>, ParseErrorAt> {
        if !matches!(self.peek(), Some(Token::Where)) {
            return Ok(Vec::new());
        }
        self.advance(); // consume "where"
        let mut constraints = Vec::new();
        loop {
            let param = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "类型参数名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // cjc 兼容: where T <: Bound 或 where T: Bound
            if !self.check(&Token::Colon) && !self.check(&Token::SubType) {
                return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Colon), "`:` 或 `<:`".to_string()));
            }
            self.advance();
            let mut bounds = Vec::new();
            loop {
                let bound = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                bounds.push(bound);
                // 消费类型实参，如 Comparable<T> 中的 <T>
                if self.check(&Token::Lt) {
                    let _ = self.parse_opt_type_args()?;
                }
                if self.check(&Token::And) {
                    self.advance();
                } else {
                    break;
                }
            }
            constraints.push(crate::ast::TypeConstraint {
                param,
                bounds,
            });
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(constraints)
    }

    /// 解析结构体定义（带可见性）
    fn parse_struct_with_visibility(&mut self, visibility: Visibility) -> Result<StructDef, ParseErrorAt> {
        self.expect(Token::Struct)?;

        let name = match self.advance() {
            Some(Token::Ident(name)) => name,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "结构体名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };

        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());

        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut init: Option<crate::ast::InitDef> = None;

        while !self.check(&Token::RBrace) {
            // 跳过可选的可见性修饰符
            let _member_vis = if self.check(&Token::Public) || self.check(&Token::Private)
                || self.check(&Token::Protected) || self.check(&Token::Internal)
            {
                let tok = self.advance();
                tok
            } else {
                None
            };

            // init 构造函数
            if self.check(&Token::Init) {
                self.advance();
                self.expect(Token::LParen)?;
                let params = self.parse_params()?;
                self.expect(Token::RParen)?;
                self.expect(Token::LBrace)?;
                // 设置 receiver 以便在 body 中正确解析 this
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let body = self.parse_stmts()?;
                self.receiver_name = prev_receiver;
                self.expect(Token::RBrace)?;
                init = Some(crate::ast::InitDef { params, body });
                continue;
            }

            // cjc 兼容: 主构造器语法 — public StructName(let value: Type) { ... }
            // 当遇到结构体名后跟 ( 时，解析为构造器并忽略
            if let Some(Token::Ident(ident_name)) = self.peek().cloned() {
                if ident_name == name && self.peek_at(1) == Some(&Token::LParen) {
                    self.advance(); // consume struct name
                    self.expect(Token::LParen)?;
                    let _params = self.parse_params()?;
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LBrace) => depth += 1,
                            Some(Token::RBrace) => depth -= 1,
                            None => return self.bail(ParseError::UnexpectedEof),
                            _ => {}
                        }
                    }
                    continue;
                }
            }

            // static 前缀
            if self.check(&Token::Static) {
                self.advance();
            }
            // override 前缀
            if self.check(&Token::Override) {
                self.advance();
            }

            // struct 内部方法 → 转为外部方法
            if self.check(&Token::Func) {
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let mut func = self.parse_function_with_visibility(Visibility::Public)?;
                self.receiver_name = prev_receiver;
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                let has_self = func.params.iter().any(|p| p.name == "self" || p.name == "this");
                if !has_self {
                    func.params.insert(0, crate::ast::Param {
                        name: "this".to_string(),
                        ty: Type::Struct(name.clone(), type_params.iter().map(|t| Type::TypeParam(t.clone())).collect()),
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    });
                }
                methods.push(func);
                continue;
            }

            // 普通字段
            if self.check(&Token::Var) || self.check(&Token::Let) || self.check(&Token::Const) {
                self.advance();
            }
            let field_name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            let ty = if self.check(&Token::Colon) {
                self.advance();
                self.parse_type()?
            } else {
                Type::Unit
            };
            let default = if self.check(&Token::Assign) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };

            fields.push(FieldDef {
                name: field_name,
                ty,
                default,
                is_static: false,
            });

            if self.check(&Token::Comma) || self.check(&Token::Semicolon) {
                self.advance();
            }
        }

        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;

        // 将 struct 内部方法存入 pending_methods，在解析完成后合并到 functions
        self.pending_struct_methods.extend(methods);

        Ok(StructDef { visibility, name, type_params, constraints, fields, init })
    }

    /// 解析枚举定义（支持无关联值或单关联类型变体，如 Ok(Int64)）
    fn parse_enum(&mut self) -> Result<EnumDef, ParseErrorAt> {
        self.parse_enum_with_visibility(Visibility::default())
    }

    /// 解析枚举定义（带可见性）
    fn parse_enum_with_visibility(&mut self, visibility: Visibility) -> Result<EnumDef, ParseErrorAt> {
        self.expect(Token::Enum)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "枚举名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 解析可选的泛型类型参数 <T, E: Bound, ...>
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        self.expect(Token::LBrace)?;
        let mut variants = Vec::new();
        while !self.check(&Token::RBrace) {
            // cjc 兼容: enum 内部方法 → 转为外部方法 func EnumName.method(this, ...)
            if self.check(&Token::Func) {
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let mut func = self.parse_function_with_visibility(Visibility::Public)?;
                self.receiver_name = prev_receiver;
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                let has_self = func.params.iter().any(|p| p.name == "self" || p.name == "this");
                if !has_self {
                    func.params.insert(0, crate::ast::Param {
                        name: "this".to_string(),
                        ty: Type::Struct(name.clone(), type_params.iter().map(|t| Type::TypeParam(t.clone())).collect()),
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    });
                }
                self.pending_struct_methods.push(func);
                continue;
            }

            // cjc 兼容: 跳过可选的 | 前缀
            if self.check(&Token::Pipe) {
                self.advance();
            }
            let v_name = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "变体名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            let payload = if self.check(&Token::LParen) {
                self.advance();
                let ty = self.parse_type()?;
                self.expect(Token::RParen)?;
                Some(ty)
            } else {
                None
            };
            variants.push(EnumVariant {
                name: v_name,
                payload,
            });
            if self.check(&Token::Comma) {
                self.advance();
            }
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        Ok(EnumDef { visibility, name, type_params, constraints, variants })
    }

    /// 解析接口定义（支持继承、默认实现、关联类型）
    /// interface Name: Parent1, Parent2 { type Element; func method(args): Ret; func default_method(args): Ret { body } }
    fn parse_interface_with_visibility(&mut self, visibility: Visibility) -> Result<crate::ast::InterfaceDef, ParseErrorAt> {
        self.expect(Token::Interface)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // E6: 泛型接口类型参数 interface Foo<T>
        let (type_params, constraints) = self.parse_type_params_with_constraints()?;
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // 解析接口继承 : Parent1, Parent2 或 <: Parent1 & Parent2 (cjc)
        let parents = if self.check(&Token::Colon) || self.check(&Token::SubType) {
            self.advance();
            let mut ps = Vec::new();
            loop {
                let mut parent_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "父接口名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                // 跳过泛型参数 <T>, <K, V> 等
                if self.check(&Token::Lt) {
                    parent_name.push('<');
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::Lt) => { depth += 1; parent_name.push('<'); }
                            Some(Token::Shr) => { depth -= 2; if depth > 0 { parent_name.push('>'); } }
                            Some(Token::Gt) => { depth -= 1; if depth > 0 { parent_name.push('>'); } }
                            Some(Token::Comma) => parent_name.push(','),
                            Some(Token::Ident(n)) => parent_name.push_str(&n),
                            Some(_) => {}
                            None => return self.bail(ParseError::UnexpectedEof),
                        }
                    }
                    parent_name.push('>');
                }
                ps.push(parent_name);
                if self.check(&Token::Comma) || self.check(&Token::And) {
                    self.advance();
                } else {
                    break;
                }
            }
            ps
        } else {
            Vec::new()
        };
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        while !self.check(&Token::RBrace) {
            // 关联类型: type Element; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Semicolon)?;
                assoc_types.push(crate::ast::AssocTypeDef { name: type_name });
                continue;
            }
            // 跳过可选的可见性修饰符和 static/override
            while matches!(self.peek(), Some(Token::Public) | Some(Token::Private)
                | Some(Token::Protected) | Some(Token::Internal)
                | Some(Token::Static) | Some(Token::Override) | Some(Token::Open))
            {
                self.advance();
            }
            self.expect(Token::Func)?;
            let m_name = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // 跳过方法的泛型参数 <T>
            if self.check(&Token::Lt) {
                self.advance();
                let mut depth = 1;
                while depth > 0 {
                    match self.advance() {
                        Some(Token::Lt) => depth += 1,
                        Some(Token::Gt) => depth -= 1,
                        None => return self.bail(ParseError::UnexpectedEof),
                        _ => {}
                    }
                }
            }
            self.expect(Token::LParen)?;
            let params = self.parse_params()?;
            self.expect(Token::RParen)?;
            let return_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            // 判断有无默认实现 { body } 或者纯签名（分号可选）
            let default_body = if self.check(&Token::LBrace) {
                self.advance();
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Some(body)
            } else {
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                None
            };
            methods.push(crate::ast::InterfaceMethod {
                name: m_name,
                params,
                return_type,
                default_body,
            });
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        Ok(crate::ast::InterfaceDef {
            visibility,
            name,
            type_params,
            constraints,
            parents,
            methods,
            assoc_types,
        })
    }

    /// 解析 extend 定义
    /// extend TypeName: InterfaceName { type Element = ConcreteType; func method(...): ... { ... } }
    fn parse_extend(&mut self) -> Result<crate::ast::ExtendDef, ParseErrorAt> {
        self.expect(Token::Extend)?;
        // C1.3: 泛型 extend<T> Foo<T> <: Bar where T <: Baz { ... }
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        let target_type = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(Token::TypeString) => "String".to_string(),
            Some(Token::TypeBool) => "Bool".to_string(),
            Some(Token::TypeInt64) => "Int64".to_string(),
            Some(Token::TypeInt32) => "Int32".to_string(),
            Some(Token::TypeInt16) => "Int16".to_string(),
            Some(Token::TypeInt8) => "Int8".to_string(),
            Some(Token::TypeFloat64) => "Float64".to_string(),
            Some(Token::TypeFloat32) => "Float32".to_string(),
            Some(Token::TypeFloat16) => "Float16".to_string(),
            Some(Token::TypeUInt8) => "UInt8".to_string(),
            Some(Token::TypeUInt16) => "UInt16".to_string(),
            Some(Token::TypeUInt32) => "UInt32".to_string(),
            Some(Token::TypeUInt64) => "UInt64".to_string(),
            Some(Token::TypeRune) => "Rune".to_string(),
            Some(tok) => {
                self.current_type_params = prev_params;
                return self.bail(ParseError::UnexpectedToken(tok, "类型名".to_string()));
            }
            None => {
                self.current_type_params = prev_params;
                return self.bail(ParseError::UnexpectedEof);
            }
        };
        // skip optional type args on the target (e.g. Foo<T> or Foo<Bar<T>>)
        if self.check(&Token::Lt) {
            self.advance();
            let mut depth = 1;
            while depth > 0 {
                match self.advance() {
                    Some(Token::Lt) => depth += 1,
                    Some(Token::Gt) => depth -= 1,
                    Some(Token::Shr) => depth -= 2,
                    None => break,
                    _ => {}
                }
            }
        }
        // 可选: <: 实现的接口 (或 : )
        let interface = if self.check(&Token::SubType) {
            self.advance();
            Some(match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => {
                    self.current_type_params = prev_params;
                    return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string()));
                }
                None => {
                    self.current_type_params = prev_params;
                    return self.bail(ParseError::UnexpectedEof);
                }
            })
        } else if self.check(&Token::Colon) {
            self.advance();
            Some(match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => {
                    self.current_type_params = prev_params;
                    return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string()));
                }
                None => {
                    self.current_type_params = prev_params;
                    return self.bail(ParseError::UnexpectedEof);
                }
            })
        } else {
            None
        };
        // skip generic args on interface (e.g. IJsonSerializable<ArrayList<T>>)
        if self.check(&Token::Lt) {
            self.advance();
            let mut depth = 1;
            while depth > 0 {
                match self.advance() {
                    Some(Token::Lt) => depth += 1,
                    Some(Token::Gt) => depth -= 1,
                    Some(Token::Shr) => depth -= 2,
                    None => break,
                    _ => {}
                }
            }
        }
        // 解析可选 where 子句
        if self.check(&Token::Where) {
            self.advance();
            loop {
                let param_name = match self.advance_ident() {
                    Some(n) => n,
                    None => break,
                };
                if self.check(&Token::SubType) {
                    self.advance();
                } else if self.check(&Token::Colon) {
                    self.advance();
                }
                let mut bounds = Vec::new();
                loop {
                    match self.advance_ident() {
                        Some(b) => bounds.push(b),
                        None => break,
                    }
                    // 跳过约束上的泛型参数 Equatable<K>, IJsonValueSerializable<V> 等
                    if self.check(&Token::Lt) {
                        self.advance();
                        let mut depth = 1;
                        while depth > 0 {
                            match self.advance() {
                                Some(Token::Lt) => depth += 1,
                                Some(Token::Gt) => depth -= 1,
                                Some(Token::Shr) => depth -= 2,
                                None => break,
                                _ => {}
                            }
                        }
                    }
                    if self.check(&Token::And) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                constraints.push(TypeConstraint { param: param_name, bounds });
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        let mut assoc_type_bindings = Vec::new();
        while !self.check(&Token::RBrace) {
            // 关联类型绑定: type Element = ConcreteType; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Assign)?;
                let ty = self.parse_type()?;
                self.expect(Token::Semicolon)?;
                assoc_type_bindings.push((type_name, ty));
                continue;
            }
            // 跳过注解 @AnnotationName
            while self.check(&Token::At) {
                self.advance(); // consume @
                if let Some(Token::Ident(_)) = self.peek() {
                    self.advance(); // consume annotation name
                }
                // 跳过可选的注解参数 (...)
                if self.check(&Token::LParen) {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LParen) => depth += 1,
                            Some(Token::RParen) => depth -= 1,
                            None => break,
                            _ => {}
                        }
                    }
                }
            }
            // 跳过可选的可见性/static/override 修饰符
            let mut method_vis = Visibility::Public;
            let mut is_static_method = false;
            loop {
                match self.peek() {
                    Some(Token::Public) => { self.advance(); method_vis = Visibility::Public; }
                    Some(Token::Private) => { self.advance(); method_vis = Visibility::Private; }
                    Some(Token::Protected) => { self.advance(); method_vis = Visibility::Protected; }
                    Some(Token::Internal) => { self.advance(); method_vis = Visibility::Internal; }
                    Some(Token::Override) | Some(Token::Open) => { self.advance(); }
                    Some(Token::Static) => { self.advance(); is_static_method = true; }
                    _ => break,
                }
            }
            // 方法: func name(args): Ret { body }
            let prev_receiver = self.receiver_name.clone();
            self.receiver_name = Some("this".to_string());
            let mut func = self.parse_function_with_visibility(method_vis)?;
            self.receiver_name = prev_receiver;
            let has_self = func.params.iter().any(|p| p.name == "self" || p.name == "this");
            if !has_self && !is_static_method {
                func.params.insert(0, crate::ast::Param {
                    name: "this".to_string(),
                    ty: Type::Struct(target_type.clone(), vec![]),
                    default: None,
                    variadic: false, is_named: false, is_inout: false,
                });
            }
            // 重命名为 TypeName.methodName 格式
            let method_name = if func.name.contains('.') {
                func.name.clone()
            } else {
                format!("{}.{}", target_type, func.name)
            };
            methods.push(Function {
                name: method_name,
                ..func
            });
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        Ok(crate::ast::ExtendDef {
            target_type,
            type_params,
            constraints,
            interface,
            assoc_type_bindings,
            methods,
        })
    }

    /// 解析类定义
    fn parse_class_with_visibility(&mut self, visibility: Visibility) -> Result<crate::ast::ClassDef, ParseErrorAt> {
        // 解析可选修饰符: abstract / sealed / open
        let is_abstract = self.check(&Token::Abstract);
        if is_abstract { self.advance(); }
        let is_sealed = self.check(&Token::Sealed);
        if is_sealed { self.advance(); }
        let is_open = self.check(&Token::Open);
        if is_open { self.advance(); }
        self.expect(Token::Class)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "类名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 解析可选的泛型类型参数 <T, U: Bound, ...>
        let (type_params, constraints) = self.parse_type_params_with_constraints()?;
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // cjc: 使用 <: 表示继承 (class Foo <: Base & Interface1 & Interface2)
        let (extends, implements) = if self.check(&Token::SubType) {
            self.advance();
            let mut types = Vec::new();
            loop {
                let mut type_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "父类或接口名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                // 跳过泛型参数 <T>, <K, V> 等
                if self.check(&Token::Lt) {
                    type_name.push('<');
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::Lt) => { depth += 1; type_name.push('<'); }
                            Some(Token::Shr) => { depth -= 2; if depth > 0 { type_name.push('>'); } }
                            Some(Token::Gt) => { depth -= 1; if depth > 0 { type_name.push('>'); } }
                            Some(Token::Comma) => type_name.push(','),
                            Some(Token::Ident(n)) => type_name.push_str(&n),
                            Some(_) => {}
                            None => return self.bail(ParseError::UnexpectedEof),
                        }
                    }
                    type_name.push('>');
                }
                types.push(type_name);
                if self.check(&Token::And) {
                    self.advance();
                } else {
                    break;
                }
            }
            // 第一个为 extends（父类），其余为 implements（接口）
            if types.is_empty() {
                (None, Vec::new())
            } else {
                (Some(types[0].clone()), types[1..].to_vec())
            }
        } else {
            (None, Vec::new())
        };
        // P6: Primary constructor — class Foo(var x: Int64, var y: Int64) { ... }
        let mut primary_ctor_params = Vec::new();
        if self.check(&Token::LParen) {
            self.advance();
            while !self.check(&Token::RParen) {
                // 跳过可选的 var/let
                if self.check(&Token::Var) || self.check(&Token::Let) {
                    self.advance();
                }
                let pname = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                    }
                };
                self.expect(Token::Colon)?;
                let pty = self.parse_type()?;
                let pdefault = if self.check(&Token::Assign) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                primary_ctor_params.push(Param {
                    name: pname,
                    ty: pty,
                    default: pdefault,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                });
                if !self.check(&Token::Comma) { break; }
                self.advance();
            }
            self.expect(Token::RParen)?;
        }
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut init = None;
        let mut deinit = None;
        let mut static_init = None;
        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) {
            // 跳过类成员上的宏装饰符（如 @JsonCust, @JsonName, @JsonIgnore 等）
            while self.check(&Token::At) {
                self.advance(); // consume @
                if let Some(Token::Ident(_)) = self.peek() {
                    self.advance(); // consume macro name
                    // 跳过可选的宏参数 [args] 或 (args)
                    if self.check(&Token::LBracket) {
                        self.advance();
                        let mut depth = 1;
                        while depth > 0 {
                            match self.advance() {
                                Some(Token::LBracket) => depth += 1,
                                Some(Token::RBracket) => depth -= 1,
                                None => break,
                                _ => {}
                            }
                        }
                    } else if self.check(&Token::LParen) {
                        self.advance();
                        let mut depth = 1;
                        while depth > 0 {
                            match self.advance() {
                                Some(Token::LParen) => depth += 1,
                                Some(Token::RParen) => depth -= 1,
                                None => break,
                                _ => {}
                            }
                        }
                    }
                }
            }
            let member_vis = if self.check(&Token::Private) {
                self.advance();
                crate::ast::Visibility::Private
            } else if self.check(&Token::Public) {
                self.advance();
                crate::ast::Visibility::Public
            } else if self.check(&Token::Protected) {
                self.advance();
                crate::ast::Visibility::Protected
            } else if self.check(&Token::Internal) {
                self.advance();
                crate::ast::Visibility::Internal
            } else {
                crate::ast::Visibility::default()
            };
            // C1.2: static let/var 字段
            let is_static = if self.check(&Token::Static) {
                self.advance();
                true
            } else {
                false
            };
            if self.check(&Token::Var) || self.check(&Token::Let) || self.check(&Token::Const) {
                self.advance();
                let f_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    self.parse_type()?
                } else {
                    Type::Unit
                };
                let default = if self.check(&Token::Assign) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                fields.push(crate::ast::FieldDef {
                    name: f_name,
                    ty,
                    default,
                    is_static,
                });
            } else if is_static && self.check(&Token::Init) {
                // P3.11: static init() — 静态初始化块
                self.advance(); // consume init
                self.expect(Token::LParen)?;
                self.expect(Token::RParen)?;
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                static_init = Some(body);
            } else if self.check(&Token::Init) {
                self.advance();
                self.expect(Token::LParen)?;
                let params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // init 体内 this 可用，指向当前正在构造的对象
                self.receiver_name = Some("this".to_string());
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.receiver_name = None;
                init = Some(crate::ast::InitDef { params, body });
            } else if self.check(&Token::Tilde) {
                // cjc: ~init 析构函数
                self.advance(); // consume ~
                self.expect(Token::Init)?; // consume init
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                deinit = Some(body);
            } else if self.check(&Token::Prop) {
                // prop name: Type { get() { ... } set(value) { ... } }
                self.advance();
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                self.expect(Token::LBrace)?;
                // 解析 get/set 块
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            // 脱糖为 getter 方法: ClassName.__get_propName(this): Type
                            methods.push(crate::ast::ClassMethod {
                                override_: false,
                                func: crate::ast::Function {
                                    visibility: member_vis.clone(),
                                    name: format!("{}.__get_{}", name, prop_name),
                                    type_params: vec![],
                                    constraints: vec![],
                                    params: vec![Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(name.clone(), vec![]),
                                        default: None,
                                        variadic: false, is_named: false, is_inout: false,
                                    }],
                                    return_type: Some(prop_ty.clone()),
                                    throws: None,
                                    body,
                                    extern_import: None,
                                },
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "setter 参数名".to_string())),
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            // 脱糖为 setter 方法: ClassName.__set_propName(this, value)
                            methods.push(crate::ast::ClassMethod {
                                override_: false,
                                func: crate::ast::Function {
                                    visibility: member_vis.clone(),
                                    name: format!("{}.__set_{}", name, prop_name),
                                    type_params: vec![],
                                    constraints: vec![],
                                    params: vec![
                                        Param {
                                            name: "this".to_string(),
                                            ty: Type::Struct(name.clone(), vec![]),
                                            default: None,
                                            variadic: false, is_named: false, is_inout: false,
                                        },
                                        Param {
                                            name: val_name,
                                            ty: prop_ty.clone(),
                                            default: None,
                                            variadic: false, is_named: false, is_inout: false,
                                        },
                                    ],
                                    return_type: None,
                                    throws: None,
                                    body,
                                    extern_import: None,
                                },
                            });
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Ident(kw), "get 或 set".to_string(),
                            ));
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
            } else if matches!(self.peek(), Some(Token::Ident(ref n)) if n == &name) && matches!(self.peek_next(), Some(Token::LParen)) {
                // E8: 命名构造函数 ClassName(public let field: Type, ...) { ... }
                self.advance(); // consume class name
                self.expect(Token::LParen)?;
                let ctor_params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // 将参数转为字段
                for p in &ctor_params {
                    fields.push(crate::ast::FieldDef {
                        name: p.name.clone(),
                        ty: p.ty.clone(),
                        default: p.default.clone(),
                        is_static: false,
                    });
                }
                if self.check(&Token::LBrace) {
                    self.advance();
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    init = Some(crate::ast::InitDef { params: ctor_params, body });
                }
            } else if self.check(&Token::Open) || self.check(&Token::Static) || self.check(&Token::Override) || self.check(&Token::Func) || self.check(&Token::Operator) {
                // cjc: open / static / override / operator 修饰符在方法前
                if self.check(&Token::Open) {
                    self.advance(); // 消费 open，cjwasm 不区分 open/非 open
                }
                // P2.4: 记录 static 修饰符
                let is_static = self.check(&Token::Static);
                if is_static {
                    self.advance(); // 消费 static
                    // P3.11: static init() { ... } 静态初始化块
                    if self.check(&Token::Init) {
                        self.advance(); // 消费 init
                        self.expect(Token::LParen)?;
                        self.expect(Token::RParen)?;
                        self.expect(Token::LBrace)?;
                        let body = self.parse_stmts()?;
                        self.expect(Token::RBrace)?;
                        // 编译为类的静态初始化函数 ClassName.__static_init
                        methods.push(crate::ast::ClassMethod {
                            func: Function {
                                visibility: member_vis.clone(),
                                name: format!("{}.__static_init", name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![],
                                return_type: None,
                                throws: None,
                                body,
                                extern_import: None,
                            },
                            override_: false,
                        });
                        continue;
                    }
                }
                let override_ = self.check(&Token::Override);
                if override_ {
                    self.advance();
                }
                // 跳过 override 后可能的 visibility 修饰符 (override public func)
                if matches!(self.peek(), Some(Token::Public) | Some(Token::Private) | Some(Token::Protected) | Some(Token::Internal)) {
                    self.advance();
                }
                // P3.1: operator func +/-/*/==/</>/<=/>=
                let is_operator = self.check(&Token::Operator);
                if is_operator {
                    self.advance(); // 消费 operator
                }
                self.expect(Token::Func)?;
                let (m_name, type_params) = if is_operator {
                    // 运算符方法名：解析运算符 token，转为 __op_xxx
                    let op_name = match self.advance() {
                        Some(Token::Plus) => "op_add",
                        Some(Token::Minus) => "op_sub",
                        Some(Token::Star) => "op_mul",
                        Some(Token::Slash) => "op_div",
                        Some(Token::Percent) => "op_mod",
                        Some(Token::Eq) => "op_eq",
                        Some(Token::NotEq) => "op_ne",
                        Some(Token::Lt) => "op_lt",
                        Some(Token::Gt) => "op_gt",
                        Some(Token::LtEq) => "op_le",
                        Some(Token::GtEq) => "op_ge",
                        Some(Token::LBracket) => {
                            self.expect(Token::RBracket)?;
                            "op_index"
                        }
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "运算符".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    (format!("{}.{}", name, op_name), vec![])
                } else {
                    match self.advance() {
                        Some(Token::Ident(n)) => {
                            let tp = self.parse_type_params()?;
                            (format!("{}.{}", name, n), tp)
                        }
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    }
                };
                // 合并类的泛型参数与方法自身的泛型参数，使类的 T 在方法体和返回类型中可识别为 TypeParam
                let mut merged_type_params = self.current_type_params.clone();
                merged_type_params.extend(type_params.clone());
                let prev_params = std::mem::replace(&mut self.current_type_params, merged_type_params);
                self.expect(Token::LParen)?;
                let mut params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // cjc 兼容: 无 self/this 时添加隐式 this 参数（P2.4: static 方法除外）
                let has_self = params.iter().any(|p| p.name == "self" || p.name == "this");
                if !has_self && !is_static {
                    params.insert(0, crate::ast::Param {
                        name: "this".to_string(),
                        ty: Type::Struct(name.clone(), type_params.iter().map(|t| Type::TypeParam(t.clone())).collect()),
                        default: None,
                        variadic: false, is_named: false, is_inout: false,
                    });
                }
                self.receiver_name = Some(params.first().map(|p| p.name.clone()).unwrap_or_else(|| "this".to_string()));
                let return_type = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // P2.5: 支持抽象方法（无 body，以 ; 结尾）
                let body = if self.check(&Token::Semicolon) || self.check(&Token::RBrace) {
                    // 抽象方法或声明：无函数体
                    if self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    vec![] // 空 body
                } else {
                    self.expect(Token::LBrace)?;
                    let stmts = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    stmts
                };
                self.receiver_name = None;
                self.current_type_params = prev_params;
                methods.push(crate::ast::ClassMethod {
                    override_,
                    func: crate::ast::Function {
                        visibility: member_vis,
                        name: m_name,
                        type_params,
                        constraints: vec![],
                        params,
                        return_type,
                        throws: None,
                        body,
                        extern_import: None,
                    },
                });
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "var、init、~init 或 func".to_string(),
                ));
            }
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        let mut class_def = crate::ast::ClassDef {
            visibility,
            name,
            type_params,
            constraints,
            is_abstract,
            is_sealed,
            is_open,
            extends,
            implements,
            fields,
            init,
            deinit,
            static_init,
            methods,
            primary_ctor_params: primary_ctor_params.clone(),
        };
        // P6: 展开主构造函数参数为字段 + init
        if !primary_ctor_params.is_empty() {
            for p in &primary_ctor_params {
                class_def.fields.push(FieldDef {
                    name: p.name.clone(),
                    ty: p.ty.clone(),
                    default: p.default.clone(),
                    is_static: false,
                });
            }
            if class_def.init.is_none() {
                let init_body: Vec<Stmt> = primary_ctor_params.iter().map(|p| {
                    Stmt::Assign {
                        target: AssignTarget::Field {
                            object: "this".to_string(),
                            field: p.name.clone(),
                        },
                        value: Expr::Var(p.name.clone()),
                    }
                }).collect();
                class_def.init = Some(InitDef {
                    params: primary_ctor_params,
                    body: init_body,
                });
            }
        }
        Ok(class_def)
    }

    /// M2: 解析宏函数定义 `macro func name(params): Tokens { body }`
    fn parse_macro_def(&mut self, visibility: Visibility) -> Result<MacroDef, ParseErrorAt> {
        self.expect(Token::Macro)?;
        // C2.2: 支持 `macro func Name(...)` 和 `macro Name(...)` (无 func)
        if self.check(&Token::Func) {
            self.advance();
        }

        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "宏名称".to_string()));
            }
        };

        self.expect(Token::LParen)?;
        let mut params = Vec::new();
        while !self.check(&Token::RParen) {
            let param_name = match self.advance_ident() {
                Some(n) => n,
                None => break,
            };
            self.expect(Token::Colon)?;
            let param_ty = self.parse_type()?;
            params.push(Param {
                name: param_name,
                ty: param_ty,
                default: None,
                variadic: false,
                is_named: false,
                is_inout: false,
            });
            if !self.check(&Token::RParen) {
                self.expect(Token::Comma)?;
            }
        }
        self.expect(Token::RParen)?;

        // 可选返回类型 `: Tokens`
        if self.check(&Token::Colon) {
            self.advance();
            let _ret_ty = self.parse_type()?;
        }

        self.expect(Token::LBrace)?;
        let mut body = Vec::new();
        while !self.check(&Token::RBrace) {
            body.push(self.parse_stmt()?);
        }
        self.expect(Token::RBrace)?;

        Ok(MacroDef {
            visibility,
            name,
            params,
            body,
        })
    }

    /// 解析函数定义（支持方法名 StructName.methodName）
    fn parse_function(&mut self) -> Result<Function, ParseErrorAt> {
        self.parse_function_with_visibility(Visibility::default())
    }

    /// 解析函数定义（带可见性）
    fn parse_function_with_visibility(&mut self, visibility: Visibility) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;

        let (name, type_params, mut constraints) = match self.advance_ident() {
            Some(n) => {
                let (tp, tc) = self.parse_type_params_with_constraints()?;
                let full_name = if self.check(&Token::Dot) {
                    self.advance();
                    let method = match self.advance_ident() {
                        Some(m) => m,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string()));
                        }
                    };
                    format!("{}.{}", n, method)
                } else {
                    n
                };
                (full_name, tp, tc)
            }
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "标识符".to_string()));
            }
        };

        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());

        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;

        let prev_receiver = self.receiver_name.clone();
        // 方法体: 当 name 含 '.' 或首参为 self/this 时，允许 body 内使用 this
        // 当从 struct/enum 内部解析方法时, prev_receiver 已预设为 "this"
        self.receiver_name = if name.contains('.') {
            params.first().map(|p| p.name.clone())
        } else if params.first().map_or(false, |p| p.name == "self" || p.name == "this") {
            params.first().map(|p| p.name.clone())
        } else if prev_receiver.is_some() {
            // 保持从 struct/enum 方法体继承的 receiver（cjc 内部方法无显式 self 参数）
            prev_receiver.clone()
        } else {
            None
        };

        // cjc 没有 throws 关键字，保留为 None
        let throws: Option<String> = None;

        let return_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        // 解析可选的 where 子句
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);

        self.expect(Token::LBrace)?;
        let body = self.parse_stmts()?;
        self.expect(Token::RBrace)?;

        self.receiver_name = prev_receiver;
        self.current_type_params = prev_params;

        Ok(Function {
            visibility,
            name,
            type_params,
            constraints,
            params,
            return_type,
            throws,
            body,
            extern_import: None,
        })
    }

    /// cjc 兼容: 解析 main() { ... } 形式的入口函数（无需 func 关键字）
    fn parse_main_function(&mut self, visibility: Visibility) -> Result<Function, ParseErrorAt> {
        // consume "main"
        self.advance();
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;

        let return_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(Token::LBrace)?;
        let body = self.parse_stmts()?;
        self.expect(Token::RBrace)?;

        Ok(Function {
            visibility,
            name: "main".to_string(),
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: None,
            body,
            extern_import: None,
        })
    }

    /// 解析参数列表
    fn parse_params(&mut self) -> Result<Vec<Param>, ParseErrorAt> {
        let mut params = Vec::new();
        if self.check(&Token::RParen) {
            return Ok(params);
        }

        loop {
            // 跳过可选的可见性修饰符 (public let x: T 模式)
            if matches!(self.peek(), Some(Token::Public) | Some(Token::Private) | Some(Token::Protected) | Some(Token::Internal)) {
                self.advance();
            }
            // 跳过可选的 let/var
            if matches!(self.peek(), Some(Token::Let) | Some(Token::Var)) {
                self.advance();
            }
            // P6: inout 参数
            let is_inout = if self.check(&Token::Inout) {
                self.advance();
                true
            } else {
                false
            };
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(Token::This) => "this".to_string(),
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // P2.9: 命名参数 name!: Type = default
            let is_named = if self.check(&Token::Bang) {
                self.advance();
                true
            } else {
                false
            };
            self.expect(Token::Colon)?;
            let ty = self.parse_type()?;
            // 检查是否为可变参数 (Type...)
            let variadic = if self.check(&Token::DotDotDot) {
                self.advance();
                true
            } else {
                false
            };
            let default = if self.check(&Token::Assign) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param { name, ty, default, variadic, is_named, is_inout });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(params)
    }

    /// 解析类型（含 ?T 前缀和 T? 后缀 → Option<T>）
    fn parse_type(&mut self) -> Result<Type, ParseErrorAt> {
        // P2.1: ?T 前缀语法 → Option<T>
        if self.check(&Token::Question) {
            self.advance();
            let inner = self.parse_base_type()?;
            return Ok(Type::Option(Box::new(inner)));
        }
        let mut ty = self.parse_base_type()?;
        // T? 后缀也是 Option<T> 的语法糖
        while self.check(&Token::Question) {
            self.advance();
            ty = Type::Option(Box::new(ty));
        }
        // T! 是非空断言（当前为恒等）
        while self.check(&Token::Bang) {
            self.advance();
        }
        Ok(ty)
    }

    /// 解析基础类型（不含 ? ! 后缀）
    fn parse_base_type(&mut self) -> Result<Type, ParseErrorAt> {
        match self.advance() {
            Some(Token::TypeInt8) => Ok(Type::Int8),
            Some(Token::TypeInt16) => Ok(Type::Int16),
            Some(Token::TypeInt32) => Ok(Type::Int32),
            Some(Token::TypeInt64) => Ok(Type::Int64),
            Some(Token::TypeIntNative) => Ok(Type::IntNative),
            Some(Token::TypeUInt8) => Ok(Type::UInt8),
            Some(Token::TypeUInt16) => Ok(Type::UInt16),
            Some(Token::TypeUInt32) => Ok(Type::UInt32),
            Some(Token::TypeUInt64) => Ok(Type::UInt64),
            Some(Token::TypeUIntNative) => Ok(Type::UIntNative),
            Some(Token::TypeFloat16) => Ok(Type::Float16),
            Some(Token::TypeFloat32) => Ok(Type::Float32),
            Some(Token::TypeFloat64) => Ok(Type::Float64),
            Some(Token::TypeRune) => Ok(Type::Rune),
            Some(Token::TypeBool) => Ok(Type::Bool),
            // P2.3: (T1, T2, ...) -> R 函数类型 或 (T1, T2, ...) 元组类型
            Some(Token::LParen) => {
                let mut types = Vec::new();
                if !self.check(&Token::RParen) {
                    types.push(self.parse_type()?);
                    while self.check(&Token::Comma) {
                        self.advance();
                        types.push(self.parse_type()?);
                    }
                }
                self.expect(Token::RParen)?;
                // 如果后面跟着 ->，则为函数类型；否则为元组类型
                if self.check(&Token::Arrow) {
                    self.advance(); // consume ->
                    let ret = self.parse_type()?;
                    Ok(Type::Function {
                        params: types,
                        ret: Box::new(Some(ret)),
                    })
                } else {
                    // 元组类型 (T1, T2, ...) 或单元素括号类型 (T)
                    if types.len() == 1 {
                        // (T) 是括号包裹的类型，不是元组
                        Ok(types.into_iter().next().unwrap())
                    } else {
                        Ok(Type::Tuple(types))
                    }
                }
            }
            Some(Token::TypeNothing) => Ok(Type::Nothing),
            Some(Token::TypeUnit) => Ok(Type::Unit),
            Some(Token::TypeString) => Ok(Type::String),
            Some(Token::TypeArray) => {
                self.expect(Token::Lt)?;
                let elem_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Array(Box::new(elem_type)))
            }
            Some(Token::TypeTuple) => {
                // Tuple<T1, T2, ...> 语法
                self.expect(Token::Lt)?;
                let mut types = Vec::new();
                loop {
                    types.push(self.parse_type()?);
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else if self.try_consume_gt() {
                        break;
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
                Ok(Type::Tuple(types))
            }
            Some(Token::TypeRange) => Ok(Type::Range),
            Some(Token::TypeOption) => {
                self.expect(Token::Lt)?;
                let inner_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Option(Box::new(inner_type)))
            }
            Some(Token::TypeResult) => {
                self.expect(Token::Lt)?;
                let ok_type = self.parse_type()?;
                self.expect(Token::Comma)?;
                let err_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Result(Box::new(ok_type), Box::new(err_type)))
            }
            Some(Token::TypeSlice) => {
                self.expect(Token::Lt)?;
                let elem_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Slice(Box::new(elem_type)))
            }
            Some(Token::TypeMap) => {
                self.expect(Token::Lt)?;
                let key_type = self.parse_type()?;
                self.expect(Token::Comma)?;
                let val_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Map(Box::new(key_type), Box::new(val_type)))
            }
            Some(Token::Ident(name)) => {
                // C2.3: Tokens 宏上下文类型
                if name == "Tokens" {
                    return Ok(Type::Tokens);
                }
                if self.check(&Token::Lt) {
                    self.advance();
                    let mut type_args = Vec::new();
                    loop {
                        type_args.push(self.parse_type()?);
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else if self.try_consume_gt() {
                            break;
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `>`".to_string(),
                            ));
                        }
                    }
                    Ok(Type::Struct(name, type_args))
                } else if self.current_type_params.contains(&name) {
                    Ok(Type::TypeParam(name))
                } else if let Some(alias_ty) = self.type_aliases.get(&name).cloned() {
                    Ok(alias_ty)
                } else {
                    Ok(Type::Struct(name, vec![]))
                }
            }
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok, "类型".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 将语句列表转换为 Expr：
    /// - 空列表 → Expr::Integer(0) 回退值
    /// - 只有一条 Expr 语句且无前置语句 → 直接使用该表达式
    /// - 最后一条是 Expr 语句 → Block(前置语句, Some(最后的表达式))
    /// - 其他情况 → Block(所有语句, None)，保留 return/let 等
    fn stmts_to_block_expr(stmts: Vec<Stmt>) -> Box<Expr> {
        if stmts.is_empty() {
            return Box::new(Expr::Integer(0));
        }
        // 检查最后一条是否是 Expr 语句
        let last_is_expr = matches!(stmts.last(), Some(Stmt::Expr(_)));
        if last_is_expr {
            let mut stmts = stmts;
            let last = stmts.pop().unwrap();
            let result = if let Stmt::Expr(e) = last { Some(Box::new(e)) } else { unreachable!() };
            if stmts.is_empty() {
                // 单个表达式，直接返回（兼容已有行为）
                result.unwrap()
            } else {
                Box::new(Expr::Block(stmts, result))
            }
        } else {
            // 最后一条不是表达式（如 return、let 等），用 Block 包装保留所有语句
            Box::new(Expr::Block(stmts, None))
        }
    }

    /// 解析语句列表
    fn parse_stmts(&mut self) -> Result<Vec<Stmt>, ParseErrorAt> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && self.peek().is_some() {
            // 跳过可选的分号 (cjc 兼容)
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// 解析语句
    fn parse_stmt(&mut self) -> Result<Stmt, ParseErrorAt> {
        match self.peek() {
            Some(Token::Let) => {
                self.advance();
                // P1.3: 支持 let _ = expr 通配符赋值
                if self.check(&Token::Underscore) {
                    self.advance();
                    let ty = if self.check(&Token::Colon) {
                        self.advance();
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    self.expect(Token::Assign)?;
                    let value = self.parse_expr()?;
                    return Ok(Stmt::Let { pattern: Pattern::Wildcard, ty, value });
                }
                let first = match self.advance() {
                    Some(Token::Ident(name)) => name,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "变量名或类型名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                let pattern = if self.check(&Token::LBrace) {
                    self.advance();
                    let mut fields = Vec::new();
                    while !self.check(&Token::RBrace) {
                        let fname = match self.advance() {
                            Some(Token::Ident(n)) => n,
                            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                            None => return self.bail(ParseError::UnexpectedEof),
                        };
                        let binding = if self.check(&Token::Colon) {
                            self.advance();
                            match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "绑定名".to_string())),
                                None => return self.bail(ParseError::UnexpectedEof),
                            }
                        } else {
                            fname.clone()
                        };
                        fields.push((fname, Pattern::Binding(binding)));
                        if !self.check(&Token::RBrace) {
                            self.expect(Token::Comma)?;
                        }
                    }
                    self.expect(Token::RBrace)?;
                    Pattern::Struct { name: first, fields }
                } else {
                    Pattern::Binding(first)
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    self.parse_expr()?
                } else {
                    Expr::Tuple(vec![])
                };
                Ok(Stmt::Let { pattern, ty, value })
            }
            Some(Token::Var) => {
                self.advance();
                let name = match self.advance() {
                    Some(Token::Ident(name)) => name,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "变量名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    self.parse_expr()?
                } else {
                    // 无初始值: var x: Type — 使用默认值
                    Expr::Tuple(vec![])
                };
                Ok(Stmt::Var { name, ty, value })
            }
            Some(Token::Return) => {
                self.advance();
                if self.check(&Token::RBrace) {
                    Ok(Stmt::Return(None))
                } else {
                    Ok(Stmt::Return(Some(self.parse_expr()?)))
                }
            }
            Some(Token::While) => {
                self.advance();
                // cjc 兼容: while (let pattern <- expr) 或 while let pattern = expr
                let is_paren_let = self.check(&Token::LParen) && matches!(self.peek_next(), Some(Token::Let));
                let is_let = self.check(&Token::Let);
                if is_paren_let {
                    self.advance(); // consume (
                    self.advance(); // consume let
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Assign), "`=` 或 `<-`".to_string()));
                    }
                    self.advance();
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::WhileLet { pattern, expr, body })
                } else if is_let {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Assign), "`=` 或 `<-`".to_string()));
                    }
                    self.advance();
                    // 使用受限表达式解析，避免 { 被误认为结构体初始化
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::WhileLet { pattern, expr, body })
                } else {
                    let cond = self.parse_expr()?;
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::While { cond, body })
                }
            }
            Some(Token::For) => {
                self.advance();
                // 支持 for (i in x) 和 for i in x 两种语法 (cjc 兼容)
                let has_paren = self.check(&Token::LParen);
                if has_paren {
                    self.advance();
                }
                // 支持单变量 for (x in ...) 和元组解构 for ((x, y) in ...)
                let var = if self.check(&Token::LParen) {
                    // 元组解构: ((x, y) in ...) — 消费 ( 和内部变量, 直到 )
                    self.advance(); // consume (
                    let mut vars = Vec::new();
                    loop {
                        match self.advance() {
                            Some(Token::Ident(n)) => vars.push(n),
                            Some(Token::Underscore) => vars.push("_".to_string()),
                            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "循环变量名".to_string())),
                            None => return self.bail(ParseError::UnexpectedEof),
                        }
                        if !self.check(&Token::Comma) { break; }
                        self.advance();
                    }
                    self.expect(Token::RParen)?;
                    vars.join(",")
                } else {
                    match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "循环变量名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    }
                };
                self.expect(Token::In)?;
                let iterable = if has_paren {
                    let e = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    e
                } else {
                    self.parse_for_iterable()?
                };
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Stmt::For { var, iterable, body })
            }
            Some(Token::Break) => {
                self.advance();
                Ok(Stmt::Break)
            }
            Some(Token::Continue) => {
                self.advance();
                Ok(Stmt::Continue)
            }
            Some(Token::Loop) => {
                self.advance();
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Stmt::Loop { body })
            }
            // P6: do-while 循环
            Some(Token::Do) => {
                self.advance();
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.expect(Token::While)?;
                // cjc 兼容: while (cond) 或 while cond
                let has_paren = self.check(&Token::LParen);
                if has_paren { self.advance(); }
                let cond = self.parse_expr()?;
                if has_paren { self.expect(Token::RParen)?; }
                Ok(Stmt::DoWhile { body, cond })
            }
            // P6: const 声明
            Some(Token::Const) => {
                self.advance();
                // const var x = ... 或 const x: Type = ...
                if self.check(&Token::Var) || self.check(&Token::Let) {
                    self.advance(); // skip optional var/let
                }
                let name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "常量名".to_string()));
                    }
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(Token::Assign)?;
                let value = self.parse_expr()?;
                Ok(Stmt::Const { name, ty, value })
            }
            Some(Token::At) => {
                let (at_start, _) = self.at();
                self.advance(); // consume @
                match self.peek().cloned() {
                    Some(Token::Ident(name)) if name == "Assert" || name == "Expect" => {
                        let is_assert = name == "Assert";
                        self.advance(); // consume Assert/Expect
                        self.expect(Token::LParen)?;
                        let left = self.parse_expr()?;
                        // 双参数形式 @Assert(a, b) 或单参数形式 @Assert(cond)
                        let right = if self.check(&Token::Comma) {
                            self.advance();
                            self.parse_expr()?
                        } else {
                            Expr::Bool(true)
                        };
                        self.expect(Token::RParen)?;
                        if is_assert {
                            Ok(Stmt::Assert { left, right, line: at_start })
                        } else {
                            Ok(Stmt::Expect { left, right, line: at_start })
                        }
                    }
                    // M2: 用户自定义宏调用 @MacroName[args] 或 @MacroName(args)
                    Some(Token::Ident(name)) => {
                        let macro_name = name.clone();
                        self.advance(); // consume macro name
                        let mut args = Vec::new();
                        if self.check(&Token::LBracket) {
                            // @MacroName[arg1, arg2, ...]
                            self.advance();
                            while !self.check(&Token::RBracket) {
                                args.push(self.parse_expr()?);
                                if !self.check(&Token::RBracket) {
                                    let _ = self.check(&Token::Comma) && { self.advance(); true };
                                }
                            }
                            self.expect(Token::RBracket)?;
                        } else if self.check(&Token::LParen) {
                            // @MacroName(arg1, arg2, ...)
                            self.advance();
                            while !self.check(&Token::RParen) {
                                args.push(self.parse_expr()?);
                                if !self.check(&Token::RParen) {
                                    let _ = self.check(&Token::Comma) && { self.advance(); true };
                                }
                            }
                            self.expect(Token::RParen)?;
                        }
                        Ok(Stmt::MacroExpand { name: macro_name, args })
                    }
                    _ => {
                        self.bail(ParseError::UnexpectedToken(Token::At, "@Assert、@Expect 或 @宏名".to_string()))
                    }
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                // 检查是否是赋值或复合赋值语句
                let (is_assign, bin_op) = match self.peek() {
                    Some(Token::Assign) => (true, None),
                    Some(Token::PlusEq) => (true, Some(BinOp::Add)),
                    Some(Token::MinusEq) => (true, Some(BinOp::Sub)),
                    Some(Token::StarEq) => (true, Some(BinOp::Mul)),
                    Some(Token::SlashEq) => (true, Some(BinOp::Div)),
                    Some(Token::PercentEq) => (true, Some(BinOp::Mod)),
                    _ => (false, None),
                };
                if is_assign {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    let target = self.expr_to_assign_target(expr)?;
                    let value = match bin_op {
                        None => rhs,
                        Some(op) => Expr::Binary {
                            op,
                            left: Box::new(self.assign_target_to_expr(&target)),
                            right: Box::new(rhs),
                        },
                    };
                    return Ok(Stmt::Assign { target, value });
                }
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// 将赋值目标转回表达式（用于复合赋值的 RHS 展开：x += 1 => x = x + 1）
    fn assign_target_to_expr(&self, target: &AssignTarget) -> Expr {
        match target {
            AssignTarget::Var(name) => Expr::Var(name.clone()),
            AssignTarget::Index { array, index } => Expr::Index {
                array: Box::new(Expr::Var(array.clone())),
                index: index.clone(),
            },
            AssignTarget::Field { object, field } => Expr::Field {
                object: Box::new(Expr::Var(object.clone())),
                field: field.clone(),
            },
            AssignTarget::OptionalChainField { object, field } => Expr::OptionalChain {
                object: object.clone(),
                field: field.clone(),
            },
        }
    }

    /// 将表达式转换为赋值目标
    fn expr_to_assign_target(&self, expr: Expr) -> Result<AssignTarget, ParseErrorAt> {
        match expr {
            Expr::Var(name) => Ok(AssignTarget::Var(name)),
            Expr::Index { array, index } => {
                if let Expr::Var(name) = *array {
                    Ok(AssignTarget::Index {
                        array: name,
                        index,
                    })
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Assign,
                        "简单数组访问".to_string(),
                    ))
                }
            }
            Expr::Field { object, field } => {
                if let Expr::Var(name) = *object {
                    Ok(AssignTarget::Field {
                        object: name,
                        field,
                    })
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Assign,
                        "简单字段访问".to_string(),
                    ))
                }
            }
            Expr::OptionalChain { object, field } => {
                // 可选链赋值: obj?.field = value
                Ok(AssignTarget::OptionalChainField { object, field })
            }
            _ => {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Assign,
                    "可赋值的目标".to_string(),
                ))
            }
        }
    }

    /// 解析表达式（顶层为空值合并）
    fn parse_expr(&mut self) -> Result<Expr, ParseErrorAt> {
        self.parse_null_coalesce()
    }

    /// 解析空值合并 (??)
    fn parse_null_coalesce(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_logical_or()?;
        while matches!(self.peek(), Some(Token::QuestionQuestion)) {
            self.advance();
            let right = self.parse_logical_or()?;
            left = Expr::NullCoalesce {
                option: Box::new(left),
                default: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析逻辑或 (||)
    fn parse_logical_or(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_logical_and()?;
        while matches!(self.peek(), Some(Token::OrOr)) {
            self.advance();
            let right = self.parse_logical_and()?;
            left = Expr::Binary {
                op: BinOp::LogicalOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析逻辑与 (&&)
    fn parse_logical_and(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek(), Some(Token::AndAnd)) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op: BinOp::LogicalAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析比较表达式
    fn parse_comparison(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_bitwise_or()?;

        // P3.4: 先检查 `is` 关键字 — expr is Type
        if self.check(&Token::Is) {
            self.advance();
            let target_ty = self.parse_type()?;
            return Ok(Expr::IsType {
                expr: Box::new(left),
                target_ty,
            });
        }

        // P6: `!in` 运算符 — expr !in collection
        if self.check(&Token::Bang) {
            if matches!(self.peek_next(), Some(Token::In)) {
                self.advance(); // consume !
                self.advance(); // consume in
                let right = self.parse_bitwise_or()?;
                return Ok(Expr::Binary {
                    op: BinOp::NotIn,
                    left: Box::new(left),
                    right: Box::new(right),
                });
            }
        }

        while let Some(op) = match self.peek() {
            Some(Token::Eq) => Some(BinOp::Eq),
            Some(Token::NotEq) => Some(BinOp::NotEq),
            Some(Token::Lt) => {
                // 启发式区分泛型 Array<T> 与比较 a < b：
                // 如果 < 后是类型关键字，视为泛型（返回 None，不当作比较）
                // 如果 < 后是 Ident，再看其后第三个 token 是否 > 或 , （泛型上下文）
                let next = self.peek_next();
                let is_type_keyword = matches!(next, Some(Token::TypeInt64 | Token::TypeInt32 | Token::TypeFloat64 | Token::TypeFloat32 | Token::TypeBool | Token::TypeString
                    | Token::TypeInt8 | Token::TypeInt16 | Token::TypeUInt8 | Token::TypeUInt16 | Token::TypeUInt32 | Token::TypeUInt64 | Token::TypeRune));
                let is_generic_ident = matches!(next, Some(Token::Ident(_)))
                    && matches!(self.peek_at(2), Some(Token::Gt | Token::Comma));
                if is_type_keyword || is_generic_ident {
                    None
                } else {
                    Some(BinOp::Lt)
                }
            }
            Some(Token::Gt) => Some(BinOp::Gt),
            Some(Token::LtEq) => Some(BinOp::LtEq),
            Some(Token::GtEq) => Some(BinOp::GtEq),
            _ => None,
        } {
            self.advance();
            let right = self.parse_bitwise_or()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// 解析按位或 |
    fn parse_bitwise_or(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_bitwise_xor()?;
        while matches!(self.peek(), Some(Token::Pipe)) {
            self.advance();
            let right = self.parse_bitwise_xor()?;
            left = Expr::Binary {
                op: BinOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析按位异或 ^
    fn parse_bitwise_xor(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_bitwise_and()?;
        while matches!(self.peek(), Some(Token::Caret)) {
            self.advance();
            let right = self.parse_bitwise_and()?;
            left = Expr::Binary {
                op: BinOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析按位与 &
    fn parse_bitwise_and(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_shift()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::Binary {
                op: BinOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析移位 << >> >>>
    fn parse_shift(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_additive()?;
        while let Some(op) = match self.peek() {
            Some(Token::Shl) => Some(BinOp::Shl),
            Some(Token::Shr) => Some(BinOp::Shr),
            _ => None,
        } {
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析加减法表达式
    fn parse_additive(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_multiplicative()?;

        while let Some(op) = match self.peek() {
            Some(Token::Plus) => Some(BinOp::Add),
            Some(Token::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// 解析乘除法表达式
    fn parse_multiplicative(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_power()?;

        while let Some(op) = match self.peek() {
            Some(Token::Star) => Some(BinOp::Mul),
            Some(Token::Slash) => Some(BinOp::Div),
            Some(Token::Percent) => Some(BinOp::Mod),
            _ => None,
        } {
            self.advance();
            let right = self.parse_power()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// 解析幂运算 (**)，右结合
    fn parse_power(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_unary()?;
        if matches!(self.peek(), Some(Token::StarStar)) {
            self.advance();
            let right = self.parse_power()?;
            left = Expr::Binary {
                op: BinOp::Pow,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析一元表达式 (!, -)
    fn parse_unary(&mut self) -> Result<Expr, ParseErrorAt> {
        if matches!(self.peek(), Some(Token::Bang)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            });
        }
        if matches!(self.peek(), Some(Token::Minus)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
            });
        }
        if matches!(self.peek(), Some(Token::Tilde)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::BitNot,
                expr: Box::new(expr),
            });
        }
        self.parse_postfix()
    }

    /// 解析后缀表达式 (数组访问, 字段访问, 方法调用)
    fn parse_postfix(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.peek() {
                Some(Token::LBracket) => {
                    // 数组访问 arr[index] 或 切片表达式 arr[start..end], arr[start..], arr[..end], arr[..]
                    self.advance();

                    // 检查是否是 [..] 或 [..end] (以 DotDot 开头)
                    if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                        self.advance(); // consume ..
                        let end = if self.check(&Token::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        self.expect(Token::RBracket)?;
                        expr = Expr::SliceExpr {
                            array: Box::new(expr),
                            start: None,
                            end,
                        };
                    } else {
                        // 有起始表达式
                        let start_expr = self.parse_expr()?;

                        // 检查是否 parse_expr 错误地解析为 Range（应该是 SliceExpr）
                        if let Expr::Range { start, end, .. } = start_expr {
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start: Some(start),
                                end: Some(end),
                            };
                        } else if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                            // 检查是否是切片表达式 [start..] 或 [start..end]
                            self.advance(); // consume ..
                            let end = if self.check(&Token::RBracket) {
                                None
                            } else {
                                Some(Box::new(self.parse_expr()?))
                            };
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start: Some(Box::new(start_expr)),
                                end,
                            };
                        } else {
                            // 普通索引访问 [index]
                            self.expect(Token::RBracket)?;
                            expr = Expr::Index {
                                array: Box::new(expr),
                                index: Box::new(start_expr),
                            };
                        }
                    }
                }
                // P6: 可选链 obj?.field / obj?.method()
                Some(Token::Question) if matches!(self.peek_next(), Some(Token::Dot)) => {
                    self.advance(); // consume ?
                    self.advance(); // consume .
                    let field = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                        }
                    };
                    expr = Expr::OptionalChain {
                        object: Box::new(expr),
                        field,
                    };
                }
                Some(Token::Dot) => {
                    // 字段访问、方法调用或元组索引
                    self.advance();
                    // 检查是否为元组索引 (.0, .1, ...)
                    if let Some(Token::Integer(n)) = self.peek() {
                        let idx = *n as u32;
                        self.advance();
                        expr = Expr::TupleIndex {
                            object: Box::new(expr),
                            index: idx,
                        };
                        continue;
                    }
                    let name = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(tok) => {
                            return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };

                    if self.check(&Token::LParen) {
                        // 方法调用 obj.method(args)
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: name,
                            args,
                            named_args,
                        };
                    } else {
                        // 字段访问 obj.field
                        expr = Expr::Field {
                            object: Box::new(expr),
                            field: name,
                        };
                    }
                }
                Some(Token::As) => {
                    self.advance();
                    let target_ty = self.parse_type()?;
                    expr = Expr::Cast {
                        expr: Box::new(expr),
                        target_ty,
                    };
                }
                Some(Token::Question) => {
                    // ? 运算符：expr? 提前返回 Err/None
                    self.advance();
                    expr = Expr::Try(Box::new(expr));
                }
                // P6: Trailing closure — f(args) { params => body }
                // Only after a Call or MethodCall, check for `{ ident =>` pattern
                Some(Token::LBrace) if matches!(&expr, Expr::Call { .. } | Expr::MethodCall { .. }) => {
                    // Peek ahead to check if this looks like a lambda: { ident => ... } or { => ... }
                    let looks_like_lambda = matches!(self.peek_next(), Some(Token::FatArrow))
                        || (matches!(self.peek_next(), Some(Token::Ident(_))) && matches!(self.peek_at(2), Some(Token::FatArrow) | Some(Token::Colon) | Some(Token::Comma)));
                    if looks_like_lambda {
                        // Consume { and parse the lambda body
                        self.advance(); // consume {
                        let closure = if self.check(&Token::FatArrow) {
                            // { => body } — 无参 lambda
                            self.advance();
                            let body = self.parse_expr()?;
                            self.expect(Token::RBrace)?;
                            Expr::Lambda { params: vec![], return_type: None, body: Box::new(body) }
                        } else {
                            // { x: T, y: T => body } — 有参 lambda
                            let params = self.parse_lambda_params()?;
                            self.expect(Token::FatArrow)?;
                            let body = self.parse_expr()?;
                            self.expect(Token::RBrace)?;
                            Expr::Lambda { params, return_type: None, body: Box::new(body) }
                        };
                        match expr {
                            Expr::Call { name, type_args, args, named_args: _ } => {
                                let mut all_args = args;
                                all_args.push(closure);
                                expr = Expr::Call { name, type_args, args: all_args, named_args: vec![] };
                            }
                            Expr::MethodCall { object, method, args, named_args: _ } => {
                                let mut all_args = args;
                                all_args.push(closure);
                                expr = Expr::MethodCall { object, method, args: all_args, named_args: vec![] };
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 解析基础表达式
    fn parse_primary(&mut self) -> Result<Expr, ParseErrorAt> {
        match self.advance() {
            Some(Token::Integer(n)) => {
                // 检查是否是范围表达式 (但在切片上下文 [start..] 中不解析为范围)
                let is_range_operator = self.check(&Token::DotDot) || self.check(&Token::DotDotEq);
                let is_slice_context = self.peek_next() == Some(&Token::RBracket);
                if is_range_operator && !is_slice_context {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    let end = self.parse_primary()?;
                    // P2.6: 可选步长 `: step`
                    let step = if self.check(&Token::Colon) {
                        self.advance();
                        Some(Box::new(self.parse_primary()?))
                    } else {
                        None
                    };
                    return Ok(Expr::Range {
                        start: Box::new(Expr::Integer(n)),
                        end: Box::new(end),
                        inclusive,
                        step,
                    });
                }
                Ok(Expr::Integer(n))
            }
            Some(Token::Float(f)) => Ok(Expr::Float(f)),
            Some(Token::Float32(f)) => Ok(Expr::Float32(f)),
            // 带类型后缀的字面量
            Some(Token::Float32Suffix(f)) => Ok(Expr::Float32(f)),
            Some(Token::Float16Suffix(f)) => Ok(Expr::Float16(f)),
            Some(Token::Int32Suffix(n)) => Ok(Expr::Int32(n)),
            Some(Token::Int16Suffix(n)) => Ok(Expr::Int16(n)),
            Some(Token::Int8Suffix(n)) => Ok(Expr::Int8(n)),
            // 基本类型作为构造/转换函数: Int32(expr), Float16(expr), String(expr) 等
            Some(Token::TypeInt8) => self.parse_type_call("Int8"),
            Some(Token::TypeInt16) => self.parse_type_call("Int16"),
            Some(Token::TypeInt32) => self.parse_type_call("Int32"),
            Some(Token::TypeInt64) => self.parse_type_call("Int64"),
            Some(Token::TypeUInt8) => self.parse_type_call("UInt8"),
            Some(Token::TypeUInt16) => self.parse_type_call("UInt16"),
            Some(Token::TypeUInt32) => self.parse_type_call("UInt32"),
            Some(Token::TypeUInt64) => self.parse_type_call("UInt64"),
            Some(Token::TypeFloat16) => self.parse_type_call("Float16"),
            Some(Token::TypeFloat32) => self.parse_type_call("Float32"),
            Some(Token::TypeFloat64) => self.parse_type_call("Float64"),
            Some(Token::TypeString) => self.parse_type_call("String"),
            Some(Token::TypeBool) => self.parse_type_call("Bool"),
            Some(Token::TypeRune) => self.parse_type_call("Rune"),
            Some(Token::This) => Ok(Expr::Var(self.receiver_name.clone().unwrap_or_else(|| "this".to_string()))),
            Some(Token::Super) => {
                if self.check(&Token::LParen) {
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::SuperCall {
                        method: "init".to_string(),
                        args,
                        named_args,
                    })
                } else if self.check(&Token::Dot) {
                    self.advance();
                    let method = match self.advance() {
                        Some(Token::Ident(m)) => m,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    self.expect(Token::LParen)?;
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::SuperCall { method, args, named_args })
                } else {
                    self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Dot),
                        "super. 或 super(".to_string(),
                    ))
                }
            }
            Some(Token::CharLit(c)) => Ok(Expr::Rune(c)),
            Some(Token::True) => Ok(Expr::Bool(true)),
            Some(Token::False) => Ok(Expr::Bool(false)),
            Some(Token::StringLit(s)) => self.parse_string_or_interpolated(s),
            Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => Ok(Expr::String(s)),
            // Option/Result 构造器
            Some(Token::Some) => {
                self.expect(Token::LParen)?;
                let value = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(Expr::Some(Box::new(value)))
            }
            Some(Token::None) => Ok(Expr::None),
            Some(Token::Ok) => {
                self.expect(Token::LParen)?;
                let value = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(Expr::Ok(Box::new(value)))
            }
            Some(Token::Err) => {
                self.expect(Token::LParen)?;
                let value = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(Expr::Err(Box::new(value)))
            }
            // 类型转换构造函数 T(e) - cjc 兼容 (as 在 cjc 中返回 Option)
            Some(tok) if matches!(tok, Token::TypeInt64 | Token::TypeInt32 | Token::TypeInt16 | Token::TypeInt8
                | Token::TypeUInt64 | Token::TypeUInt32 | Token::TypeUInt16 | Token::TypeUInt8
                | Token::TypeFloat64 | Token::TypeFloat32 | Token::TypeBool) => {
                if self.check(&Token::LParen) {
                    self.advance();
                    let arg = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    let name = match tok {
                        Token::TypeInt64 => "Int64",
                        Token::TypeInt32 => "Int32",
                        Token::TypeInt16 => "Int16",
                        Token::TypeInt8 => "Int8",
                        Token::TypeUInt64 => "UInt64",
                        Token::TypeUInt32 => "UInt32",
                        Token::TypeUInt16 => "UInt16",
                        Token::TypeUInt8 => "UInt8",
                        Token::TypeFloat64 => "Float64",
                        Token::TypeFloat32 => "Float32",
                        Token::TypeBool => "Bool",
                        _ => unreachable!(),
                    };
                    Ok(Expr::Call { name: name.to_string(), type_args: None, args: vec![arg], named_args: vec![] })
                } else {
                    self.bail(ParseError::UnexpectedToken(tok, "类型转换需要 T(expr) 形式".to_string()))
                }
            }
            // throw 表达式
            Some(Token::Throw) => {
                let value = self.parse_expr()?;
                Ok(Expr::Throw(Box::new(value)))
            }
            // try 块（支持 try-catch-finally 和 try-with-resources）
            Some(Token::Try) => {
                // P6: try-with-resources: try (resource = expr) { ... }
                let resources = if self.check(&Token::LParen) {
                    let saved_pos = self.pos;
                    self.advance();
                    // 检查是否是 try (let/var name = expr) 形式
                    if matches!(self.peek(), Some(Token::Let) | Some(Token::Var)) {
                        let mut res = Vec::new();
                        loop {
                            self.advance(); // consume let/var
                            let name = self.advance_ident().ok_or_else(|| ParseErrorAt {
                                error: ParseError::UnexpectedEof,
                                byte_start: self.at().0, byte_end: self.at().1,
                            })?;
                            self.expect(Token::Assign)?;
                            let expr = self.parse_expr()?;
                            res.push((name, expr));
                            if !self.check(&Token::Comma) { break; }
                            self.advance(); // consume comma
                        }
                        self.expect(Token::RParen)?;
                        res
                    } else {
                        // Not try-with-resources, restore position
                        self.pos = saved_pos;
                        vec![]
                    }
                } else {
                    vec![]
                };
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                // catch 是可选的（try-with-resources 或 try-finally 可无 catch）
                if !self.check(&Token::Catch) && !self.check(&Token::Finally) {
                    return Ok(Expr::TryBlock { resources, body, catch_var: None, catch_types: vec![], catch_body: vec![], finally_body: Some(vec![]) });
                }
                if self.check(&Token::Finally) {
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let finally_stmts = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    return Ok(Expr::TryBlock { resources, body, catch_var: None, catch_types: vec![], catch_body: vec![], finally_body: Some(finally_stmts) });
                }
                self.expect(Token::Catch)?;
                let mut catch_var = None;
                let mut catch_types = Vec::new();
                if self.check(&Token::LParen) {
                    self.advance();
                    catch_var = match self.advance() {
                        Some(Token::Ident(v)) => Some(v),
                        Some(Token::Underscore) => Some("_".to_string()),
                        _ => return self.bail(ParseError::UnexpectedEof),
                    };
                    // C1.4: catch (e: TypeA | TypeB) 多类型 catch
                    if self.check(&Token::Colon) {
                        self.advance();
                        loop {
                            let ty_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(tok) => {
                                    // fallback: parse as type
                                    let _ = tok;
                                    break;
                                }
                                None => break,
                            };
                            catch_types.push(ty_name);
                            if self.check(&Token::Pipe) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                }
                self.expect(Token::LBrace)?;
                let catch_body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                let finally_body = if self.check(&Token::Finally) {
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let stmts = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Some(stmts)
                } else {
                    None
                };
                Ok(Expr::TryBlock { resources, body, catch_var, catch_types, catch_body, finally_body })
            }
            Some(Token::Ident(name)) => {
                // 仅当首字母大写时解析为枚举变体 (Color.Red)，否则 . 后续为字段/方法
                // 变体名也必须首字母大写，避免将静态方法 Point.origin() 误解析
                let looks_like_type = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if looks_like_type && self.check(&Token::Dot) {
                    if let Some(Token::Ident(ref v)) = self.peek_next() {
                        let variant_looks_like_type = v.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                        if variant_looks_like_type {
                            self.advance();
                            let variant = match self.advance() {
                                Some(Token::Ident(v)) => v,
                                _ => unreachable!(),
                            };
                            let arg = if self.check(&Token::LParen) {
                                self.advance();
                                if self.check(&Token::RParen) {
                                    self.advance();
                                    None
                                } else {
                                    let e = self.parse_expr()?;
                                    self.expect(Token::RParen)?;
                                    Some(Box::new(e))
                                }
                            } else {
                                None
                            };
                            return Ok(Expr::VariantConst {
                                enum_name: name,
                                variant_name: variant,
                                arg,
                            });
                        }
                    }
                }
                // 解析可选类型实参 (如 identity<Int64> 或 Pair<Int64,String>)
                let type_args = self.parse_opt_type_args()?;
                // 检查是否是函数调用、构造函数调用或结构体初始化
                match self.peek() {
                    Some(Token::LParen) => {
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        // 首字母大写视为构造函数调用，否则为普通函数调用
                        if looks_like_type {
                            Ok(Expr::ConstructorCall { name, type_args, args, named_args })
                        } else {
                            Ok(Expr::Call { name, type_args, args, named_args })
                        }
                    }
                    Some(Token::LBrace) if looks_like_type => {
                        // 仅对类型名（首字母大写）解析为结构体初始化
                        self.advance();
                        let fields = self.parse_struct_fields()?;
                        self.expect(Token::RBrace)?;
                        Ok(Expr::StructInit { name, type_args, fields })
                    }
                    _ => Ok(Expr::Var(name)),
                }
            }
            Some(Token::LParen) => {
                // 检查是否是 Lambda: (x: T, ...): R { body } 或 (): R { body }
                // 通过检查 ): 或 ident : 来判断
                if self.check(&Token::RParen) {
                    // (): R { body } 或空元组
                    self.advance(); // consume )
                    if self.check(&Token::Colon) {
                        return self.parse_lambda_rest(vec![]);
                    }
                    // () 空元组
                    return Ok(Expr::Tuple(vec![]));
                }
                // 检查是否是 (ident : 开头的 Lambda
                if let Some(Token::Ident(_)) = self.peek() {
                    if let Some(Token::Colon) = self.peek_next() {
                        // 这是 Lambda
                        let params = self.parse_lambda_params()?;
                        self.expect(Token::RParen)?;
                        return self.parse_lambda_rest(params);
                    }
                }
                // 解析第一个表达式
                let first = self.parse_expr()?;
                if self.check(&Token::Comma) {
                    // 这是元组字面量 (a, b, ...)
                    self.advance();
                    let mut elements = vec![first];
                    if !self.check(&Token::RParen) {
                        loop {
                            elements.push(self.parse_expr()?);
                            if !self.check(&Token::Comma) {
                                break;
                            }
                            self.advance();
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::Tuple(elements))
                } else {
                    // 普通括号表达式
                    self.expect(Token::RParen)?;
                    Ok(first)
                }
            }
            Some(Token::LBrace) => {
                // { => body } — 无参 lambda
                if self.check(&Token::FatArrow) {
                    self.advance(); // consume =>
                    let body = self.parse_expr()?;
                    self.expect(Token::RBrace)?;
                    return Ok(Expr::Lambda {
                        params: vec![],
                        return_type: None,
                        body: Box::new(body),
                    });
                }
                // { x: T => body } — 有参 lambda
                if let Some(Token::Ident(_)) = self.peek() {
                    if let Some(Token::Colon) = self.peek_next() {
                        let params = self.parse_lambda_params()?;
                        self.expect(Token::FatArrow)?;
                        let body = self.parse_expr()?;
                        self.expect(Token::RBrace)?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None,
                            body: Box::new(body),
                        });
                    }
                }
                // 块表达式 { stmt; stmt; expr? }
                let stmts = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                let (stmts, result) = if let Some(Stmt::Expr(e)) = stmts.last() {
                    let len = stmts.len();
                    if len == 1 {
                        (Vec::new(), Some(Box::new(e.clone())))
                    } else {
                        (stmts[..len - 1].to_vec(), Some(Box::new(e.clone())))
                    }
                } else {
                    (stmts, None)
                };
                Ok(Expr::Block(stmts, result))
            }
            // P2.7: Array<T>(size, init) 动态数组构造
            Some(Token::TypeArray) => {
                self.expect(Token::Lt)?;
                let elem_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if !self.check(&Token::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.check(&Token::Comma) {
                            break;
                        }
                        self.advance();
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Expr::ConstructorCall {
                    name: "Array".to_string(),
                    type_args: Some(vec![elem_type]),
                    args,
                    named_args: vec![],
                })
            }
            Some(Token::LBracket) => {
                // 数组字面量 [1, 2, 3]
                let mut elements = Vec::new();
                if !self.check(&Token::RBracket) {
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.check(&Token::Comma) {
                            break;
                        }
                        self.advance();
                    }
                }
                self.expect(Token::RBracket)?;
                Ok(Expr::Array(elements))
            }
            Some(Token::If) => {
                // cjc 兼容: if (let pattern <- expr) 或 if let pattern = expr
                let is_paren_let = self.check(&Token::LParen) && matches!(self.peek_next(), Some(Token::Let));
                let is_let = self.check(&Token::Let);
                if is_paren_let {
                    self.advance(); // consume (
                    self.advance(); // consume let
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Assign), "`=` 或 `<-`".to_string()));
                    }
                    self.advance();
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let then_stmts = self.parse_stmts()?;
                    // P3: 将所有语句包装为 Block 表达式，保留副作用
                    let then_expr = {
                        let (block_stmts, block_result) = if then_stmts.is_empty() {
                            (Vec::new(), None)
                        } else if let Some(Stmt::Expr(e)) = then_stmts.last() {
                            let result_expr = Box::new(e.clone());
                            (then_stmts[..then_stmts.len() - 1].to_vec(), Some(result_expr))
                        } else {
                            (then_stmts.clone(), None)
                        };
                        Box::new(Expr::Block(block_stmts, block_result))
                    };
                    self.expect(Token::RBrace)?;
                    let else_branch = if self.check(&Token::Else) {
                        self.advance();
                        // P1.1: 支持 else if 链式语法
                        if self.check(&Token::If) {
                            let else_if_expr = self.parse_expr()?;
                            Some(Box::new(else_if_expr))
                        } else {
                            self.expect(Token::LBrace)?;
                            let else_stmts = self.parse_stmts()?;
                            let else_expr = {
                                let (block_stmts, block_result) = if else_stmts.is_empty() {
                                    (Vec::new(), None)
                                } else if let Some(Stmt::Expr(e)) = else_stmts.last() {
                                    let result_expr = Box::new(e.clone());
                                    (else_stmts[..else_stmts.len() - 1].to_vec(), Some(result_expr))
                                } else {
                                    (else_stmts.clone(), None)
                                };
                                Some(Box::new(Expr::Block(block_stmts, block_result)))
                            };
                            self.expect(Token::RBrace)?;
                            else_expr
                        }
                    } else {
                        None
                    };
                    Ok(Expr::IfLet {
                        pattern,
                        expr,
                        then_branch: then_expr,
                        else_branch,
                    })
                } else if is_let {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(self.peek().cloned().unwrap_or(Token::Assign), "`=` 或 `<-`".to_string()));
                    }
                    self.advance();
                    // 使用受限表达式解析，避免 { 被误认为结构体初始化
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::LBrace)?;
                    let then_stmts = self.parse_stmts()?;
                    // P3: 将所有语句包装为 Block 表达式，保留副作用
                    let then_expr = {
                        let (block_stmts, block_result) = if then_stmts.is_empty() {
                            (Vec::new(), None)
                        } else if let Some(Stmt::Expr(e)) = then_stmts.last() {
                            let result_expr = Box::new(e.clone());
                            (then_stmts[..then_stmts.len() - 1].to_vec(), Some(result_expr))
                        } else {
                            (then_stmts.clone(), None)
                        };
                        Box::new(Expr::Block(block_stmts, block_result))
                    };
                    self.expect(Token::RBrace)?;
                    let else_branch = if self.check(&Token::Else) {
                        self.advance();
                        // P1.1: 支持 else if 链式语法
                        if self.check(&Token::If) {
                            let else_if_expr = self.parse_expr()?;
                            Some(Box::new(else_if_expr))
                        } else {
                            self.expect(Token::LBrace)?;
                            let else_stmts = self.parse_stmts()?;
                            let else_expr = {
                                let (block_stmts, block_result) = if else_stmts.is_empty() {
                                    (Vec::new(), None)
                                } else if let Some(Stmt::Expr(e)) = else_stmts.last() {
                                    let result_expr = Box::new(e.clone());
                                    (else_stmts[..else_stmts.len() - 1].to_vec(), Some(result_expr))
                                } else {
                                    (else_stmts.clone(), None)
                                };
                                Some(Box::new(Expr::Block(block_stmts, block_result)))
                            };
                            self.expect(Token::RBrace)?;
                            else_expr
                        }
                    } else {
                        None
                    };
                    Ok(Expr::IfLet {
                        pattern,
                        expr,
                        then_branch: then_expr,
                        else_branch,
                    })
                } else {
                    let cond = self.parse_expr()?;
                    self.expect(Token::LBrace)?;
                    let then_stmts = self.parse_stmts()?;
                    // 将 if 块体包装为 Expr::Block，保留所有语句（含 return/let 等）
                    let then_branch = Self::stmts_to_block_expr(then_stmts);
                    self.expect(Token::RBrace)?;

                    let else_branch = if self.check(&Token::Else) {
                        self.advance();
                        // P1.1: 支持 else if 链式语法
                        if self.check(&Token::If) {
                            // else if → 递归解析 if 表达式作为 else 分支
                            let else_if_expr = self.parse_expr()?;
                            Some(Box::new(else_if_expr))
                        } else {
                            self.expect(Token::LBrace)?;
                            let else_stmts = self.parse_stmts()?;
                            let else_expr = Self::stmts_to_block_expr(else_stmts);
                            self.expect(Token::RBrace)?;
                            Some(else_expr)
                        }
                    } else {
                        None
                    };

                    Ok(Expr::If {
                        cond: Box::new(cond),
                        then_branch,
                        else_branch,
                    })
                }
            }
            Some(Token::Match) => {
                // 支持 match (x) { 和 match x { 两种语法 (cjc 兼容)
                let has_paren = self.check(&Token::LParen);
                if has_paren {
                    self.advance();
                }
                // 使用受限的表达式解析，不允许解析结构体初始化
                let expr = self.parse_match_subject()?;
                if has_paren {
                    self.expect(Token::RParen)?;
                }
                self.expect(Token::LBrace)?;
                let arms = self.parse_match_arms()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Match {
                    expr: Box::new(expr),
                    arms,
                })
            }
            // P5.1: spawn { block } — 单线程桩实现
            Some(Token::Spawn) => {
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Spawn { body })
            }
            // M2/C3: quote(stmts) — 编译时 AST 构建
            // 支持语句列表或 raw token 模式
            Some(Token::Quote) => {
                self.expect(Token::LParen)?;
                let save_pos = self.pos;
                // 尝试将 quote 体解析为语句列表
                let mut body = Vec::new();
                let mut parse_ok = true;
                while !self.check(&Token::RParen) {
                    match self.parse_stmt() {
                        Ok(s) => {
                            body.push(s);
                            if self.check(&Token::Semicolon) {
                                self.advance();
                            }
                        }
                        Err(_) => {
                            parse_ok = false;
                            break;
                        }
                    }
                }
                if parse_ok {
                    self.expect(Token::RParen)?;
                    Ok(Expr::Quote { body, splices: vec![], raw_tokens: None })
                } else {
                    // C3.3: 回退为 raw tokens 模式
                    self.pos = save_pos;
                    let mut raw = String::new();
                    let mut depth = 1i32;
                    while depth > 0 {
                        match self.peek() {
                            Some(Token::LParen) => { depth += 1; raw.push('('); self.advance(); }
                            Some(Token::RParen) => {
                                depth -= 1;
                                if depth > 0 { raw.push(')'); }
                                self.advance();
                            }
                            None => break,
                            _ => {
                                let tok = self.advance().unwrap();
                                raw.push_str(&format!("{:?} ", tok));
                            }
                        }
                    }
                    Ok(Expr::Quote { body: vec![], splices: vec![], raw_tokens: Some(raw.trim().to_string()) })
                }
            }
            // P5.2: synchronized(lock) { block } — 单线程桩实现
            Some(Token::Synchronized) => {
                self.expect(Token::LParen)?;
                let lock = self.parse_expr()?;
                self.expect(Token::RParen)?;
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Synchronized {
                    lock: Box::new(lock),
                    body,
                })
            }
            // C3.1/C3.2: $ 拼接语法 — $var 或 $(expr)
            Some(Token::Dollar) => {
                if self.check(&Token::LParen) {
                    // $(expr)
                    self.advance();
                    let inner = self.parse_expr()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Splice { expr: Box::new(inner) })
                } else if let Some(name) = self.advance_ident() {
                    // $varName
                    Ok(Expr::Splice { expr: Box::new(Expr::Var(name)) })
                } else {
                    self.bail(ParseError::UnexpectedToken(Token::Dollar, "$ 后应跟标识符或 (expr)".to_string()))
                }
            }
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok, "表达式".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析结构体字段初始化
    fn parse_struct_fields(&mut self) -> Result<Vec<(String, Expr)>, ParseErrorAt> {
        let mut fields = Vec::new();
        if self.check(&Token::RBrace) {
            return Ok(fields);
        }

        loop {
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            self.expect(Token::Colon)?;
            let value = self.parse_expr()?;
            fields.push((name, value));

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(fields)
    }

    /// 基本类型 token 作为构造/转换函数调用: Int32(expr) 等
    fn parse_type_call(&mut self, type_name: &str) -> Result<Expr, ParseErrorAt> {
        if self.check(&Token::LParen) {
            self.advance();
            let (args, named_args) = self.parse_args()?;
            self.expect(Token::RParen)?;
            Ok(Expr::Call {
                name: type_name.to_string(),
                type_args: None,
                args,
                named_args,
            })
        } else if self.check(&Token::Dot) {
            // TypeName.staticMethod(...) — 静态方法调用
            Ok(Expr::Var(type_name.to_string()))
        } else {
            Ok(Expr::Var(type_name.to_string()))
        }
    }

    /// 解析 Lambda 参数列表: x: T, y: T
    fn parse_lambda_params(&mut self) -> Result<Vec<(String, Type)>, ParseErrorAt> {
        let mut params = Vec::new();

        loop {
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            self.expect(Token::Colon)?;
            let ty = self.parse_type()?;
            params.push((name, ty));

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(params)
    }

    /// 解析 Lambda 表达式的剩余部分: : ReturnType { body }
    fn parse_lambda_rest(&mut self, params: Vec<(String, Type)>) -> Result<Expr, ParseErrorAt> {
        if self.check(&Token::Colon) {
            self.advance();
        } else {
            return self.bail(ParseError::UnexpectedToken(
                self.peek().cloned().unwrap_or(Token::Colon),
                "`:` (返回类型)".to_string(),
            ));
        }
        let return_type = Some(self.parse_type()?);
        self.expect(Token::LBrace)?;
        let body = self.parse_expr()?;
        self.expect(Token::RBrace)?;
        Ok(Expr::Lambda {
            params,
            return_type,
            body: Box::new(body),
        })
    }

    /// 解析函数调用参数（支持位置参数和命名参数 name!: value）
    fn parse_args(&mut self) -> Result<(Vec<Expr>, Vec<(String, Expr)>), ParseErrorAt> {
        let mut args = Vec::new();
        let mut named_args = Vec::new();
        if self.check(&Token::RParen) {
            return Ok((args, named_args));
        }

        loop {
            // P2.9: 检查是否是命名参数 name!: value
            if let Some(Token::Ident(_)) = self.peek() {
                if matches!(self.peek_at(1), Some(Token::Bang)) && matches!(self.peek_at(2), Some(Token::Colon)) {
                    // 命名参数
                    let name = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        _ => unreachable!(),
                    };
                    self.advance(); // skip !
                    self.advance(); // skip :
                    let value = self.parse_expr()?;
                    named_args.push((name, value));
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                    continue;
                }
            }
            args.push(self.parse_expr()?);
            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok((args, named_args))
    }

    /// 解析 match 的主题表达式 (不包括结构体初始化)
    fn parse_match_subject(&mut self) -> Result<Expr, ParseErrorAt> {
        // 只解析简单表达式: 变量、字面量、函数调用、字段访问等
        // 不解析结构体初始化 (因为 { 会被误认为 match body)
        let mut expr = match self.advance() {
            Some(Token::Integer(n)) => Expr::Integer(n),
            Some(Token::Float(f)) => Expr::Float(f),
            Some(Token::Float32(f)) => Expr::Float32(f),
            Some(Token::Float32Suffix(f)) => Expr::Float32(f),
            Some(Token::Float16Suffix(f)) => Expr::Float16(f),
            Some(Token::Int32Suffix(n)) => Expr::Int32(n),
            Some(Token::Int16Suffix(n)) => Expr::Int16(n),
            Some(Token::Int8Suffix(n)) => Expr::Int8(n),
            Some(Token::This) => Expr::Var(self.receiver_name.clone().unwrap_or_else(|| "this".to_string())),
            Some(Token::True) => Expr::Bool(true),
            Some(Token::False) => Expr::Bool(false),
            Some(Token::StringLit(s)) => self.parse_string_or_interpolated(s)?,
            Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => Expr::String(s),
            Some(Token::Ident(name)) => {
                if self.check(&Token::LParen) {
                    // 函数调用
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Expr::Call { name, type_args: None, args, named_args }
                } else {
                    Expr::Var(name)
                }
            }
            Some(Token::LParen) => {
                let expr = self.parse_expr()?;
                self.expect(Token::RParen)?;
                expr
            }
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "match 表达式".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };

        // 处理后缀表达式 (字段访问、数组索引)
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.advance();
                    let field = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    expr = Expr::Field {
                        object: Box::new(expr),
                        field,
                    };
                }
                Some(Token::LBracket) => {
                    // 数组访问或切片表达式
                    self.advance();

                    // 检查是否是 [..] 或 [..end] (以 DotDot 开头)
                    if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                        self.advance(); // consume ..
                        let end = if self.check(&Token::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        self.expect(Token::RBracket)?;
                        expr = Expr::SliceExpr {
                            array: Box::new(expr),
                            start: None,
                            end,
                        };
                    } else {
                        let start_expr = self.parse_expr()?;

                        // 检查是否 parse_expr 错误地解析为 Range（应该是 SliceExpr）
                        if let Expr::Range { start, end, .. } = start_expr {
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start: Some(start),
                                end: Some(end),
                            };
                        } else if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                            self.advance(); // consume ..
                            let end = if self.check(&Token::RBracket) {
                                None
                            } else {
                                Some(Box::new(self.parse_expr()?))
                            };
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start: Some(Box::new(start_expr)),
                                end,
                            };
                        } else {
                            self.expect(Token::RBracket)?;
                            expr = Expr::Index {
                                array: Box::new(expr),
                                index: Box::new(start_expr),
                            };
                        }
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 解析 for 循环的可迭代表达式 (不包括结构体初始化)
    fn parse_for_iterable(&mut self) -> Result<Expr, ParseErrorAt> {
        // 支持: 变量、范围表达式、函数调用、数组字面量
        // 不支持: 结构体初始化 (因为 { 会被误认为 for body)
        match self.advance() {
            Some(Token::Integer(n)) => {
                // 检查是否是范围表达式
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    let end = match self.advance() {
                        Some(Token::Integer(end)) => Expr::Integer(end),
                        Some(Token::Ident(name)) => Expr::Var(name),
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "范围终点".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    // P2.6: 可选步长 `: step`
                    let step = if self.check(&Token::Colon) {
                        self.advance();
                        Some(Box::new(self.parse_primary()?))
                    } else {
                        None
                    };
                    Ok(Expr::Range {
                        start: Box::new(Expr::Integer(n)),
                        end: Box::new(end),
                        inclusive,
                        step,
                    })
                } else {
                    Ok(Expr::Integer(n))
                }
            }
            Some(Token::Ident(name)) => {
                if self.check(&Token::LParen) {
                    // 函数调用
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call { name, type_args: None, args, named_args })
                } else if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    // 变量开头的范围 (如 start..end)
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    let end = match self.advance() {
                        Some(Token::Integer(end)) => Expr::Integer(end),
                        Some(Token::Ident(name)) => Expr::Var(name),
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "范围终点".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    // P2.6: 可选步长 `: step`
                    let step = if self.check(&Token::Colon) {
                        self.advance();
                        Some(Box::new(self.parse_primary()?))
                    } else {
                        None
                    };
                    Ok(Expr::Range {
                        start: Box::new(Expr::Var(name)),
                        end: Box::new(end),
                        inclusive,
                        step,
                    })
                } else {
                    // 普通变量
                    Ok(Expr::Var(name))
                }
            }
            Some(Token::LBracket) => {
                // 数组字面量 [1, 2, 3]
                let mut elements = Vec::new();
                if !self.check(&Token::RBracket) {
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.check(&Token::Comma) {
                            break;
                        }
                        self.advance();
                    }
                }
                self.expect(Token::RBracket)?;
                Ok(Expr::Array(elements))
            }
            Some(Token::This) => Ok(Expr::Var(self.receiver_name.clone().unwrap_or_else(|| "this".to_string()))),
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok, "for 循环可迭代表达式".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析 match 分支列表
    fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, ParseErrorAt> {
        let mut arms = Vec::new();

        while !self.check(&Token::RBrace) && self.peek().is_some() {
            // cjc: match 分支使用 case 关键字
            if matches!(self.peek(), Some(Token::Case)) {
                self.advance();
            }
            let pattern = self.parse_pattern()?;

            // 可选的守卫条件 (cjc 用 where，cjwasm 兼容 if)
            let guard = if self.check(&Token::If) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else if matches!(self.peek(), Some(Token::Where)) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };

            self.expect(Token::FatArrow)?;
            // match arm body: 支持多语句直到下一个 case / } / ,
            let body = if self.check(&Token::LBrace) {
                self.advance();
                let stmts = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                let (stmts, result) = if let Some(Stmt::Expr(e)) = stmts.last() {
                    let len = stmts.len();
                    (stmts[..len-1].to_vec(), Some(e.clone()))
                } else {
                    (stmts, None)
                };
                Box::new(Expr::Block(stmts, result.map(Box::new)))
            } else {
                let mut stmts = Vec::new();
                while !matches!(self.peek(), Some(Token::Case) | Some(Token::RBrace) | None) && !self.check(&Token::Comma) {
                    stmts.push(self.parse_stmt()?);
                    if self.check(&Token::Semicolon) {
                        self.advance();
                    }
                }
                if stmts.len() == 1 {
                    if let Stmt::Expr(e) = &stmts[0] {
                        Box::new(e.clone())
                    } else {
                        Box::new(Expr::Block(stmts, None))
                    }
                } else if stmts.is_empty() {
                    Box::new(Expr::Tuple(vec![]))
                } else {
                    let (stmts, result) = if let Some(Stmt::Expr(e)) = stmts.last() {
                        let len = stmts.len();
                        (stmts[..len-1].to_vec(), Some(e.clone()))
                    } else {
                        (stmts, None)
                    };
                    Box::new(Expr::Block(stmts, result.map(Box::new)))
                }
            };

            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });

            // 可选的逗号分隔
            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        Ok(arms)
    }

    /// 解析模式
    fn parse_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        self.parse_or_pattern()
    }

    /// 解析 or 模式 (1 | 2 | 3)
    fn parse_or_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        let mut pattern = self.parse_primary_pattern()?;

        while self.check(&Token::Pipe) {
            self.advance();
            let right = self.parse_primary_pattern()?;
            pattern = match pattern {
                Pattern::Or(mut patterns) => {
                    patterns.push(right);
                    Pattern::Or(patterns)
                }
                _ => Pattern::Or(vec![pattern, right]),
            };
        }

        Ok(pattern)
    }

    /// 解析基础模式
    fn parse_primary_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        match self.peek() {
            Some(Token::Underscore) => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            // Some/None/Ok/Err 作为模式中的变体
            Some(Token::Some) => {
                self.advance();
                let binding = if self.check(&Token::LParen) {
                    self.advance();
                    let b = match self.advance() {
                        Some(Token::Ident(id)) => Some(id),
                        Some(Token::Underscore) => None,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联值绑定名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    self.expect(Token::RParen)?;
                    b
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "Some".to_string(),
                    binding,
                })
            }
            Some(Token::None) => {
                self.advance();
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "None".to_string(),
                    binding: None,
                })
            }
            Some(Token::Ok) => {
                self.advance();
                let binding = if self.check(&Token::LParen) {
                    self.advance();
                    let b = match self.advance() {
                        Some(Token::Ident(id)) => Some(id),
                        Some(Token::Underscore) => None,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联值绑定名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    self.expect(Token::RParen)?;
                    b
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Ok".to_string(),
                    binding,
                })
            }
            Some(Token::Err) => {
                self.advance();
                let binding = if self.check(&Token::LParen) {
                    self.advance();
                    let b = match self.advance() {
                        Some(Token::Ident(id)) => Some(id),
                        Some(Token::Underscore) => None,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联值绑定名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    self.expect(Token::RParen)?;
                    b
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Err".to_string(),
                    binding,
                })
            }
            Some(Token::Integer(n)) => {
                let n = *n;
                self.advance();
                // 检查是否是范围模式
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    if let Some(Token::Integer(end)) = self.advance() {
                        return Ok(Pattern::Range {
                            start: Literal::Integer(n),
                            end: Literal::Integer(end),
                            inclusive,
                        });
                    }
                }
                Ok(Pattern::Literal(Literal::Integer(n)))
            }
            Some(Token::True) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(true)))
            }
            Some(Token::False) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(false)))
            }
            Some(Token::StringLit(s)) => {
                let s_str = match s.clone() {
                    StringOrInterpolated::Plain(s) => s,
                    StringOrInterpolated::Interpolated(_) => {
                        let (byte_start, byte_end) = self.at_prev();
                        return Err(ParseErrorAt {
                            error: ParseError::UnexpectedToken(
                                Token::StringLit(s.clone()),
                                "模式中不支持字符串插值".to_string(),
                            ),
                            byte_start,
                            byte_end,
                        });
                    }
                };
                self.advance();
                Ok(Pattern::Literal(Literal::String(s_str)))
            }
            Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => {
                let s = s.clone();
                self.advance();
                Ok(Pattern::Literal(Literal::String(s)))
            }
            Some(Token::Ident(name)) => {
                let name = name.clone();
                self.advance();
                let looks_like_type = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if looks_like_type && self.check(&Token::Dot) {
                    if let Some(Token::Ident(_)) = self.peek_next() {
                        self.advance();
                        let variant = match self.advance() {
                            Some(Token::Ident(v)) => v.clone(),
                            _ => unreachable!(),
                        };
                        let binding = if self.check(&Token::LParen) {
                            self.advance();
                            let b = match self.advance() {
                                Some(Token::Ident(id)) => id,
                                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "关联值绑定名".to_string())),
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            Some(b)
                        } else {
                            None
                        };
                        return Ok(Pattern::Variant {
                            enum_name: name,
                            variant_name: variant,
                            binding,
                        });
                    }
                }
                if self.check(&Token::LBrace) {
                    self.advance();
                    let fields = self.parse_pattern_fields()?;
                    self.expect(Token::RBrace)?;
                    Ok(Pattern::Struct { name, fields })
                } else if self.check(&Token::Colon) {
                    // P3.5: 类型测试模式 x: Type
                    self.advance();
                    let ty = self.parse_type()?;
                    Ok(Pattern::TypeTest { binding: name, ty })
                } else {
                    Ok(Pattern::Binding(name))
                }
            }
            Some(Token::LParen) => {
                self.advance();
                let mut patterns = Vec::new();
                if !self.check(&Token::RParen) {
                    loop {
                        patterns.push(self.parse_pattern()?);
                        if !self.check(&Token::Comma) {
                            break;
                        }
                        self.advance();
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Pattern::Tuple(patterns))
            }
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok.clone(), "模式".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析结构体解构字段
    fn parse_pattern_fields(&mut self) -> Result<Vec<(String, Pattern)>, ParseErrorAt> {
        let mut fields = Vec::new();
        if self.check(&Token::RBrace) {
            return Ok(fields);
        }

        loop {
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };

            // 可选的 : pattern, 如果没有则使用同名绑定
            let pattern = if self.check(&Token::Colon) {
                self.advance();
                self.parse_pattern()?
            } else {
                Pattern::Binding(name.clone())
            };

            fields.push((name, pattern));

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(fields)
    }

    /// 解析字符串（普通或插值）
    fn parse_string_or_interpolated(&mut self, s: StringOrInterpolated) -> Result<Expr, ParseErrorAt> {
        match s {
            StringOrInterpolated::Plain(text) => Ok(Expr::String(text)),
            StringOrInterpolated::Interpolated(parts) => {
                let mut result_parts = Vec::new();
                for part in parts {
                    match part {
                        StringPart::Literal(text) => {
                            if !text.is_empty() {
                                result_parts.push(InterpolatePart::Literal(text));
                            }
                        }
                        StringPart::Interpolation(expr_text) => {
                            // 解析表达式文本
                            let expr = self.parse_interpolation_expr(&expr_text)?;
                            result_parts.push(InterpolatePart::Expr(Box::new(expr)));
                        }
                    }
                }
                Ok(Expr::Interpolate(result_parts))
            }
        }
    }

    /// 解析插值表达式内的文本
    fn parse_interpolation_expr(&self, expr_text: &str) -> Result<Expr, ParseErrorAt> {
        use crate::lexer::Lexer;

        let lexer = Lexer::new(expr_text);
        let tokens: Result<Vec<_>, _> = lexer.collect();
        let tokens = tokens.map_err(|e| ParseErrorAt {
            error: ParseError::UnknownType(format!("插值表达式词法错误: {}", e)),
            byte_start: self.at_prev().0,
            byte_end: self.at_prev().1,
        })?;

        let mut parser = Parser::new(tokens);
        parser.parse_expr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    #[test]
    fn test_parse_function() {
        let source = "func add(a: Int64, b: Int64): Int64 { return a + b }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].name, "add");
    }

    #[test]
    fn test_parse_struct() {
        let source = "struct Point { x: Int64, y: Int64 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.structs.len(), 1);
        assert_eq!(program.structs[0].name, "Point");
        assert_eq!(program.structs[0].fields.len(), 2);
    }

    #[test]
    fn test_parse_array() {
        let source = "func test(): Int64 { let arr = [1, 2, 3] return arr[0] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_loop() {
        let source = "func test(): Int64 { var sum: Int64 = 0 for i in 0..10 { sum = sum + i } return sum }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        // 检查函数体包含 for 语句
        assert!(matches!(&program.functions[0].body[1], Stmt::For { .. }));
    }

    #[test]
    fn test_parse_match() {
        let source = "func test(n: Int64): Int64 { match n { 0 => 100, 1 => 200, _ => 999 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_or_pattern() {
        let source = "func test(n: Int64): Int64 { match n { 1 | 2 | 3 => 10, _ => 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_range_pattern() {
        let source = "func test(n: Int64): Int64 { match n { 0..10 => 1, 10..100 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_if_else() {
        let source = "func test(x: Int64): Int64 { if x > 0 { return 1 } else { return 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].body.is_empty());
    }

    #[test]
    fn test_parse_while() {
        let source = "func test(): Int64 { var n: Int64 = 0 while n < 10 { n = n + 1 } return n }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(matches!(&program.functions[0].body[1], Stmt::While { .. }));
    }

    #[test]
    fn test_parse_struct_init() {
        let source = r#"
            struct Point { x: Int64, y: Int64 }
            func test(): Int64 {
                let p = Point { x: 1, y: 2 }
                return p.x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.structs.len(), 1);
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_guard() {
        let source = "func test(n: Int64): Int64 { match n { x if x < 0 => 1, 0 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_in_array() {
        let source = "func test(): Int64 { let arr = [1, 2, 3] var s: Int64 = 0 for x in arr { s = s + x } return s }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        let for_stmt = program.functions[0]
            .body
            .iter()
            .find(|s| matches!(s, Stmt::For { .. }));
        assert!(for_stmt.is_some());
    }

    #[test]
    fn test_parse_for_range_inclusive() {
        let source = "func test(): Int64 { var s: Int64 = 0 for i in 1..=5 { s = s + i } return s }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_logical_ops() {
        let source = "func test(): Int64 { if true && false || !true { return 0 } return 1 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_unary_neg() {
        let source = "func test(): Int64 { let x = -1 let y = -(-2) return x + y }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_block_expr() {
        let source = "func test(): Int64 { let x = { let a = 1 let b = 2 a + b } return x }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_compound_assign() {
        let source = "func test() { var x: Int64 = 0 x += 1 x -= 1 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].body.len(), 3);
    }

    #[test]
    fn test_parse_var_with_type() {
        let source = "func test() { var x: Int64 = 0 var y: Float64 = 3.14 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].body.len(), 2);
    }

    #[test]
    fn test_parse_lambda_arrow_syntax() {
        // Lambda syntax: (x: T): R { body }
        let source = "func test() { let f = (x: Int64): Int64 { x * 2 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);

        if let Stmt::Let { value, .. } = &program.functions[0].body[0] {
            assert!(matches!(value, Expr::Lambda { .. }));
        } else {
            panic!("应该是 let 语句");
        }
    }

    #[test]
    fn test_parse_lambda_brace_syntax() {
        // Lambda syntax: { x: T => body }
        let source = "func test() { let f = { x: Int64 => x + 1 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);

        if let Stmt::Let { value, .. } = &program.functions[0].body[0] {
            assert!(matches!(value, Expr::Lambda { .. }));
        } else {
            panic!("应该是 let 语句");
        }
    }

    // === 覆盖率补充：Parser 单元测试 ===

    #[test]
    fn test_parse_import_path() {
        let source = "import bar.baz.foo\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_import_as() {
        let source = "import std.math as m\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_import_plain() {
        let source = "import std.io\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_package_declaration() {
        let source = "package test.app\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(program.package_name.is_some());
    }

    #[test]
    fn test_parse_interface_with_default_and_assoc() {
        let source = r#"
            interface Describable {
                type Element;
                func describe(): String;
                func default_method(): Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
        assert!(!program.interfaces[0].assoc_types.is_empty());
    }

    #[test]
    fn test_parse_interface_with_inheritance() {
        let source = r#"
            interface Base { func id(): Int64; }
            interface Extended: Base { func extra(): Int64; }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 2);
    }

    #[test]
    fn test_parse_extend_with_assoc_type() {
        let source = r#"
            struct Foo { x: Int64 }
            extend Foo: SomeInterface {
                type Element = Int64;
                func method(): Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.extends.is_empty());
    }

    #[test]
    fn test_parse_class_with_init_deinit_prop() {
        let source = r#"
            class MyClass {
                var x: Int64;
                var y: String;
                init(x: Int64) { this.x = x }
                ~init { }
                prop value: Int64 {
                    get() { return this.x }
                    set(v) { this.x = v }
                }
                func method(self: MyClass): Int64 { return self.x }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        let cls = &program.classes[0];
        assert!(cls.init.is_some());
        assert!(cls.deinit.is_some());
        // methods include the generated getter/setter + method
        assert!(!cls.methods.is_empty());
    }

    #[test]
    fn test_parse_class_override_method() {
        let source = r#"
            open class Base {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func get(self: Base): Int64 { return self.x }
            }
            class Derived <: Base {
                init(x: Int64) { super(x) }
                override func get(self: Derived): Int64 { return self.x * 2 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_class_abstract_sealed() {
        let source = r#"
            abstract class Shape {
                var name: String;
            }
            sealed class Container {
                var size: Int64;
                init(size: Int64) { this.size = size }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_function_with_throw() {
        let source = r#"
            func validate(x: Int64): Int64 {
                if x < 0 { throw 0 }
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_try_catch_finally() {
        let source = r#"
            func main(): Int64 {
                try {
                    throw 1
                } catch(e) {
                    return 0
                } finally {
                    let x = 1
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_enum_with_where() {
        let source = r#"
            enum Container<T> where T: Comparable {
                Full(T)
                Empty
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
        assert!(!program.enums[0].type_params.is_empty());
    }

    #[test]
    fn test_parse_struct_with_type_constraint() {
        let source = r#"
            struct Wrapper<T: Hashable> { inner: T }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 1);
    }

    #[test]
    fn test_parse_variadic_and_default_params() {
        let source = r#"
            func f(x: Int64, y: Int64 = 10): Int64 { return x + y }
            func g(args: Int64...): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 2);
        // Check default param
        assert!(program.functions[0].params[1].default.is_some());
        // Check variadic
        assert!(program.functions[1].params[0].variadic);
    }

    #[test]
    fn test_parse_type_annotations() {
        let source = r#"
            func main() {
                let a: Array<Int64> = [1, 2]
                let t: Tuple<Int64, Int64> = (1, 2)
                let o: Option<Int64> = None
                let r: Result<Int64, String> = Ok(1)
                let rng: Range = 0..10
                let c: Rune = 'A'
                let u: Unit = ()
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_super_call() {
        let source = r#"
            open class Base {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func get(self: Base): Int64 { return self.x }
            }
            class Child <: Base {
                init(x: Int64) { super(x) }
                func get2(self: Child): Int64 { return super.get() }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_where_clause_function() {
        let source = r#"
            func compare<T>(a: T, b: T): Int64 where T: Comparable {
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].constraints.is_empty());
    }

    #[test]
    fn test_parse_class_implements() {
        let source = r#"
            interface I { func foo(): Int64; }
            class C <: I {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func foo(self: C): Int64 { return self.x }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        // cjc: <: 后的第一个类型为 extends（解析时不区分类/接口）
        assert!(program.classes[0].extends.is_some());
    }

    #[test]
    fn test_parse_for_in_range_and_array() {
        let source = r#"
            func main(): Int64 {
                for i in 0..10 { }
                for v in [1, 2, 3] { }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_arms() {
        let source = r#"
            func main(): Int64 {
                return match 5 {
                    0 => 1,
                    x if x > 10 => 2,
                    1 | 2 | 3 => 3,
                    _ => 0
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_if_let() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                if let Some(v) = o {
                    return v
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_while_let() {
        let source = r#"
            func main(): Int64 {
                var o: Option<Int64> = Some(1)
                while let Some(v) = o {
                    o = None
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_multi_constraint() {
        let source = r#"
            func process<T: Comparable & Hashable>(x: T): T {
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].constraints.len(), 1);
        // Single constraint with two bounds
        assert_eq!(program.functions[0].constraints[0].bounds.len(), 2);
    }

    #[test]
    fn test_parse_class_with_generic_method() {
        let source = r#"
            class Container {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func transform<T>(self: Container): Int64 {
                    return self.x
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_parse_func_with_where() {
        let source = r#"
            func process<T>(x: T) where T <: Comparable { }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].constraints.is_empty());
    }

    // === 覆盖率补充：解析器错误处理路径 ===

    #[test]
    fn test_parse_error_display() {
        let err = ParseErrorAt {
            error: ParseError::UnexpectedEof,
            byte_start: 10,
            byte_end: 20,
        };
        let display = format!("{}", err);
        assert!(display.contains("10"));
        assert!(display.contains("20"));
    }

    #[test]
    fn test_parse_error_source() {
        use std::error::Error;
        let err = ParseErrorAt {
            error: ParseError::UnexpectedEof,
            byte_start: 0,
            byte_end: 0,
        };
        assert!(err.source().is_some());
    }

    #[test]
    fn test_line_column_from_source() {
        let source = "abc\ndef\nghi";
        assert_eq!(super::line_column_from_source(source, 0), (1, 1));
        assert_eq!(super::line_column_from_source(source, 3), (1, 4));
        assert_eq!(super::line_column_from_source(source, 4), (2, 1));
        assert_eq!(super::line_column_from_source(source, 7), (2, 4));
        assert_eq!(super::line_column_from_source(source, 8), (3, 1));
        // Past end
        assert_eq!(super::line_column_from_source(source, 100), (3, 4));
    }

    #[test]
    fn test_parse_visibility() {
        let source = r#"
            public func foo(): Int64 { return 1 }
            private func bar(): Int64 { return 2 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 2);
    }

    #[test]
    fn test_parse_struct_standalone() {
        let source = r#"
            struct Empty {}
            struct Single { x: Int64 }
            struct Multi { x: Int64, y: Float64, z: Bool }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 3);
    }

    #[test]
    fn test_parse_enum_variants() {
        let source = r#"
            enum Color { Red Green Blue }
            enum Value {
                Num(Int64)
                Str(String)
                Empty
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 2);
        assert_eq!(program.enums[0].variants.len(), 3);
    }

    #[test]
    fn test_parse_extern_func() {
        let source = r#"
            @import("env", "print")
            foreign func print(msg: String)
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(program.functions[0].extern_import.is_some());
    }

    #[test]
    fn test_parse_extern_func_no_attr() {
        let source = r#"
            foreign func console_log(msg: String): Int64
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(program.functions[0].extern_import.is_some());
    }

    #[test]
    fn test_parse_complex_expressions() {
        let source = r#"
            func main(): Int64 {
                let a = 1 + 2 * 3
                let b = (1 + 2) * 3
                let c = -5
                let d = !true
                let e = ~0xFF
                let f = 2 ** 3
                return a + b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_cast_expression() {
        let source = r#"
            func main(): Int64 {
                let a = 42 as Float64
                let b = 1.5 as Int64
                return b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_null_coalesce() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = None
                let v = o ?? 42
                return v
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_try_operator() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                let v = o?
                return v
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_block_expression() {
        let source = r#"
            func main(): Int64 {
                let x = {
                    let a = 10
                    a + 20
                }
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_range_expression() {
        let source = r#"
            func main(): Int64 {
                let r = 0..10
                let r2 = 0..=10
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_with_guard() {
        let source = r#"
            func main(): Int64 {
                let x = 5
                return match x {
                    n if n > 10 => 1,
                    n if n > 0 => 2,
                    _ => 0
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_compound_assignment() {
        let source = r#"
            func main(): Int64 {
                var x: Int64 = 10
                x += 5
                x -= 2
                x *= 3
                x /= 2
                x %= 7
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_loop_break_continue() {
        let source = r#"
            func main(): Int64 {
                var i: Int64 = 0
                loop {
                    i = i + 1
                    if i > 10 { break }
                    if i % 2 == 0 { continue }
                }
                return i
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_method_call() {
        let source = r#"
            struct Foo { x: Int64 }
            func main(): Int64 {
                let f = Foo { x: 42 }
                return f.x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_string_interpolation_expr() {
        let source = r#"
            func main(): Int64 {
                let name = "world"
                let s = "Hello ${name}!"
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_class_with_type_params() {
        let source = r#"
            class Container<T> {
                var value: T;
                init(v: T) { this.value = v }
                func get(self: Container<T>): T { return self.value }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(!program.classes[0].type_params.is_empty());
    }

    #[test]
    fn test_parse_multiple_imports() {
        let source = r#"
            import std.io
            import math
            import bar.baz.foo
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 3);
    }

    // === 覆盖率补充：错误处理路径 ===

    fn parse_should_fail(source: &str) -> ParseErrorAt {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        parser.parse_program().unwrap_err()
    }

    #[test]
    fn test_parse_error_unexpected_token() {
        let err = parse_should_fail("func 123() {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_missing_rparen() {
        let err = parse_should_fail("func foo( : Int64 { return 0 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_missing_lbrace() {
        let err = parse_should_fail("func foo(): Int64 return 0");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_empty_source() {
        // Empty source should just produce empty program
        let lexer = Lexer::new("");
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(program.functions.is_empty());
    }

    #[test]
    fn test_parse_error_bad_type_annotation() {
        let err = parse_should_fail("func foo(x: ): Int64 { return 0 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_unclosed_struct() {
        let err = parse_should_fail("struct Foo { x: Int64");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..) | ParseError::UnexpectedEof));
    }

    #[test]
    fn test_parse_error_bad_import() {
        let err = parse_should_fail("import 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_let() {
        let err = parse_should_fail("func main() { let 123 = 1 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_var() {
        let err = parse_should_fail("func main() { var 123 = 1 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_class_member() {
        let err = parse_should_fail("class Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_enum_variant() {
        let err = parse_should_fail("enum Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_interface_method() {
        let err = parse_should_fail("interface Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extend() {
        let err = parse_should_fail("extend 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_at_prev_position() {
        // Test that bail_at works with position tracking
        let err = parse_should_fail("func foo()): { return 0 }");
        assert!(err.byte_start > 0 || err.byte_end > 0);
    }

    // --- 更多错误路径覆盖 ---

    #[test]
    fn test_parse_error_bad_package_name() {
        // package 后面跟非标识符
        let err = parse_should_fail("package 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_package_path_part() {
        // package a.123 - 包路径中有数字
        let err = parse_should_fail("package a.123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_unexpected_top_level() {
        // 顶层出现非法 token（let/var 现在是允许的，所以使用 return）
        let err = parse_should_fail("return 1");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_import_item() {
        // import 123
        let err = parse_should_fail("import 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_import_path_bad_part() {
        // import a.123 - 路径中有数字
        let err = parse_should_fail("import a.123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_import_alias_missing_name() {
        // import foo as
        let err = parse_should_fail("import foo as");
        assert!(matches!(err.error, ParseError::UnexpectedEof));
    }

    #[test]
    fn test_parse_error_bad_extern_import_attr() {
        // @import(123, "foo") foreign func
        let err = parse_should_fail(r#"@import(123, "foo") foreign func bar()"#);
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extern_import_name() {
        // @import("env", 123) foreign func
        let err = parse_should_fail(r#"@import("env", 123) foreign func bar()"#);
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extern_func_name() {
        // foreign func 123()
        let err = parse_should_fail("foreign func 123()");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_name() {
        // struct 123 {}
        let err = parse_should_fail("struct 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_field_name() {
        // struct Foo { 123: Int64 }
        let err = parse_should_fail("struct Foo { 123: Int64 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_field_type() {
        // struct Foo { x: }
        let err = parse_should_fail("struct Foo { x: }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_enum_name() {
        // enum 123 {}
        let err = parse_should_fail("enum 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_interface_name() {
        // interface 123 {}
        let err = parse_should_fail("interface 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_class_name() {
        // class 123 {}
        let err = parse_should_fail("class 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_function_name() {
        // func 123() {}
        let err = parse_should_fail("func 123() {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_for_iterable() {
        // for i in 123.456 {} - float as iterable
        let err = parse_should_fail("func main() { for i in {} {} }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_match_subject() {
        // match 无效token
        let err = parse_should_fail("func main(): Int64 { return match {} {} }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_import_dotted_path() {
        // import baz.qux.foo (cjc 风格)
        let source = "import baz.qux.foo\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].module_path.len(), 3);
    }

    #[test]
    fn test_parse_for_variable_range() {
        let source = r#"
            func main(): Int64 {
                let n: Int64 = 5
                var sum: Int64 = 0
                for i in 0..n {
                    sum = sum + i
                }
                return sum
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_array_literal() {
        let source = r#"
            func main(): Int64 {
                var sum: Int64 = 0
                for i in [1, 2, 3] {
                    sum = sum + i
                }
                return sum
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_with_bool_and_string_patterns() {
        let source = r#"
            func main(): Int64 {
                let b: Bool = true
                let r1 = match b {
                    true => 1,
                    false => 0
                }
                let s: String = "hello"
                let r2 = match s {
                    "hello" => 10,
                    _ => 0
                }
                return r1 + r2
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }
}
