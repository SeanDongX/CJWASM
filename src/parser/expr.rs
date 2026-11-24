//! 表达式解析：parse_expr、parse_primary、二元/一元/后缀、lambda、match、for 等。

use super::{ParseError, ParseErrorAt, Parser};
use crate::ast::*;
use crate::lexer::{StringOrInterpolated, StringPart, Token};

impl Parser {
    /// 将组合表达式 `a ~> b` 脱糖为单参闭包 `{ x => b(a(x)) }`
    fn desugar_composition(left: Expr, right: Expr) -> Expr {
        let arg = "__compose_arg".to_string();
        let inner = Expr::Invoke {
            callee: Box::new(left),
            args: vec![Expr::Var(arg.clone())],
            named_args: vec![],
        };
        let outer = Expr::Invoke {
            callee: Box::new(right),
            args: vec![inner],
            named_args: vec![],
        };
        Expr::Lambda {
            params: vec![(arg, Type::TypeParam("_".to_string()))],
            return_type: None,
            body: Box::new(outer),
        }
    }

    /// 解析表达式（顶层为管道，再空值合并）
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_pipeline()?;
        // 函数组合 a ~> b：脱糖为单参闭包 λx -> b(a(x))
        while self.check(&Token::Composition) {
            self.advance();
            let right = self.parse_pipeline()?;
            left = Self::desugar_composition(left, right);
        }
        // 通用范围表达式 a..b / a..=b（起点不限于整数字面量，如 `a..b`、`x..-1`）。
        // 整数字面量开头的范围已在 parse_primary 中处理；此处覆盖变量/复杂表达式起点。
        loop {
            let inclusive = if self.check(&Token::DotDot) {
                false
            } else if self.check(&Token::DotDotEq) {
                true
            } else {
                break;
            };
            self.advance();
            let end = if self.check(&Token::RBracket)
                || self.check(&Token::RParen)
                || self.check(&Token::Comma)
                || self.check(&Token::RBrace)
            {
                // 开放式范围 a..
                Box::new(Expr::Integer(i64::MAX))
            } else {
                Box::new(self.parse_pipeline()?)
            };
            let step = if self.check(&Token::Colon) {
                self.advance();
                Some(Box::new(self.parse_pipeline()?))
            } else {
                None
            };
            left = Expr::Range {
                start: Box::new(left),
                end,
                inclusive,
                step,
            };
        }
        Ok(left)
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
        let mut left = self.parse_is_expr()?;

