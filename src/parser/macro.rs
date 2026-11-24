use crate::ast::Expr;
use crate::lexer::Token;
use crate::parser::{ParseError, ParseErrorAt, Parser};

impl Parser {
    /// 解析宏调用: @MacroName 或 @MacroName(args)
    ///
    /// 语法:
    /// - @MacroName
    /// - @MacroName(arg1, arg2, ...)
    ///
    /// 示例:
    /// - @Assert(a, b)
    /// - @Expect(result, 42)
    /// - @Deprecated
    /// - @sourceFile
    /// 解析宏调用的参数列表（当前 token 必须是 `(`），消费到匹配的 `)`。
    ///
    /// 容忍 unittest 宏 DSL 中的几种实参形态（不做语义处理）：
    /// - `@Assert[validator](cond)`
    /// - `@Assert(a, b, delta: 0.1)` — 命名/键值实参
    /// - `@Called(obj.field = _)` — 赋值型实参
    pub(crate) fn parse_macro_args(&mut self) -> Result<Vec<Expr>, ParseErrorAt> {
        self.expect(Token::LParen)?;
        // unittest mock DSL 中 `_` 可作为实参通配符（如 @On(a.intOpt(_))），
        // 仅在解析宏实参期间允许。
        let prev_wildcard = self.allow_wildcard_expr;
        self.allow_wildcard_expr = true;
        let result = self.parse_macro_args_inner();
        self.allow_wildcard_expr = prev_wildcard;
        result
    }

    fn parse_macro_args_inner(&mut self) -> Result<Vec<Expr>, ParseErrorAt> {
        let mut args = Vec::new();
        if !self.check(&Token::RParen) {
            loop {
                // 命名实参 `name: value`（如 delta: 0.1）——只保留 value
                if matches!(self.peek(), Some(Token::Ident(_)))
                    && matches!(self.peek_at(1), Some(Token::Colon))
                {
                    self.advance(); // name
                    self.advance(); // :
                }
                args.push(self.parse_macro_arg_value()?);
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance(); // ,
                // 允许尾随逗号 @Assert(a, b,)
                if self.check(&Token::RParen) {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        Ok(args)
    }

    /// 解析单个宏实参，容忍 `_` 通配符和 `lhs = rhs` 赋值型实参。
    fn parse_macro_arg_value(&mut self) -> Result<Expr, ParseErrorAt> {
        if self.check(&Token::Underscore) {
            self.advance();
            return Ok(Expr::Var("_".to_string()));
        }
        let lhs = self.parse_expr()?;
        if self.check(&Token::Assign) {
            self.advance();
            if self.check(&Token::Underscore) {
                self.advance();
                return Ok(Expr::Var("_".to_string()));
            }
            return self.parse_expr();
        }
        Ok(lhs)
    }

    pub(crate) fn parse_macro_call(&mut self) -> Result<Expr, ParseErrorAt> {
        // 1. 期望 @ 符号
        self.expect(Token::At)?;

        // 2. 解析宏名称
        let name = match self.advance() {
            Some(Token::Ident(s)) => s,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "macro name".to_string(),
                ));
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };

        // 2b. 可选的宏属性/类型实参列表：@Assert[validator]、@AssertThrows[Exception]。
        //     整体平衡跳过，不做展开。
        if self.check(&Token::LBracket) {
            self.skip_balanced_mixed()?;
        }

        // 3. 解析参数列表 (可选)
        let mut args = if self.check(&Token::LParen) {
            self.parse_macro_args()?
        } else {
            Vec::new()
        };

        // 3b. 宏的尾随块参数：@EnsurePreparedToMock { => ... }
        //     （lambda 形式的块，作为宏实参接纳，不做展开）
        if self.check(&Token::LBrace) && self.brace_starts_lambda() {
            args.push(self.parse_primary()?);
        }

        Ok(Expr::Macro { name, args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Token;
    use logos::Logos;

    fn tokenize(source: &str) -> Vec<(usize, Token, usize)> {
        Token::lexer(source)
            .spanned()
            .filter_map(|(tok, span)| tok.ok().map(|t| (span.start, t, span.end)))
            .collect()
    }

    #[test]
    fn test_parse_macro_no_args() {
        let source = "@Deprecated";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Deprecated");
                assert_eq!(args.len(), 0);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_macro_with_args() {
        let source = "@Assert(a, b)";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Assert");
                assert_eq!(args.len(), 2);

                // 检查参数
                match &args[0] {
                    Expr::Var(v) => assert_eq!(v, "a"),
                    _ => panic!("Expected variable 'a'"),
                }
                match &args[1] {
                    Expr::Var(v) => assert_eq!(v, "b"),
                    _ => panic!("Expected variable 'b'"),
                }
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_macro_with_complex_args() {
        let source = "@Expect(x + y, 42)";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Expect");
                assert_eq!(args.len(), 2);

                // 第一个参数是二元表达式
                match &args[0] {
                    Expr::Binary { .. } => {},
                    _ => panic!("Expected binary expression"),
                }

                // 第二个参数是整数字面量
                match &args[1] {
                    Expr::Integer(42) => {},
                    _ => panic!("Expected integer 42"),
                }
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    #[test]
    fn test_parse_source_file_macro() {
        let source = "@sourceFile";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "sourceFile");
                assert_eq!(args.len(), 0);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    /// @Assert[validator](args) — 前缀方括号属性列表应被平衡跳过。
    #[test]
    fn test_parse_macro_with_bracket_attribute() {
        let source = "@Assert[checkNotNone](a)";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Assert");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    /// @Assert(a, b, delta: 0.1) — 命名/键值实参应被接纳（只保留 value）。
    #[test]
    fn test_parse_macro_with_named_arg() {
        let source = "@Assert(a, b, delta: 0.1)";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Assert");
                assert_eq!(args.len(), 3);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    /// @Called(obj.field = _) — 赋值型实参（mock DSL）应被接纳。
    #[test]
    fn test_parse_macro_with_assignment_arg() {
        let source = "@Called(e.varProp = _)";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "Called");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }

    /// @On(a.intOpt(_)) — 嵌套调用实参中的 `_` 通配符应被接纳。
    #[test]
    fn test_parse_macro_with_nested_wildcard_arg() {
        let source = "@On(a.intOpt(_))";
        let tokens = tokenize(source);
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().unwrap();

        match expr {
            Expr::Macro { name, args } => {
                assert_eq!(name, "On");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected macro call, got {:?}", expr),
        }
    }
}
