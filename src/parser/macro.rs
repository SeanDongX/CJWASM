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

        // 3. 解析参数列表 (可选)
        let args = if self.check(&Token::LParen) {
            self.advance(); // 消费 (

            let mut args = Vec::new();

            // 解析参数列表
            if !self.check(&Token::RParen) {
                loop {
                    args.push(self.parse_expr()?);

                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance(); // 消费 ,
                }
            }

            self.expect(Token::RParen)?;
            args
        } else {
            Vec::new()
        };

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
}
