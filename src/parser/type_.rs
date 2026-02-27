//! 类型解析：parse_type、parse_base_type、类型参数与 where 子句。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::{Type, TypeConstraint};
use crate::lexer::Token;

impl Parser {
    /// 判断 token 是否为类型的有效起始（避免将 n < 10 的 < 误解析为类型实参）
    pub(crate) fn is_type_start(t: &Token) -> bool {
        matches!(
            t,
            Token::TypeInt8
                | Token::TypeInt16
                | Token::TypeInt32
                | Token::TypeInt64
                | Token::TypeIntNative
                | Token::TypeUInt8
                | Token::TypeUInt16
                | Token::TypeUInt32
                | Token::TypeUInt64
                | Token::TypeUIntNative
                | Token::TypeFloat16
                | Token::TypeFloat32
                | Token::TypeFloat64
                | Token::TypeRune
                | Token::TypeBool
                | Token::TypeNothing
                | Token::TypeUnit
                | Token::TypeVArray
                | Token::TypeThis
                | Token::TypeString
                | Token::TypeArray
                | Token::TypeTuple
                | Token::TypeRange
                | Token::TypeOption
                | Token::TypeResult
                | Token::TypeSlice
                | Token::TypeMap
                | Token::LParen
                | Token::Question
                | Token::Ident(_)
        )
    }

    /// 解析可选类型实参 <Type1, Type2, ...>，用于调用与实例化
    pub(crate) fn parse_opt_type_args(&mut self) -> Result<Option<Vec<Type>>, ParseErrorAt> {
        if !self.check(&Token::Lt) {
            return Ok(None);
        }
        if !self.peek_next().map(Self::is_type_start).unwrap_or(false) {
            return Ok(None);
        }
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
            } else if self.check(&Token::Gt) {
                self.advance();
                break;
            } else if self.check(&Token::Shr) {
                self.advance();
                self.pushback = Some(Token::Gt);
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
    pub(crate) fn parse_type_params_with_constraints(
        &mut self,
    ) -> Result<(Vec<String>, Vec<TypeConstraint>), ParseErrorAt> {
        if !self.check(&Token::Lt) {
            return Ok((Vec::new(), Vec::new()));
        }
        self.advance();
        let mut params = Vec::new();
        let mut constraints = Vec::new();
        loop {
            let p = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "类型参数名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            if self.check(&Token::Colon) || self.check(&Token::SubType) {
                self.advance();
                let mut bounds = Vec::new();
                loop {
                    let bound = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    bounds.push(bound);
                    if self.check(&Token::And) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                constraints.push(TypeConstraint {
                    param: p.clone(),
                    bounds,
                });
            }
            params.push(p);
            if self.check(&Token::Comma) {
                self.advance();
            } else if self.check(&Token::Gt) {
                self.advance();
                break;
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Comma),
                    "`,` 或 `>`".to_string(),
                ));
            }
        }
        Ok((params, constraints))
    }

    /// 解析泛型类型参数列表 <T, U, ...>（不解析约束）
    pub(crate) fn parse_type_params(&mut self) -> Result<Vec<String>, ParseErrorAt> {
        let (params, _) = self.parse_type_params_with_constraints()?;
        Ok(params)
    }

    /// 解析 where 子句：where T: Bound1 & Bound2, U: Bound3
    pub(crate) fn parse_where_clause(&mut self) -> Result<Vec<TypeConstraint>, ParseErrorAt> {
        if !matches!(self.peek(), Some(Token::Where)) {
            return Ok(Vec::new());
        }
        self.advance();
        let mut constraints = Vec::new();
        loop {
            let param = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "类型参数名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            if !self.check(&Token::Colon) && !self.check(&Token::SubType) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Colon),
                    "`:` 或 `<:`".to_string(),
                ));
            }
            self.advance();
            let mut bounds = Vec::new();
            loop {
                let bound = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
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
            constraints.push(TypeConstraint { param, bounds });
            if self.check(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(constraints)
    }

    /// 解析类型（含 ?T 前缀和 T? 后缀 → Option<T>）
    pub(crate) fn parse_type(&mut self) -> Result<Type, ParseErrorAt> {
        if self.check(&Token::Question) {
            self.advance();
            let inner = self.parse_base_type()?;
            return Ok(Type::Option(Box::new(inner)));
        }
        let mut ty = self.parse_base_type()?;
        while self.check(&Token::Question) {
            self.advance();
            ty = Type::Option(Box::new(ty));
        }
        while self.check(&Token::Bang) {
            self.advance();
        }
        Ok(ty)
    }

    /// 解析基础类型（不含 ? ! 后缀）
    pub(crate) fn parse_base_type(&mut self) -> Result<Type, ParseErrorAt> {
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
            Some(Token::LParen) => {
                let mut types = Vec::new();
                if !self.check(&Token::RParen) {
                    if matches!(self.peek(), Some(Token::Ident(_)))
                        && matches!(self.peek_next(), Some(Token::Colon))
                    {
                        self.advance();
                        self.advance();
                    }
                    types.push(self.parse_type()?);
                    while self.check(&Token::Comma) {
                        self.advance();
                        if matches!(self.peek(), Some(Token::Ident(_)))
                            && matches!(self.peek_next(), Some(Token::Colon))
                        {
                            self.advance();
                            self.advance();
                        }
                        types.push(self.parse_type()?);
                    }
                }
                self.expect(Token::RParen)?;
                if self.check(&Token::Arrow) {
                    self.advance();
                    let ret = self.parse_type()?;
                    Ok(Type::Function {
                        params: types,
                        ret: Box::new(Some(ret)),
                    })
                } else {
                    if types.len() == 1 {
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
                self.expect(Token::Lt)?;
                let mut types = Vec::new();
                loop {
                    types.push(self.parse_type()?);
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else if self.check(&Token::Gt) {
                        self.advance();
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
            Some(Token::TypeThis) => Ok(Type::This), // P2: This 类型
            Some(Token::TypeOption) => {
                self.expect(Token::Lt)?;
                let inner_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Option(Box::new(inner_type)))
            }
            Some(Token::TypeResult) => {
                if self.check(&Token::Lt) {
                    self.advance();
                    let ok_type = self.parse_type()?;
                    let err_type = if self.check(&Token::Comma) {
                        self.advance();
                        self.parse_type()?
                    } else {
                        Type::Struct("Exception".to_string(), vec![])
                    };
                    self.expect(Token::Gt)?;
                    Ok(Type::Result(Box::new(ok_type), Box::new(err_type)))
                } else {
                    Ok(Type::Struct("Result".to_string(), vec![]))
                }
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
                // P1: 检查是否为限定类型 (pkg.Module.Type)
                let mut path = vec![name.clone()];
                while self.check(&Token::Dot) {
                    self.advance();
                    if let Some(Token::Ident(segment)) = self.advance() {
                        path.push(segment);
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Dot),
                            "标识符".to_string(),
                        ));
                    }
                }

                // 如果有多个路径段，返回限定类型
                if path.len() > 1 {
                    return Ok(Type::Qualified(path));
                }

                // 单个标识符的处理
                if self.check(&Token::Lt) {
                    self.advance();
                    let mut type_args = Vec::new();
                    loop {
                        type_args.push(self.parse_type()?);
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        } else if self.check(&Token::Shr) {
                            self.advance();
                            self.pushback = Some(Token::Gt);
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
            Some(tok) => self.bail_at(
                ParseError::UnexpectedToken(tok, "类型".to_string()),
                self.at_prev(),
            ),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }
}
