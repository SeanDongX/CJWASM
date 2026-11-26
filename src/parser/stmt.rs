//! 语句解析：parse_stmt、parse_stmts、parse_stmts_until_case_or_rbrace、assign_target_to_expr、expr_to_assign_target。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::{AssignTarget, BinOp, Expr, Pattern, Stmt, Type, Visibility};
use crate::lexer::Token;

impl Parser {
    /// 跳过成对分组 `open ... close`（支持嵌套），若当前 token 不是 `open` 则什么也不做。
    /// 用于宏调用的 `[...]` 输入与 `(...)` 参数：内容可能是任意语法，只做平衡消费。
    fn skip_balanced_group(
        &mut self,
        open: &Token,
        close: &Token,
    ) -> Result<(), ParseErrorAt> {
        if !self.check(open) {
            return Ok(());
        }
        self.advance();
        let mut depth = 1u32;
        while depth > 0 {
            match self.advance() {
                Some(t)
                    if std::mem::discriminant(&t) == std::mem::discriminant(open) =>
                {
                    depth += 1
                }
                Some(t)
                    if std::mem::discriminant(&t) == std::mem::discriminant(close) =>
                {
                    depth -= 1
                }
                Some(_) => {}
                None => return self.bail(ParseError::UnexpectedEof),
            }
        }
        Ok(())
    }

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
        // 规范 4(1)：表达式由操作数+操作符组成；两个相邻表达式/语句之间
        // 必须由 `;` 或换行分隔。同一行内直接相邻的语句非法。
        let mut need_sep = false;
        while !self.check(&Token::RBrace) && self.peek().is_some() {
            if self.check(&Token::Semicolon) {
                self.advance();
                need_sep = false;
                continue;
            }
            // 下一 token 为 `{` 时存在“尾随 Lambda / 块表达式”歧义，不强制分隔符。
            // 无 source（如部分单元测试）时无法判定换行，跳过检查。
            if need_sep
                && !self.source.is_empty()
                && !self.newline_before_current()
                && !self.check(&Token::LBrace)
            {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "语句之间缺少分隔符（`;` 或换行）".to_string(),
                ));
            }
            stmts.push(self.parse_stmt()?);
            need_sep = true;
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

    /// 无类型标注的 let/var 直接绑定 Lambda 时，若 Lambda 参数混用“带标注/省略标注”，
    /// 编译器无法从上下文推断省略参数的类型，需报错（5.5(10)）。
    fn check_unannotated_lambda_binding(
        &mut self,
        ty: &Option<Type>,
        value: &Expr,
        value_start: usize,
    ) -> Result<(), ParseErrorAt> {
        if ty.is_none()
            && matches!(value, Expr::Lambda { .. })
            && self.lambda_tokens_mixed_annotations(value_start)
        {
            return self.bail(ParseError::UnknownType(
                "无法从上下文推断 Lambda 参数类型：参数类型标注必须全部省略或全部标注"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// 扫描 `[value_start, self.pos)` 中的 `{ params => ... }`，判断参数是否混用类型标注。
    ///
    /// 仅做 token 级判定：按顶层逗号切分参数，参数名后出现顶层 `:` 视为带标注；
    /// 泛型 `<>`、括号/方括号内的逗号与冒号不计入顶层。
    fn lambda_tokens_mixed_annotations(&self, value_start: usize) -> bool {
        let end = self.pos.min(self.tokens.len());
        if value_start >= end {
            return false;
        }
        let toks: Vec<&Token> = (value_start..end).map(|i| &self.tokens[i].1).collect();
        if !matches!(toks.first(), Some(Token::LBrace)) {
            return false;
        }
        let mut any_annot = false;
        let mut any_unannot = false;
        let mut has_name = false;
        let mut has_colon = false;
        let mut angle = 0i32;
        let mut paren = 0i32;
        let mut bracket = 0i32;
        for tok in toks.iter().skip(1) {
            let top = angle == 0 && paren == 0 && bracket == 0;
            match tok {
                Token::FatArrow if top => {
                    if has_name {
                        if has_colon {
                            any_annot = true;
                        } else {
                            any_unannot = true;
                        }
                    }
                    break;
                }
                Token::Comma if top => {
                    if has_name {
                        if has_colon {
                            any_annot = true;
                        } else {
                            any_unannot = true;
                        }
                    }
                    has_name = false;
                    has_colon = false;
                }
                Token::Colon if top => has_colon = true,
                Token::Lt => angle += 1,
                Token::Gt => angle = (angle - 1).max(0),
                Token::Shr => angle = (angle - 2).max(0),
                Token::LParen => paren += 1,
                Token::RParen => paren = (paren - 1).max(0),
                Token::LBracket => bracket += 1,
                Token::RBracket => bracket = (bracket - 1).max(0),
                Token::Ident(_) | Token::Underscore
                    if top && !has_name && !has_colon =>
                {
                    has_name = true;
                }
                _ => {}
            }
        }
        any_annot && any_unannot
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
                let pattern = if self.check(&Token::LParen) || self.looks_like_variant_pattern() {
                    self.parse_pattern()?
                } else {
                    let first = match self.advance_binding_ident() {
                        Some(name) => name,
                        None => {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Ident(String::new())),
                                "变量名或类型名".to_string(),
                            ))
                        }
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
                // cjc: let x!: T — `!` 表示确定赋值声明（无初始化式），仅作语法修饰
                if self.check(&Token::Bang) {
                    self.advance();
                }
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 支持无初始化的 let 声明: let x: Type（延迟赋值，常见于 try 块前）
                let mut value_start = self.pos;
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    value_start = self.pos;
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
                self.check_unannotated_lambda_binding(&ty, &value, value_start)?;
                self.reject_radix_int_float_init(ty.as_ref(), value_start, &value)?;
                Ok(Stmt::Let { pattern, ty, value })
            }
            Some(Token::Var) => {
                self.advance();
                // var 声明允许元组解构与（单构造器）枚举构造器模式：var C(c) = C(16)。
                // 多构造器枚举的构造器模式不可反驳性由语义检查（irrefutable 规则）拒绝。
                let pattern = if self.check(&Token::LParen) || self.looks_like_variant_pattern() {
                    self.parse_pattern()?
                } else {
                    // cjc: var _ = expr — 求值右侧后丢弃（丢弃绑定）
                    if self.check(&Token::Underscore) {
                        self.advance();
                        Pattern::Wildcard
                    } else {
                        match self.advance_binding_ident() {
                            Some(name) => Pattern::Binding(name),
                            None => {
                                return self.bail(ParseError::UnexpectedToken(
                                    self.peek().cloned().unwrap_or(Token::Assign),
                                    "变量名".to_string(),
                                ))
                            }
                        }
                    }
                };
                // cjc: var x!: T — `!` 表示确定赋值声明（无初始化式），仅作语法修饰
                if self.check(&Token::Bang) {
                    self.advance();
                }
                let ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 支持无初始化的变量声明: var x: Int64
                let mut value_start = self.pos;
                let value = if self.check(&Token::Assign) {
                    self.advance();
                    value_start = self.pos;
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
                self.check_unannotated_lambda_binding(&ty, &value, value_start)?;
                self.reject_radix_int_float_init(ty.as_ref(), value_start, &value)?;
                Ok(Stmt::Var { pattern, ty, value })
            }
            Some(Token::Return) => {
                self.advance();
                if self.check(&Token::RBrace) || self.check(&Token::Semicolon) {
                    Ok(Stmt::Return(None))
                } else {
                    let expr = self.parse_expr()?;
                    // `return lhs = rhs`：赋值表达式返回 Unit，脱糖为块内赋值
                    let bin_op = match self.peek() {
                        Some(Token::Assign) => Some(None),
                        Some(Token::PlusEq) => Some(Some(BinOp::Add)),
                        Some(Token::MinusEq) => Some(Some(BinOp::Sub)),
                        Some(Token::StarEq) => Some(Some(BinOp::Mul)),
                        Some(Token::SlashEq) => Some(Some(BinOp::Div)),
                        Some(Token::PercentEq) => Some(Some(BinOp::Mod)),
                        _ => None,
                    };
                    if let Some(op) = bin_op {
                        self.advance();
                        let rhs = self.parse_expr()?;
                        let target = self.expr_to_assign_target(expr)?;
                        let value = match op {
                            None => rhs,
                            Some(bin) => Expr::Binary {
                                op: bin,
                                left: Box::new(self.assign_target_to_expr(&target)),
                                right: Box::new(rhs),
                            },
                        };
                        Ok(Stmt::Return(Some(Expr::Block(
                            vec![Stmt::Assign { target, value }],
                            None,
                        ))))
                    } else {
                        Ok(Stmt::Return(Some(expr)))
                    }
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
                    // 括号内是完整表达式上下文，可用 parse_expr 支持二元表达式
                    let expr = Box::new(self.parse_expr()?);
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
                    // 非括号 while-let 主体后紧跟 `{` 是循环体，禁止尾随 `{` 当作 lambda
                    let saved_no_lambda = self.no_trailing_lambda;
                    self.no_trailing_lambda = true;
                    let subject = self.parse_match_subject();
                    self.no_trailing_lambda = saved_no_lambda;
                    let expr = Box::new(subject?);
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Ok(Stmt::WhileLet {
                        pattern,
                        expr,
                        body,
                    })
                } else {
                    // 非括号条件后紧跟 `{` 是循环体，禁止尾随 `{` 当作 lambda
                    let saved_no_lambda = self.no_trailing_lambda;
                    self.no_trailing_lambda = true;
                    let cond = self.parse_expr();
                    self.no_trailing_lambda = saved_no_lambda;
                    let cond = cond?;
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
                let (var, tuple_vars, binding_pattern) = if self.check(&Token::LParen) {
                    self.advance();
                    let mut patterns = Vec::new();
                    loop {
                        // 元组元素可为简单名、_ 或嵌套模式（如 (Foo(x), _)）
                        patterns.push(self.parse_pattern()?);
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
                    let all_simple = patterns
                        .iter()
                        .all(|p| matches!(p, Pattern::Binding(_) | Pattern::Wildcard));
                    if all_simple {
                        let names = patterns
                            .into_iter()
                            .map(|p| match p {
                                Pattern::Binding(n) => n,
                                _ => "_".to_string(),
                            })
                            .collect();
                        ("__for_tuple_destr".to_string(), names, None)
                    } else {
                        (
                            "__for_pat".to_string(),
                            Vec::new(),
                            Some(Pattern::Tuple(patterns)),
                        )
                    }
                } else if matches!(
                    self.peek(),
                    Some(Token::Ident(_))
                        | Some(Token::Some)
                        | Some(Token::None)
                        | Some(Token::Ok)
                        | Some(Token::Err)
                        | Some(Token::Underscore)
                ) && (matches!(self.peek_next(), Some(Token::LParen))
                    || matches!(
                        self.peek(),
                        Some(Token::Some)
                            | Some(Token::None)
                            | Some(Token::Ok)
                            | Some(Token::Err)
                    ))
                {
                    // 解构模式循环变量：for (Foo(i) in ...)
                    let pattern = self.parse_pattern()?;
                    match pattern {
                        Pattern::Binding(n) => (n, Vec::new(), None),
                        other => ("__for_pat".to_string(), Vec::new(), Some(other)),
                    }
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
                    (v, Vec::new(), None)
                };
                self.expect(Token::In)?;
                // 非括号 for 的迭代表达式后 `{` 是循环体，禁止尾随 `{` 当作 lambda
                let saved_no_lambda = self.no_trailing_lambda;
                if !has_paren {
                    self.no_trailing_lambda = true;
                }
                let iterable = self.parse_for_iterable()?;
                // cjc: for (x in iter where cond) { ... } — 跳过 where 过滤子句
                if self.check(&Token::Where) {
                    self.advance();
                    self.parse_expr()?; // 解析并丢弃 where 条件
                }
                self.no_trailing_lambda = saved_no_lambda;
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
                if let Some(pat) = binding_pattern {
                    // 解构模式：插入 let <pattern> = __for_pat
                    body.insert(
                        0,
                        Stmt::Let {
                            pattern: pat,
                            ty: None,
                            value: Expr::Var(var.clone()),
                        },
                    );
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
                let saved_no_lambda = self.no_trailing_lambda;
                if !has_paren {
                    self.no_trailing_lambda = true;
                }
                let cond = self.parse_expr();
                self.no_trailing_lambda = saved_no_lambda;
                let cond = cond?;
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
                // cjc: 局部 const func Name(...) { ... } — const 修饰的局部函数声明
                if self.check(&Token::Func) {
                    let mut f = self.parse_function_with_visibility(Visibility::default())?;
                    f.is_const = true;
                    return Ok(Stmt::LocalFunc(f));
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
                self.advance(); // @
                let name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "宏名称".to_string(),
                        ))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                // 可选的宏属性/类型实参列表：@Assert[validator]、@AssertThrows[Exception]
                if self.check(&Token::LBracket) {
                    self.skip_balanced_mixed()?;
                }
                // 可选的实参列表
                let args = if self.check(&Token::LParen) {
                    self.parse_macro_args()?
                } else {
                    Vec::new()
                };
                match name.as_str() {
                    // @Assert(...) / @Expect(...) 保留结构化语句（已有语义/代码生成）
                    "Assert" => {
                        let mut it = args.into_iter();
                        let left = it.next().unwrap_or(Expr::Bool(true));
                        let right = it.next().unwrap_or(Expr::Bool(true));
                        Ok(Stmt::Assert {
                            left,
                            right,
                            line: at_start,
                        })
                    }
                    "Expect" => {
                        let mut it = args.into_iter();
                        let left = it.next().unwrap_or(Expr::Bool(true));
                        let right = it.next().unwrap_or(Expr::Bool(true));
                        Ok(Stmt::Expect {
                            left,
                            right,
                            line: at_start,
                        })
                    }
                    // 其余 unittest 宏（@PowerAssert/@Fail/@AssertThrows/@FailExpect/@On/@Called 等）
                    // 仅做语法接纳，不展开语义。CHIR 对未知宏降级为 Nop。
                    // 继续解析后缀（如 @On(...).returns(0)、@Called(...).times(2)）。
                    _ => {
                        let mut all_args = args;
                        // 宏的尾随块参数：@EnsurePreparedToMock { => ... }
                        if self.check(&Token::LBrace) && self.brace_starts_lambda() {
                            all_args.push(self.parse_primary()?);
                        }
                        let expr = self.parse_postfix_from_expr(Expr::Macro {
                            name,
                            args: all_args,
                        })?;
                        Ok(Stmt::Expr(expr))
                    }
                }
            }
            // cjc: `_ = expr` 丢弃赋值——求值右侧后丢弃
            Some(Token::Underscore) => {
                self.advance();
                self.expect(Token::Assign)?;
                let value = self.parse_expr()?;
                Ok(Stmt::Expr(value))
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
                    // a **= b → a = a ** b（幂运算，右结合由 parse_power 保证）
                    Some(Token::StarStarEq) => (true, Some(BinOp::Pow)),
                    // a ||= b → a = a || b（短路由 Binary(LogicalOr) 保证）
                    Some(Token::OrOrEq) => (true, Some(BinOp::LogicalOr)),
                    // a &&= b → a = a && b
                    Some(Token::AndAndEq) => (true, Some(BinOp::LogicalAnd)),
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
            AssignTarget::ExprField { object, field } => Expr::Field {
                object: object.clone(),
                field: field.clone(),
            },
            AssignTarget::OptionalField { object, field } => Expr::OptionalChain {
                object: object.clone(),
                field: field.clone(),
            },
            AssignTarget::Slice { array, start, end } => Expr::SliceExpr {
                array: array.clone(),
                start: start.clone(),
                end: end.clone(),
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
                            // 复杂对象（方法调用/索引/可选链等）：expr.field 赋值目标
                            let mut object = current;
                            for f in path[1..].iter().rev() {
                                object = Expr::Field {
                                    object: Box::new(object),
                                    field: f.clone(),
                                };
                            }
                            return Ok(AssignTarget::ExprField {
                                object: Box::new(object),
                                field: path[0].clone(),
                            });
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
                    Expr::Var(name) => Ok(AssignTarget::Index {
                        array: name,
                        index,
                    }),
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
                    other => {
                        // 复杂对象（可选链/方法调用等）索引：expr[i] = value
                        return Ok(AssignTarget::ExprIndex {
                            expr: Box::new(other),
                            index,
                        });
                    }
                }
            }
            Expr::SliceExpr { array, start, end } => Ok(AssignTarget::Slice {
                array,
                start,
                end,
            }),
            Expr::OptionalChain { object, field } => {
                // 可选链赋值 a?.b = v：保留可选链形态，交由语义检查
                // 4.23(15)/4.13.4 判定（目标字段可变且基础为引用类型时才合法）。
                Ok(AssignTarget::OptionalField { object, field })
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
    /// 官方语义：进制前缀整数字面量（0x/0o/0b）不能初始化 Float 类型注解
    ///（十进制整数字面量可经字面量协调转换，a06_47 / 2.1.11 a11_02/03/04 负例）。
    /// AST 不保留进制信息，故在 token 层判定：初始化式首个 token 为
    /// `IntegerRadix` 且整体为裸整数表达式时拒绝。
    fn reject_radix_int_float_init(
        &self,
        ty: Option<&Type>,
        value_start: usize,
        value: &Expr,
    ) -> Result<(), ParseErrorAt> {
        if !matches!(
            ty,
            Some(Type::Float16) | Some(Type::Float32) | Some(Type::Float64)
        ) {
            return Ok(());
        }
        if matches!(self.tokens.get(value_start), Some((_, Token::IntegerRadix(_), _)))
            && matches!(value, Expr::Integer(_))
        {
            let at = (
                self.tokens.get(value_start).map(|t| t.0).unwrap_or(0),
                self.tokens.get(value_start).map(|t| t.2).unwrap_or(0),
            );
            return self.bail_at(
                ParseError::UnknownType(
                    "进制前缀（0x/0o/0b）整数字面量不能初始化 Float 类型注解".to_string(),
                ),
                at,
            );
        }
        Ok(())
    }

    fn default_value_for_type(&self, ty: &Type) -> Expr {
        // 合成的默认值包一层空语句 + 尾表达式 Block：语义透明（codegen/chir 把
        // 带尾值的 Block 编译为其尾值），但让 sema 能区分“无初始化式（合成默认
        // 值）”与“显式初始化为默认值”（如 `let x: Int32 = 0`），避免后者被
        // 误判为延迟赋值声明（is_default_value 只认 Block 包装形态）。
        let raw = match ty {
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
        };
        Expr::Block(Vec::new(), Some(Box::new(raw)))
    }
}

#[cfg(test)]
mod stmt_tests {
    use crate::pipeline::parse_source;

    fn ok(src: &str) {
        if let Err(e) = parse_source(src) {
            panic!("parse failed: {}\nsource:\n{}", e, src);
        }
    }

    #[test]
    fn test_tuple_destructuring_assignment() {
        ok(r#"
class C {
    var x = 0
}
main() {
    var x = 0
    var y = 0
    var b = true
    var c = C()
    (x, c.x) = (3, 4)
    (x, b,) = (5, true,)
    (_, x) = (1, 2)
    (x, (b, y)) = (3, (true, 4))
}
"#);
    }

    #[test]
    fn test_tuple_let_patterns() {
        ok(r#"
enum Color {
    Red(Int64)
}
main() {
    let (x, y) = ("S", 1.0)
    let Red(a) = Red(22)
}
"#);
    }

    #[test]
    fn test_for_nested_variant_pattern() {
        ok(r#"
enum E { Foo(Int64) }
main() {
    var count = 0
    for ((Foo(x), _) in [(Foo(1), 'a')]) {
        count += x
    }
}
"#);
    }

    #[test]
    fn test_return_assignment_expression() {
        ok(r#"
class C {
    static func f() {
        var x = 1
        return x = 2
    }
}
main() {}
"#);
    }

    #[test]
    fn test_while_condition_call_with_empty_block() {
        // `while (cond()) {}` 中的 `{}` 是循环体，不能被当作尾随空闭包
        ok(r#"
class Q {
    func remove(): Option<Int64> { None }
}
main() {
    let q = Q()
    while (q.remove().isNone()) {}
}
"#);
    }

    #[test]
    fn test_newline_tuple_assignment_after_identifier() {
        // 上一行以标识符结尾时，下一行的元组赋值不能被吞成调用参数
        ok(r#"
func add5(x: Int64): Int64 { x + 5 }
main() {
    var x = 0
    var b = true
    var f = add5
    (x, b) = (1, true)
}
"#);
    }

    // ==== w9b: 语句分隔（同/异行）====

    #[test]
    fn test_w9b_adjacent_statements_same_line_rejected() {
        // 规范 4(1)：同一行内两个相邻表达式缺操作符/分隔符非法
        assert!(parse_source("main() { 1 2 }").is_err());
        assert!(parse_source("main() {\n    let a = 1\n    let b = 2 a b\n}").is_err());
        assert!(parse_source("func f1(): Int64 { 0 }\nfunc f2(): Int64 { 1 }\nmain() { f1() f2() }").is_err());
    }

    #[test]
    fn test_w9b_statements_separated_ok() {
        // 换行分隔合法
        ok("main() {\n    let a = 1\n    let b = 2\n    let c = a + b\n}");
        // 分号分隔合法
        ok("main() { let a = 1; let b = 2; let c = a + b }");
    }
}
