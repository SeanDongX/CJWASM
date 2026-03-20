//! 表达式解析：parse_expr、parse_primary、二元/一元/后缀、lambda、match、for 等。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::*;
use crate::lexer::{StringOrInterpolated, StringPart, Token};

impl Parser {
    /// 解析表达式（顶层为管道，再空值合并）
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ParseErrorAt> {
        self.parse_pipeline()
    }

    /// 解析管道 (|>): left |> right 表示 right(left)
    pub(crate) fn parse_pipeline(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_null_coalesce()?;
        while matches!(self.peek(), Some(Token::Pipeline)) {
            self.advance();
            let right = self.parse_null_coalesce()?;
            left = Expr::Binary {
                op: BinOp::Pipeline,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析空值合并 (??)
    pub(crate) fn parse_null_coalesce(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_logical_or(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_logical_and(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_comparison(&mut self) -> Result<Expr, ParseErrorAt> {
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
            // 泛型实参由 parse_primary/parse_postfix 的 parse_opt_type_args 系列逻辑处理；
            // 比较表达式层面的 `<` 必须按关系运算符解析，避免把 `Int8(1) < Int8(1)` 误判为泛型。
            Some(Token::Lt) => Some(BinOp::Lt),
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
    pub(crate) fn parse_bitwise_or(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_bitwise_xor(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_bitwise_and(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_shift(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_additive(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_multiplicative(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_power(&mut self) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_unary(&mut self) -> Result<Expr, ParseErrorAt> {
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
        // 前缀自增/自减
        if matches!(self.peek(), Some(Token::Incr)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::PrefixIncr(Box::new(expr)));
        }
        if matches!(self.peek(), Some(Token::Decr)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::PrefixDecr(Box::new(expr)));
        }
        self.parse_postfix()
    }

    /// 解析后缀表达式 (数组访问, 字段访问, 方法调用)
    pub(crate) fn parse_postfix(&mut self) -> Result<Expr, ParseErrorAt> {
        let expr = self.parse_primary()?;
        self.parse_postfix_from_expr(expr)
    }

    /// 从已解析的 base 表达式继续解析后缀操作 (.method()、[idx] 等)
    pub(crate) fn parse_postfix_from_expr(&mut self, mut expr: Expr) -> Result<Expr, ParseErrorAt> {
        loop {
            match self.peek() {
                Some(Token::LBracket) => {
                    self.advance();
                    // 支持 arr[..end] 语法（从开头到 end）
                    if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                        self.advance(); // consume .. or ..=
                                        // 支持 arr[..] 语法（整个数组）
                        let end = if self.check(&Token::RBracket) {
                            Expr::Integer(i64::MAX)
                        } else {
                            self.parse_expr()?
                        };
                        self.expect(Token::RBracket)?;
                        expr = Expr::SliceExpr {
                            array: Box::new(expr),
                            start: Box::new(Expr::Integer(0)),
                            end: Box::new(end),
                        };
                    } else {
                        let first = self.parse_expr()?;
                        if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                            self.advance(); // consume .. or ..=
                                            // 支持 arr[start..] 语法（从 start 到结尾）
                            let end = if self.check(&Token::RBracket) {
                                // arr[start..] - 到数组末尾
                                Expr::Integer(i64::MAX) // 使用最大值表示到末尾
                            } else {
                                self.parse_expr()?
                            };
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start: Box::new(first),
                                end: Box::new(end),
                            };
                        } else {
                            self.expect(Token::RBracket)?;
                            expr = Expr::Index {
                                array: Box::new(expr),
                                index: Box::new(first),
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
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                        }
                    };
                    if self.check(&Token::LParen) {
                        // obj?.method(args) — 可选链方法调用（简化为 MethodCall）
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: field,
                            args,
                            named_args,
                            type_args: None,
                        };
                    } else {
                        expr = Expr::OptionalChain {
                            object: Box::new(expr),
                            field,
                        };
                    }
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
                        Some(Token::BacktickStringLit(StringOrInterpolated::Plain(name))) => name,
                        Some(Token::None) => "None".to_string(),
                        Some(Token::Some) => "Some".to_string(),
                        Some(Token::Ok) => "Ok".to_string(),
                        Some(Token::Err) => "Err".to_string(),
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "字段名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };

                    // 检查是否是方法调用（跨行的 ( 不视为方法调用，避免 Int8.Max\n(a,b) 误解析）
                    if self.check(&Token::LParen) && !self.newline_before_current() {
                        // 方法调用 obj.method(args)
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: name,
                            args,
                            named_args,
                            type_args: None,
                        };
                    } else if self.check(&Token::Lt) && self.looks_like_generic_method_call() {
                        // 泛型方法调用 obj.method<T>(args)
                        // 只有当 < 后面看起来像类型参数时才解析
                        self.advance();
                        let mut types = vec![self.parse_type()?];
                        while self.check(&Token::Comma) {
                            self.advance();
                            types.push(self.parse_type()?);
                        }
                        self.expect(Token::Gt)?;

                        // 泛型参数后面必须跟 (
                        if self.check(&Token::LParen) {
                            self.advance();
                            let (args, named_args) = self.parse_args()?;
                            self.expect(Token::RParen)?;
                            expr = Expr::MethodCall {
                                object: Box::new(expr),
                                method: name,
                                args,
                                named_args,
                                type_args: Some(types),
                            };
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Semicolon),
                                "( after generic type arguments".to_string(),
                            ));
                        }
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
                Some(Token::Incr) => {
                    self.advance();
                    expr = Expr::PostfixIncr(Box::new(expr));
                }
                Some(Token::Decr) => {
                    self.advance();
                    expr = Expr::PostfixDecr(Box::new(expr));
                }
                Some(Token::Question) => {
                    // ? 运算符：expr? 提前返回 Err/None
                    self.advance();
                    expr = Expr::Try(Box::new(expr));
                }
                // P6: Trailing closure — f(args) { params => body } 或 expr.method { params => body }
                // 支持 Call、MethodCall、Field（如 .any { x => ... }），以及 Var（如 map { x => f(x) }）
                // 也支持 ConstructorCall（如 Array(n) { i => ... }）
                Some(Token::LBrace)
                    if matches!(
                        &expr,
                        Expr::Call { .. }
                            | Expr::MethodCall { .. }
                            | Expr::Field { .. }
                            | Expr::Var(_)
                            | Expr::ConstructorCall { .. }
                    ) =>
                {
                    // Peek ahead to check if this looks like a lambda: { ident => ... } or { => ... }
                    let looks_like_lambda = matches!(self.peek_next(), Some(Token::FatArrow))
                        || (self.peek_next_ident_like()
                            && matches!(
                                self.peek_at(2),
                                Some(Token::FatArrow) | Some(Token::Colon) | Some(Token::Comma)
                            ));
                    if looks_like_lambda {
                        // Consume { and parse the lambda body
                        self.advance(); // consume {
                        let closure = if self.check(&Token::FatArrow) {
                            // { => body } — 无参 lambda；body 可为语句块
                            self.advance();
                            let body = self.parse_lambda_body()?;
                            Expr::Lambda {
                                params: vec![],
                                return_type: None,
                                body: Box::new(body),
                            }
                        } else if let Some(Token::Comma) = self.peek_next() {
                            // { a, b, c => body } — 多参无类型 lambda（trailing closure）
                            let mut params = Vec::new();
                            loop {
                                let name = if self.check(&Token::Underscore) {
                                    self.advance();
                                    "_".to_string()
                                } else {
                                    match self.advance_ident() {
                                        Some(n) => n,
                                        None => {
                                            let tok = self.advance().unwrap_or(Token::Semicolon);
                                            return self.bail(ParseError::UnexpectedToken(
                                                tok,
                                                "参数名".to_string(),
                                            ));
                                        }
                                    }
                                };
                                params.push((name, Type::Int64)); // 默认使用 Int64，避免 TypeParam
                                if self.check(&Token::FatArrow) {
                                    self.advance();
                                    break;
                                }
                                if !self.check(&Token::Comma) {
                                    return self.bail(ParseError::UnexpectedToken(
                                        self.peek().cloned().unwrap_or(Token::Comma),
                                        "`,` 或 `=>`".to_string(),
                                    ));
                                }
                                self.advance();
                            }
                            let body = self.parse_lambda_body()?;
                            Expr::Lambda {
                                params,
                                return_type: None,
                                body: Box::new(body),
                            }
                        } else {
                            // { x: T, y: T => body } — 有参 lambda；body 可为语句块（let/var/for/return 等）
                            let params = self.parse_lambda_params()?;
                            self.expect(Token::FatArrow)?;
                            let body = self.parse_lambda_body()?;
                            Expr::Lambda {
                                params,
                                return_type: None,
                                body: Box::new(body),
                            }
                        };
                        expr = match expr {
                            Expr::Call {
                                name,
                                type_args,
                                args,
                                named_args: _,
                            } => {
                                let mut all_args = args;
                                all_args.push(closure);
                                Expr::Call {
                                    name,
                                    type_args,
                                    args: all_args,
                                    named_args: vec![],
                                }
                            }
                            Expr::MethodCall {
                                object,
                                method,
                                args,
                                named_args: _,
                                type_args,
                            } => {
                                let mut all_args = args;
                                all_args.push(closure);
                                Expr::MethodCall {
                                    object,
                                    method,
                                    args: all_args,
                                    named_args: vec![],
                                    type_args,
                                }
                            }
                            Expr::Field {
                                object: obj,
                                field: method_name,
                            } => Expr::MethodCall {
                                object: obj,
                                method: method_name,
                                args: vec![closure],
                                named_args: vec![],
                                type_args: None,
                            },
                            Expr::Var(func_name) => Expr::Call {
                                name: func_name,
                                type_args: None,
                                args: vec![closure],
                                named_args: vec![],
                            },
                            Expr::ConstructorCall {
                                name,
                                type_args,
                                args,
                                named_args: _,
                            } => {
                                let mut all_args = args;
                                all_args.push(closure);
                                Expr::ConstructorCall {
                                    name,
                                    type_args,
                                    args: all_args,
                                    named_args: vec![],
                                }
                            }
                            _ => unreachable!(),
                        };
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 从已有的 postfix 表达式继续解析二元操作符（用于模式中的守卫表达式）
    /// 处理 &&、||、??、|> 等，优先级与正常表达式一致
    pub(crate) fn parse_guard_binary_rest(&mut self, mut left: Expr) -> Result<Expr, ParseErrorAt> {
        // && (比 || 优先级高)
        while self.check(&Token::AndAnd) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op: BinOp::LogicalAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        // ||
        while self.check(&Token::OrOr) {
            self.advance();
            let right = self.parse_logical_and()?;
            left = Expr::Binary {
                op: BinOp::LogicalOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        // ??
        if self.check(&Token::QuestionQuestion) {
            self.advance();
            let right = self.parse_logical_or()?;
            left = Expr::NullCoalesce {
                option: Box::new(left),
                default: Box::new(right),
            };
        }
        // |>
        while self.check(&Token::Pipeline) {
            self.advance();
            let right = self.parse_null_coalesce()?;
            left = Expr::Binary {
                op: BinOp::Pipeline,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// 解析基础表达式
    pub(crate) fn parse_primary(&mut self) -> Result<Expr, ParseErrorAt> {
        match self.advance() {
            // 宏调用 @MacroName 或 @MacroName(args)
            Some(Token::At) => {
                self.pos -= 1; // 回退，让 parse_macro_call 重新消费 @
                self.parse_macro_call()
            }
            Some(Token::Integer(n)) => {
                // 可选整数字面量后缀（如 0x0Au8 / 1i32），消费掉避免后缀被当作独立标识符。
                if matches!(
                    self.peek(),
                    Some(Token::Ident(ref s))
                        if s == "u8"
                            || s == "u16"
                            || s == "u32"
                            || s == "u64"
                            || s == "i8"
                            || s == "i16"
                            || s == "i32"
                            || s == "i64"
                )
                    || matches!(
                        self.peek(),
                        Some(Token::TypeUInt8)
                            | Some(Token::TypeUInt16)
                            | Some(Token::TypeUInt32)
                            | Some(Token::TypeUInt64)
                            | Some(Token::TypeInt8)
                            | Some(Token::TypeInt16)
                            | Some(Token::TypeInt32)
                            | Some(Token::TypeInt64)
                    )
                {
                    self.advance();
                }
                // 检查是否是范围表达式
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    // 支持开放式范围 2.. (没有结束值)
                    let end = if self.check(&Token::RBracket)
                        || self.check(&Token::RParen)
                        || self.check(&Token::Comma)
                    {
                        // 2.. 后面是 ] 或 ) 或 , 说明是开放式范围
                        Box::new(Expr::Integer(i64::MAX))
                    } else {
                        // 允许负数结尾，如 1..-2 或 1..=-3:4
                        Box::new(self.parse_unary()?)
                    };
                    // P2.6: 可选步长 `: step`（支持负步长 -1）
                    let step = if self.check(&Token::Colon) {
                        self.advance();
                        Some(Box::new(self.parse_unary()?))
                    } else {
                        None
                    };
                    return Ok(Expr::Range {
                        start: Box::new(Expr::Integer(n)),
                        end,
                        inclusive,
                        step,
                    });
                }
                Ok(Expr::Integer(n))
            }
            Some(Token::Float(f)) | Some(Token::Float64Suffix(f)) => Ok(Expr::Float(f)),
            Some(Token::Float16(f)) => Ok(Expr::Float32(f)),
            Some(Token::Float32(f)) => Ok(Expr::Float32(f)),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Ok(Expr::Var(n)),
                None => self.bail_at(
                    ParseError::UnexpectedToken(Token::This, "this 仅可在方法体内使用".to_string()),
                    self.at_prev(),
                ),
            },
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
                    let field_or_method = match self.advance_ident() {
                        Some(m) => m,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "字段名或方法名".to_string(),
                            ));
                        }
                    };
                    // 检查是否为方法调用 super.method(...) 或字段访问 super.field
                    if self.check(&Token::LParen) {
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        Ok(Expr::SuperCall {
                            method: field_or_method,
                            args,
                            named_args,
                        })
                    } else {
                        // super.field 字段访问 - 转换为 Expr::SuperFieldAccess
                        Ok(Expr::SuperFieldAccess {
                            field: field_or_method,
                        })
                    }
                } else {
                    self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Dot),
                        "super. 或 super(".to_string(),
                    ))
                }
            }
            Some(Token::ByteLit(b)) => Ok(Expr::Integer(b as i64)),
            Some(Token::CharLit(c)) => Ok(Expr::Rune(c)),
            Some(Token::RuneLit(c)) => Ok(Expr::Rune(c)),
            Some(Token::True) => Ok(Expr::Bool(true)),
            Some(Token::False) => Ok(Expr::Bool(false)),
            Some(Token::StringLit(s)) => self.parse_string_or_interpolated(s),
            Some(Token::BacktickStringLit(StringOrInterpolated::Interpolated(parts))) => {
                self.parse_string_or_interpolated(StringOrInterpolated::Interpolated(parts))
            }
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(name))) => {
                self.pushback = Some(Token::Ident(name));
                self.parse_primary()
            }
            Some(Token::RawStringLit(s))
            | Some(Token::MultiLineStringLit(s))
            | Some(Token::HashRawStringLit(s))
            | Some(Token::SingleQuoteStringLit(s)) => Ok(Expr::String(s)),
            // Option/Result 构造器（允许 Some(e1,e2,...) 解析为 Some(Tuple(...))）
            Some(Token::Some) => {
                // 支持 Some<T>(value) 泛型语法
                if self.check(&Token::Lt) {
                    let _type_args = self.parse_opt_type_args()?;
                }
                self.expect(Token::LParen)?;
                let first = self.parse_expr()?;
                let value = if self.check(&Token::Comma) {
                    let mut elts = vec![first];
                    while self.check(&Token::Comma) {
                        self.advance();
                        elts.push(self.parse_expr()?);
                    }
                    Expr::Tuple(elts)
                } else {
                    first
                };
                self.expect(Token::RParen)?;
                Ok(Expr::Some(Box::new(value)))
            }
            Some(Token::None) => {
                // 支持 None<T> 泛型语法
                if self.check(&Token::Lt) {
                    let _type_args = self.parse_opt_type_args()?;
                    // None<T> 仍然解析为 Expr::None，类型参数在类型检查时使用
                }
                Ok(Expr::None)
            }
            Some(Token::Ok) => {
                self.expect(Token::LParen)?;
                let first = self.parse_expr()?;
                let value = if self.check(&Token::Comma) {
                    let mut elts = vec![first];
                    while self.check(&Token::Comma) {
                        self.advance();
                        elts.push(self.parse_expr()?);
                    }
                    Expr::Tuple(elts)
                } else {
                    first
                };
                self.expect(Token::RParen)?;
                Ok(Expr::Ok(Box::new(value)))
            }
            Some(Token::Err) => {
                self.expect(Token::LParen)?;
                let first = self.parse_expr()?;
                let value = if self.check(&Token::Comma) {
                    let mut elts = vec![first];
                    while self.check(&Token::Comma) {
                        self.advance();
                        elts.push(self.parse_expr()?);
                    }
                    Expr::Tuple(elts)
                } else {
                    first
                };
                self.expect(Token::RParen)?;
                Ok(Expr::Err(Box::new(value)))
            }
            // 类型转换/构造函数 T(e) 或 T(e1, e2, ...) - cjc 兼容；无 ( 时视为类型名引用 Var("Rune") 等
            Some(tok)
                if matches!(
                    tok,
                    Token::TypeInt64
                        | Token::TypeInt32
                        | Token::TypeInt16
                        | Token::TypeInt8
                        | Token::TypeIntNative
                        | Token::TypeUIntNative
                        | Token::TypeUInt64
                        | Token::TypeUInt32
                        | Token::TypeUInt16
                        | Token::TypeUInt8
                        | Token::TypeFloat64
                        | Token::TypeFloat32
                        | Token::TypeFloat16
                        | Token::TypeBool
                        | Token::TypeRune
                        | Token::TypeString
                ) =>
            {
                let name = match tok {
                    Token::TypeInt64 => "Int64",
                    Token::TypeInt32 => "Int32",
                    Token::TypeInt16 => "Int16",
                    Token::TypeInt8 => "Int8",
                    Token::TypeIntNative => "IntNative",
                    Token::TypeUIntNative => "UIntNative",
                    Token::TypeUInt64 => "UInt64",
                    Token::TypeUInt32 => "UInt32",
                    Token::TypeUInt16 => "UInt16",
                    Token::TypeUInt8 => "UInt8",
                    Token::TypeFloat64 => "Float64",
                    Token::TypeFloat32 => "Float32",
                    Token::TypeFloat16 => "Float16",
                    Token::TypeBool => "Bool",
                    Token::TypeRune => "Rune",
                    Token::TypeString => "String",
                    _ => unreachable!(),
                };
                if self.check(&Token::LParen) {
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call {
                        name: name.to_string(),
                        type_args: None,
                        args,
                        named_args,
                    })
                } else if self.check(&Token::Lt) {
                    let type_args = self.parse_opt_type_args()?.unwrap_or_default();
                    Ok(Expr::Call {
                        name: name.to_string(),
                        type_args: Some(type_args),
                        args: vec![],
                        named_args: vec![],
                    })
                } else {
                    // 不 advance：类型名已由 match 消费，留 '.' 给后续 postfix 解析为 UInt64.Max
                    Ok(Expr::Var(name.to_string()))
                }
            }
            // Option<T> / Result<T,E> 作为表达式（Option<String>.None、Result<Int64,String>.Ok(x) 等）
            Some(Token::TypeOption) => {
                let name = "Option".to_string();
                if self.check(&Token::LParen) {
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call {
                        name,
                        type_args: None,
                        args,
                        named_args,
                    })
                } else if self.check(&Token::Lt) {
                    let type_args = self.parse_opt_type_args()?.unwrap_or_default();
                    Ok(Expr::Call {
                        name,
                        type_args: Some(type_args),
                        args: vec![],
                        named_args: vec![],
                    })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(Token::TypeResult) => {
                let name = "Result".to_string();
                if self.check(&Token::LParen) {
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call {
                        name,
                        type_args: None,
                        args,
                        named_args,
                    })
                } else if self.check(&Token::Lt) {
                    let type_args = self.parse_opt_type_args()?.unwrap_or_default();
                    Ok(Expr::Call {
                        name,
                        type_args: Some(type_args),
                        args: vec![],
                        named_args: vec![],
                    })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            // throw 表达式
            Some(Token::Throw) => {
                let value = self.parse_expr()?;
                Ok(Expr::Throw(Box::new(value)))
            }
            // return 在表达式上下文（如 match arm body）解析为 Expr::Return
            Some(Token::Return) => {
                let value = if self.check(&Token::RBrace)
                    || self.check(&Token::Semicolon)
                    || self.check(&Token::Comma)
                {
                    None
                } else {
                    Some(Box::new(self.parse_expr()?))
                };
                Ok(Expr::Return(value))
            }
            Some(Token::Break) => Ok(Expr::Break),
            Some(Token::Continue) => Ok(Expr::Continue),
            // try 块（支持 try-catch-finally 和 try-with-resources）
            Some(Token::Try) => {
                // P6: try-with-resources: try (resource = expr) { ... }
                let resources = if self.check(&Token::LParen) {
                    let saved_pos = self.pos;
                    self.advance();
                    // 检查是否是 try (let/var name = expr) 或 try (name = expr) 形式
                    let has_let_var = matches!(self.peek(), Some(Token::Let) | Some(Token::Var));
                    // 或者 name = expr（不带 let/var 的资源绑定）
                    let has_bare_assign =
                        self.peek_ident_like() && matches!(self.peek_at(1), Some(Token::Assign));
                    if has_let_var || has_bare_assign {
                        let mut res = Vec::new();
                        loop {
                            // 跳过可选的 let/var
                            if matches!(self.peek(), Some(Token::Let) | Some(Token::Var)) {
                                self.advance();
                            }
                            let name = self.advance_ident().ok_or_else(|| ParseErrorAt {
                                error: ParseError::UnexpectedEof,
                                byte_start: self.at().0,
                                byte_end: self.at().1,
                            })?;
                            self.expect(Token::Assign)?;
                            let expr = self.parse_expr()?;
                            res.push((name, expr));
                            if !self.check(&Token::Comma) {
                                break;
                            }
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
                    // try-with-resources without catch/finally: auto-generate empty finally
                    return Ok(Expr::TryBlock {
                        resources,
                        body,
                        catch_var: None,
                        catch_type: None,
                        catch_body: vec![],
                        finally_body: Some(vec![]),
                    });
                }
                if self.check(&Token::Finally) {
                    // try-finally without catch
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let finally_stmts = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    return Ok(Expr::TryBlock {
                        resources,
                        body,
                        catch_var: None,
                        catch_type: None,
                        catch_body: vec![],
                        finally_body: Some(finally_stmts),
                    });
                }
                self.expect(Token::Catch)?;
                let (catch_var, catch_type) = if self.check(&Token::LParen) {
                    self.advance();
                    let var = if self.check(&Token::Underscore) {
                        self.advance();
                        None // catch (_: Type)
                    } else {
                        Some(self.advance_ident().ok_or_else(|| ParseErrorAt {
                            error: ParseError::UnexpectedEof,
                            byte_start: self.at().0,
                            byte_end: self.at().1,
                        })?)
                    };
                    // P2: catch (e: Exception) 或 catch (e: E1 | E2) 异常类型模式
                    let catch_type = if self.check(&Token::Colon) {
                        self.advance();
                        let ty = self.parse_type()?;
                        // 跳过额外的 | Type（多异常类型 catch，取第一个类型）
                        while self.check(&Token::Pipe) {
                            self.advance();
                            self.parse_type()?;
                        }
                        Some(ty)
                    } else {
                        None
                    };
                    self.expect(Token::RParen)?;
                    (var, catch_type)
                } else {
                    (None, None)
                };
                self.expect(Token::LBrace)?;
                let catch_body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                // 解析可选的 finally 块
                let finally_body = if self.check(&Token::Finally) {
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let stmts = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    Some(stmts)
                } else {
                    None
                };
                Ok(Expr::TryBlock {
                    resources,
                    body,
                    catch_var,
                    catch_type,
                    catch_body,
                    finally_body,
                })
            }
            // 关键字 quote 在表达式上下文中当作标识符（如 quote(...) 宏调用）
            // quote(...) 内容是原始 token 序列，不能用表达式解析器处理（内部可能含关键字如 extend）
            Some(Token::Quote) => {
                if self.check(&Token::LParen) {
                    // 跳过 quote(...) 的完整内容，平衡括号计数
                    self.advance(); // 消费 '('
                    let mut depth = 1usize;
                    while depth > 0 {
                        match self.advance() {
                            Some(Token::LParen) => depth += 1,
                            Some(Token::RParen) => depth -= 1,
                            None => return self.bail(ParseError::UnexpectedEof),
                            _ => {}
                        }
                    }
                    // 返回占位符表达式
                    return Ok(Expr::Integer(0));
                }
                // 不跟 ( 时，当作普通标识符
                self.pushback = Some(Token::Ident("quote".to_string()));
                return self.parse_primary();
            }
            Some(Token::Ident(name)) => {
                // 仅当首字母大写时解析为枚举变体 (Color.Red)，否则 . 后续为字段/方法
                // 变体名也必须首字母大写，避免将静态方法 Point.origin() 误解析
                let looks_like_type = name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);
                if looks_like_type && self.check(&Token::Dot) {
                    if let Some(v) = match self.peek_next() {
                        Some(Token::Ident(v)) => Some(v),
                        Some(Token::BacktickStringLit(StringOrInterpolated::Plain(v))) => Some(v),
                        _ => None,
                    } {
                        let variant_looks_like_type =
                            v.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                        if variant_looks_like_type {
                            self.advance();
                            let variant = match self.advance_ident() {
                                Some(v) => v,
                                None => {
                                    let tok = self.advance().unwrap_or(Token::Semicolon);
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "变体名".to_string(),
                                    ));
                                }
                            };
                            let arg = if self.check(&Token::LParen) {
                                self.advance();
                                if self.check(&Token::RParen) {
                                    self.advance();
                                    None
                                } else {
                                    // 允许 Variant(e1) 或 Variant(e1, e2, ...) 解析为单 expr 或 Tuple
                                    let first = self.parse_expr()?;
                                    let e = if self.check(&Token::Comma) {
                                        let mut elements = vec![first];
                                        while self.check(&Token::Comma) {
                                            self.advance();
                                            elements.push(self.parse_expr()?);
                                        }
                                        Expr::Tuple(elements)
                                    } else {
                                        first
                                    };
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
                            Ok(Expr::ConstructorCall {
                                name,
                                type_args,
                                args,
                                named_args,
                            })
                        } else {
                            Ok(Expr::Call {
                                name,
                                type_args,
                                args,
                                named_args,
                            })
                        }
                    }
                    Some(Token::LBrace) if looks_like_type => {
                        // 仅对类型名（首字母大写）解析为结构体初始化
                        self.advance();
                        let fields = self.parse_struct_fields()?;
                        self.expect(Token::RBrace)?;
                        Ok(Expr::StructInit {
                            name,
                            type_args,
                            fields,
                        })
                    }
                    _ => Ok(Expr::Var(name)),
                }
            }
            Some(Token::LParen) => {
                // 检查是否是 Lambda: (x: T, ...): R { body } 或 (): R { body }
                // 当前 token 是 (，已被 advance() 消费
                if self.peek() == Some(&Token::RParen) {
                    // (): R { body } 或空元组
                    self.advance(); // consume )
                    if self.check(&Token::Colon) {
                        return self.parse_lambda_rest(vec![]);
                    }
                    // () 空元组
                    return Ok(Expr::Tuple(vec![]));
                }
                // 检查是否是 (ident : 开头的 Lambda；当前已消费 (，用 peek/peek_at(1) 看括号内
                // 支持两种形式: ( params ): R { body } 与 ( params ) => body（params 后直接 => 无 )）
                if (self.peek_ident_like() || matches!(self.peek(), Some(Token::Underscore)))
                    && matches!(self.peek_at(1), Some(Token::Colon))
                {
                    let params = self.parse_lambda_params()?;
                    if self.check(&Token::FatArrow) {
                        // ( params ) => body — params 后直接 =>，无 )
                        self.advance();
                        self.expect(Token::LBrace)?;
                        let body = self.parse_lambda_body()?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None,
                            body: Box::new(body),
                        });
                    }
                    self.expect(Token::RParen)?;
                    return self.parse_lambda_rest(params);
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
                // { => body } — 无参 lambda 简写
                if self.check(&Token::FatArrow) {
                    self.advance(); // consume =>
                    let body = self.parse_lambda_body()?;
                    return Ok(Expr::Lambda {
                        params: vec![],
                        return_type: None,
                        body: Box::new(body),
                    });
                }
                // 检查是否是 Lambda: { x: T => body } 或 { x => body }（单参无类型）或 { a, b => body }（多参无类型）
                // 也支持 { _: T, x: T => body }（第一个参数为通配符）
                // 注意：当前 { 已被 advance() 消费，用 peek() 看第一个 token，peek_at(1) 看第二个；仅当第二个 token 为 : , 或 => 时才按 lambda 解析，否则按块解析
                let second = self.peek_at(1);
                let is_lambda = (self.peek_ident_like()
                    || matches!(self.peek(), Some(Token::Underscore)))
                    && matches!(
                        second,
                        Some(Token::Colon) | Some(Token::Comma) | Some(Token::FatArrow)
                    );
                if is_lambda {
                    if matches!(second, Some(Token::Colon)) {
                        // Lambda { x: T, y: T => body }；{ 已消费，解析参数
                        let params = self.parse_lambda_params()?;
                        self.expect(Token::FatArrow)?;
                        let body = self.parse_lambda_body()?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None,
                            body: Box::new(body),
                        });
                    } else if matches!(second, Some(Token::Comma)) {
                        // 多参无类型 Lambda { a, b, c => body }
                        let mut params = Vec::new();
                        loop {
                            let name = if self.check(&Token::Underscore) {
                                self.advance();
                                "_".to_string()
                            } else {
                                match self.advance_ident() {
                                    Some(n) => n,
                                    None => {
                                        let tok = self.advance().unwrap_or(Token::Semicolon);
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "参数名".to_string(),
                                        ));
                                    }
                                }
                            };
                            params.push((name, Type::Int64)); // 默认使用 Int64
                            if self.check(&Token::FatArrow) {
                                self.advance();
                                break;
                            }
                            if !self.check(&Token::Comma) {
                                return self.bail(ParseError::UnexpectedToken(
                                    self.peek().cloned().unwrap_or(Token::Comma),
                                    "`,` 或 `=>`".to_string(),
                                ));
                            }
                            self.advance();
                        }
                        let body = self.parse_lambda_body()?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None,
                            body: Box::new(body),
                        });
                    } else if matches!(second, Some(Token::FatArrow)) {
                        // 单参无类型 Lambda { name => body }
                        let name = self.advance_ident().expect("Ident");
                        self.advance(); // consume =>
                        let body = self.parse_lambda_body()?;
                        return Ok(Expr::Lambda {
                            params: vec![(name, Type::Int64)], // 默认使用 Int64
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
            // P2.7: Array<T>(size, init) 或 Array<T>(size, repeat: value) 动态数组构造
            // 也支持 Array(size, repeat: value) 类型推断形式
            Some(Token::TypeArray) => {
                let elem_type = if self.check(&Token::Lt) {
                    self.advance();
                    let ty = self.parse_type()?;
                    self.expect(Token::Gt)?;
                    Some(ty)
                } else {
                    None
                };
                self.expect(Token::LParen)?;
                let (args, named_args) = self.parse_args()?;
                self.expect(Token::RParen)?;
                Ok(Expr::ConstructorCall {
                    name: "Array".to_string(),
                    type_args: elem_type.map(|t| vec![t]),
                    args,
                    named_args,
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
                let is_paren_let =
                    self.check(&Token::LParen) && matches!(self.peek_next(), Some(Token::Let));
                let is_let = self.check(&Token::Let);
                if is_paren_let {
                    self.advance(); // consume (
                    self.advance(); // consume let
                    let pattern = self.parse_pattern()?;
                    if !self.check(&Token::Assign) && !self.check(&Token::LeftArrow) {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "`=` 或 `<-`".to_string(),
                        ));
                    }
                    self.advance();
                    // cjc: if (let pattern <- expr && extra_cond && ...) — 用 parse_expr 消费完整条件
                    let expr = Box::new(self.parse_expr()?);
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let then_stmts = self.parse_stmts()?;
                    // P3: 将所有语句包装为 Block 表达式，保留副作用
                    let then_expr = {
                        let (block_stmts, block_result) = if then_stmts.is_empty() {
                            (Vec::new(), None)
                        } else if let Some(Stmt::Expr(e)) = then_stmts.last() {
                            let result_expr = Box::new(e.clone());
                            (
                                then_stmts[..then_stmts.len() - 1].to_vec(),
                                Some(result_expr),
                            )
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
                                    (
                                        else_stmts[..else_stmts.len() - 1].to_vec(),
                                        Some(result_expr),
                                    )
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
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Assign),
                            "`=` 或 `<-`".to_string(),
                        ));
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
                            (
                                then_stmts[..then_stmts.len() - 1].to_vec(),
                                Some(result_expr),
                            )
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
                                    (
                                        else_stmts[..else_stmts.len() - 1].to_vec(),
                                        Some(result_expr),
                                    )
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
                    // cjc 兼容: if ( cond ) 先消费 (，使 parse_expr 解析完整条件而非仅 ( 内首表达式
                    let has_paren = self.check(&Token::LParen);
                    if has_paren {
                        self.advance();
                    }
                    let cond = self.parse_expr()?;
                    if has_paren {
                        self.expect(Token::RParen)?;
                    }
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
                // 支持 match (x) {、match x {、match { 三种语法 (cjc 兼容)
                let has_paren = self.check(&Token::LParen);
                if has_paren {
                    self.advance();
                }
                let expr = if self.check(&Token::LBrace) && self.peek_at(1) != Some(&Token::RBrace)
                {
                    // match { case ... } 无主体，视为 match (()) { ... }
                    // 但 match {} 不是有效的无主体语法（空块应该是主体）
                    Box::new(Expr::Block(vec![], None))
                } else {
                    Box::new(self.parse_match_subject()?)
                };
                if has_paren {
                    self.expect(Token::RParen)?;
                }
                self.expect(Token::LBrace)?;
                let arms = self.parse_match_arms()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Match { expr, arms })
            }
            // P5.1: spawn { block } — 单线程桩实现
            Some(Token::Spawn) => {
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Spawn { body })
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
            // unsafe { block } 作为表达式（如 write(unsafe { v.rawData() })），用 Block 包一层 Stmt::UnsafeBlock
            Some(Token::Unsafe) => {
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Block(
                    vec![crate::ast::Stmt::UnsafeBlock { body }],
                    None,
                ))
            }
            // 允许某些关键字作为变量名引用（如 `loop` 被用作变量名时）
            Some(Token::Loop) => Ok(Expr::Var("loop".to_string())),
            Some(tok) => self.bail_at(
                ParseError::UnexpectedToken(tok, "表达式".to_string()),
                self.at_prev(),
            ),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析结构体字段初始化
    pub(crate) fn parse_struct_fields(&mut self) -> Result<Vec<(String, Expr)>, ParseErrorAt> {
        let mut fields = Vec::new();
        if self.check(&Token::RBrace) {
            return Ok(fields);
        }

        eprintln!(
            "DEBUG parse_struct_fields: current token = {:?}",
            self.peek()
        );
        loop {
            let name = match self.advance_ident() {
                Some(name) => name,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    eprintln!(
                        "DEBUG parse_struct_fields: unexpected token {:?} at position",
                        tok
                    );
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                }
            };
            if self.check(&Token::Colon) {
                self.advance();
            } else if self.check(&Token::Assign) {
                self.advance();
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Colon),
                    "`:` 或 `=`".to_string(),
                ));
            }
            let value = self.parse_expr()?;
            fields.push((name, value));

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(fields)
    }

    /// 解析 Lambda 参数列表: x: T, y: T 或 x: T = default 或 name = expr（类型推断）
    pub(crate) fn parse_lambda_params(&mut self) -> Result<Vec<(String, Type)>, ParseErrorAt> {
        let mut params = Vec::new();

        loop {
            let name = if self.check(&Token::Underscore) {
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
            let ty = if self.check(&Token::Colon) {
                self.advance();
                self.parse_type()?
            } else if self.check(&Token::Assign) {
                // name = default：跳过默认值，类型推断
                self.advance();
                let _ = self.parse_expr()?;
                Type::Int64 // 默认使用 Int64
            } else if self.check(&Token::FatArrow) {
                // 单参无类型 lambda: { name => body }，不消费 =>
                params.push((name, Type::Int64)); // 默认使用 Int64
                break;
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Colon),
                    "`:` 或 `=`".to_string(),
                ));
            };
            params.push((name, ty));

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(params)
    }

