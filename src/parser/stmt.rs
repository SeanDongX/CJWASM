//! 语句解析：parse_stmt、parse_stmts、parse_stmts_until_case_or_rbrace、assign_target_to_expr、expr_to_assign_target。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::{AssignTarget, BinOp, Expr, Pattern, Stmt, Type, Visibility};
use crate::lexer::Token;

impl Parser {
    /// 将语句列表转换为 Expr：
    /// - 空列表 → Expr::Integer(0) 回退值
    /// - 只有一条 Expr 语句且无前置语句 → 直接使用该表达式
    /// - 最后一条是 Expr 语句 → Block(前置语句, Some(最后的表达式))
    /// - 其他情况 → Block(所有语句, None)，保留 return/let 等
    pub(crate) fn stmts_to_block_expr(stmts: Vec<Stmt>) -> Box<Expr> {
        if stmts.is_empty() {
            return Box::new(Expr::Integer(0));
        }
        let last_is_expr = matches!(stmts.last(), Some(Stmt::Expr(_)));
        if last_is_expr {
            let mut stmts = stmts;
            let last = stmts.pop().unwrap();
            let result = if let Stmt::Expr(e) = last {
                Some(Box::new(e))
            } else {
                unreachable!()
            };
            if stmts.is_empty() {
                result.unwrap()
            } else {
                Box::new(Expr::Block(stmts, result))
            }
        } else {
            Box::new(Expr::Block(stmts, None))
        }
    }