        // P6: `!in` 运算符 — expr !in collection
        if self.check(&Token::Bang) {
            if matches!(self.peek_next(), Some(Token::In)) {
                self.advance(); // consume !
                self.advance(); // consume in
                let right = self.parse_is_expr()?;
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
                // 仅当 < 后是类型关键字且紧跟 > 或 , 时才视为泛型实参列表。
                let next = self.peek_next();
                let is_type_keyword = matches!(
                    next,
                    Some(
                        Token::TypeInt64
                            | Token::TypeInt32
                            | Token::TypeFloat64
                            | Token::TypeFloat32
                            | Token::TypeBool
                            | Token::TypeString
                            | Token::TypeInt8
                            | Token::TypeInt16
                            | Token::TypeUInt8
                            | Token::TypeUInt16
                            | Token::TypeUInt32
                            | Token::TypeUInt64
                            | Token::TypeRune
                    )
                ) && matches!(self.peek_at(2), Some(Token::Gt | Token::Comma));
                let is_generic_ident = matches!(next, Some(Token::Ident(_)))
                    && matches!(self.peek_at(2), Some(Token::Gt | Token::Comma))
                    && self.has_generic_close();
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
            let right = self.parse_is_expr()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// 解析 `is` 类型检查（优先级高于 ==/!=，低于位运算）
    pub(crate) fn parse_is_expr(&mut self) -> Result<Expr, ParseErrorAt> {
        let mut left = self.parse_bitwise_or()?;
        while self.check(&Token::Is) {
            self.advance();
            let target_ty = self.parse_type()?;
            left = Expr::IsType {
                expr: Box::new(left),
                target_ty,
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
            // `-a..b`：负号作用于范围起点而非整个范围
            if let Expr::Range {
                start,
                end,
                inclusive,
                step,
            } = expr
            {
                return Ok(Expr::Range {
                    start: Box::new(Expr::Unary {
                        op: UnaryOp::Neg,
                        expr: start,
                    }),
                    end,
                    inclusive,
                    step,
                });
            }
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
                // 立即调用表达式，如零参 lambda `{ => expr }()`：
                // 无捕获、无参数的 lambda 直接求值其主体（语义等价于立即调用）。
                Some(Token::LParen) => {
                    if let Expr::Lambda { params, body, .. } = &expr {
                        if params.is_empty() {
                            self.advance(); // consume (
                            let (args, named_args) = self.parse_args()?;
                            self.expect(Token::RParen)?;
                            if args.is_empty() && named_args.is_empty() {
                                expr = (**body).clone();
                                continue;
                            }
                        }
                    }
                    // 函数值调用：任意表达式后跟 (args)，如 arr[0](x)、{x => x}(1)
                    if !self.newline_before_current() {
                        self.advance();
                        let (args, named_args) = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::Invoke {
                            callee: Box::new(expr),
                            args,
                            named_args,
                        };
                        continue;
                    }
                    break;
                }
                Some(Token::LBracket) => {
                    // 跨行的 `[` 视为新语句（数组字面量），而不是上一行表达式的索引，
                    // 例如连续两行数组字面量 `[1, 2]\n[3, 4]`。
                    if self.newline_before_current() {
                        break;
                    }
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
                        // 多参数下标（自定义 operator[]）：op[a, b, c]
                        if self.check(&Token::Comma) {
                            let mut elems = vec![first];
                            while self.check(&Token::Comma) {
                                self.advance();
                                // 容错尾随逗号
                                if self.check(&Token::RBracket) {
                                    break;
                                }
                                elems.push(self.parse_expr()?);
                            }
                            self.expect(Token::RBracket)?;
                            expr = Expr::Index {
                                array: Box::new(expr),
                                index: Box::new(Expr::Tuple(elems)),
                            };
                        } else if let Expr::Range {
                            start,
                            end,
                            inclusive: _,
                            step: _,
                        } = first
                        {
                            self.expect(Token::RBracket)?;
                            expr = Expr::SliceExpr {
                                array: Box::new(expr),
                                start,
                                end,
                            };
                        } else if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
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
                            // 多索引支持：c[i, j]（索引运算符重载可接受多个参数）
                            let index = if self.check(&Token::Comma) {
                                let mut elts = vec![first];
                                while self.check(&Token::Comma) {
                                    self.advance();
                                    if self.check(&Token::RBracket) {
                                        break;
                                    }
                                    elts.push(self.parse_expr()?);
                                }
                                Expr::Tuple(elts)
                            } else {
                                first
                            };
                            self.expect(Token::RBracket)?;
                            expr = Expr::Index {
                                array: Box::new(expr),
                                index: Box::new(index),
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
                    } else if self.check(&Token::Lt) {
                        // obj?.method<T> — 可选链方法带显式类型实参
                        let type_args = self.parse_opt_type_args()?;
                        if self.check(&Token::LParen) {
                            self.advance();
                            let (args, named_args) = self.parse_args()?;
                            self.expect(Token::RParen)?;
                            expr = Expr::MethodCall {
                                object: Box::new(expr),
                                method: field,
                                args,
                                named_args,
                                type_args,
                            };
                        } else {
                            expr = Expr::OptionalChain {
                                object: Box::new(expr),
                                field,
                            };
                        }
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

                        // 泛型参数后面必须跟 ( 或尾随闭包 { ... }
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
                        } else if self.check(&Token::LBrace) {
                            // obj.method<T> { ... } — 泛型方法调用 + 尾随闭包，
                            // 例如 std.unittest 的 Generators.generate<Int64> { ... }
                            let closure = match self.parse_primary()? {
                                e @ Expr::Lambda { .. } => e,
                                body => Expr::Lambda {
                                    params: vec![],
                                    return_type: None,
                                    body: Box::new(body),
                                },
                            };
                            expr = Expr::MethodCall {
                                object: Box::new(expr),
                                method: name,
                                args: vec![closure],
                                named_args: vec![],
                                type_args: Some(types),
                            };
                        } else {
                            // 泛型方法引用：obj.method<T>（无调用括号）
                            // 例如 std.unittest mock 中的 let f = x.FUNC_GENERIC<E>
                            expr = Expr::MethodCall {
                                object: Box::new(expr),
                                method: name,
                                args: vec![],
                                named_args: vec![],
                                type_args: Some(types),
                            };
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
                    // ? 运算符：expr? 提前返回 Err/None；
                    // 也支持可选调用尾随块：s?.f?{} / s?.f?{ ... }
                    self.advance();
                    let inner = Expr::Try(Box::new(expr));
                    if self.check(&Token::LBrace) {
                        let closure = self.parse_primary()?;
                        expr = Expr::Invoke {
                            callee: Box::new(inner),
                            args: vec![closure],
                            named_args: vec![],
                        };
                    } else {
                        expr = inner;
                    }
                }
                // P6: Trailing closure — f(args) { params => body } 或 expr.method { params => body }
                // 支持 Call、MethodCall、Field（如 .any { x => ... }），以及 Var（如 map { x => f(x) }）
                // 也支持 ConstructorCall（如 Array(n) { i => ... }）
                Some(Token::LBrace)
                    if !self.no_trailing_lambda
                        && !self.newline_before_current()
                        && matches!(
                            &expr,
                            Expr::Call { .. }
                                | Expr::MethodCall { .. }
                                | Expr::Field { .. }
                                | Expr::OptionalChain { .. }
                                | Expr::Var(_)
                                | Expr::ConstructorCall { .. }
                                | Expr::Invoke { .. }
                        ) =>
                {
                    // Peek ahead to check if this looks like a lambda: { ident => ... } or { => ... }
                    let looks_like_lambda = matches!(self.peek_next(), Some(Token::FatArrow))
                        || (matches!(self.peek_next(), Some(Token::Ident(_)))
                            && matches!(
                                self.peek_at(2),
                                Some(Token::FatArrow) | Some(Token::Colon) | Some(Token::Comma)
                            ));
                    let closure = if looks_like_lambda {
                        // Consume { and parse the lambda body
                        self.advance(); // consume {
                        if self.check(&Token::RBrace) {
                            // {} — 空 lambda（可选链 s?.f{} 等空体）
                            self.advance();
                            Expr::Lambda {
                                params: vec![],
                                return_type: None,
                                body: Box::new(Expr::Block(vec![], None)),
                            }
                        } else if self.check(&Token::FatArrow) {
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
                                let name = match self.advance() {
                                    Some(Token::Ident(n)) => n,
                                    Some(Token::Underscore) => "_".to_string(),
                                    Some(tok) => {
                                        return self.bail(ParseError::UnexpectedToken(
                                            tok,
                                            "参数名".to_string(),
                                        ));
                                    }
                                    None => return self.bail(ParseError::UnexpectedEof),
                                };
                                // 参数可带类型注解：{ a, b: Int32 => ... }
                                let ty = if self.check(&Token::Colon) {
                                    self.advance();
                                    self.parse_type()?
                                } else {
                                    Type::Int64 // 默认使用 Int64，避免 TypeParam
                                };
                                params.push((name, ty));
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
                        }
                    } else {
                        // 表达式体尾随 lambda：f{-1} / obj.m{expr} / f{ stmt; expr }
                        // parse_primary 把 `{ ... }` 解析为 Lambda 或 Block，
                        // Block 在尾随 lambda 语义下等价于零参 lambda 的表达式体。
                        match self.parse_primary()? {
                            e @ Expr::Lambda { .. } => e,
                            body => Expr::Lambda {
                                params: vec![],
                                return_type: None,
                                body: Box::new(body),
                            },
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
                        Expr::OptionalChain { object, field } => Expr::MethodCall {
                            object,
                            method: field,
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
                        Expr::Invoke {
                            callee,
                            args,
                            named_args,
                        } => {
                            let mut all_args = args;
                            all_args.push(closure);
                            Expr::Invoke {
                                callee,
                                args: all_args,
                                named_args,
                            }
                        }
                        _ => unreachable!(),
                    };
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 从已有的 postfix 表达式继续解析二元操作符（用于模式中的守卫表达式）
    /// 处理 &&、||、??、|> 等，优先级与正常表达式一致
    pub(crate) fn parse_guard_binary_rest(&mut self, mut left: Expr) -> Result<Expr, ParseErrorAt> {
        // 比较运算（无选择器 match 的 case 表达式，如 `case score < 60 =>`）
        while let Some(op) = match self.peek() {
            Some(Token::Lt) => Some(BinOp::Lt),
            Some(Token::Gt) => Some(BinOp::Gt),
            Some(Token::LtEq) => Some(BinOp::LtEq),
            Some(Token::GtEq) => Some(BinOp::GtEq),
            Some(Token::Eq) => Some(BinOp::Eq),
            Some(Token::NotEq) => Some(BinOp::NotEq),
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

    /// 词法层把以小数点开头的浮点字面量（`.5`、`.2e3`、`.23f32`）拆成了
    /// `Dot` + 数字 token。当前 token 是 `Dot` 且下一个是数字时，用源码切片
    /// 重建原始字面量文本并解析为浮点数。
    ///
    /// 调用前提：`self.peek()` 为 `Dot`。成功时消费两个 token 并返回数值。
    pub(crate) fn try_leading_dot_float(&mut self) -> Option<f64> {
        if !self.check(&Token::Dot) {
            return None;
        }
        let numeric = matches!(
            self.peek_next(),
            Some(Token::Integer(_))
                | Some(Token::Float(_))
                | Some(Token::Float32(_))
                | Some(Token::Float64Suffix(_))
        );
        if !numeric {
            return None;
        }
        let tok_idx = self.pos;
        let start = self.tokens.get(tok_idx)?.0;
        let end = self.tokens.get(tok_idx + 1)?.2;
        let raw = self.source.get(start..end)?.to_string();
        let mut text: String = raw.chars().filter(|c| *c != '_').collect();
        for suffix in ["f64", "f32", "f16"] {
            if text.ends_with(suffix) {
                text.truncate(text.len() - suffix.len());
                break;
            }
        }
        if text.ends_with('f') {
            text.pop();
        }
        if !text.starts_with('.') {
            return None;
        }
        let value = format!("0{}", text).parse::<f64>().ok()?;
        self.advance(); // Dot
        self.advance(); // 数字 token
        Some(value)
    }

    /// 整数字面量作为表达式主元，并在其后识别范围表达式（`a..b` / `a..=b`）。
    ///
    /// `lit` 为已构造好的字面量表达式（保留无后缀 / 带后缀的区分）。
    fn finish_int_primary(&mut self, lit: Expr) -> Result<Expr, ParseErrorAt> {
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
                // 结束值：范围运算符优先级最低，按完整表达式解析（如 1..3 + 3）
                Box::new(self.parse_pipeline()?)
            };
            // P2.6: 可选步长 `: step`（支持负步长 -1）
            let step = if self.check(&Token::Colon) {
                self.advance();
                Some(Box::new(self.parse_pipeline()?))
            } else {
                None
            };
            return Ok(Expr::Range {
                start: Box::new(lit),
                end,
                inclusive,
                step,
            });
        }
        Ok(lit)
    }

    /// 解析基础表达式
    pub(crate) fn parse_primary(&mut self) -> Result<Expr, ParseErrorAt> {
        // 循环表达式作为值使用（如 `let x = for (...) { ... }`）：
        // 语句形式解析后包成 Block 表达式，类型为 Unit。
        if matches!(
            self.peek(),
            Some(Token::For) | Some(Token::While) | Some(Token::Do)
        ) || (self.check(&Token::Loop) && matches!(self.peek_next(), Some(Token::LBrace)))
        {
            let stmt = self.parse_stmt()?;
            return Ok(Expr::Block(vec![stmt], None));
        }
        // 前置小数点浮点字面量 `.5`：词法层拆成 Dot + 数字
        if self.check(&Token::Dot) {
            if let Some(value) = self.try_leading_dot_float() {
                return Ok(Expr::Float(value));
            }
        }
        match self.advance() {
            // 宏调用 @MacroName 或 @MacroName(args)
            Some(Token::At) => {
                self.pos -= 1; // 回退，让 parse_macro_call 重新消费 @
                self.parse_macro_call()
            }
            // `_` 通配符表达式：宏实参（如 @On(a.intOpt(_))）以及
            // 解构赋值左侧（如 `(c.f, _) = pair`）。仅在此处作为 Var("_") 接纳，
            // 后续由赋值目标转换/宏处理决定其语义。
            Some(Token::Underscore) => {
                Ok(Expr::Var("_".to_string()))
            }
            Some(Token::Integer(n)) => {
                // 整数字面量后缀已在词法层识别为独立 token（`IntegerSuffixed`），
                // 此处不再向后窥探并吞掉同名标识符（否则会跨行吞掉变量名）。
                self.finish_int_primary(Expr::Integer(n))
            }
            Some(Token::IntegerSuffixed((n, suf))) => {
                self.finish_int_primary(Expr::IntegerSuffixed(n, suf))
            }
            Some(Token::Float(f)) | Some(Token::Float64Suffix(f)) => Ok(Expr::Float(f)),
            Some(Token::Float32(f)) => Ok(Expr::Float32(f)),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Ok(Expr::Var(n)),
                None => self.bail_at(
                    ParseError::UnexpectedToken(Token::This, "this 仅可在方法体内使用".to_string()),
                    self.at_prev(),
                ),
            },
            Some(Token::Super) => {
                // 规范 6.1.2.4(17)：静态成员函数不能引用 super
                if self.in_static_method {
                    return self.bail_at(
                        ParseError::UnexpectedToken(
                            Token::Super,
                            "静态方法不能使用 super".to_string(),
                        ),
                        self.at_prev(),
                    );
                }
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
                    let field_or_method = match self.advance() {
                        Some(Token::Ident(m)) => m,
                        Some(tok) => {
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "字段名或方法名".to_string(),
                            ))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    // 检查是否为方法调用 super.method(...) 或字段访问 super.field
                    // 也支持泛型方法调用 super.method<T>(...)
                    if self.check(&Token::Lt) {
                        let _ = self.parse_opt_type_args()?;
                    }
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
                } else if self.check(&Token::LBracket) {
                    // super[i] — 由外层后缀解析继续处理索引
                    Ok(Expr::Var("super".to_string()))
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
            Some(Token::StringLit(s)) | Some(Token::BacktickStringLit(s)) => {
                self.parse_string_or_interpolated(s)
            }
            Some(Token::MultiLineStringLit(s)) => self.parse_multiline_string(s),
            Some(Token::RawStringLit(s)) | Some(Token::HashRawStringLit(s)) | Some(Token::SingleQuoteStringLit(s)) => {
                Ok(Expr::String(s))
            }
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
                    || self.check(&Token::RParen)
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
                    // 或者 name: Type = expr（带类型注解的资源绑定）
                    let has_bare_assign = matches!(self.peek(), Some(Token::Ident(_)))
                        && matches!(self.peek_at(1), Some(Token::Assign) | Some(Token::Colon));
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
                            // 可选的类型注解 name: Type
                            if self.check(&Token::Colon) {
                                self.advance();
                                let _ = self.parse_type()?;
                            }
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
                    let var = match self.advance() {
                        Some(Token::Ident(v)) => Some(v),
                        Some(Token::Underscore) => None, // catch (_: Type)
                        _ => return self.bail(ParseError::UnexpectedEof),
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
                let mut catch_body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                // 多个 catch 子句（try { } catch (...) { } catch (...) { }）：
                // AST 仅保留第一个 catch，其余做语法接纳后丢弃。
                while self.check(&Token::Catch) {
                    self.advance();
                    if self.check(&Token::LParen) {
                        self.skip_balanced_mixed()?;
                    }
                    self.expect(Token::LBrace)?;
                    let _ = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                }
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
                    if let Some(Token::Ident(ref v)) = self.peek_next() {
                        let variant_looks_like_type =
                            v.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                        // `Type.Method<T>` 形式更可能是泛型方法引用而非枚举变体，
                        // 交给 postfix 的泛型方法调用路径处理。
                        let followed_by_type_args =
                            matches!(self.peek_at(2), Some(Token::Lt));
                        if variant_looks_like_type && !followed_by_type_args {
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
                // 但 `a < UInt32.Max` 这类比较：`<` 后基础类型再跟 `.`，不能当作泛型实参
                let type_args = if matches!(self.peek(), Some(Token::Lt))
                    && !matches!(self.peek_at(2), Some(Token::Dot))
                    && self.has_generic_close()
                {
                    self.parse_opt_type_args()?
                } else {
                    None
                };
                // 检查是否是函数调用、构造函数调用或结构体初始化
                // 跨行的 `(` 视为新语句/元组起点，而不是上一行标识符的调用，避免
                // `var f = g\n(x, y) = ...` 把元组赋值吞成调用参数。
                match self.peek() {
                    Some(Token::LParen) if !self.newline_before_current() => {
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
                        // `TypeName<T...> { params => body }` 形式（如 VArray<Int64,$5>{i=>i}）
                        // 是带尾随闭包的构造调用，不是结构体字面量。
                        let is_lambda = matches!(self.peek_next(), Some(Token::FatArrow))
                            || (matches!(self.peek_next(), Some(Token::Ident(_)))
                                && matches!(self.peek_at(2), Some(Token::FatArrow)));
                        if is_lambda {
                            return Ok(Expr::ConstructorCall {
                                name,
                                type_args,
                                args: Vec::new(),
                                named_args: Vec::new(),
                            });
                        }
                        // 花括号内容不是字段初始化（如 `Column { stmt; stmt }`）时，
                        // 这是 `f { ... }` 尾随闭包调用而非结构体字面量。
                        if !self.looks_like_struct_init_body() {
                            return Ok(Expr::Var(name));
                        }
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
                if matches!(self.peek(), Some(Token::Ident(_)) | Some(Token::Underscore))
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
                // 解析第一个表达式（括号内允许 `_` 通配符，用于元组解构赋值 (_, a) = ...）
                let prev_wildcard = self.allow_wildcard_expr;
                self.allow_wildcard_expr = true;
                let first_result = self.parse_expr();
                let first = match first_result {
                    Ok(e) => e,
                    Err(e) => {
                        self.allow_wildcard_expr = prev_wildcard;
                        return Err(e);
                    }
                };
                self.allow_wildcard_expr = prev_wildcard;
                if self.check(&Token::Comma) {
                    // 这是元组字面量 (a, b, ...)
                    self.advance();
                    let mut elements = vec![first];
                    if !self.check(&Token::RParen) {
                        loop {
                            let prev_wildcard = self.allow_wildcard_expr;
                            self.allow_wildcard_expr = true;
                            let elem = self.parse_expr();
                            self.allow_wildcard_expr = prev_wildcard;
                            elements.push(elem?);
                            if !self.check(&Token::Comma) {
                                break;
                            }
                            self.advance();
                            // 允许尾随逗号：(a, b,)
                            if self.check(&Token::RParen) {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::Tuple(elements))
                } else {
                    // 括号中的赋值表达式：(x = v)、(x += v) —— 结果类型为 Unit
                    let assign_bin = match self.peek() {
                        Some(Token::Assign) => Some(None),
                        Some(Token::PlusEq) => Some(Some(BinOp::Add)),
                        Some(Token::MinusEq) => Some(Some(BinOp::Sub)),
                        Some(Token::StarEq) => Some(Some(BinOp::Mul)),
                        Some(Token::SlashEq) => Some(Some(BinOp::Div)),
                        Some(Token::PercentEq) => Some(Some(BinOp::Mod)),
                        Some(Token::ShlEq) => Some(Some(BinOp::Shl)),
                        Some(Token::ShrEq) => Some(Some(BinOp::Shr)),
                        Some(Token::AndEq) => Some(Some(BinOp::BitAnd)),
                        Some(Token::PipeEq) => Some(Some(BinOp::BitOr)),
                        Some(Token::CaretEq) => Some(Some(BinOp::BitXor)),
                        Some(Token::StarStarEq) => Some(Some(BinOp::Pow)),
                        Some(Token::OrOrEq) => Some(Some(BinOp::LogicalOr)),
                        Some(Token::AndAndEq) => Some(Some(BinOp::LogicalAnd)),
                        _ => None,
                    };
                    if let Some(bin_op) = assign_bin {
                        self.advance();
                        let rhs = self.parse_expr()?;
                        let target = self.expr_to_assign_target(first)?;
                        let value = match bin_op {
                            None => rhs,
                            Some(op) => Expr::Binary {
                                op,
                                left: Box::new(self.assign_target_to_expr(&target)),
                                right: Box::new(rhs),
                            },
                        };
                        self.expect(Token::RParen)?;
                        return Ok(Expr::Block(
                            vec![Stmt::Assign { target, value }],
                            None,
                        ));
                    }
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
                let second = self.peek_at(1).cloned();
                let is_lambda =
                    matches!(self.peek(), Some(Token::Ident(_)) | Some(Token::Underscore))
                        && matches!(
                            &second,
                            Some(Token::Colon) | Some(Token::Comma) | Some(Token::FatArrow)
                        );
                if is_lambda {
                    if matches!(&second, Some(Token::Colon)) {
                        // Lambda { x: T, y: T => body }；{ 已消费，解析参数
                        let params = self.parse_lambda_params()?;
                        self.expect(Token::FatArrow)?;
                        let body = self.parse_lambda_body()?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None,
                            body: Box::new(body),
                        });
                    } else if matches!(&second, Some(Token::Comma)) {
                        // 多参无类型 Lambda { a, b, c => body }
                        let mut params = Vec::new();
                        loop {
                            let name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "参数名".to_string(),
                                    ));
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            // 参数可带类型注解：{ a, b: Int32 => ... }
                            let ty = if self.check(&Token::Colon) {
                                self.advance();
                                self.parse_type()?
                            } else {
                                Type::Int64 // 默认使用 Int64
                            };
                            params.push((name, ty));
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
                    } else if matches!(&second, Some(Token::FatArrow)) {
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
            // P6: VArray<T, N>(...) 值类型定长数组构造 — 兼容优先降级为 Array<T>(N, init)
            // N 为编译期常量（如 `4`、`$0`、`$10000`），支持 `repeat:` 命名参数
            Some(Token::TypeVArray) => {
                let mut elem_len: Option<(Option<Type>, i64)> = None;
                if self.check(&Token::Lt) {
                    self.advance();
                    let elem_ty = self.parse_type()?;
                    self.expect(Token::Comma)?;
                    let len = self.parse_varray_len()?.unwrap_or(0);
                    if self.check(&Token::Gt) {
                        self.advance();
                    } else if self.check(&Token::Shr) {
                        self.advance();
                        self.pushback = Some(Token::Gt);
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Gt),
                            "`>`".to_string(),
                        ));
                    }
                    elem_len = Some((Some(elem_ty), len));
                }
                // `VArray<T, $N> { i => ... }`：尾随闭包初始化，等价于 Array(N) { i => ... }
                let is_trailing_lambda = self.check(&Token::LBrace)
                    && (matches!(self.peek_next(), Some(Token::FatArrow))
                        || (matches!(self.peek_next(), Some(Token::Ident(_)))
                            && matches!(
                                self.peek_at(2),
                                Some(Token::FatArrow | Token::Colon | Token::Comma)
                            )));
                if is_trailing_lambda {
                    let (elem_ty, len) = elem_len.unwrap_or((None, 0));
                    return Ok(Expr::ConstructorCall {
                        name: "Array".to_string(),
                        type_args: elem_ty.map(|t| vec![t]),
                        args: vec![Expr::Integer(len)],
                        named_args: vec![],
                    });
                }
                self.expect(Token::LParen)?;
                let (mut args, named_args) = self.parse_args()?;
                self.expect(Token::RParen)?;
                let mut init: Option<Expr> = None;
                for (n, e) in named_args {
                    if n == "repeat" {
                        init = Some(e);
                    }
                }
                // 兼容：若无可识别的 repeat 命名参数，退化为使用最后一个位置参数作为初值
                let init = init.or_else(|| args.pop()).unwrap_or(Expr::Integer(0));
                let (elem_ty, len) = elem_len.unwrap_or((None, 0));
                Ok(Expr::ConstructorCall {
                    name: "Array".to_string(),
                    type_args: elem_ty.map(|t| vec![t]),
                    args: vec![Expr::Integer(len), init],
                    named_args: vec![],
                })
            }
            // Range<T>(...) / Range(...) 构造表达式 — 兼容降级为同名构造调用
            Some(Token::TypeRange) => {
                let type_args = if self.check(&Token::Lt) {
                    self.advance();
                    let mut tys = vec![self.parse_type()?];
                    while self.check(&Token::Comma) {
                        self.advance();
                        tys.push(self.parse_type()?);
                    }
                    if self.check(&Token::Gt) {
                        self.advance();
                    } else if self.check(&Token::Shr) {
                        self.advance();
                        self.pushback = Some(Token::Gt);
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Gt),
                            "`>`".to_string(),
                        ));
                    }
                    Some(tys)
                } else {
                    None
                };
                self.expect(Token::LParen)?;
                let (args, named_args) = self.parse_args()?;
                self.expect(Token::RParen)?;
                Ok(Expr::ConstructorCall {
                    name: "Range".to_string(),
                    type_args,
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
                    // 非括号 if-let 主体后 `{` 是 then 体，禁止尾随 lambda
                    let saved_no_lambda = self.no_trailing_lambda;
                    self.no_trailing_lambda = true;
                    let subject = self.parse_match_subject();
                    self.no_trailing_lambda = saved_no_lambda;
                    let expr = Box::new(subject?);
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
                    // 非括号条件后紧跟 `{` 是语句块体，禁止把尾随 `{` 当作 lambda
                    let saved_no_lambda = self.no_trailing_lambda;
                    if !has_paren {
                        self.no_trailing_lambda = true;
                    }
                    let cond = self.parse_expr()?;
                    self.no_trailing_lambda = saved_no_lambda;
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
                let expr = if has_paren {
                    // 括号内是完整表达式上下文，`{` 不会被误认为 match 体，
                    // 因此用 parse_expr 支持 `match (i % 4)` 等二元表达式主体。
                    Box::new(self.parse_expr()?)
                } else if self.check(&Token::LBrace) && self.peek_at(1) != Some(&Token::RBrace)
                {
                    // match { case ... } 无主体，视为 match (()) { ... }
                    // 但 match {} 不是有效的无主体语法（空块应该是主体）
                    Box::new(Expr::Block(vec![], None))
                } else if has_paren {
                    // match (完整表达式) { ... }：括号内是普通表达式（可含运算符）
                    Box::new(self.parse_expr()?)
                } else {
                    // 非括号主体后紧跟 `{` 是 match 体，禁止把尾随 `{` 当作 lambda
                    let saved_no_lambda = self.no_trailing_lambda;
                    self.no_trailing_lambda = true;
                    let subject = self.parse_match_subject();
                    self.no_trailing_lambda = saved_no_lambda;
                    Box::new(subject?)
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
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => {
                    eprintln!(
                        "DEBUG parse_struct_fields: unexpected token {:?} at position",
                        tok
                    );
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                }
                None => return self.bail(ParseError::UnexpectedEof),
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
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(Token::Underscore) => "_".to_string(),
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
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
            // 允许尾随逗号：f(a, b,)
            if self.check(&Token::RParen) {
                break;
            }
            // C 互操作：inout 实参修饰符（按值传递，仅做语法接纳）
            if self.check(&Token::Inout) {
                self.advance();
            }
            // P2.9: 命名参数 name!: value 或 name: value (cjc 兼容)
            if let Some(Token::Ident(_)) = self.peek() {
                let next = self.peek_at(1).cloned();
                let is_named_bang = matches!(next, Some(Token::Bang))
                    && matches!(self.peek_at(2), Some(Token::Colon));
                let is_named_colon = matches!(next, Some(Token::Colon));
                if is_named_bang {
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
                if is_named_colon {
                    let name = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        _ => unreachable!(),
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
                // 宏 DSL 实参（如 @PowerAssert 内的 materializeInt(b = 2)）：
                // 仅在解析宏实参期间允许 `name = value` 形式
                if self.allow_wildcard_expr && matches!(next, Some(Token::Assign)) {
                    let name = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        _ => unreachable!(),
                    };
                    self.advance(); // skip =
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
        // 一元前缀：match (!x)、match (-x)
        if matches!(
            self.peek(),
            Some(Token::Bang) | Some(Token::Minus) | Some(Token::Tilde)
        ) {
            let e = self.parse_unary()?;
            return self.parse_postfix_from_expr(e);
        }
        // 只解析简单表达式: 变量、字面量、函数调用、字段访问等
        // 不解析结构体初始化 (因为 { 会被误认为 match body)
        let mut expr = match self.advance() {
            Some(Token::Integer(n)) => Expr::Integer(n),
            Some(Token::IntegerSuffixed((n, suf))) => Expr::IntegerSuffixed(n, suf),
            Some(Token::Float(f)) | Some(Token::Float64Suffix(f)) => Expr::Float(f),
            Some(Token::Float32(f)) => Expr::Float32(f),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Expr::Var(n),
                None => Expr::Var("this".to_string()),
            },
            // cjc: super.field 作为 match 主题
            Some(Token::Super) => Expr::Var("super".to_string()),
            Some(Token::True) => Expr::Bool(true),
            Some(Token::False) => Expr::Bool(false),
            Some(Token::StringLit(s)) | Some(Token::BacktickStringLit(s)) => {
                self.parse_string_or_interpolated(s)?
            }
            Some(Token::MultiLineStringLit(s)) => self.parse_multiline_string(s)?,
            Some(Token::RawStringLit(s)) | Some(Token::HashRawStringLit(s)) | Some(Token::SingleQuoteStringLit(s)) => Expr::String(s),
            Some(Token::Ident(name)) => {
                // 泛型实参形式的主体：match (A<Bool>)
                if self.check(&Token::Lt) {
                    let _ = self.parse_opt_type_args()?;
                }
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
                // `()` 空元组（如 while (let _ <- ())、match (())）
                if self.check(&Token::RParen) {
                    self.advance();
                    Expr::Tuple(vec![])
                } else {
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
            }
            Some(Token::LBrace) if self.check(&Token::RBrace) => {
                // match {} — 空块不能作为 match 主体
                return self.bail(ParseError::UnexpectedToken(
                    Token::LBrace,
                    "match 表达式".to_string(),
                ));
            }
            Some(_) => {
                // 其他表达式起点（Some/None/lambda 字面量等）：回退后走通用一元表达式解析
                self.pos = self.pos.saturating_sub(1);
                return self.parse_unary();
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

    /// 解析多行字符串（可能含 ${...} 插值）。
    ///
    /// 任一插值表达式无法按表达式解析时，整体回退为普通字符串字面量，
    /// 避免含声明等未支持语法的多行插值导致编译失败。
    pub(crate) fn parse_multiline_string(
        &mut self,
        s: StringOrInterpolated,
    ) -> Result<Expr, ParseErrorAt> {
        let parts = match s {
            StringOrInterpolated::Plain(text) => return Ok(Expr::String(text)),
            StringOrInterpolated::Interpolated(parts) => parts,
        };
        let parseable = parts.iter().all(|part| match part {
            StringPart::Interpolation(text) => self.parse_interpolation_expr(text).is_ok(),
            StringPart::Literal(_) => true,
        });
        if !parseable {
            let mut raw = String::new();
            for part in &parts {
                match part {
                    StringPart::Literal(t) => raw.push_str(t),
                    StringPart::Interpolation(t) => {
                        raw.push_str("${");
                        raw.push_str(t);
                        raw.push('}');
                    }
                }
            }
            return Ok(Expr::String(raw));
        }
        self.parse_string_or_interpolated(StringOrInterpolated::Interpolated(parts))
    }

    /// 解析插值表达式内的文本。
    ///
    /// 规范允许 `${...}` 内前导分号（如 `${;;;1 + 2}`），但插值内容必须是表达式；
    /// 含声明的插值需要类型检查，这里不展开，交由表达式解析器判定。
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
        // 插值表达式在原上下文中求值：继承 `this` 可用性与当前泛型参数，
        // 避免方法体 `"${this.x}"` 被误判为“this 仅可在方法体内使用”。
        parser.receiver_name = self.receiver_name.clone();
        parser.current_type_params = self.current_type_params.clone();
        // 跳过前导分号（`${;;;expr}`）
        while parser.check(&Token::Semicolon) {
            parser.advance();
        }
        if parser.peek().is_none() {
            return Err(ParseErrorAt {
                error: ParseError::UnexpectedEof,
                byte_start: self.at_prev().0,
                byte_end: self.at_prev().1,
            });
        }
        // `${ var a = 1; var b = 2; a + b }`：插值内允许语句序列，块的值取最后一条表达式
        let mut stmts = parser.parse_stmts()?;
        if stmts.len() == 1 {
            if let Some(Stmt::Expr(e)) = stmts.pop() {
                return Ok(e);
            }
        }
        let result = if matches!(stmts.last(), Some(Stmt::Expr(_))) {
            match stmts.pop() {
                Some(Stmt::Expr(e)) => Some(Box::new(e)),
                _ => None,
            }
        } else {
            None
        };
        Ok(Expr::Block(stmts, result))
    }

    /// 当前 token 为 `{` 时，判断花括号内容是否像结构体字段初始化列表。
    ///
    /// 用于区分 `S { x: 1 }`（结构体字面量）与 `Column { stmt; stmt }`（尾随闭包调用）。
    /// 字段形态：`name:`、`name =`、简写 `name,` / `name }`；空 `{}` 视为字段列表。
    fn looks_like_struct_init_body(&self) -> bool {
        match self.peek_next() {
            Some(Token::RBrace) => true,
            Some(Token::Ident(_)) => matches!(
                self.peek_at(2),
                Some(Token::Colon) | Some(Token::Assign) | Some(Token::Comma) | Some(Token::RBrace)
            ),
            _ => false,
        }
    }

    /// 检查 < 后面是否看起来像泛型方法调用
    /// 当前 token 为 `{` 时，判断该花括号是否开始一个闭包（lambda），
    /// 形如 `{ => ... }`、`{ x => ... }`、`{ _ => ... }`、`{ x: T => ... }`、
    /// `{ a, b => ... }`。用于区分结构体初始化与尾随闭包。
    pub(crate) fn brace_starts_lambda(&self) -> bool {
        match self.peek_next() {
            Some(Token::FatArrow) => true,
            Some(Token::Ident(_)) | Some(Token::Underscore) => {
                // `{ x => ... }`：参数名后直接 =>
                if matches!(self.peek_at(2), Some(Token::FatArrow)) {
                    return true;
                }
                // `{ x: T => ... }` / `{ a, b => ... }` 与结构体初始化 `{ x: value }`
                // 形似，需向前扫描顶层是否存在 `=>` 才能区分。
                if matches!(
                    self.peek_at(2),
                    Some(Token::Colon) | Some(Token::Comma)
                ) {
                    let mut depth = 0i32;
                    let mut i = self.pos;
                    while i < self.tokens.len() {
                        match &self.tokens[i].1 {
                            Token::LBrace => depth += 1,
                            Token::RBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Token::FatArrow if depth == 1 => return true,
                            _ => {}
                        }
                        i += 1;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// 用于区分 obj.method<T>() 和 obj.field < value
    fn looks_like_generic_method_call(&self) -> bool {
        // 启发式：区分 obj.method<Type>(args) 与比较运算符 obj.field < value
        let next = match self.peek_at(1) {
            Some(n) => n,
            None => return false,
        };
        // `x < Float64.NaN` 这类比较：`<` 后的基础类型紧跟 '.'（静态成员访问）时
        // 是右值，不是类型实参。
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
                | Token::TypeString
        ) && matches!(self.peek_at(2), Some(Token::Dot))
        {
            return false;
        }
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
        if matches!(next, Token::Ident(_)) {
            return matches!(self.peek_at(2), Some(Token::Gt | Token::Comma));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_ok(src: &str) {
        let tokens: Result<Vec<_>, _> = Lexer::new(src).collect();
        let tokens = tokens.expect("lex error");
        let mut parser = Parser::new(tokens).with_source(src);
        parser
            .parse_program()
            .unwrap_or_else(|e| panic!("期望解析成功，实际报错: {:?}", e));
    }

    #[test]
    fn test_match_paren_binary_subject() {
        // match (i % 4) — 括号内主体支持完整二元表达式
        parse_ok("func f(i: Int64) { let x = match (i % 4) { case _ => 0 } }");
        parse_ok("func f(i: Int64) { let x = match (i + 1) { case _ => 0 } }");
    }

    #[test]
    fn test_trailing_lambda_expression_body() {
        // f{-1} / obj.m{expr} — 表达式体尾随 lambda
        parse_ok("func f() { let x = foo{-1} }");
        parse_ok("func f() { let x = obj.getOrDefault{-1} }");
        parse_ok("func f() { let x = foo { stmt } }");
    }

    #[test]
    fn test_call_trailing_comma() {
        parse_ok("func f() { g(1, 2,) }");
        parse_ok("func f() { g() }");
    }

    #[test]
    fn test_multi_arg_index() {
        // 自定义 operator[] 的多参数下标
        parse_ok("func f(op: Int64) { let x = op[1, 2, false] }");
    }

    #[test]
    fn test_destructor_with_parens() {
        parse_ok("class A { ~init() {} }");
        parse_ok("class A { ~init {} }");
    }

    #[test]
    fn test_bodyless_prop_in_class() {
        // 抽象/无体属性声明（无 get/set 块）
        parse_ok("abstract class A { public mut prop x: Int64\n public prop y: Int64 }");
    }

    #[test]
    fn test_import_internal_segment() {
        parse_ok("import std.unittest.mock.internal.*");
        parse_ok("import std.unittest.mock.internal.{createMock, createSpy}");
    }

    #[test]
    fn test_protected_class_at_top_level() {
        // `protected` 在顶层也可以是类的可见性修饰符，而非 protected package
        parse_ok("protected class A {}");
    }

    #[test]
    fn test_for_in_block_not_swallowed_as_lambda() {
        // for x in arr { ... } 的 { 是循环体，不能被解析为尾随 lambda
        parse_ok("func f() { for x in arr { x } }");
    }

    #[test]
    fn test_return_as_paren_expression() {
        // `(x ?? return)` — return 作为表达式，右括号终止其可选的返回值
        parse_ok("func f(): Int64 { let m = (x ?? return)\n 0 }");
    }
}

mod expr_tests {
    use crate::pipeline::parse_source;

    fn ok(src: &str) {
        if let Err(e) = parse_source(src) {
            panic!("parse failed: {}\nsource:\n{}", e, src);
        }
    }

    #[test]
    fn test_match_parenthesized_expression_subject() {
        ok(r#"
main() {
    var i = 0
    match (i + 10) {
        case 10 => 1
        case _ => 0
    }
}
"#);
    }

    #[test]
    fn test_consecutive_array_literal_statements() {
        ok(r#"
func foo() { 321 }
main() {
    [1, 2, 3]
    ['a', 'b', 'c']
    [0x23.2p1, 1.2, 3.21e5]
    [1..3, 2..4, 3..=6]
    [[1, 2], [3, 4]]
    [foo, { => 42 }]
    return 0
}
"#);
    }

    #[test]
    fn test_optional_chain_forms() {
        ok(r#"
class S {
    var id = 1
    func f() {}
    func vf() {}
}
main() {
    let s: ?S = S()
    s?.f?{}
    s?.vf{}
    s?.id<Int64>
    s?.a
}
"#);
    }

    #[test]
    fn test_multi_index_expression() {
        ok(r#"
class C {
    operator func [](i: Int64, j: Int64): Int64 { i + j }
}
main() {
    let c = C()
    let v = c[0, 1]
}
"#);
    }

    #[test]
    fn test_interpolation_with_statements() {
        ok(r#"
main() {
    let a = "Bla ${ var x = 1; var y = 2; x + y } bla"
    let b = "${ 1 + 1 }"
    return 0
}
"#);
    }

    #[test]
    fn test_hex_literal_range() {
        ok(r#"
main() {
    let a = 0x1..5
    let b = -0x83c91..=0o3470601:5
}
"#);
    }

    #[test]
    fn test_range_with_step_precedence() {
        ok(r#"
main() {
    let a = 0 * 1 .. 3 + 3:1 + 1
}
"#);
    }

    #[test]
    fn test_super_index_assignment() {
        ok(r#"
open class A {}
class B <: A {
    func g() {
        super[0] = 1
    }
}
main() {}
"#);
    }
}