    /// 解析 Lambda body（含结束的 `}`）并转为 Expr。
    /// 调用前必须已消费 `=>` 和开头的 `{`；始终使用 parse_stmts 支持多语句体（if/for/while 等开头）。
    pub(crate) fn parse_lambda_body(&mut self) -> Result<Expr, ParseErrorAt> {
        let stmts = self.parse_stmts()?;
        self.expect(Token::RBrace)?;
        let body = if let Some(Stmt::Expr(e)) = stmts.last() {
            let len = stmts.len();
            if len == 1 {
                e.clone()
            } else {
                Expr::Block(stmts[..len - 1].to_vec(), Some(Box::new(e.clone())))
            }
        } else {
            Expr::Block(stmts, None)
        };
        Ok(body)
    }

    /// 解析 Lambda 表达式的剩余部分: : ReturnType { body }
    pub(crate) fn parse_lambda_rest(
        &mut self,
        params: Vec<(String, Type)>,
    ) -> Result<Expr, ParseErrorAt> {
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
        let stmts = self.parse_stmts()?;
        self.expect(Token::RBrace)?;
        let body = if let Some(Stmt::Expr(e)) = stmts.last() {
            let len = stmts.len();
            if len == 1 {
                e.clone()
            } else {
                Expr::Block(stmts[..len - 1].to_vec(), Some(Box::new(e.clone())))
            }
        } else {
            Expr::Block(stmts, None)
        };
        Ok(Expr::Lambda {
            params,
            return_type,
            body: Box::new(body),
        })
    }

