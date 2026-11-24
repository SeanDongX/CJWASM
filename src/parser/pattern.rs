//! 模式解析：parse_pattern、parse_or_pattern、parse_primary_pattern、parse_pattern_fields。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::{Expr, Literal, Pattern};
use crate::lexer::{StringOrInterpolated, Token};

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

    /// 解析基础模式（含字面量常量模式后的比较守卫，如 case b'0' <= c && c <= b'9'）
    pub(crate) fn parse_primary_pattern(&mut self) -> Result<Pattern, ParseErrorAt> {
        let pattern = self.parse_primary_pattern_base()?;
        // 无选择器 match 的常量/表达式守卫：字面量开头后接比较/逻辑运算
        if let Pattern::Literal(ref lit) = pattern {
            if matches!(
                self.peek(),
                Some(Token::Lt)
                    | Some(Token::Gt)
                    | Some(Token::LtEq)
                    | Some(Token::GtEq)
                    | Some(Token::Eq)
                    | Some(Token::NotEq)
                    | Some(Token::AndAnd)
                    | Some(Token::OrOr)
            ) {
                let base = match lit {
                    Literal::Integer(n) => Expr::Integer(*n),
                    Literal::Float(f) => Expr::Float(*f),
                    Literal::Bool(b) => Expr::Bool(*b),
                    Literal::String(s) => Expr::String(s.clone()),
                    Literal::Rune(c) => Expr::Rune(*c),
                };
                let full = self.parse_guard_binary_rest(base)?;
                return Ok(Pattern::Guard(Box::new(full)));
            }
        }
        Ok(pattern)
    }

    /// 解析基础模式（不含顶层守卫转换）
    fn parse_primary_pattern_base(&mut self) -> Result<Pattern, ParseErrorAt> {
        // 前置小数点浮点模式 `.8`（词法层拆成 Dot + 数字）
        if self.check(&Token::Dot) {
            if let Some(value) = self.try_leading_dot_float() {
                return Ok(Pattern::Literal(Literal::Float(value)));
            }
        }
        match self.peek() {
            // 浮点常量模式（Float64/Float32）
            Some(Token::Float(_))
            | Some(Token::Float64Suffix(_))
            | Some(Token::Float32(_)) => {
                let value = match self.advance() {
                    Some(Token::Float(f)) | Some(Token::Float64Suffix(f)) => f,
                    Some(Token::Float32(f)) => f as f64,
                    _ => unreachable!(),
                };
                Ok(Pattern::Literal(Literal::Float(value)))
            }
            Some(Token::Underscore) => {
                self.advance();
                // cjc: 支持匿名类型测试模式 case _: Type =>
                if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    Ok(Pattern::TypeTest {
                        binding: "_".to_string(),
                        ty,
                    })
                } else {
                    Ok(Pattern::Wildcard)
                }
            }
            Some(Token::Some) => {
                self.advance();
                let payload = if self.check(&Token::LParen) {
                    self.advance();
                    // 支持嵌套模式：Some((x, y)) 或 Some(x)
                    let pattern = self.parse_pattern()?;
                    self.expect(Token::RParen)?;
                    Some(Box::new(pattern))
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "Some".to_string(),
                    payload,
                })
            }
            Some(Token::None) => {
                self.advance();
                Ok(Pattern::Variant {
                    enum_name: "Option".to_string(),
                    variant_name: "None".to_string(),
                    payload: None,
                })
            }
            Some(Token::Ok) => {
                self.advance();
                let payload = if self.check(&Token::LParen) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.expect(Token::RParen)?;
                    Some(Box::new(pattern))
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Ok".to_string(),
                    payload,
                })
            }
            Some(Token::Err) => {
                self.advance();
                let payload = if self.check(&Token::LParen) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.expect(Token::RParen)?;
                    Some(Box::new(pattern))
                } else {
                    None
                };
                Ok(Pattern::Variant {
                    enum_name: "Result".to_string(),
                    variant_name: "Err".to_string(),
                    payload,
                })
            }
            Some(Token::Integer(n)) | Some(Token::IntegerSuffixed((n, _))) => {
                let n = *n;
                self.advance();
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    if let Some(Token::Integer(end)) | Some(Token::IntegerSuffixed((end, _))) =
                        self.advance()
                    {
                        return Ok(Pattern::Range {
                            start: Literal::Integer(n),
                            end: Literal::Integer(end),
                            inclusive,
                        });
                    }
                }
                Ok(Pattern::Literal(Literal::Integer(n)))
            }
            Some(Token::ByteLit(b)) => {
                let b = *b;
                self.advance();
                Ok(Pattern::Literal(Literal::Integer(b as i64)))
            }
            Some(Token::CharLit(c)) => {
                let c = *c;
                self.advance();
                Ok(Pattern::Literal(Literal::Rune(c)))
            }
            Some(Token::RuneLit(c)) => {
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
            Some(Token::MultiLineStringLit(s)) => {
                let s = match s.clone() {
                    StringOrInterpolated::Plain(t) => t,
                    StringOrInterpolated::Interpolated(parts) => parts
                        .iter()
                        .map(|p| match p {
                            crate::lexer::StringPart::Literal(t) => t.clone(),
                            crate::lexer::StringPart::Interpolation(t) => format!("${{{}}}", t),
                        })
                        .collect::<String>(),
                };
                self.advance();
                Ok(Pattern::Literal(Literal::String(s)))
            }
            Some(Token::RawStringLit(s)) | Some(Token::HashRawStringLit(s)) | Some(Token::SingleQuoteStringLit(s)) => {
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
                            // 支持嵌套模式
                            if self.check(&Token::RParen) {
                                // 无参数变体
                                self.advance();
                                return Ok(Pattern::Variant {
                                    enum_name: name,
                                    variant_name: variant,
                                    payload: None,
                                });
                            }

                            // 解析一个或多个模式（逗号分隔，多个时包成 Tuple）
                            let first = self.parse_pattern()?;
                            let payload = if self.check(&Token::Comma) {
                                let mut patterns = vec![first];
                                while self.check(&Token::Comma) {
                                    self.advance();
                                    patterns.push(self.parse_pattern()?);
                                }
                                Pattern::Tuple(patterns)
                            } else {
                                first
                            };
                            self.expect(Token::RParen)?;
                            return Ok(Pattern::Variant {
                                enum_name: name,
                                variant_name: variant,
                                payload: Some(Box::new(payload)),
                            });
                        } else {
                            return Ok(Pattern::Variant {
                                enum_name: name,
                                variant_name: variant,
                                payload: None,
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
                } else if matches!(
                    self.peek(),
                    Some(Token::Lt)
                        | Some(Token::Gt)
                        | Some(Token::LtEq)
                        | Some(Token::GtEq)
                        | Some(Token::Eq)
                        | Some(Token::NotEq)
                ) {
                    // 无选择器 match 的表达式模式：case score < 60 =>
                    let base = Expr::Var(name.clone());
                    let full_expr = self.parse_guard_binary_rest(base)?;
                    Ok(Pattern::Guard(Box::new(full_expr)))
                } else if self.check(&Token::LParen) {
                    // 构造器/变体模式 Name(...)（仓颉枚举变体名可为小写，如 case add(y, m)）
                    self.advance();
                    let payload = if self.check(&Token::RParen) {
                        None
                    } else {
                        // cjc: 变体可以有多个关联值，如 Short(r, _) 或 Full(s, r, _, _)
                        let first = self.parse_pattern()?;
                        if self.check(&Token::Comma) {
                            let mut patterns = vec![first];
                            while self.check(&Token::Comma) {
                                self.advance();
                                patterns.push(self.parse_pattern()?);
                            }
                            Some(Box::new(Pattern::Tuple(patterns)))
                        } else {
                            Some(Box::new(first))
                        }
                    };
                    self.expect(Token::RParen)?;
                    Ok(Pattern::Variant {
                        enum_name: String::new(),
                        variant_name: name,
                        payload,
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
                    eprintln!(
                        "DEBUG pattern parse_struct_fields: unexpected token {:?}",
                        tok
                    );
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
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

#[cfg(test)]
mod pattern_tests {
    use crate::pipeline::parse_source;

    fn ok(src: &str) {
        if let Err(e) = parse_source(src) {
            panic!("parse failed: {}\nsource:\n{}", e, src);
        }
    }

    #[test]
    fn test_constant_guard_pattern() {
        ok(r#"
func unhex(c: Byte): Byte {
    match {
        case b'0' <= c && c <= b'9' => c - b'0'
        case b'a' <= c && c <= b'f' => c - b'a' + 10
        case _ => 0
    }
}
main() {}
"#);
    }

    #[test]
    fn test_lowercase_variant_pattern() {
        ok(r#"
enum Math {
    | add(Int64, Int64)
    | mul(Int64, Int64)
}
func doMath(p: Math) {
    match (p) {
        case add(y, m) => y + m
        case mul(y, m) => y * m
    }
}
main() {}
"#);
    }
}
