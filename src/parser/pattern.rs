//! 模式解析：parse_pattern、parse_or_pattern、parse_primary_pattern、parse_pattern_fields。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::{Expr, Literal, Pattern};
use crate::lexer::{Token, StringOrInterpolated};

impl Parser {
    /// 解析模式
    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        self.parse_or_pattern()
    }

    /// 解析 or 模式 (1 | 2 | 3)
    pub(crate) fn parse_or_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
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

    /// 解析变体绑定列表：ident | _ [: Type] (, ident | _ [: Type])* 或嵌套模式 Some(x) 等
    fn parse_variant_bindings(&mut self) -> Result<Vec<Option<String>>, ParseErrorAt> {
        let mut bindings = Vec::new();
        loop {
            let b = match self.peek() {
                Some(Token::Ident(_)) => {
                    let id = match self.advance() {
                        Some(Token::Ident(id)) => id,
                        _ => unreachable!(),
                    };
                    // 跳过可选类型注解 : Type（如 Some(p: PropDecl)）
                    if self.check(&Token::Colon) {
                        self.advance();
                        let _ = self.parse_type()?;
                    }
                    Some(id)
                }
                Some(Token::Underscore) => {
                    self.advance();
                    // 跳过可选类型注解 : Type
                    if self.check(&Token::Colon) {
                        self.advance();
                        let _ = self.parse_type()?;
                    }
                    None
                }
                _ => {
                    // 复杂嵌套模式（如 Some(Some(x))），解析后丢弃绑定名
                    let _ = self.parse_primary_pattern()?;
                    None
                }
            };
            bindings.push(b);
            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }
        Ok(bindings)
    }

    /// 解析基础模式
    pub(crate) fn parse_primary_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        match self.peek() {
            Some(Token::Underscore) => {
                self.advance();
                // 支持 _: Type 类型测试模式（绑定丢弃）
                if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    return Ok(Pattern::TypeTest { binding: "_".to_string(), ty });
                }
                Ok(Pattern::Wildcard)
            }
            Some(Token::Some) => {
                self.advance();
                let bindings = if self.check(&Token::LParen) {
                    self.advance();
                    let b = self.parse_variant_bindings()?;
                    self.expect(Token::RParen)?;
                    b
                } else {
                    vec![]
                };
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "Some".to_string(),
                    bindings,
                })
            }
            Some(Token::None) => {
                self.advance();
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "None".to_string(),
                    bindings: vec![],
                })
            }
            Some(Token::Ok) => {
                self.advance();
                let bindings = if self.check(&Token::LParen) {
                    self.advance();
                    let b = self.parse_variant_bindings()?;
                    self.expect(Token::RParen)?;
                    b
                } else {
                    vec![]
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Ok".to_string(),
                    bindings,
                })
            }
            Some(Token::Err) => {
                self.advance();
                let bindings = if self.check(&Token::LParen) {
                    self.advance();
                    let b = self.parse_variant_bindings()?;
                    self.expect(Token::RParen)?;
                    b
                } else {
                    vec![]
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Err".to_string(),
                    bindings,
                })
            }
            Some(Token::Integer(n)) => {
                let n = *n;
                self.advance();
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
            Some(Token::CharLit(c)) => {
                let c = *c;
                self.advance();
                Ok(Pattern::Literal(Literal::Rune(c)))
            }
            Some(Token::True) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(true)))
            }
            Some(Token::False) => {
                self.advance();
                Ok(Pattern::Literal(Literal::Bool(false)))
            }
            Some(Token::StringLit(s)) | Some(Token::BacktickStringLit(s)) => {
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
            Some(Token::Break) => {
                self.advance();
                Ok(Pattern::Binding("break".to_string()))
            }
            Some(Token::Continue) => {
                self.advance();
                Ok(Pattern::Binding("continue".to_string()))
            }
            Some(Token::Ident(name)) => {
                let name = name.clone();
                self.advance();
                let looks_like_type = name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);
                if looks_like_type && self.check(&Token::Dot) {
                    if let Some(Token::Ident(_)) = self.peek_next() {
                        self.advance();
                        let variant = match self.advance() {
                            Some(Token::Ident(v)) => v.clone(),
                            _ => unreachable!(),
                        };
                        if self.check(&Token::LParen) {
                            self.advance();
                            // ident ) or ident : Type ) or _ ) or _ : Type ) or multi-binding a, b, c
                            let is_binding_list =
                                matches!(self.peek(), Some(Token::Ident(_)) | Some(Token::Underscore))
                                    && matches!(self.peek_next(), Some(Token::RParen) | Some(Token::Colon) | Some(Token::Comma));
                            if is_binding_list {
                                let bindings = self.parse_variant_bindings()?;
                                self.expect(Token::RParen)?;
                                return Ok(Pattern::Variant {
                                    enum_name: name,
                                    variant_name: variant,
                                    bindings,
                                });
                            } else if self.check(&Token::RParen) {
                                self.advance();
                                let call_expr = Expr::MethodCall {
                                    object: Box::new(Expr::Var(name.clone())),
                                    method: variant.clone(),
                                    args: vec![],
                                    named_args: vec![],
                                };
                                let postfix = self.parse_postfix_from_expr(call_expr)?;
                                let full_expr = self.parse_guard_binary_rest(postfix)?;
                                return Ok(Pattern::Guard(Box::new(full_expr)));
                            } else {
                                let (args, named_args) = self.parse_args()?;
                                self.expect(Token::RParen)?;
                                let call_expr = Expr::MethodCall {
                                    object: Box::new(Expr::Var(name.clone())),
                                    method: variant.clone(),
                                    args,
                                    named_args,
                                };
                                let postfix = self.parse_postfix_from_expr(call_expr)?;
                                let full_expr = self.parse_guard_binary_rest(postfix)?;
                                return Ok(Pattern::Guard(Box::new(full_expr)));
                            }
                        } else {
                            return Ok(Pattern::Variant {
                                enum_name: name,
                                variant_name: variant,
                                bindings: vec![],
                            });
                        }
                    }
                }
                if self.check(&Token::LBrace) {
                    self.advance();
                    let fields = self.parse_pattern_fields()?;
                    self.expect(Token::RBrace)?;
                    Ok(Pattern::Struct { name, fields })
                } else if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    Ok(Pattern::TypeTest { binding: name, ty })
                } else if self.check(&Token::Dot) {
                    let base = Expr::Var(name.clone());
                    let postfix_expr = self.parse_postfix_from_expr(base)?;
                    let full_expr = self.parse_guard_binary_rest(postfix_expr)?;
                    return Ok(Pattern::Guard(Box::new(full_expr)));
                } else if looks_like_type && self.check(&Token::LParen) {
                    self.advance();
                    let bindings = if self.check(&Token::RParen) {
                        vec![]
                    } else {
                        self.parse_variant_bindings()?
                    };
                    self.expect(Token::RParen)?;
                    Ok(Pattern::Variant {
                        enum_name: String::new(),
                        variant_name: name,
                        bindings,
                    })
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
            Some(tok) => self.bail_at(
                ParseError::UnexpectedToken(tok.clone(), "模式".to_string()),
                self.at_prev(),
            ),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析结构体解构字段
    pub(crate) fn parse_pattern_fields(&mut self) -> Result<Vec<(String, Pattern)>, ParseErrorAt> {
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
}
