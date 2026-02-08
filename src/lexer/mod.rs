use logos::Logos;

#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(skip r"[ \t\r\n]+")]  // 跳过空白
#[logos(skip r"//[^\n]*")]    // 跳过单行注释
pub enum Token {
    // 关键字
    #[token("func")]
    Func,
    #[token("let")]
    Let,
    #[token("var")]
    Var,
    #[token("return")]
    Return,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("while")]
    While,
    #[token("for")]
    For,
    #[token("in")]
    In,
    #[token("match")]
    Match,
    #[token("struct")]
    Struct,
    #[token("_", priority = 3)]
    Underscore,

    // 类型
    #[token("Int64")]
    TypeInt64,
    #[token("Int32")]
    TypeInt32,
    #[token("Float64")]
    TypeFloat64,
    #[token("Bool")]
    TypeBool,
    #[token("Unit")]
    TypeUnit,
    #[token("String")]
    TypeString,
    #[token("Array")]
    TypeArray,

    // 字面量
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().ok())]
    Integer(i64),

    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>().ok())]
    Float(f64),

    #[token("true")]
    True,
    #[token("false")]
    False,

    // 字符串字面量
    #[regex(r#""[^"]*""#, |lex| {
        let s = lex.slice();
        s[1..s.len()-1].to_string()
    })]
    StringLit(String),

    // 标识符
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // 运算符
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("=")]
    Assign,
    #[token("==")]
    Eq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token(".")]
    Dot,
    #[token("..")]
    DotDot,
    #[token("..=")]
    DotDotEq,
    #[token("=>")]
    FatArrow,
    #[token("|")]
    Pipe,

    // 分隔符
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token("->")]
    Arrow,
}

pub struct Lexer<'a> {
    inner: logos::Lexer<'a, Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            inner: Token::lexer(source),
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<(usize, Token, usize), String>;

    fn next(&mut self) -> Option<Self::Item> {
        let token = self.inner.next()?;
        let span = self.inner.span();
        match token {
            Ok(tok) => Some(Ok((span.start, tok, span.end))),
            Err(_) => Some(Err(format!(
                "未知字符: '{}'",
                &self.inner.source()[span.start..span.end]
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let source = "func add(a: Int64, b: Int64) -> Int64 { return a + b }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[0], Token::Func);
        assert_eq!(tokens[1], Token::Ident("add".to_string()));
    }

    #[test]
    fn test_string_literal() {
        let source = r#"let s = "hello world""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::StringLit("hello world".to_string()));
    }

    #[test]
    fn test_array_tokens() {
        let source = "let arr: Array<Int64> = [1, 2, 3]";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::TypeArray);
        assert_eq!(tokens[4], Token::Lt);
        assert_eq!(tokens[5], Token::TypeInt64);
        assert_eq!(tokens[6], Token::Gt);
    }

    #[test]
    fn test_struct_tokens() {
        let source = "struct Point { x: Int64, y: Int64 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[0], Token::Struct);
        assert_eq!(tokens[1], Token::Ident("Point".to_string()));
    }
}