    /// 解析语句列表
    pub(crate) fn parse_stmts(&mut self) -> Result<Vec<Stmt>, ParseErrorAt> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && self.peek().is_some() {
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// 解析语句直到遇到 case 或 }（用于 match 臂 body）
    pub(crate) fn parse_stmts_until_case_or_rbrace(&mut self) -> Result<Vec<Stmt>, ParseErrorAt> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace)
            && !matches!(self.peek(), Some(Token::Case))
            && !self.check(&Token::Comma)
            && self.peek().is_some()
        {
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// 解析语句
    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, ParseErrorAt> {
        match self.peek() {
            Some(Token::Let) => {
                self.advance();
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
                    return Ok(Stmt::Let {
                        pattern: Pattern::Wildcard,
                        ty,
                        value,
                    });
                }
                let pattern = if self.check(&Token::LParen) {
                    self.parse_pattern()?
                } else {
                    let first = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(tok) => {
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "变量名或类型名".to_string(),
                            ))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    if self.check(&Token::LBrace) {
                        self.advance();
                        let mut fields = Vec::new();
                        while !self.check(&Token::RBrace) {
                            let fname = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "字段名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            let binding = if self.check(&Token::Colon) {
                                self.advance();
                                match self.advance() {
                                    Some(Token::Ident(n)) => n,
                                    Some(tok) => {
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "绑定名".to_string(),
                                        ))
                                    }
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
                        Pattern::Struct {
                            name: first,
                            fields,
                        }
                    } else {
                        Pattern::Binding(first)
                    }
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 支持无初始化的 let 声明: let x: Type（延迟赋值，常见于 try 块前）
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    self.parse_expr()?
                } else {
                    if let Some(ref t) = ty {
                        self.default_value_for_type(t)
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "类型注解或初始化值".to_string(),
                        ));
                    }
                };
                Ok(Stmt::Let { pattern, ty, value })
            }
            Some(Token::Var) => {
                self.advance();
                let pattern = if self.check(&Token::LParen) {
                    self.parse_pattern()?
                } else {
                    let name = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "变量名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    Pattern::Binding(name)
                };
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 支持无初始化的变量声明: var x: Int64
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    self.parse_expr()?
                } else {
                    // 无初始化值，使用类型的默认值
                    if let Some(ref t) = ty {
                        self.default_value_for_type(t)
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "类型注解或初始化值".to_string(),
                        ));
                    }
                };
                Ok(Stmt::Var { pattern, ty, value })
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
                let is_paren_let =
                    self.check(&Token::LParen) && matches!(self.peek_next(), Some(Token::Let));
                let is_let = self.check(&Token::Let);
                if is_paren_let {
                    self.advance();
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "`=` 或 `<-`".to_string(),
                        ));
                    }
                    self.advance();
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::WhileLet {
                        pattern,
                        expr,
                        body,
                    })
                } else if is_let {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "`=` 或 `<-`".to_string(),
                        ));
                    }
                    self.advance();
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::WhileLet {
                        pattern,
                        expr,
                        body,
                    })
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
                let has_paren = self.check(&Token::LParen);
                if has_paren {
                    self.advance();
                }
                let (var, tuple_vars) = if self.check(&Token::LParen) {
                    self.advance();
                    let mut names = Vec::new();
                    loop {
                        let n = match self.advance() {
                            Some(Token::Ident(n)) => n,
                            Some(Token::Underscore) => "_".to_string(),
                            Some(tok) => {
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "元组元素名".to_string(),
                                ))
                            }
                            None => return self.bail(ParseError::UnexpectedEof),
                        };
                        names.push(n);
                        if self.check(&Token::RParen) {
                            self.advance();
                            break;
                        }
                        if !self.check(&Token::Comma) {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `)`".to_string(),
                            ));
                        }
                        self.advance();
                    }
                    ("__for_tuple_destr".to_string(), names)
                } else {
                    let v = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(Token::Underscore) => "_".to_string(),
                        // 允许关键字作为循环变量名（如 `loop`）
                        Some(Token::Loop) => "loop".to_string(),
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "循环变量名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    (v, Vec::new())
                };
                self.expect(Token::In)?;
                let iterable = self.parse_for_iterable()?;
                // cjc: for (x in iter where cond) { ... } — 跳过 where 过滤子句
                if self.check(&Token::Where) {
                    self.advance();
                    self.parse_expr()?; // 解析并丢弃 where 条件
                }
                if has_paren {
                    self.expect(Token::RParen)?;
                }
                self.expect(Token::LBrace)?;
                let mut body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                if !tuple_vars.is_empty() {
                    let mut prefix = Vec::new();
                    for (i, name) in tuple_vars.into_iter().enumerate() {
                        if name != "_" {
                            prefix.push(Stmt::Let {
                                pattern: Pattern::Binding(name.clone()),
                                ty: None,
                                value: Expr::TupleIndex {
                                    object: Box::new(Expr::Var(var.clone())),
                                    index: i as u32,
                                },
                            });
                        }
                    }
                    prefix.extend(body);
                    body = prefix;
                }
                Ok(Stmt::For {
                    var,
                    iterable,
                    body,
                })
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
            Some(Token::Unsafe) => {
                self.advance();
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Stmt::UnsafeBlock { body })
            }
            Some(Token::Do) => {
                self.advance();
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.expect(Token::While)?;
                let has_paren = self.check(&Token::LParen);
                if has_paren {
                    self.advance();
                }
                let cond = self.parse_expr()?;
                if has_paren {
                    self.expect(Token::RParen)?;
                }
                Ok(Stmt::DoWhile { body, cond })
            }
            Some(Token::Const) => {
                self.advance();
                if self.check(&Token::Var) || self.check(&Token::Let) {
                    self.advance();
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
            Some(Token::Func) => {
                let f = self.parse_function_with_visibility(Visibility::default())?;
                Ok(Stmt::LocalFunc(f))
            }
            Some(Token::At) => {
                let (at_start, _) = self.at();
                self.advance();
                match self.peek().cloned() {
                    Some(Token::Ident(name)) if name == "Assert" || name == "Expect" => {
                        let is_assert = name == "Assert";
                        self.advance();
                        self.expect(Token::LParen)?;
                        let left = self.parse_expr()?;
                        let right = if self.check(&Token::Comma) {
                            self.advance();
                            self.parse_expr()?
                        } else {
                            Expr::Bool(true)
                        };
                        self.expect(Token::RParen)?;
                        if is_assert {
                            Ok(Stmt::Assert {
                                left,
                                right,
                                line: at_start,
                            })
                        } else {
                            Ok(Stmt::Expect {
                                left,
                                right,
                                line: at_start,
                            })
                        }
                    }
                    _ => self.bail(ParseError::UnexpectedToken(
                        Token::At,
                        "@Assert 或 @Expect".to_string(),
                    )),
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                let (is_assign, bin_op) = match self.peek() {
                    Some(Token::Assign) => (true, None),
                    Some(Token::PlusEq) => (true, Some(BinOp::Add)),
                    Some(Token::MinusEq) => (true, Some(BinOp::Sub)),
                    Some(Token::StarEq) => (true, Some(BinOp::Mul)),
                    Some(Token::SlashEq) => (true, Some(BinOp::Div)),
                    Some(Token::PercentEq) => (true, Some(BinOp::Mod)),
                    Some(Token::ShlEq) => (true, Some(BinOp::Shl)),
                    Some(Token::ShrEq) => (true, Some(BinOp::Shr)),
                    Some(Token::AndEq) => (true, Some(BinOp::BitAnd)),
                    Some(Token::PipeEq) => (true, Some(BinOp::BitOr)),
                    Some(Token::CaretEq) => (true, Some(BinOp::BitXor)),
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

    /// 将赋值目标转回表达式（用于复合赋值的 RHS 展开）
    pub(crate) fn assign_target_to_expr(&self, target: &AssignTarget) -> Expr {
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
            AssignTarget::FieldPath { base, fields } => {
                let mut expr = Expr::Var(base.clone());
                for f in fields {
                    expr = Expr::Field {
                        object: Box::new(expr),
                        field: f.clone(),
                    };
                }
                expr
            }
            AssignTarget::IndexPath {
                base,
                fields,
                index,
            } => {
                let mut expr = Expr::Var(base.clone());
                for f in fields {
                    expr = Expr::Field {
                        object: Box::new(expr),
                        field: f.clone(),
                    };
                }
                Expr::Index {
                    array: Box::new(expr),
                    index: index.clone(),
                }
            }
            AssignTarget::ExprIndex { expr, index } => Expr::Index {
                array: expr.clone(),
                index: index.clone(),
            },
            AssignTarget::Tuple(targets) => {
                let elts = targets
                    .iter()
                    .map(|t| self.assign_target_to_expr(t))
                    .collect();
                Expr::Tuple(elts)
            }
            AssignTarget::SuperField { field } => Expr::SuperFieldAccess {
                field: field.clone(),
            },
        }
    }

    /// 将表达式转换为赋值目标
    pub(crate) fn expr_to_assign_target(&self, expr: Expr) -> Result<AssignTarget, ParseErrorAt> {
        match expr {
            Expr::Var(name) => Ok(AssignTarget::Var(name)),
            Expr::Index { array, index } => {
                let mut path = Vec::new();
                let mut current = *array;
                loop {
                    match current {
                        Expr::Var(name) => {
                            path.reverse();
                            if path.is_empty() {
                                return Ok(AssignTarget::Index { array: name, index });
                            }
                            return Ok(AssignTarget::IndexPath {
                                base: name,
                                fields: path,
                                index,
                            });
                        }
                        Expr::Field {
                            object: o,
                            field: f,
                        } => {
                            path.push(f);
                            current = *o;
                        }
                        _ => {
                            // 复杂表达式（如方法调用）作为数组
                            // 重建表达式路径
                            let mut expr = current;
                            for field in path.iter().rev() {
                                expr = Expr::Field {
                                    object: Box::new(expr),
                                    field: field.clone(),
                                };
                            }
                            return Ok(AssignTarget::ExprIndex {
                                expr: Box::new(expr),
                                index,
                            });
                        }
                    }
                }
            }
            Expr::Field { object, field } => {
                let mut path = vec![field];
                let mut current = *object;
                loop {
                    match current {
                        Expr::Var(name) => {
                            path.reverse();
                            return if path.len() == 1 {
                                Ok(AssignTarget::Field {
                                    object: name,
                                    field: path.into_iter().next().unwrap(),
                                })
                            } else {
                                Ok(AssignTarget::FieldPath {
                                    base: name,
                                    fields: path,
                                })
                            };
                        }
                        Expr::Field {
                            object: o,
                            field: f,
                        } => {
                            path.push(f);
                            current = *o;
                        }
                        _ => {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Assign,
                                "简单字段访问".to_string(),
                            ))
                        }
                    }
                }
            }
            Expr::Tuple(elts) => {
                let mut targets = Vec::new();
                for e in elts {
                    targets.push(self.expr_to_assign_target(e)?);
                }
                Ok(AssignTarget::Tuple(targets))
            }
            Expr::Index { array, index } => {
                // 支持数组索引赋值: arr[i] = value 或 obj.field[i] = value
                match *array {
                    Expr::Var(name) => Ok(AssignTarget::Index { array: name, index }),
                    Expr::Field { object, field } => {
                        // obj.field[i] = value
                        let mut path = vec![field];
                        let mut current = *object;
                        loop {
                            match current {
                                Expr::Var(name) => {
                                    path.reverse();
                                    return Ok(AssignTarget::IndexPath {
                                        base: name,
                                        fields: path,
                                        index,
                                    });
                                }
                                Expr::Field {
                                    object: o,
                                    field: f,
                                } => {
                                    path.push(f);
                                    current = *o;
                                }
                                _ => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        Token::Assign,
                                        "简单字段访问".to_string(),
                                    ))
                                }
                            }
                        }
                    }
                    _ => {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Assign,
                            "简单数组访问".to_string(),
                        ))
                    }
                }
            }
            Expr::SuperFieldAccess { field } => {
                // super.field = value
                Ok(AssignTarget::SuperField { field })
            }
            _ => {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Assign,
                    "可赋值的目标".to_string(),
                ))
            }
        }
    }

    /// 为类型生成默认值表达式
    fn default_value_for_type(&self, ty: &Type) -> Expr {
        match ty {
            Type::Int64 | Type::Int32 | Type::Int16 | Type::Int8 | Type::IntNative => {
                Expr::Integer(0)
            }
            Type::UInt64 | Type::UInt32 | Type::UInt16 | Type::UInt8 | Type::UIntNative => {
                Expr::Integer(0)
            }
            Type::Float64 => Expr::Float(0.0),
            Type::Float32 => Expr::Float32(0.0),
            Type::Bool => Expr::Bool(false),
            Type::String => Expr::String(String::new()),
            Type::Rune => Expr::Rune('\0'),
            _ => Expr::Integer(0), // 其他类型默认为0
        }
    }
}