    /// 解析函数调用参数（支持位置参数和命名参数 name!: value）
    pub(crate) fn parse_args(&mut self) -> Result<(Vec<Expr>, Vec<(String, Expr)>), ParseErrorAt> {
        let mut args = Vec::new();
        let mut named_args = Vec::new();
        if self.check(&Token::RParen) {
            return Ok((args, named_args));
        }

        loop {
            // P2.9: 命名参数 name!: value 或 name: value (cjc 兼容)
            if self.peek_ident_like() {
                let next = self.peek_at(1).cloned();
                let is_named_bang = matches!(next, Some(Token::Bang))
                    && matches!(self.peek_at(2), Some(Token::Colon));
                let is_named_colon = matches!(next, Some(Token::Colon));
                if is_named_bang {
                    let name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                        }
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
                if is_named_colon {
                    let name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                        }
                    };
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
    pub(crate) fn parse_match_subject(&mut self) -> Result<Expr, ParseErrorAt> {
        // 只解析简单表达式: 变量、字面量、函数调用、字段访问等
        // 不解析结构体初始化 (因为 { 会被误认为 match body)
        let mut expr = match self.advance() {
            Some(Token::Integer(n)) => {
                // 与 parse_primary 一致：允许并吞掉整数字面量后缀。
                if matches!(
                    self.peek(),
                    Some(Token::Ident(ref s))
                        if s == "u8"
                            || s == "u16"
                            || s == "u32"
                            || s == "u64"
                            || s == "i8"
                            || s == "i16"
                            || s == "i32"
                            || s == "i64"
                )
                    || matches!(
                        self.peek(),
                        Some(Token::TypeUInt8)
                            | Some(Token::TypeUInt16)
                            | Some(Token::TypeUInt32)
                            | Some(Token::TypeUInt64)
                            | Some(Token::TypeInt8)
                            | Some(Token::TypeInt16)
                            | Some(Token::TypeInt32)
                            | Some(Token::TypeInt64)
                    )
                {
                    self.advance();
                }
                Expr::Integer(n)
            }
            Some(Token::Float(f)) | Some(Token::Float64Suffix(f)) => Expr::Float(f),
            Some(Token::Float16(f)) => Expr::Float32(f),
            Some(Token::Float32(f)) => Expr::Float32(f),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Expr::Var(n),
                None => Expr::Var("this".to_string()),
            },
            // cjc: super.field 作为 match 主题
            Some(Token::Super) => Expr::Var("super".to_string()),
            Some(Token::True) => Expr::Bool(true),
            Some(Token::False) => Expr::Bool(false),
            Some(Token::StringLit(s)) => self.parse_string_or_interpolated(s)?,
            Some(Token::BacktickStringLit(StringOrInterpolated::Interpolated(parts))) => {
                self.parse_string_or_interpolated(StringOrInterpolated::Interpolated(parts))?
            }
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(name))) => {
                if self.check(&Token::LParen) {
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Expr::Call {
                        name,
                        type_args: None,
                        args,
                        named_args,
                    }
                } else {
                    Expr::Var(name)
                }
            }
            Some(Token::RawStringLit(s))
            | Some(Token::MultiLineStringLit(s))
            | Some(Token::HashRawStringLit(s))
            | Some(Token::SingleQuoteStringLit(s)) => Expr::String(s),
            Some(Token::Ident(name)) => {
                if self.check(&Token::LParen) {
                    // 函数调用
                    self.advance();
                    let (args, named_args) = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Expr::Call {
                        name,
                        type_args: None,
                        args,
                        named_args,
                    }
                } else {
                    Expr::Var(name)
                }
            }
            Some(Token::LParen) => {
                let first = self.parse_expr()?;
                let expr = if self.check(&Token::Comma) {
                    let mut elts = vec![first];
                    while self.check(&Token::Comma) {
                        self.advance();
                        elts.push(self.parse_expr()?);
                    }
                    Expr::Tuple(elts)
                } else {
                    first
                };
                self.expect(Token::RParen)?;
                expr
            }
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(tok, "match 表达式".to_string()))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };

        // 处理后缀表达式 (字段访问、方法调用、数组索引、类型转换)
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.advance();
                    let field = match self.advance() {
                        Some(Token::Ident(name)) => name,
                        Some(Token::BacktickStringLit(StringOrInterpolated::Plain(name))) => name,
                        Some(Token::None) => "None".to_string(),
                        Some(Token::Some) => "Some".to_string(),
                        Some(Token::Ok) => "Ok".to_string(),
                        Some(Token::Err) => "Err".to_string(),
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "字段名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    if self.check(&Token::LParen) {
                        // 方法调用 obj.method(args)，如 Registration.find(name)
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: field,
                            args,
                            named_args,
                            type_args: None,
                        };
                    } else {
                        expr = Expr::Field {
                            object: Box::new(expr),
                            field,
                        };
                    }
                }
                Some(Token::LBracket) => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    expr = Expr::Index {
                        array: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                Some(Token::As) => {
                    self.advance();
                    let target_ty = self.parse_type()?;
                    expr = Expr::Cast {
                        expr: Box::new(expr),
                        target_ty,
                    };
                }
                // 可选链 obj?.field / obj?.method()
                Some(Token::Question) if matches!(self.peek_next(), Some(Token::Dot)) => {
                    self.advance(); // consume ?
                    self.advance(); // consume .
                    let field = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                        }
                    };
                    if self.check(&Token::LParen) {
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: field,
                            args,
                            named_args,
                            type_args: None,
                        };
                    } else {
                        expr = Expr::OptionalChain {
                            object: Box::new(expr),
                            field,
                        };
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 解析 for 循环的可迭代表达式（支持范围 expr..expr : step）
    pub(crate) fn parse_for_iterable(&mut self) -> Result<Expr, ParseErrorAt> {
        let start = self.parse_expr()?;
        if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
            let inclusive = self.check(&Token::DotDotEq);
            self.advance();
            let end = self.parse_expr()?;
            let step = if self.check(&Token::Colon) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            Ok(Expr::Range {
                start: Box::new(start),
                end: Box::new(end),
                inclusive,
                step,
            })
        } else {
            Ok(start)
        }
    }

    /// 解析 match 分支列表
    pub(crate) fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, ParseErrorAt> {
        let mut arms = Vec::new();

        while !self.check(&Token::RBrace) && self.peek().is_some() {
            // cjc: match 分支使用 case 关键字
            if matches!(self.peek(), Some(Token::Case)) {
                self.advance();
            }
            let pattern = self.parse_pattern()?;

            // 可选的守卫条件 (cjc 用 where / == expr，cjwasm 兼容 if)
            let guard = if self.check(&Token::If) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else if matches!(self.peek(), Some(Token::Where)) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else if self.check(&Token::Eq) {
                // cjc: case pattern == expr => body 表示 subject == expr
                self.advance();
                let right = self.parse_expr()?;
                Some(Box::new(Expr::Binary {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Var("__match_val".to_string())),
                    right: Box::new(right),
                }))
            } else {
                None
            };

            // cjc 兼容: case pattern => body 或 case pattern = body；跳过中间的 ++/-- 或多余的 break/continue
            while self.check(&Token::Incr)
                || self.check(&Token::Decr)
                || self.check(&Token::Break)
                || self.check(&Token::Continue)
            {
                self.advance();
            }
            if !self.check(&Token::FatArrow) && !self.check(&Token::Assign) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::FatArrow),
                    "`=>` 或 `=`".to_string(),
                ));
            }
            self.advance();
            // cjc: case X => 后可为多句语句，直到下一个 case 或 }
            let body_stmts = self.parse_stmts_until_case_or_rbrace()?;
            let body = Box::new(if body_stmts.is_empty() {
                Expr::Block(vec![], None)
            } else {
                // 与块表达式解析保持一致：最后一条 Stmt::Expr 提升为 block result，
                // 使得 match arm body 能在表达式上下文中正确产生值。
                let mut stmts = body_stmts;
                let result = if let Some(Stmt::Expr(_)) = stmts.last() {
                    if let Some(Stmt::Expr(e)) = stmts.pop() {
                        Some(Box::new(e))
                    } else {
                        None
                    }
                } else {
                    None
                };
                Expr::Block(stmts, result)
            });

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

    /// 解析字符串（普通或插值）
    pub(crate) fn parse_string_or_interpolated(
        &mut self,
        s: StringOrInterpolated,
    ) -> Result<Expr, ParseErrorAt> {
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
    pub(crate) fn parse_interpolation_expr(&self, expr_text: &str) -> Result<Expr, ParseErrorAt> {
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

    /// 检查 < 后面是否看起来像泛型方法调用
    /// 用于区分 obj.method<T>() 和 obj.field < value
    fn looks_like_generic_method_call(&self) -> bool {
        // 启发式：区分 obj.method<Type>(args) 与比较运算符 obj.field < value
        let next = match self.peek_at(1) {
            Some(n) => n,
            None => return false,
        };
        // 内置类型关键字（非 Ident）在表达式中不能作为比较右值，肯定是类型实参
        if matches!(
            next,
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
                | Token::TypeString
                | Token::TypeArray
                | Token::TypeTuple
                | Token::TypeRange
                | Token::TypeOption
                | Token::TypeResult
                | Token::TypeSlice
                | Token::TypeMap
                | Token::TypeVArray
                | Token::LParen
                | Token::LBracket
        ) {
            return true;
        }
        // 对于标识符，只有当 <Ident> 后面紧跟 > 或 , 时才认为是泛型类型实参
        // 否则（如 < range.start）视为比较运算符
        if matches!(
            next,
            Token::Ident(_) | Token::BacktickStringLit(StringOrInterpolated::Plain(_))
        ) {
            return matches!(self.peek_at(2), Some(Token::Gt | Token::Comma));
        }
        false
    }
}
