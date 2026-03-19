//! 声明解析：parse_program、parse_struct、parse_class、parse_enum、parse_function 等。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::*;
use crate::lexer::{StringOrInterpolated, Token};

impl Parser {
    pub fn parse_program(&mut self) -> Result<Program, ParseErrorAt> {
        let mut package_name = None;
        let mut imports = Vec::new();
        let mut structs = Vec::new();
        let mut interfaces = Vec::new();
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        let mut enums = Vec::new();
        let mut extends = Vec::new();
        let mut type_aliases: Vec<(String, Type)> = Vec::new();
        let mut constants = Vec::new();

        // 解析可选的 package 声明（cjc: package prefix.path，支持点分路径）
        // cjc: macro package 宏包 — 忽略 macro 修饰符
        if self.check(&Token::Macro) {
            self.advance(); // 消费 macro
        }
        if self.check(&Token::Protected) {
            self.advance();
            if !self.check(&Token::Package) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "package".to_string(),
                ));
            }
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
        } else if self.check(&Token::Package) {
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
        if self.check(&Token::Protected) {
            self.advance();
            if !self.check(&Token::Package) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "package".to_string(),
                ));
            }
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

        // 解析 import 语句
        while self.check(&Token::Import) {
            imports.push(self.parse_import()?);
        }

        while let Some(_tok) = self.peek() {
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

            let extern_import = if self.check(&Token::At) {
                if matches!(self.peek_next(), Some(Token::Import)) {
                    Some(self.parse_import_attr()?)
                } else {
                    let skip_next = self.skip_optional_attributes()?;
                    if skip_next {
                        self.skip_next_top_level_decl()?;
                    }
                    continue;
                }
            } else if self.check(&Token::Protected) {
                self.advance();
                self.expect(Token::Package)?;
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
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                        }
                    };
                    name = format!("{}.{}", name, part);
                }
                package_name = Some(name);
                continue;
            } else {
                None
            };

            if self.check(&Token::Foreign) {
                self.advance();
                // cjc: foreign { func ... } 块语法（多个 FFI 函数）
                if self.check(&Token::LBrace) {
                    self.advance(); // consume {
                    while !self.check(&Token::RBrace) && self.peek().is_some() {
                        functions.push(self.parse_extern_func(visibility.clone(), None)?);
                    }
                    self.expect(Token::RBrace)?;
                } else {
                    functions.push(self.parse_extern_func(visibility, extern_import)?);
                }
            } else {
                match self.peek() {
                    Some(Token::Struct) => {
                        structs.push(self.parse_struct_with_visibility(visibility)?)
                    }
                    Some(Token::Interface) => {
                        interfaces.push(self.parse_interface_with_visibility(visibility)?)
                    }
                    Some(Token::Class)
                    | Some(Token::Abstract)
                    | Some(Token::Sealed)
                    | Some(Token::Open) => {
                        classes.push(self.parse_class_with_visibility(visibility)?)
                    }
                    Some(Token::Enum) => enums.push(self.parse_enum_with_visibility(visibility)?),
                    Some(Token::Extend) => extends.push(self.parse_extend()?),
                    // P2.2: type Name = Type
                    Some(Token::TypeAlias) => {
                        self.advance();
                        let alias_name = match self.advance_ident() {
                            Some(n) => n,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "类型别名名称".to_string(),
                                ));
                            }
                        };
                        self.expect(Token::Assign)?;
                        let target_ty = self.parse_type()?;
                        self.type_aliases
                            .insert(alias_name.clone(), target_ty.clone());
                        type_aliases.push((alias_name, target_ty));
                    }
                    Some(Token::Func) => {
                        functions.push(self.parse_function_with_visibility(visibility)?)
                    }
                    // cjc: unsafe func — 忽略 unsafe 修饰符，当普通函数处理
                    Some(Token::Unsafe) => {
                        self.advance(); // consume unsafe
                        if self.check(&Token::Func) {
                            functions.push(self.parse_function_with_visibility(visibility)?)
                        } else {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "func (unsafe 后期望 func)".to_string(),
                            ));
                        }
                    }
                    // cjc: macro FuncName(...) — 宏函数声明，跳过整个宏函数体
                    Some(Token::Macro) => {
                        self.advance(); // consume macro
                                        // 可能接 func 关键字（如 macro func Name）或直接是函数名
                        if self.check(&Token::Func) {
                            functions.push(self.parse_function_with_visibility(visibility)?)
                        } else {
                            // macro Name(params): RetType { body } — 无 func 关键字，跳过到匹配的 }
                            // 消费函数名
                            let _ = self.advance_ident();
                            // 跳过 (params): RetType until {
                            let mut depth = 0i32;
                            loop {
                                match self.peek() {
                                    Some(Token::LBrace) => {
                                        depth += 1;
                                        self.advance();
                                        if depth == 1 {
                                            break;
                                        }
                                    }
                                    Some(Token::RBrace) => {
                                        if depth > 0 {
                                            depth -= 1;
                                        }
                                        self.advance();
                                    }
                                    None => break,
                                    _ => {
                                        self.advance();
                                    }
                                }
                            }
                            // 消费整个 body { ... }
                            let mut brace_depth = 1i32;
                            while brace_depth > 0 {
                                match self.advance() {
                                    Some(Token::LBrace) => brace_depth += 1,
                                    Some(Token::RBrace) => brace_depth -= 1,
                                    None => break,
                                    _ => {}
                                }
                            }
                        }
                    }
                    // cjc: main() 无需 func 关键字 (main 是保留字)
                    Some(Token::Main) => {
                        functions.push(self.parse_main_function(visibility)?);
                    }
                    // cjc: 顶层常量 let/var/const name: Type = expr
                    Some(Token::Let) | Some(Token::Var) | Some(Token::Const) => {
                        constants.push(self.parse_top_level_const()?);
                    }
                    // cjc: public import ... — 带可见性的导入语句
                    Some(Token::Import) => {
                        imports.push(self.parse_import()?);
                    }
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(
                            tok.clone(),
                            "struct、interface、class、enum、extend、func 或 foreign func"
                                .to_string(),
                        ))
                    }
                    None => break,
                }
            }
        }
        functions.extend(self.pending_struct_methods.drain(..));
        Ok(Program {
            package_name,
            imports,
            structs,
            interfaces,
            classes,
            enums,
            functions,
            extends,
            type_aliases,
            constants,
        })
    }

    /// 解析 import 语句 (cjc: import path.to.Item 或 import path.to.*)
    pub(crate) fn parse_import(&mut self) -> Result<Import, ParseErrorAt> {
        self.expect(Token::Import)?;

        // cjc 风格: import path.to.Item 或 import path.to.* 或 import path.to.Item as alias
        // 或 import path.to.{Item1, Item2, ...}
        let first = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string()));
            }
        };

        let mut module_path = vec![first];
        while self.check(&Token::Dot) {
            self.advance();
            // 检查是否为通配符 *
            if self.check(&Token::Star) {
                self.advance();
                // import path.to.* → 导入所有项
                return Ok(Import {
                    module_path,
                    items: None,
                    alias: None,
                });
            }
            // 检查是否为花括号导入列表 {Item1, Item2, ...}
            if self.check(&Token::LBrace) {
                self.advance();
                let mut items = Vec::new();
                loop {
                    let item = if self.check(&Token::Star) {
                        self.advance();
                        "*".to_string()
                    } else {
                        match self.advance_ident() {
                            Some(n) => n,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "导入项名称".to_string(),
                                ));
                            }
                        }
                    };
                    items.push(item);

                    if self.check(&Token::Comma) {
                        self.advance();
                        // 允许尾随逗号
                        if self.check(&Token::RBrace) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                return Ok(Import {
                    module_path,
                    items: Some(items),
                    alias: None,
                });
            }
            let part = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string()));
                }
            };
            module_path.push(part);
        }
        let alias = if self.check(&Token::As) {
            self.advance();
            match self.advance_ident() {
                Some(n) => Some(n),
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "导入别名".to_string()));
                }
            }
        } else {
            None
        };
        Ok(Import {
            module_path,
            items: None,
            alias,
        })
    }

    /// 解析顶层常量 (let/var/const name [: Type] = expr)
    pub(crate) fn parse_top_level_const(&mut self) -> Result<ConstDef, ParseErrorAt> {
        self.advance(); // consume let/var/const
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "常量名".to_string()));
            }
        };
        let ty = if self.check(&Token::Colon) {
            self.advance();
            self.parse_type()?
        } else {
            // 无类型注解时用 Int64 占位，codegen 可依 init 推断
            Type::Int64
        };
        self.expect(Token::Assign)?;
        let init = self.parse_expr()?;
        if self.check(&Token::Semicolon) {
            self.advance();
        }
        Ok(ConstDef { name, ty, init })
    }

    /// 解析 @import("module", "name") 属性（用于 extern func 前）
    pub(crate) fn parse_import_attr(&mut self) -> Result<ExternImport, ParseErrorAt> {
        self.expect(Token::At)?;
        self.expect(Token::Import)?;
        self.expect(Token::LParen)?;
        let module = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s)))
            | Some(Token::BacktickStringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字符串字面量 (模块名)".to_string(),
                ))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::Comma)?;
        let name = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s)))
            | Some(Token::BacktickStringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字符串字面量 (导入名)".to_string(),
                ))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::RParen)?;
        Ok(ExternImport { module, name })
    }

    /// 解析 extern func 声明（无 body；可选 extern_import 来自前导 @import）
    pub(crate) fn parse_extern_func(
        &mut self,
        visibility: Visibility,
        extern_import: Option<ExternImport>,
    ) -> Result<Function, ParseErrorAt> {
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
            Some(ExternImport {
                module: "env".to_string(),
                name: name.clone(),
            })
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
    pub(crate) fn parse_struct(&mut self) -> Result<StructDef, ParseErrorAt> {
        self.parse_struct_with_visibility(Visibility::default())
    }

    /// 解析结构体定义（带可见性）
    pub(crate) fn parse_struct_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<StructDef, ParseErrorAt> {
        self.expect(Token::Struct)?;

        let name = match self.advance_ident() {
            Some(name) => name,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "结构体名".to_string()));
            }
        };

        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());

        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // cjc 兼容: struct Name <: Interface1 & Interface2 { } — 结构体实现多个接口
        if self.check(&Token::SubType) {
            self.advance();
            let mut bounds = Vec::new();
            loop {
                let bound = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string()));
                    }
                };
                bounds.push(bound);
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
                param: name.clone(),
                bounds,
            });
        }
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();

        while !self.check(&Token::RBrace) {
            // cjc 兼容: 主构造函数前可有可见性，如 public StructName(...)
            if self.check(&Token::Public)
                || self.check(&Token::Private)
                || self.check(&Token::Protected)
                || self.check(&Token::Internal)
            {
                self.advance();
            }
            // cjc 兼容: struct 主构造函数 StructName(var a: T, var b: U) { } — 参数即字段
            if self.peek_ident_eq(&name) && matches!(self.peek_next(), Some(Token::LParen)) {
                self.advance(); // consume struct name
                self.expect(Token::LParen)?;
                // 解析 (public|private|...)? (var|let)? name: Type, ... 作为主构造参数，并转为 fields
                while !self.check(&Token::RParen) {
                    if self.check(&Token::Public)
                        || self.check(&Token::Private)
                        || self.check(&Token::Protected)
                        || self.check(&Token::Internal)
                    {
                        self.advance();
                    }
                    if self.check(&Token::Var) || self.check(&Token::Let) {
                        self.advance();
                    }
                    let param_name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::RParen);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                        }
                    };
                    // cjc 兼容: struct 主构造参数支持必需命名参数标记 name!: Type
                    if self.check(&Token::Bang) {
                        self.advance();
                    }
                    self.expect(Token::Colon)?;
                    let ty = self.parse_type()?;
                    fields.push(FieldDef {
                        name: param_name,
                        ty,
                        default: None,
                    });
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(Token::RParen)?;
                // 跳过可选的 { body } 或 ;
                if self.check(&Token::LBrace) {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LBrace) => depth += 1,
                            Some(Token::RBrace) => depth -= 1,
                            None => return self.bail(ParseError::UnexpectedEof),
                            _ => {}
                        }
                    }
                } else if self.check(&Token::Semicolon) {
                    self.advance();
                }
                continue;
            }

            // cjc 兼容: [const] init 构造函数 — 解析并忽略 body（cjwasm 通过字段顺序构造）
            let is_const_init =
                self.check(&Token::Const) && matches!(self.peek_next(), Some(Token::Init));
            if self.check(&Token::Init) || is_const_init {
                if is_const_init {
                    self.advance(); // consume 'const'
                }
                self.advance(); // consume 'init'
                self.expect(Token::LParen)?;
                let _params = self.parse_params()?;
                self.expect(Token::RParen)?;
                self.expect(Token::LBrace)?;
                // 跳过 init body（平衡大括号）
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

            // cjc 兼容: struct 内部方法 → 转为外部方法 func StructName.method(self, ...)
            if self.check(&Token::Func) {
                // 预设 receiver_name 使方法体内 this 可用
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let mut func = self.parse_function_with_visibility(Visibility::Public)?;
                self.receiver_name = prev_receiver;
                // 重命名为 StructName.methodName
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                // 添加隐式 self 参数（如果没有）
                let has_self = func
                    .params
                    .iter()
                    .any(|p| p.name == "self" || p.name == "this");
                if !has_self {
                    func.params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                methods.push(func);
                continue;
            }

            // 普通字段
            // cjc 兼容: 跳过可选的可见性、static、及 var/let 前缀 (public/private [static] var/let name: Type)
            if self.check(&Token::Public)
                || self.check(&Token::Private)
                || self.check(&Token::Protected)
                || self.check(&Token::Internal)
            {
                self.advance();
            }
            // static 后跟 let/var/const → 静态字段，消费 static；static 后跟 func/init 等 → 下一轮匹配 Func 等，消费 static 并 continue
            if self.check(&Token::Static) {
                match self.peek_next() {
                    Some(Token::Let) | Some(Token::Var) | Some(Token::Const) => {
                        self.advance(); // 静态字段
                    }
                    _ => {
                        self.advance(); // static func / static init 等，交给下一轮
                        continue;
                    }
                }
            }
            // public override operator func ... → 跳过修饰符，下一轮匹配 Func；若消费了 operator 则设标志供 parse_function 解析运算符名
            while self.check(&Token::Override)
                || self.check(&Token::Redef)
                || self.check(&Token::Operator)
                || self.check(&Token::Mut)
            {
                if self.check(&Token::Operator) {
                    self.parsing_operator_func = true;
                }
                self.advance();
            }
            if self.check(&Token::Func) {
                continue; // 下一轮循环会匹配上面的「if self.check(&Token::Func)」并解析方法
            }
            // prop name: Type { get() { ... } set(value) { ... } } — 与带主构造分支相同逻辑
            if self.check(&Token::Prop) {
                self.advance();
                let prop_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()));
                    }
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                self.expect(Token::LBrace)?;
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
                            methods.push(crate::ast::Function {
                                visibility: crate::ast::Visibility::Public,
                                name: format!("{}.__get_{}", name, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![Param {
                                    name: "this".to_string(),
                                    ty: Type::Struct(name.clone(), vec![]),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: Some(prop_ty.clone()),
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = if self.check(&Token::Underscore) {
                                self.advance();
                                "_".to_string()
                            } else {
                                match self.advance_ident() {
                                    Some(n) => n,
                                    None => {
                                        let tok = self.advance().unwrap_or(Token::Semicolon);
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "setter 参数名".to_string(),
                                        ));
                                    }
                                }
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(crate::ast::Function {
                                visibility: crate::ast::Visibility::Public,
                                name: format!("{}.__set_{}", name, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![
                                    Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(name.clone(), vec![]),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                    Param {
                                        name: val_name,
                                        ty: prop_ty.clone(),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                ],
                                return_type: None,
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Ident(kw),
                                "get 或 set".to_string(),
                            ));
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                continue;
            }
            if self.check(&Token::Var) || self.check(&Token::Let) || self.check(&Token::Const) {
                self.advance();
            }
            let field_name = match self.advance_ident() {
                Some(name) => name,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                }
            };
            // 支持 name = expr（无类型）或 name: Type [= expr]
            let (ty, default) = if self.check(&Token::Assign) {
                self.advance();
                (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
            } else if self.check(&Token::Colon) {
                self.advance();
                let ty = self.parse_type()?;
                let default = if self.check(&Token::Assign) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                (ty, default)
            } else {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字段: 期望 : 类型 或 = 值".to_string(),
                ));
            };

            fields.push(FieldDef {
                name: field_name,
                ty,
                default,
            });

            if self.check(&Token::Comma) || self.check(&Token::Semicolon) {
                self.advance();
            }
        }

        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;

        // 将 struct 内部方法存入 pending_methods，在解析完成后合并到 functions
        self.pending_struct_methods.extend(methods);

        Ok(StructDef {
            visibility,
            name,
            type_params,
            constraints,
            fields,
        })
    }

    /// 解析枚举定义（支持无关联值或单关联类型变体，如 Ok(Int64)）
    pub(crate) fn parse_enum(&mut self) -> Result<EnumDef, ParseErrorAt> {
        self.parse_enum_with_visibility(Visibility::default())
    }

    /// 解析枚举定义（带可见性）
    pub(crate) fn parse_enum_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<EnumDef, ParseErrorAt> {
        self.expect(Token::Enum)?;
        let name = match self.advance() {
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(n))) => n,
            Some(Token::Ident(n)) => n,
            // cjc: 允许关键字作为枚举名 (如 Result, Option 等)
            Some(Token::TypeResult) => "Result".to_string(),
            Some(Token::TypeOption) => "Option".to_string(),
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "枚举名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 解析可选的泛型类型参数 <T, E: Bound, ...>
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // cjc: enum Name <: Protocol<Type> { ... } — 消费可选的 <: Bound 或 <: Bound<T>
        if self.check(&Token::SubType)
            || self.check(&Token::Colon)
            || (self.check(&Token::Lt)
                && (matches!(self.peek_next(), Some(Token::Colon)) || self.peek_next_ident_like()))
        {
            if self.check(&Token::Lt)
                && (matches!(self.peek_next(), Some(Token::Colon)) || self.peek_next_ident_like())
            {
                self.advance(); // Lt
                if self.check(&Token::Colon) {
                    self.advance();
                }
            } else {
                self.advance();
            }
            if self.advance_ident().is_none() {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "协议/约束名".to_string()));
            };
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            // cjc: enum Name <: A & B & C { ... } — 多继承约束用 & 连接
            while self.check(&Token::And) {
                self.advance(); // consume &
                                // 消费下一个约束名
                if self.advance_ident().is_none() {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "协议/约束名".to_string()));
                }
                // 可选的泛型参数
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        let _ = self.parse_type()?;
                        if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        }
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        self.expect(Token::LBrace)?;
        let mut variants = Vec::new();
        while !self.check(&Token::RBrace) {
            // cjc 兼容: 跳过 enum 内方法前的可见性修饰符（如 public operator func）
            while self.check(&Token::Public)
                || self.check(&Token::Private)
                || self.check(&Token::Protected)
                || self.check(&Token::Internal)
            {
                self.advance();
            }
            // cjc: enum 方法支持多种修饰符顺序，如
            // public redef static func f() {}
            // static public func f() {}
            let mut is_static_method = false;
            loop {
                if self.check(&Token::Public)
                    || self.check(&Token::Private)
                    || self.check(&Token::Protected)
                    || self.check(&Token::Internal)
                    || self.check(&Token::Open)
                    || self.check(&Token::Override)
                    || self.check(&Token::Redef)
                    || self.check(&Token::Mut)
                    || self.check(&Token::Unsafe)
                {
                    self.advance();
                    continue;
                }
                if self.check(&Token::Static) {
                    is_static_method = true;
                    self.advance();
                    continue;
                }
                break;
            }
            // cjc 兼容: enum 内部 prop 声明，解析后丢弃
            if self.check(&Token::Prop) {
                self.advance(); // consume prop
                let _ = self.advance(); // prop name
                if self.check(&Token::Colon) {
                    self.advance();
                    let _ = self.parse_type()?;
                }
                // 跳过 { get() { ... } set(...) { ... } } 块
                self.expect(Token::LBrace)?;
                let mut depth = 1usize;
                while depth > 0 {
                    match self.advance() {
                        Some(Token::LBrace) => depth += 1,
                        Some(Token::RBrace) => depth -= 1,
                        None => break,
                        _ => {}
                    }
                }
                continue;
            }
            // cjc 兼容: operator func（与 class 一致，由 parse_function 按运算符解析）
            if self.check(&Token::Operator) {
                self.parsing_operator_func = true;
                self.advance();
            }
            // cjc 兼容: enum 内部方法 → 转为外部方法 func EnumName.method(this, ...)
            if self.check(&Token::Func) {
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let mut func = self.parse_function_with_visibility(Visibility::Public)?;
                self.receiver_name = prev_receiver;
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                let has_self = func
                    .params
                    .iter()
                    .any(|p| p.name == "self" || p.name == "this");
                if !has_self && !is_static_method {
                    func.params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                self.pending_struct_methods.push(func);
                continue;
            }

            // cjc 兼容: 跳过可选的 | 前缀
            if self.check(&Token::Pipe) {
                self.advance();
            }
            // cjc: `| ...` 是枚举的 "catch-all" 占位符，跳过即可
            if self.check(&Token::DotDotDot) {
                self.advance();
                continue;
            }
            let v_name = match self.advance() {
                Some(Token::BacktickStringLit(StringOrInterpolated::Plain(n))) => n,
                Some(Token::Ident(n)) => n,
                // cjc: 关键字可作为变体名 (Ok, Err, Some, None, Option, etc.)
                Some(Token::Ok) => "Ok".to_string(),
                Some(Token::Err) => "Err".to_string(),
                Some(Token::Some) => "Some".to_string(),
                Some(Token::None) => "None".to_string(),
                Some(Token::TypeOption) => "Option".to_string(),
                Some(Token::TypeResult) => "Result".to_string(),
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "变体名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            let payload = if self.check(&Token::LParen) {
                self.advance();
                let mut types = vec![self.parse_type()?];
                while self.check(&Token::Comma) {
                    self.advance();
                    types.push(self.parse_type()?);
                }
                self.expect(Token::RParen)?;
                let ty = if types.len() == 1 {
                    types.into_iter().next().unwrap()
                } else {
                    Type::Tuple(types)
                };
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
        Ok(EnumDef {
            visibility,
            name,
            type_params,
            constraints,
            variants,
        })
    }

    /// 解析接口定义（支持继承、默认实现、关联类型）
    /// interface Name: Parent1, Parent2 { type Element; func method(args): Ret; func default_method(args): Ret { body } }
    pub(crate) fn parse_interface_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<crate::ast::InterfaceDef, ParseErrorAt> {
        self.expect(Token::Interface)?;
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string()));
            }
        };
        // 解析接口继承 : Parent1, Parent2 或 <: Parent (cjc)
        let parents = if self.check(&Token::Colon) || self.check(&Token::SubType) {
            self.advance();
            let mut ps = Vec::new();
            loop {
                ps.push(match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "父接口名".to_string()));
                    }
                });
                // cjc: 多继承用 & 分隔 (interface Foo <: A & B)；也兼容逗号
                if self.check(&Token::And) || self.check(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            ps
        } else {
            Vec::new()
        };
        // 兜底: 父接口名后 <T> 若未消费，此处消费（如 interface Foo <: Bar<T> {）
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        while !self.check(&Token::RBrace) {
            // 关联类型: type Element; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string()));
                    }
                };
                self.expect(Token::Semicolon)?;
                assoc_types.push(crate::ast::AssocTypeDef { name: type_name });
                continue;
            }
            self.skip_optional_attributes()?;
            // cjc: 接口中的 prop 声明，脱糖为抽象 getter/setter 方法
            if self.check(&Token::Prop) {
                self.advance(); // consume prop
                let prop_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()));
                    }
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                // 抽象 prop 可以没有 { get()... } 块（纯签名声明）
                if !self.check(&Token::LBrace) {
                    methods.push(crate::ast::InterfaceMethod {
                        name: format!("__get_{}", prop_name),
                        type_params: vec![],
                        constraints: vec![],
                        params: vec![],
                        return_type: Some(prop_ty),
                        default_body: None,
                    });
                    continue;
                }
                self.advance(); // consume {
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            let default_body = if self.check(&Token::LBrace) {
                                self.advance();
                                self.receiver_name = Some("this".to_string());
                                let body = self.parse_stmts()?;
                                self.receiver_name = None;
                                self.expect(Token::RBrace)?;
                                Some(body)
                            } else {
                                if self.check(&Token::Semicolon) {
                                    self.advance();
                                }
                                None
                            };
                            methods.push(crate::ast::InterfaceMethod {
                                name: format!("__get_{}", prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![],
                                return_type: Some(prop_ty.clone()),
                                default_body,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = if self.check(&Token::Underscore) {
                                self.advance();
                                "_".to_string()
                            } else {
                                match self.advance_ident() {
                                    Some(n) => n,
                                    None => {
                                        let tok = self.advance().unwrap_or(Token::Semicolon);
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "setter 参数名".to_string(),
                                        ));
                                    }
                                }
                            };
                            self.expect(Token::RParen)?;
                            let default_body = if self.check(&Token::LBrace) {
                                self.advance();
                                self.receiver_name = Some("this".to_string());
                                let body = self.parse_stmts()?;
                                self.receiver_name = None;
                                self.expect(Token::RBrace)?;
                                Some(body)
                            } else {
                                if self.check(&Token::Semicolon) {
                                    self.advance();
                                }
                                None
                            };
                            methods.push(crate::ast::InterfaceMethod {
                                name: format!("__set_{}", prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![Param {
                                    name: val_name,
                                    ty: prop_ty.clone(),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: None,
                                default_body,
                            });
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                continue;
            }
            // cjc: 接口方法可有 static / public / protected / override / mut 修饰符（忽略）
            while self.check(&Token::Static)
                || self.check(&Token::Public)
                || self.check(&Token::Protected)
                || self.check(&Token::Override)
                || self.check(&Token::Redef)
                || self.check(&Token::Open)
                || self.check(&Token::Mut)
            {
                self.advance();
            }
            // After consuming modifiers, could still be prop or func
            if self.check(&Token::Prop) {
                // continue循环头部会重新处理 prop
                continue;
            }
            self.expect(Token::Func)?;
            let (m_name, type_params, mut constraints) = match self.advance_ident() {
                Some(n) => {
                    let (tp, tc) = self.parse_type_params_with_constraints()?;
                    (n, tp, tc)
                }
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string()));
                }
            };
            let prev_type_params =
                std::mem::replace(&mut self.current_type_params, type_params.clone());
            self.expect(Token::LParen)?;
            let params = self.parse_params()?;
            self.expect(Token::RParen)?;
            let return_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            // 可选 where 子句（如 func write<T>(v: T): Unit where T <: ToString { ... }）
            let where_constraints = self.parse_where_clause()?;
            constraints.extend(where_constraints);
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            // 判断有无默认实现 { body } 或者纯签名（分号可选，cjc 兼容）
            let default_body = if self.check(&Token::LBrace) {
                self.advance();
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let body = self.parse_stmts()?;
                self.receiver_name = prev_receiver;
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
                type_params,
                constraints,
                params,
                return_type,
                default_body,
            });
            self.current_type_params = prev_type_params;
        }
        self.expect(Token::RBrace)?;
        Ok(crate::ast::InterfaceDef {
            visibility,
            name,
            parents,
            methods,
            assoc_types,
        })
    }

    /// 解析 extend 定义
    /// extend TypeName: InterfaceName { type Element = ConcreteType; func method(...): ... { ... } }
    pub(crate) fn parse_extend(&mut self) -> Result<crate::ast::ExtendDef, ParseErrorAt> {
        self.expect(Token::Extend)?;
        // cjc: extend<T> Array<T> — 可选的扩展泛型参数
        if self.check(&Token::Lt) {
            let _ = self.parse_type_params()?;
        }
        let target_type = match self.advance() {
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(n))) => n,
            Some(Token::Ident(n)) => n,
            Some(Token::TypeRune) => "Rune".to_string(),
            Some(Token::TypeInt64) => "Int64".to_string(),
            Some(Token::TypeInt32) => "Int32".to_string(),
            Some(Token::TypeInt16) => "Int16".to_string(),
            Some(Token::TypeInt8) => "Int8".to_string(),
            Some(Token::TypeUInt64) => "UInt64".to_string(),
            Some(Token::TypeUInt32) => "UInt32".to_string(),
            Some(Token::TypeUInt16) => "UInt16".to_string(),
            Some(Token::TypeUInt8) => "UInt8".to_string(),
            Some(Token::TypeIntNative) => "IntNative".to_string(),
            Some(Token::TypeUIntNative) => "UIntNative".to_string(),
            Some(Token::TypeString) => "String".to_string(),
            Some(Token::TypeBool) => "Bool".to_string(),
            Some(Token::TypeArray) => "Array".to_string(),
            Some(Token::TypeFloat64) => "Float64".to_string(),
            Some(Token::TypeFloat32) => "Float32".to_string(),
            Some(Token::TypeFloat16) => "Float16".to_string(),
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "类型名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 目标类型的泛型实参 Array<T>
        let _ = self.parse_opt_type_args()?;
        // 可选: 实现的接口 (cjc: extend Type <: InterfaceName 或 <: InterfaceName<T>)
        // 兼容: <: 为 SubType；或 < 与 : 分开为 Lt+Colon；或误写为 < InterfaceName 即 Lt+Ident
        let interface = if self.check(&Token::Colon)
            || self.check(&Token::SubType)
            || (self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Colon)))
            || (self.check(&Token::Lt) && self.peek_next_ident_like())
        {
            if self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Colon)) {
                self.advance(); // Lt
                self.advance(); // Colon
            } else if self.check(&Token::Lt) && self.peek_next_ident_like() {
                self.advance(); // Lt，接口名在下一 token
            } else {
                self.advance();
            }
            let name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string()));
                }
            };
            // 消费接口的类型实参，如 SortByExtension<T> 中的 <T>（此处确定是泛型，直接解析）
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            // cjc: extend Type <: A & B { ... } — 跳过后续 & Bound / & Bound<T>
            while self.check(&Token::And) {
                self.advance();
                if self.advance_ident().is_none() {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string()));
                };
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        let _ = self.parse_type()?;
                        if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        }
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `>`".to_string(),
                            ));
                        }
                    }
                }
            }
            Some(name)
        } else {
            None
        };
        // cjc: extend Type where T <: Bound { ... }
        let _ = self.parse_where_clause()?;
        // 兜底: 接口名或 where 中 Bound 后的 <T> 若未消费，此处消费（避免 Lt 期望 LBrace）
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        let mut assoc_type_bindings = Vec::new();
        while !self.check(&Token::RBrace) {
            // 跳过可见性修饰符 (public / private / protected / internal)
            while matches!(
                self.peek(),
                Some(Token::Public)
                    | Some(Token::Private)
                    | Some(Token::Protected)
                    | Some(Token::Internal)
            ) {
                self.advance();
            }
            // 关联类型绑定: type Element = ConcreteType; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string()));
                    }
                };
                self.expect(Token::Assign)?;
                let ty = self.parse_type()?;
                self.expect(Token::Semicolon)?;
                assoc_type_bindings.push((type_name, ty));
                continue;
            }
            if self.check(&Token::At) {
                self.skip_optional_attributes()?;
                continue;
            }
            // 方法: [public/private/...] func name(args): Ret { body } (cjc 兼容)
            let member_vis = if self.check(&Token::Public) {
                self.advance();
                crate::ast::Visibility::Public
            } else if self.check(&Token::Private) {
                self.advance();
                crate::ast::Visibility::Private
            } else if self.check(&Token::Protected) {
                self.advance();
                crate::ast::Visibility::Protected
            } else if self.check(&Token::Internal) {
                self.advance();
                crate::ast::Visibility::Internal
            } else {
                crate::ast::Visibility::default()
            };
            // prop in extend block: prop name: Type { get() { ... } set(v) { ... } }
            if self.check(&Token::Prop) {
                self.advance();
                let prop_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()));
                    }
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                self.expect(Token::LBrace)?;
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
                            methods.push(Function {
                                visibility: member_vis.clone(),
                                name: format!("{}.__get_{}", target_type, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![Param {
                                    name: "this".to_string(),
                                    ty: Type::Struct(target_type.clone(), vec![]),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: Some(prop_ty.clone()),
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = if self.check(&Token::Underscore) {
                                self.advance();
                                "_".to_string()
                            } else {
                                match self.advance_ident() {
                                    Some(n) => n,
                                    None => {
                                        let tok = self.advance().unwrap_or(Token::Semicolon);
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "setter 参数名".to_string(),
                                        ));
                                    }
                                }
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(Function {
                                visibility: member_vis.clone(),
                                name: format!("{}.__set_{}", target_type, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![
                                    Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(target_type.clone(), vec![]),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                    Param {
                                        name: val_name,
                                        ty: prop_ty.clone(),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                ],
                                return_type: None,
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                continue;
            }
            // cjc 兼容: extend 内方法前可带 static / override / operator，跳过后由 parse_function 解析
            while self.check(&Token::Static)
                || self.check(&Token::Override)
                || self.check(&Token::Redef)
                || self.check(&Token::Operator)
            {
                self.advance();
            }
            let prev_receiver = self.receiver_name.clone();
            self.receiver_name = Some("this".to_string());
            let mut func = self.parse_function_with_visibility(member_vis)?;
            self.receiver_name = prev_receiver;
            // P3: 添加隐式 this 参数（如同 struct/class 方法）
            let has_self = func
                .params
                .iter()
                .any(|p| p.name == "self" || p.name == "this");
            if !has_self && !func.name.starts_with("static ") {
                func.params.insert(
                    0,
                    crate::ast::Param {
                        name: "this".to_string(),
                        ty: Type::Struct(target_type.clone(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    },
                );
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
        Ok(crate::ast::ExtendDef {
            target_type,
            interface,
            assoc_type_bindings,
            methods,
        })
    }

    /// 解析类定义
    pub(crate) fn parse_class_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<crate::ast::ClassDef, ParseErrorAt> {
        // 解析可选修饰符: abstract / sealed / open（任意顺序，如 sealed abstract class）
        let mut is_abstract = false;
        let mut is_sealed = false;
        let mut is_open = false;
        loop {
            if self.check(&Token::Abstract) {
                self.advance();
                is_abstract = true;
            } else if self.check(&Token::Sealed) {
                self.advance();
                is_sealed = true;
            } else if self.check(&Token::Open) {
                self.advance();
                is_open = true;
            } else {
                break;
            }
        }
        self.expect(Token::Class)?;
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "类名".to_string()));
            }
        };
        // 解析可选的泛型类型参数 <T, U: Bound, ...>
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // cjc: 使用 <: 表示继承 (class Foo <: Base & Interface1 & Interface2)
        let (extends, implements) = if self.check(&Token::SubType) {
            self.advance();
            let mut types = Vec::new();
            loop {
                // 解析类型（支持泛型，如 Iterable<Token>）
                let ty = self.parse_type()?;
                // 提取基础类型名
                let type_name = match ty {
                    Type::Struct(name, _) => name,
                    _ => {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Ident("unknown".to_string()),
                            "父类或接口名".to_string(),
                        ))
                    }
                };
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
                let (pty, pdefault) = if self.check(&Token::Colon) {
                    self.advance();
                    let pty = self.parse_type()?;
                    let pdefault = if self.check(&Token::Assign) {
                        self.advance();
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    (pty, pdefault)
                } else if self.check(&Token::Assign) {
                    self.advance();
                    (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Colon),
                        "`:` 或 `=`".to_string(),
                    ));
                };
                primary_ctor_params.push(Param {
                    name: pname,
                    ty: pty,
                    default: pdefault,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                });
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
            self.expect(Token::RParen)?;
        }
        // cjc: class Foo <: Base where T <: Object { ... }
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        // 兜底: 父类/接口名后 <T> 若未消费，此处消费（如 class Foo <: Bar<T> {）
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // 可能在消费了父类泛型参数后才出现 where 子句 (class Foo <: Bar<T> where T <: Bound { ... })
        let where_constraints2 = self.parse_where_clause()?;
        constraints.extend(where_constraints2);
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut init = None;
        let mut deinit = None;
        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) {
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
            // 跳过 mut 修饰符（如 public mut prop）
            if self.check(&Token::Mut) {
                self.advance();
            }
            if self.check(&Token::Var) || self.check(&Token::Let) {
                self.advance();
                let f_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                    }
                };
                // 支持 let name = expr（无类型）或 let name: Type [= expr]
                let (ty, default) = if self.check(&Token::Assign) {
                    self.advance();
                    let prev_receiver = self.receiver_name.clone();
                    self.receiver_name = Some("this".to_string());
                    let expr = self.parse_expr()?;
                    self.receiver_name = prev_receiver;
                    (crate::ast::Type::TypeParam("_".to_string()), Some(expr))
                } else if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    let default = if self.check(&Token::Assign) {
                        self.advance();
                        let prev_receiver = self.receiver_name.clone();
                        self.receiver_name = Some("this".to_string());
                        let expr = self.parse_expr()?;
                        self.receiver_name = prev_receiver;
                        Some(expr)
                    } else {
                        None
                    };
                    (ty, default)
                } else {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(
                        tok,
                        "字段: 期望 : 类型 或 = 值".to_string(),
                    ));
                };
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                fields.push(crate::ast::FieldDef {
                    name: f_name,
                    ty,
                    default,
                });
            } else if self.check(&Token::Init) {
                self.advance();
                self.expect(Token::LParen)?;
                let params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // init(output: T) where T <: Bound { ... }
                let _ = self.parse_where_clause()?;
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        let _ = self.parse_type()?;
                        if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        }
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `>`".to_string(),
                            ));
                        }
                    }
                }
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
                let prop_name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()));
                    }
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
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
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
                            let val_name = if self.check(&Token::Underscore) {
                                self.advance();
                                "_".to_string()
                            } else {
                                match self.advance_ident() {
                                    Some(n) => n,
                                    None => {
                                        let tok = self.advance().unwrap_or(Token::Semicolon);
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "setter 参数名".to_string(),
                                        ));
                                    }
                                }
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
                                            variadic: false,
                                            is_named: false,
                                            is_inout: false,
                                        },
                                        Param {
                                            name: val_name,
                                            ty: prop_ty.clone(),
                                            default: None,
                                            variadic: false,
                                            is_named: false,
                                            is_inout: false,
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
                                Token::Ident(kw),
                                "get 或 set".to_string(),
                            ));
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
            } else if {
                // 仓颉允许用类名代替 init 作为构造函数: ClassName(let param: Type) {}
                let is_named_ctor = self.peek_ident_eq(&name);
                is_named_ctor && matches!(self.peek_next(), Some(Token::LParen))
            } {
                self.advance(); // 消费类名
                self.expect(Token::LParen)?;
                let mut params = Vec::new();
                while !self.check(&Token::RParen) {
                    // 跳过可选的可见性修饰符 (private/public/protected/internal)
                    if self.check(&Token::Private)
                        || self.check(&Token::Public)
                        || self.check(&Token::Protected)
                        || self.check(&Token::Internal)
                    {
                        self.advance();
                    }
                    // 跳过可选的 let/var 前缀
                    if self.check(&Token::Let) || self.check(&Token::Var) {
                        self.advance();
                    }
                    let pname = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "构造函数参数名".to_string(),
                            ));
                        }
                    };
                    // cjc: 支持必需命名参数 param!: Type（! 表示调用时必须使用命名参数）
                    let is_required_named = if self.check(&Token::Bang) {
                        self.advance();
                        true
                    } else {
                        false
                    };
                    let (pty, pdefault) = if self.check(&Token::Assign) {
                        self.advance();
                        (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                    } else if self.check(&Token::Colon) {
                        self.advance();
                        let pty = self.parse_type()?;
                        let pdefault = if self.check(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        (pty, pdefault)
                    } else {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "构造函数参数: 期望 : 类型 或 = 默认值".to_string(),
                        ));
                    };
                    params.push(Param {
                        name: pname,
                        ty: pty,
                        default: pdefault,
                        variadic: false,
                        is_named: is_required_named, // 必需命名参数标记为 is_named
                        is_inout: false,
                    });
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(Token::RParen)?;
                let _ = self.parse_where_clause()?;
                self.receiver_name = Some("this".to_string());
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.receiver_name = None;
                init = Some(crate::ast::InitDef { params, body });
            } else if self.check(&Token::At) {
                self.skip_optional_attributes()?;
                continue; // 跳过属性后重新循环，下一次会看到 Func 等
            } else if self.check(&Token::Open)
                || self.check(&Token::Static)
                || self.check(&Token::Override)
                || self.check(&Token::Redef)
                || self.check(&Token::Unsafe)
                || self.check(&Token::Func)
                || self.check(&Token::Operator)
            {
                // cjc: open / static / override / operator 修饰符在方法前
                if self.check(&Token::Open) {
                    self.advance(); // 消费 open，cjwasm 不区分 open/非 open
                }
                // P2.4: 记录 static 修饰符
                let mut is_static = self.check(&Token::Static);
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
                    // static const NAME: Type = expr — 类级别常量，当作字段处理
                    if self.check(&Token::Const)
                        || self.check(&Token::Let)
                        || self.check(&Token::Var)
                    {
                        self.advance(); // 消费 const/let/var
                        let f_name = match self.advance_ident() {
                            Some(n) => n,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self
                                    .bail(ParseError::UnexpectedToken(tok, "常量名".to_string()));
                            }
                        };
                        let (ty, default) = if self.check(&Token::Assign) {
                            self.advance();
                            (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                        } else if self.check(&Token::Colon) {
                            self.advance();
                            let ty = self.parse_type()?;
                            let default = if self.check(&Token::Assign) {
                                self.advance();
                                Some(self.parse_expr()?)
                            } else {
                                None
                            };
                            (ty, default)
                        } else {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "常量: 期望 : 类型 或 = 值".to_string(),
                            ));
                        };
                        if self.check(&Token::Semicolon) {
                            self.advance();
                        }
                        fields.push(crate::ast::FieldDef {
                            name: f_name,
                            ty,
                            default,
                        });
                        continue;
                    }
                }
                let override_ = self.check(&Token::Override) || self.check(&Token::Redef);
                if override_ {
                    self.advance();
                }
                // override 后可能还有 open 修饰符 (protected override open func)
                if self.check(&Token::Open) {
                    self.advance();
                }
                // 兼容 `redef static func` 等修饰符顺序
                if !is_static && self.check(&Token::Static) {
                    self.advance();
                    is_static = true;
                }
                // override prop — 带 override 的属性声明脱糖为 getter/setter 方法
                if self.check(&Token::Prop) {
                    self.advance(); // consume prop
                    let prop_name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "属性名".to_string()));
                        }
                    };
                    self.expect(Token::Colon)?;
                    let prop_ty = self.parse_type()?;
                    self.expect(Token::LBrace)?;
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
                                methods.push(crate::ast::ClassMethod {
                                    override_: true,
                                    func: crate::ast::Function {
                                        visibility: member_vis.clone(),
                                        name: format!("{}.__get_{}", name, prop_name),
                                        type_params: vec![],
                                        constraints: vec![],
                                        params: vec![Param {
                                            name: "this".to_string(),
                                            ty: Type::Struct(name.clone(), vec![]),
                                            default: None,
                                            variadic: false,
                                            is_named: false,
                                            is_inout: false,
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
                                let val_name = if self.check(&Token::Underscore) {
                                    self.advance();
                                    "_".to_string()
                                } else {
                                    match self.advance_ident() {
                                        Some(n) => n,
                                        None => {
                                            let tok = self.advance().unwrap_or(Token::Semicolon);
                                            return self.bail(ParseError::UnexpectedToken(
                                                tok,
                                                "setter 参数名".to_string(),
                                            ));
                                        }
                                    }
                                };
                                self.expect(Token::RParen)?;
                                self.receiver_name = Some("this".to_string());
                                self.expect(Token::LBrace)?;
                                let body = self.parse_stmts()?;
                                self.expect(Token::RBrace)?;
                                self.receiver_name = None;
                                methods.push(crate::ast::ClassMethod {
                                    override_: true,
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
                                                variadic: false,
                                                is_named: false,
                                                is_inout: false,
                                            },
                                            Param {
                                                name: val_name,
                                                ty: prop_ty.clone(),
                                                default: None,
                                                variadic: false,
                                                is_named: false,
                                                is_inout: false,
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
                                    Token::Ident(kw),
                                    "get 或 set".to_string(),
                                ));
                            }
                        } else {
                            break;
                        }
                    }
                    self.expect(Token::RBrace)?;
                    continue;
                }
                // P3.1: operator func +/-/*/==/</>/<=/>=
                // cjc 兼容: unsafe func — 忽略 unsafe 修饰符，当普通方法处理
                if self.check(&Token::Unsafe) {
                    self.advance(); // 消费 unsafe
                }
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
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "运算符".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    (format!("{}.{}", name, op_name), vec![])
                } else {
                    match self.advance_ident() {
                        Some(n) => {
                            let tp = self.parse_type_params()?;
                            (format!("{}.{}", name, n), tp)
                        }
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "方法名".to_string()));
                        }
                    }
                };
                // 合并类的泛型参数与方法自身的泛型参数，使类的 T 在方法体和返回类型中可识别为 TypeParam
                let mut merged_type_params = self.current_type_params.clone();
                merged_type_params.extend(type_params.clone());
                let prev_params =
                    std::mem::replace(&mut self.current_type_params, merged_type_params);
                self.expect(Token::LParen)?;
                let mut params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // cjc 兼容: 无 self/this 时添加隐式 this 参数（P2.4: static 方法除外）
                let has_self = params.iter().any(|p| p.name == "self" || p.name == "this");
                if !has_self && !is_static {
                    params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                self.receiver_name = Some(
                    params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "this".to_string()),
                );
                let return_type = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 可选 where 子句（如 func write<T>(v: T): Unit where T <: ToString { ... }）
                let mut method_constraints = vec![];
                while matches!(self.peek(), Some(Token::Where)) {
                    let w = self.parse_where_clause()?;
                    method_constraints.extend(w);
                    if self.check(&Token::Lt) {
                        self.advance();
                        loop {
                            let _ = self.parse_type()?;
                            if self.check(&Token::Gt) {
                                self.advance();
                                break;
                            }
                            if self.check(&Token::Comma) {
                                self.advance();
                            } else {
                                return self.bail(ParseError::UnexpectedToken(
                                    self.peek().cloned().unwrap_or(Token::Comma),
                                    "`,` 或 `>`".to_string(),
                                ));
                            }
                        }
                    }
                }
                // P2.5: 支持抽象方法（无 body，以 ; 结尾或后跟下一成员/可见性关键字）
                let body = if self.check(&Token::Semicolon)
                    || self.check(&Token::RBrace)
                    || matches!(
                        self.peek(),
                        Some(Token::Public)
                            | Some(Token::Private)
                            | Some(Token::Protected)
                            | Some(Token::Internal)
                            | Some(Token::Override)
                            | Some(Token::Redef)
                            | Some(Token::Open)
                            | Some(Token::Static)
                            | Some(Token::Func)
                            | Some(Token::Init)
                            | Some(Token::Tilde)
                            | Some(Token::Prop)
                            | Some(Token::Mut)
                            | Some(Token::At)
                    ) {
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
                        constraints: method_constraints,
                        params,
                        return_type,
                        throws: None,
                        body,
                        extern_import: None,
                    },
                });
            } else if self.check(&Token::Const) {
                self.advance(); // consume const
                if self.check(&Token::Init) {
                    // const init() { ... } — 编译期常量构造函数，按普通 init 处理
                    self.advance();
                    self.expect(Token::LParen)?;
                    let params = self.parse_params()?;
                    self.expect(Token::RParen)?;
                    let _ = self.parse_where_clause()?;
                    self.receiver_name = Some("this".to_string());
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    self.receiver_name = None;
                    init = Some(crate::ast::InitDef { params, body });
                } else {
                    // const FIELD_NAME: Type = value — 类级别常量，当作字段处理
                    let f_name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "常量名或 init".to_string(),
                            ));
                        }
                    };
                    let (ty, default) = if self.check(&Token::Assign) {
                        self.advance();
                        (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                    } else if self.check(&Token::Colon) {
                        self.advance();
                        let ty = self.parse_type()?;
                        let default = if self.check(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        (ty, default)
                    } else {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "常量: 期望 : 类型 或 = 值".to_string(),
                        ));
                    };
                    if self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    fields.push(crate::ast::FieldDef {
                        name: f_name,
                        ty,
                        default,
                    });
                }
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "var、let、const、init、~init 或 func".to_string(),
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
            static_init: None, // P3.11: TODO - 从 class body 中解析 static init()
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
                });
            }
            if class_def.init.is_none() {
                let init_body: Vec<Stmt> = primary_ctor_params
                    .iter()
                    .map(|p| Stmt::Assign {
                        target: AssignTarget::Field {
                            object: "this".to_string(),
                            field: p.name.clone(),
                        },
                        value: Expr::Var(p.name.clone()),
                    })
                    .collect();
                class_def.init = Some(InitDef {
                    params: primary_ctor_params,
                    body: init_body,
                });
            }
        }
        Ok(class_def)
    }

    /// 解析函数定义（支持方法名 StructName.methodName）
    pub(crate) fn parse_function(&mut self) -> Result<Function, ParseErrorAt> {
        self.parse_function_with_visibility(Visibility::default())
    }

    /// 解析函数定义（带可见性）
    pub(crate) fn parse_function_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;

        let (name, type_params, mut constraints) = if self.parsing_operator_func {
            self.parsing_operator_func = false;
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
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "运算符".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            (op_name.to_string(), vec![], vec![])
        } else {
            match self.advance_ident() {
                Some(n) => {
                    let (tp, tc) = self.parse_type_params_with_constraints()?;
                    let full_name = if self.check(&Token::Dot) {
                        self.advance();
                        let method = match self.advance_ident() {
                            Some(m) => m,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self
                                    .bail(ParseError::UnexpectedToken(tok, "方法名".to_string()));
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
        } else if params
            .first()
            .map_or(false, |p| p.name == "self" || p.name == "this")
        {
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

        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // 兜底: 若未消费 where（如某路径未调用 parse_where_clause），此处补消费
        while matches!(self.peek(), Some(Token::Where)) {
            let w = self.parse_where_clause()?;
            constraints.extend(w);
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
        }
        // 无 body 的声明：@Intrinsic / extern 等后接 @、;、或下一成员/声明（可见性、func、extend 等）
        let body = if self.check(&Token::At)
            || self.check(&Token::Semicolon)
            || matches!(
                self.peek(),
                Some(Token::Public)
                    | Some(Token::Private)
                    | Some(Token::Protected)
                    | Some(Token::Internal)
                    | Some(Token::Func)
                    | Some(Token::Extend)
                    | Some(Token::Interface)
                    | Some(Token::Class)
                    | Some(Token::Struct)
                    | Some(Token::Enum)
            ) {
            if self.check(&Token::Semicolon) {
                self.advance();
            }
            vec![]
        } else {
            self.expect(Token::LBrace)?;
            let stmts = self.parse_stmts()?;
            self.expect(Token::RBrace)?;
            stmts
        };

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
    pub(crate) fn parse_main_function(
        &mut self,
        visibility: Visibility,
    ) -> Result<Function, ParseErrorAt> {
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
    pub(crate) fn parse_params(&mut self) -> Result<Vec<Param>, ParseErrorAt> {
        let mut params = Vec::new();
        if self.check(&Token::RParen) {
            return Ok(params);
        }

        loop {
            // P6: inout 参数
            let is_inout = if self.check(&Token::Inout) {
                self.advance();
                true
            } else {
                false
            };
            let name = if self.check(&Token::This) {
                self.advance();
                "this".to_string()
            } else if self.check(&Token::Underscore) {
                self.advance();
                "_".to_string()
            } else {
                match self.advance_ident() {
                    Some(name) => name,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                    }
                }
            };
            // P2.9: 命名参数 name!: Type = default，或 name = default（类型推断）
            let is_named = if self.check(&Token::Bang) {
                self.advance();
                true
            } else {
                false
            };
            let (ty, default, variadic) = if self.check(&Token::Colon) {
                self.advance();
                let ty = self.parse_type()?;
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
                (ty, default, variadic)
            } else if self.check(&Token::Assign) {
                self.advance();
                let default = Some(self.parse_expr()?);
                (Type::TypeParam("_".to_string()), default, false)
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Colon),
                    "`:` 或 `=`".to_string(),
                ));
            };
            params.push(Param {
                name,
                ty,
                default,
                variadic,
                is_named,
                is_inout,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(params)
    }
}
