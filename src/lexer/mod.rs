use logos::Logos;

/// 将字符串字面量内部的反斜杠转义还原（支持 \n \t \" \\）。
fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 处理多行字符串：移除首行空行，并 strip 公共缩进
fn process_multiline_string(s: &str) -> String {
    let mut lines: Vec<&str> = s.lines().collect();

    // 如果第一行是空的（紧跟 """ 后换行），移除它
    if !lines.is_empty() && lines[0].trim().is_empty() {
        lines.remove(0);
    }
    // 如果最后一行只有空白（""" 前的缩进），移除它
    if !lines.is_empty() && lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }

    if lines.is_empty() {
        return String::new();
    }

    // 计算公共缩进（最小非空行的前导空白数）
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    // 移除公共缩进并拼接
    lines
        .iter()
        .map(|l| {
            if l.len() >= min_indent {
                &l[min_indent..]
            } else {
                l.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 解析多行字符串 """..."""，返回内容（不含引号）和消耗的字节数
fn lex_multiline_string(lex: &mut logos::Lexer<Token>) -> Option<String> {
    let remainder = lex.remainder();
    // 查找结束的 """
    if let Some(end_pos) = remainder.find("\"\"\"") {
        let content = &remainder[..end_pos];
        lex.bump(end_pos + 3); // 跳过内容和结束的 """
        Some(process_multiline_string(content))
    } else {
        None // 未找到结束引号
    }
}

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
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("loop")]
    Loop,
    #[token("struct")]
    Struct,
    #[token("enum")]
    Enum,

    #[token("as")]
    As,

    #[token("this")]
    This,

    #[token("_", priority = 3)]
    Underscore,

    // 类型
    #[token("Int64")]
    TypeInt64,
    #[token("Int32")]
    TypeInt32,
    #[token("Float64")]
    TypeFloat64,
    #[token("Float32")]
    TypeFloat32,
    #[token("Bool")]
    TypeBool,
    #[token("Unit")]
    TypeUnit,
    #[token("String")]
    TypeString,
    #[token("Array")]
    TypeArray,
    #[token("Range")]
    TypeRange,

    // 字面量（Float64：小数或科学计数法；Float32 后缀 f；整型）
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?|[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*", |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        s.parse::<f64>().ok()
    })]
    Float(f64),

    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*f|[0-9][0-9_]*f", |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_' && *c != 'f').collect();
        s.parse::<f32>().ok()
    })]
    Float32(f32),

    #[regex(r"0x[0-9a-fA-F][0-9a-fA-F_]*|0o[0-7][0-7_]*|0b[01][01_]*|[0-9][0-9_]*", |lex| {
        let slice = lex.slice();
        let s: String = slice.chars().filter(|c| *c != '_').collect();
        if slice.starts_with("0x") { i64::from_str_radix(&s[2..], 16).ok() }
        else if slice.starts_with("0o") { i64::from_str_radix(&s[2..], 8).ok() }
        else if slice.starts_with("0b") { i64::from_str_radix(&s[2..], 2).ok() }
        else { s.parse::<i64>().ok() }
    })]
    Integer(i64),

    #[token("true")]
    True,
    #[token("false")]
    False,

    // 多行字符串 """..."""（strip 公共缩进）
    #[token(r#"""""#, lex_multiline_string)]
    MultiLineStringLit(String),

    // 原始字符串 r"..."（不处理转义）
    #[regex(r#"r"([^"]*)""#, |lex| {
        let s = lex.slice();
        s[2..s.len() - 1].to_string()
    })]
    RawStringLit(String),

    // 字符串字面量（支持 \n \t \" \\ 转义）
    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        unescape_string(&s[1..s.len() - 1])
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
    #[token("**")]
    StarStar,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("=")]
    Assign,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("==")]
    Eq,
    #[token("!=")]
    NotEq,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("&")]
    And,
    #[token("|")]
    Pipe,
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,
    #[token("!")]
    Bang,
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
    #[token("...")]
    DotDotDot,
    #[token("=>")]
    FatArrow,

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
    fn test_string_escape() {
        let source = r#"let s = "a\nb\tc\"d\\e""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(
            tokens[3],
            Token::StringLit("a\nb\tc\"d\\e".to_string())
        );
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

    #[test]
    fn test_float_literal() {
        let source = "let x = 3.14";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::Float(3.14));
    }

    #[test]
    fn test_bool_literal() {
        let source = "let a = true let b = false";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::True);
        assert_eq!(tokens[7], Token::False);
    }

    #[test]
    fn test_comments_skipped() {
        let source = "func // 注释\n add(a: Int64, b: Int64) -> Int64 { return a + b }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[0], Token::Func);
        assert_eq!(tokens[1], Token::Ident("add".to_string()));
    }

    #[test]
    fn test_comparison_ops() {
        let source = "a == b != c < d > e <= f >= g";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert!(matches!(tokens[1], Token::Eq));
        assert!(matches!(tokens[3], Token::NotEq));
        assert!(matches!(tokens[5], Token::Lt));
        assert!(matches!(tokens[7], Token::Gt));
        assert!(matches!(tokens[9], Token::LtEq));
        assert!(matches!(tokens[11], Token::GtEq));
    }

    #[test]
    fn test_range_operators() {
        let source = "0..10 0..=10";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[1], Token::DotDot);
        assert_eq!(tokens[4], Token::DotDotEq);
    }

    #[test]
    fn test_match_tokens() {
        let source = "match n { 1 => 2, _ => 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[0], Token::Match);
        assert_eq!(tokens[4], Token::FatArrow);
        assert_eq!(tokens[7], Token::Underscore);
    }

    #[test]
    fn test_multiline_string() {
        let source = r#"let s = """
    hello
    world
    """"#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::MultiLineStringLit("hello\nworld".to_string()));
    }

    #[test]
    fn test_multiline_string_inline() {
        let source = r#"let s = """hello world""""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[3], Token::MultiLineStringLit("hello world".to_string()));
    }
}
