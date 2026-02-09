use crate::ast::*;
use crate::lexer::Token;
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
}

impl Parser {
    pub fn new(tokens: Vec<(usize, Token, usize)>) -> Self {
        Self {
            tokens,
            pos: 0,
            receiver_name: None,
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
        self.tokens.get(self.pos).map(|(_, t, _)| t)
    }

    fn peek_next(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1).map(|(_, t, _)| t)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].1.clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
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
        let mut structs = Vec::new();
        let mut functions = Vec::new();

        let mut enums = Vec::new();
        while let Some(tok) = self.peek() {
            match tok {
                Token::Struct => structs.push(self.parse_struct()?),
                Token::Enum => enums.push(self.parse_enum()?),
                Token::Func => functions.push(self.parse_function()?),
                _ => {
                    return self.bail(ParseError::UnexpectedToken(
                        tok.clone(),
                        "struct、enum 或 func".to_string(),
                    ))
                }
            }
        }
        Ok(Program {
            structs,
            enums,
            functions,
        })
    }

    /// 解析结构体定义
    fn parse_struct(&mut self) -> Result<StructDef, ParseErrorAt> {
        self.expect(Token::Struct)?;

        let name = match self.advance() {
            Some(Token::Ident(name)) => name,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "结构体名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };

        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();

        while !self.check(&Token::RBrace) {
            let field_name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
            };
            self.expect(Token::Colon)?;
            let ty = self.parse_type()?;
            fields.push(FieldDef {
                name: field_name,
                ty,
            });

            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(Token::RBrace)?;
        Ok(StructDef { name, fields })
    }

    /// 解析枚举定义（支持无关联值或单关联类型变体，如 Ok(Int64)）
    fn parse_enum(&mut self) -> Result<EnumDef, ParseErrorAt> {
        self.expect(Token::Enum)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "枚举名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::LBrace)?;
        let mut variants = Vec::new();
        while !self.check(&Token::RBrace) {
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
        Ok(EnumDef { name, variants })
    }

    /// 解析函数定义（支持方法名 StructName.methodName）
    fn parse_function(&mut self) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;

        let name = match self.advance() {
            Some(Token::Ident(n)) => {
                if self.check(&Token::Dot) {
                    self.advance();
                    let method = match self.advance() {
                        Some(Token::Ident(m)) => m,
                        Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string())),
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    format!("{}.{}", n, method)
                } else {
                    n
                }
            }
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "标识符".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };

        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;

        let prev_receiver = self.receiver_name.clone();
        self.receiver_name = if name.contains('.') {
            params.first().map(|p| p.name.clone())
        } else {
            None
        };

        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(Token::LBrace)?;
        let body = self.parse_stmts()?;
        self.expect(Token::RBrace)?;

        self.receiver_name = prev_receiver;

        Ok(Function {
            name,
            params,
            return_type,
            body,
        })
    }

    /// 解析参数列表
    fn parse_params(&mut self) -> Result<Vec<Param>, ParseErrorAt> {
        let mut params = Vec::new();
        if self.check(&Token::RParen) {
            return Ok(params);
        }

        loop {
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string())),
                None => return self.bail(ParseError::UnexpectedEof),
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
            params.push(Param { name, ty, default, variadic });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(params)
    }

    /// 解析类型
    fn parse_type(&mut self) -> Result<Type, ParseErrorAt> {
        match self.advance() {
            Some(Token::TypeInt32) => Ok(Type::Int32),
            Some(Token::TypeInt64) => Ok(Type::Int64),
            Some(Token::TypeFloat64) => Ok(Type::Float64),
            Some(Token::TypeFloat32) => Ok(Type::Float32),
            Some(Token::TypeBool) => Ok(Type::Bool),
            Some(Token::TypeUnit) => Ok(Type::Unit),
            Some(Token::TypeString) => Ok(Type::String),
            Some(Token::TypeArray) => {
                self.expect(Token::Lt)?;
                let elem_type = self.parse_type()?;
                self.expect(Token::Gt)?;
                Ok(Type::Array(Box::new(elem_type)))
            }
            Some(Token::TypeRange) => Ok(Type::Range),
            Some(Token::Ident(name)) => Ok(Type::Struct(name)),
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok, "类型".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析语句列表
    fn parse_stmts(&mut self) -> Result<Vec<Stmt>, ParseErrorAt> {
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && self.peek().is_some() {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// 解析语句
    fn parse_stmt(&mut self) -> Result<Stmt, ParseErrorAt> {
        match self.peek() {
            Some(Token::Let) => {
                self.advance();
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
                self.expect(Token::Assign)?;
                let value = self.parse_expr()?;
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
                self.expect(Token::Assign)?;
                let value = self.parse_expr()?;
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
                if self.check(&Token::Let) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.expect(Token::Assign)?;
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
                let var = match self.advance() {
                    Some(Token::Ident(name)) => name,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "循环变量名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::In)?;
                // 使用受限的表达式解析，不允许解析结构体初始化 (因为 { 会被误认为 for body)
                let iterable = self.parse_for_iterable()?;
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
            _ => {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Assign,
                    "可赋值的目标".to_string(),
                ))
            }
        }
    }

    /// 解析表达式（顶层为逻辑或）
    fn parse_expr(&mut self) -> Result<Expr, ParseErrorAt> {
        self.parse_logical_or()
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

        while let Some(op) = match self.peek() {
            Some(Token::Eq) => Some(BinOp::Eq),
            Some(Token::NotEq) => Some(BinOp::NotEq),
            Some(Token::Lt) => {
                // 检查是否是泛型语法 Array<T>
                if matches!(self.peek_next(), Some(Token::TypeInt64 | Token::TypeInt32 | Token::TypeFloat64 | Token::TypeFloat32 | Token::TypeBool | Token::TypeString | Token::Ident(_))) {
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

    /// 解析移位 << >>
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
                    // 数组访问 arr[index]
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    expr = Expr::Index {
                        array: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                Some(Token::Dot) => {
                    // 字段访问或方法调用
                    self.advance();
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
                        let args = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        expr = Expr::MethodCall {
                            object: Box::new(expr),
                            method: name,
                            args,
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
                _ => break,
            }
        }

        Ok(expr)
    }

    /// 解析基础表达式
    fn parse_primary(&mut self) -> Result<Expr, ParseErrorAt> {
        match self.advance() {
            Some(Token::Integer(n)) => {
                // 检查是否是范围表达式
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    let end = self.parse_primary()?;
                    return Ok(Expr::Range {
                        start: Box::new(Expr::Integer(n)),
                        end: Box::new(end),
                        inclusive,
                    });
                }
                Ok(Expr::Integer(n))
            }
            Some(Token::Float(f)) => Ok(Expr::Float(f)),
            Some(Token::Float32(f)) => Ok(Expr::Float32(f)),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Ok(Expr::Var(n)),
                None => self.bail_at(
                    ParseError::UnexpectedToken(Token::This, "this 仅可在方法体内使用".to_string()),
                    self.at_prev(),
                ),
            },
            Some(Token::True) => Ok(Expr::Bool(true)),
            Some(Token::False) => Ok(Expr::Bool(false)),
            Some(Token::StringLit(s)) | Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => Ok(Expr::String(s)),
            Some(Token::Ident(name)) => {
                // 仅当首字母大写时解析为枚举变体 (Color.Red)，否则 . 后续为字段/方法
                let looks_like_type = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if looks_like_type && self.check(&Token::Dot) {
                    if let Some(Token::Ident(_)) = self.peek_next() {
                        self.advance();
                        let variant = match self.advance() {
                            Some(Token::Ident(v)) => v,
                            _ => unreachable!(),
                        };
                        let arg = if self.check(&Token::LParen) {
                            self.advance();
                            let e = self.parse_expr()?;
                            self.expect(Token::RParen)?;
                            Some(Box::new(e))
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
                // 检查是否是函数调用、构造函数调用或结构体初始化
                match self.peek() {
                    Some(Token::LParen) => {
                        self.advance();
                        let args = self.parse_args()?;
                        self.expect(Token::RParen)?;
                        // 首字母大写视为构造函数调用，否则为普通函数调用
                        if looks_like_type {
                            Ok(Expr::ConstructorCall { name, args })
                        } else {
                            Ok(Expr::Call { name, args })
                        }
                    }
                    Some(Token::LBrace) => {
                        self.advance();
                        let fields = self.parse_struct_fields()?;
                        self.expect(Token::RBrace)?;
                        Ok(Expr::StructInit { name, fields })
                    }
                    _ => Ok(Expr::Var(name)),
                }
            }
            Some(Token::LParen) => {
                // 检查是否是 Lambda: (x: T, ...) -> R { body } 或 () -> R { body }
                // 通过检查 ) -> 或 ident : 来判断
                if self.check(&Token::RParen) {
                    // () -> R { body }
                    self.advance(); // consume )
                    if self.check(&Token::Arrow) {
                        return self.parse_lambda_rest(vec![]);
                    }
                    // 空括号但没有 ->，解析错误
                    return self.bail(ParseError::UnexpectedToken(Token::RParen, "表达式".to_string()));
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
                // 普通括号表达式
                let expr = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(expr)
            }
            Some(Token::LBrace) => {
                // 检查是否是 Lambda: { x: T => body }
                if let Some(Token::Ident(_)) = self.peek() {
                    if let Some(Token::Colon) = self.peek_next() {
                        // 尝试解析 Lambda { x: T, y: T => body }
                        let params = self.parse_lambda_params()?;
                        self.expect(Token::FatArrow)?;
                        let body = self.parse_expr()?;
                        self.expect(Token::RBrace)?;
                        return Ok(Expr::Lambda {
                            params,
                            return_type: None, // 类型推断
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
                if self.check(&Token::Let) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    self.expect(Token::Assign)?;
                    // 使用受限表达式解析，避免 { 被误认为结构体初始化
                    let expr = Box::new(self.parse_match_subject()?);
                    self.expect(Token::LBrace)?;
                    let then_stmts = self.parse_stmts()?;
                    let then_expr = if then_stmts.is_empty() {
                        Box::new(Expr::Integer(0))
                    } else {
                        match then_stmts.last() {
                            Some(Stmt::Expr(e)) => Box::new(e.clone()),
                            _ => Box::new(Expr::Integer(0)),
                        }
                    };
                    self.expect(Token::RBrace)?;
                    let else_branch = if self.check(&Token::Else) {
                        self.advance();
                        self.expect(Token::LBrace)?;
                        let else_stmts = self.parse_stmts()?;
                        let else_expr = if else_stmts.is_empty() {
                            None
                        } else {
                            match else_stmts.last() {
                                Some(Stmt::Expr(e)) => Some(Box::new(e.clone())),
                                _ => None,
                            }
                        };
                        self.expect(Token::RBrace)?;
                        else_expr
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
                    let then_expr = if then_stmts.is_empty() {
                        None
                    } else {
                        match then_stmts.last() {
                            Some(Stmt::Expr(e)) => Some(Box::new(e.clone())),
                            _ => None,
                        }
                    };
                    self.expect(Token::RBrace)?;

                    let else_branch = if self.check(&Token::Else) {
                        self.advance();
                        self.expect(Token::LBrace)?;
                        let else_stmts = self.parse_stmts()?;
                        let else_expr = if else_stmts.is_empty() {
                            None
                        } else {
                            match else_stmts.last() {
                                Some(Stmt::Expr(e)) => Some(Box::new(e.clone())),
                                _ => None,
                            }
                        };
                        self.expect(Token::RBrace)?;
                        else_expr
                    } else {
                        None
                    };

                    Ok(Expr::If {
                        cond: Box::new(cond),
                        then_branch: then_expr.unwrap_or_else(|| Box::new(Expr::Integer(0))),
                        else_branch,
                    })
                }
            }
            Some(Token::Match) => {
                // 使用受限的表达式解析，不允许解析结构体初始化
                let expr = self.parse_match_subject()?;
                self.expect(Token::LBrace)?;
                let arms = self.parse_match_arms()?;
                self.expect(Token::RBrace)?;
                Ok(Expr::Match {
                    expr: Box::new(expr),
                    arms,
                })
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

    /// 解析 Lambda 表达式的剩余部分: -> ReturnType { body }
    fn parse_lambda_rest(&mut self, params: Vec<(String, Type)>) -> Result<Expr, ParseErrorAt> {
        self.expect(Token::Arrow)?;
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

    /// 解析函数调用参数
    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseErrorAt> {
        let mut args = Vec::new();
        if self.check(&Token::RParen) {
            return Ok(args);
        }

        loop {
            args.push(self.parse_expr()?);
            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        Ok(args)
    }

    /// 解析 match 的主题表达式 (不包括结构体初始化)
    fn parse_match_subject(&mut self) -> Result<Expr, ParseErrorAt> {
        // 只解析简单表达式: 变量、字面量、函数调用、字段访问等
        // 不解析结构体初始化 (因为 { 会被误认为 match body)
        let mut expr = match self.advance() {
            Some(Token::Integer(n)) => Expr::Integer(n),
            Some(Token::Float(f)) => Expr::Float(f),
            Some(Token::Float32(f)) => Expr::Float32(f),
            Some(Token::This) => match self.receiver_name.clone() {
                Some(n) => Expr::Var(n),
                None => return self.bail(ParseError::UnexpectedToken(Token::This, "this 仅可在方法体内使用".to_string())),
            },
            Some(Token::True) => Expr::Bool(true),
            Some(Token::False) => Expr::Bool(false),
            Some(Token::StringLit(s)) | Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => Expr::String(s),
            Some(Token::Ident(name)) => {
                if self.check(&Token::LParen) {
                    // 函数调用
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Expr::Call { name, args }
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
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(Token::RBracket)?;
                    expr = Expr::Index {
                        array: Box::new(expr),
                        index: Box::new(index),
                    };
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
                    Ok(Expr::Range {
                        start: Box::new(Expr::Integer(n)),
                        end: Box::new(end),
                        inclusive,
                    })
                } else {
                    Ok(Expr::Integer(n))
                }
            }
            Some(Token::Ident(name)) => {
                if self.check(&Token::LParen) {
                    // 函数调用
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call { name, args })
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
                    Ok(Expr::Range {
                        start: Box::new(Expr::Var(name)),
                        end: Box::new(end),
                        inclusive,
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
            Some(tok) => self.bail_at(ParseError::UnexpectedToken(tok, "for 循环可迭代表达式".to_string()), self.at_prev()),
            None => self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
        }
    }

    /// 解析 match 分支列表
    fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, ParseErrorAt> {
        let mut arms = Vec::new();

        while !self.check(&Token::RBrace) && self.peek().is_some() {
            let pattern = self.parse_pattern()?;

            // 可选的守卫条件
            let guard = if self.check(&Token::If) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };

            self.expect(Token::FatArrow)?;
            let body = Box::new(self.parse_expr()?);

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
            Some(Token::StringLit(s)) | Some(Token::RawStringLit(s)) | Some(Token::MultiLineStringLit(s)) => {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    #[test]
    fn test_parse_function() {
        let source = "func add(a: Int64, b: Int64) -> Int64 { return a + b }";
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
        let source = "func test() -> Int64 { let arr = [1, 2, 3] return arr[0] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_loop() {
        let source = "func test() -> Int64 { var sum: Int64 = 0 for i in 0..10 { sum = sum + i } return sum }";
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
        let source = "func test(n: Int64) -> Int64 { match n { 0 => 100, 1 => 200, _ => 999 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_or_pattern() {
        let source = "func test(n: Int64) -> Int64 { match n { 1 | 2 | 3 => 10, _ => 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_range_pattern() {
        let source = "func test(n: Int64) -> Int64 { match n { 0..10 => 1, 10..100 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_if_else() {
        let source = "func test(x: Int64) -> Int64 { if x > 0 { return 1 } else { return 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].body.is_empty());
    }

    #[test]
    fn test_parse_while() {
        let source = "func test() -> Int64 { var n: Int64 = 0 while n < 10 { n = n + 1 } return n }";
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
            func test() -> Int64 {
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
        let source = "func test(n: Int64) -> Int64 { match n { x if x < 0 => 1, 0 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_in_array() {
        let source = "func test() -> Int64 { let arr = [1, 2, 3] var s: Int64 = 0 for x in arr { s = s + x } return s }";
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
        let source = "func test() -> Int64 { var s: Int64 = 0 for i in 1..=5 { s = s + i } return s }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_logical_ops() {
        let source = "func test() -> Int64 { if true && false || !true { return 0 } return 1 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_unary_neg() {
        let source = "func test() -> Int64 { let x = -1 let y = -(-2) return x + y }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_block_expr() {
        let source = "func test() -> Int64 { let x = { let a = 1 let b = 2 a + b } return x }";
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
        // Lambda syntax: (x: T) -> R { body }
        let source = "func test() { let f = (x: Int64) -> Int64 { x * 2 } }";
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
}
