use logos::Logos;

/// 字符串插值的部分
#[derive(Debug, PartialEq, Clone)]
pub enum StringPart {
    /// 字面量文本部分
    Literal(String),
    /// 插值表达式（原始文本，待 parser 解析）
    Interpolation(String),
}

/// 将字符串字面量内部的反斜杠转义还原（支持 \n \t \" \\）。
pub fn unescape_string(s: &str) -> String {
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
pub fn process_multiline_string(s: &str) -> String {
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

/// 解析字符串（普通或插值）
/// 返回 Ok(Left(String)) 为普通字符串，Ok(Right(Vec<StringPart>)) 为插值字符串
fn lex_string(lex: &mut logos::Lexer<Token>) -> Option<Result<String, Vec<StringPart>>> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut pos = 0;
    let mut parts: Vec<StringPart> = Vec::new();
    let mut current_literal = String::new();
    let mut has_interpolation = false;

    while pos < bytes.len() {
        let c = bytes[pos];

        if c == b'\\' && pos + 1 < bytes.len() {
            // 转义字符
            let next = bytes[pos + 1];
            match next {
                b'n' => current_literal.push('\n'),
                b't' => current_literal.push('\t'),
                b'"' => current_literal.push('"'),
                b'\\' => current_literal.push('\\'),
                b'$' => current_literal.push('$'), // 支持 \$ 转义
                _ => {
                    current_literal.push('\\');
                    current_literal.push(next as char);
                }
            }
            pos += 2;
        } else if c == b'$' && pos + 1 < bytes.len() && bytes[pos + 1] == b'{' {
            // 插值开始 ${
            has_interpolation = true;
            if !current_literal.is_empty() {
                parts.push(StringPart::Literal(current_literal.clone()));
                current_literal.clear();
            }
            pos += 2; // 跳过 ${

            // 查找匹配的 }，需要处理嵌套大括号
            let mut brace_depth = 1;
            let expr_start = pos;
            while pos < bytes.len() && brace_depth > 0 {
                match bytes[pos] {
                    b'{' => brace_depth += 1,
                    b'}' => brace_depth -= 1,
                    b'"' => {
                        // 跳过内嵌字符串
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != b'"' {
                            if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                                pos += 2;
                            } else {
                                pos += 1;
                            }
                        }
                    }
                    _ => {}
                }
                if brace_depth > 0 {
                    pos += 1;
                }
            }

            if brace_depth != 0 {
                return None; // 未闭合的 ${
            }

            let expr_text = std::str::from_utf8(&bytes[expr_start..pos]).ok()?;
            parts.push(StringPart::Interpolation(expr_text.to_string()));
            pos += 1; // 跳过 }
        } else if c == b'"' {
            // 字符串结束
            if has_interpolation {
                if !current_literal.is_empty() {
                    parts.push(StringPart::Literal(current_literal));
                }
                lex.bump(pos + 1); // +1 跳过结束引号
                return Some(Err(parts));
            } else {
                lex.bump(pos + 1);
                return Some(Ok(current_literal));
            }
        } else {
            current_literal.push(c as char);
            pos += 1;
        }
    }

    None // 未找到结束引号
}

/// 解析反引号字符串 `...`（支持 ${expr} 插值与 \` \n \t \\ 转义）
/// remainder 为反引号之后的内容（开头的 ` 已由 token 消耗）
fn lex_backtick_string(lex: &mut logos::Lexer<Token>) -> Option<Result<String, Vec<StringPart>>> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut pos = 0;
    let mut parts: Vec<StringPart> = Vec::new();
    let mut current_literal = String::new();
    let mut has_interpolation = false;

    while pos < bytes.len() {
        let c = bytes[pos];

        if c == b'\\' && pos + 1 < bytes.len() {
            let next = bytes[pos + 1];
            match next {
                b'n' => current_literal.push('\n'),
                b't' => current_literal.push('\t'),
                b'`' => current_literal.push('`'),
                b'\\' => current_literal.push('\\'),
                b'$' => current_literal.push('$'),
                _ => {
                    current_literal.push('\\');
                    current_literal.push(next as char);
                }
            }
            pos += 2;
        } else if c == b'$' && pos + 1 < bytes.len() && bytes[pos + 1] == b'{' {
            has_interpolation = true;
            if !current_literal.is_empty() {
                parts.push(StringPart::Literal(current_literal.clone()));
                current_literal.clear();
            }
            pos += 2;
            let mut brace_depth = 1;
            let expr_start = pos;
            while pos < bytes.len() && brace_depth > 0 {
                match bytes[pos] {
                    b'{' => brace_depth += 1,
                    b'}' => brace_depth -= 1,
                    b'"' => {
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != b'"' {
                            if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                                pos += 2;
                            } else {
                                pos += 1;
                            }
                        }
                    }
                    b'`' => {
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != b'`' {
                            if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                                pos += 2;
                            } else {
                                pos += 1;
                            }
                        }
                    }
                    _ => {}
                }
                if brace_depth > 0 {
                    pos += 1;
                }
            }
            if brace_depth != 0 {
                return None;
            }
            let expr_text = std::str::from_utf8(&bytes[expr_start..pos]).ok()?;
            parts.push(StringPart::Interpolation(expr_text.to_string()));
            pos += 1;
        } else if c == b'`' {
            if has_interpolation {
                if !current_literal.is_empty() {
                    parts.push(StringPart::Literal(current_literal));
                }
                lex.bump(pos + 1);
                return Some(Err(parts));
            } else {
                lex.bump(pos + 1);
                return Some(Ok(current_literal));
            }
        } else {
            current_literal.push(c as char);
            pos += 1;
        }
    }
    None
}

/// 词法分析入口：反引号字符串
fn lex_any_backtick_string(lex: &mut logos::Lexer<Token>) -> logos::FilterResult<StringOrInterpolated, ()> {
    match lex_backtick_string(lex) {
        Some(Ok(s)) => logos::FilterResult::Emit(StringOrInterpolated::Plain(s)),
        Some(Err(parts)) => logos::FilterResult::Emit(StringOrInterpolated::Interpolated(parts)),
        None => logos::FilterResult::Error(()),
    }
}

/// 词法分析入口：解析字符串（普通或插值）
fn lex_any_string(lex: &mut logos::Lexer<Token>) -> logos::FilterResult<StringOrInterpolated, ()> {
    match lex_string(lex) {
        Some(Ok(s)) => logos::FilterResult::Emit(StringOrInterpolated::Plain(s)),
        Some(Err(parts)) => logos::FilterResult::Emit(StringOrInterpolated::Interpolated(parts)),
        None => logos::FilterResult::Error(()),
    }
}

/// 用于区分普通字符串和插值字符串的类型
#[derive(Debug, Clone, PartialEq)]
pub enum StringOrInterpolated {
    Plain(String),
    Interpolated(Vec<StringPart>),
}

#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(skip r"[ \t\r\n]+")]  // 跳过空白
#[logos(skip r"//[^\n]*")]    // 跳过单行注释
pub enum Token {
    // 块注释 /* ... */（在 next() 中过滤不发射，与 vendor 版权头兼容）
    #[regex(r"/\*(?:[^*]|\*+[^*/])*\*/")]
    BlockComment,
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
    #[token("class")]
    Class,
    #[token("abstract")]
    Abstract,
    #[token("sealed")]
    Sealed,
    #[token("open")]
    Open,
    #[token("interface")]
    Interface,
    #[token("extend")]
    Extend,
    #[token("init")]
    Init,
    #[token("override")]
    Override,
    #[token("super")]
    Super,
    #[token("prop")]
    Prop,
    #[token("mut")]
    Mut,

    // 包/模块系统关键字 (cjc: package)
    #[token("package")]
    Package,
    #[token("import")]
    Import,
    #[token("public")]
    Public,
    #[token("private")]
    Private,
    #[token("protected")]
    Protected,
    #[token("internal")]
    Internal,

    #[token("as")]
    As,
    #[token("is")]
    Is,

    #[token("this")]
    This,

    // 错误处理关键字
    #[token("try")]
    Try,
    #[token("catch")]
    Catch,
    #[token("throw")]
    Throw,
    #[token("finally")]
    Finally,
    #[token("foreign")]
    Foreign,
    #[token("@")]
    At,

    // Option/Result 关键字 (cjwasm 扩展, cjc 中为普通标识符)
    #[token("Some")]
    Some,
    #[token("None")]
    None,
    #[token("Ok")]
    Ok,
    #[token("Err")]
    Err,

    #[token("_", priority = 3)]
    Underscore,

    // cjc 额外关键字
    #[token("const")]
    Const,
    #[token("static")]
    Static,
    #[token("redef")]
    Redef,
    #[token("operator")]
    Operator,
    #[token("unsafe")]
    Unsafe,
    #[token("do")]
    Do,
    #[token("case")]
    Case,
    #[token("where")]
    Where,
    #[token("type")]
    TypeAlias,
    #[token("main")]
    Main,
    #[token("spawn")]
    Spawn,
    #[token("synchronized")]
    Synchronized,
    #[token("macro")]
    Macro,
    #[token("quote")]
    Quote,
    #[token("inout")]
    Inout,
    #[token("with")]
    With,

    // 类型 (与 cjc release/1.0 对齐)
    #[token("Int8")]
    TypeInt8,
    #[token("Int16")]
    TypeInt16,
    #[token("Int32")]
    TypeInt32,
    #[token("Int64")]
    TypeInt64,
    #[token("IntNative")]
    TypeIntNative,
    #[token("UInt8")]
    TypeUInt8,
    #[token("UInt16")]
    TypeUInt16,
    #[token("UInt32")]
    TypeUInt32,
    #[token("UInt64")]
    TypeUInt64,
    #[token("UIntNative")]
    TypeUIntNative,
    #[token("Float16")]
    TypeFloat16,
    #[token("Float32")]
    TypeFloat32,
    #[token("Float64")]
    TypeFloat64,
    #[token("Rune")]
    TypeRune,
    #[token("Bool")]
    TypeBool,
    #[token("Nothing")]
    TypeNothing,
    #[token("Unit")]
    TypeUnit,
    #[token("VArray")]
    TypeVArray,
    #[token("This", priority = 3)]
    TypeThis,
    // cjwasm 扩展类型关键字 (cjc 中为标准库标识符)
    #[token("String")]
    TypeString,
    #[token("Array")]
    TypeArray,
    #[token("Tuple")]
    TypeTuple,
    #[token("Range")]
    TypeRange,
    #[token("Option")]
    TypeOption,
    #[token("Result")]
    TypeResult,
    #[token("Slice")]
    TypeSlice,
    #[token("Map")]
    TypeMap,

    // 字面量（Float64：小数或科学计数法；Float32 后缀 f；整型）
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?|[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*", |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        s.parse::<f64>().ok()
    })]
    Float(f64),

    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?f|[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*f|[0-9][0-9_]*f", priority = 3, callback = |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_' && *c != 'f').collect();
        s.parse::<f32>().ok()
    })]
    Float32(f32),

    #[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*|0o[0-7][0-7_]*|0b[01][01_]*|[0-9][0-9_]*", |lex| {
        let slice = lex.slice();
        let s: String = slice.chars().filter(|c| *c != '_').collect();
        if slice.len() >= 2 && (slice.starts_with("0x") || slice.starts_with("0X")) {
            i64::from_str_radix(&s[2..], 16).ok()
        } else if slice.starts_with("0o") { i64::from_str_radix(&s[2..], 8).ok() }
        else if slice.starts_with("0b") { i64::from_str_radix(&s[2..], 2).ok() }
        else { s.parse::<i64>().ok() }
    })]
    Integer(i64),

    // 字符字面量 'a' (支持转义 '\n' '\t' '\\' '\'' '\0')
    #[regex(r"'[^'\\]'|'\\[ntr\\0']'", |lex| {
        let s = lex.slice();
        let inner = &s[1..s.len()-1]; // 去除引号
        if inner.starts_with('\\') {
            match inner.chars().nth(1) {
                Some('n') => Some('\n'),
                Some('t') => Some('\t'),
                Some('r') => Some('\r'),
                Some('\\') => Some('\\'),
                Some('0') => Some('\0'),
                Some('\'') => Some('\''),
                _ => None,
            }
        } else {
            inner.chars().next()
        }
    })]
    CharLit(char),

    // Rune 字面量 r'.' (仓颉语法，支持转义)
    #[regex(r"r'[^'\\]'|r'\\[ntr\\0']'", |lex| {
        let s = lex.slice();
        let inner = &s[2..s.len()-1]; // 去除 r' 和 '
        if inner.starts_with('\\') {
            match inner.chars().nth(1) {
                Some('n') => Some('\n'),
                Some('t') => Some('\t'),
                Some('r') => Some('\r'),
                Some('\\') => Some('\\'),
                Some('0') => Some('\0'),
                Some('\'') => Some('\''),
                _ => None,
            }
        } else {
            inner.chars().next()
        }
    })]
    RuneLit(char),

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

    // 字符串字面量（支持 \n \t \" \\ 转义，以及 ${expr} 插值）
    #[token("\"", lex_any_string)]
    StringLit(StringOrInterpolated),

    // 反引号字符串 `...`（支持 ${expr} 插值与 \` \n \t \\ 转义，与仓颉 vendor 兼容）
    #[token("`", lex_any_backtick_string)]
    BacktickStringLit(StringOrInterpolated),

    // 标识符
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // 运算符 (与 cjc release/1.0 对齐)
    #[token("**")]
    StarStar,
    #[token("*")]
    Star,
    #[token("%")]
    Percent,
    #[token("/")]
    Slash,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("++")]
    Incr,
    #[token("--")]
    Decr,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("??")]
    QuestionQuestion,
    #[token("|>")]
    Pipeline,
    #[token("~>")]
    Composition,
    #[token("!")]
    Bang,
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
    #[token("==")]
    Eq,
    #[token("!=")]
    NotEq,
    #[token("=")]
    Assign,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("**=")]
    StarStarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("&&=")]
    AndAndEq,
    #[token("||=")]
    OrOrEq,
    #[token("&=")]
    AndEq,
    #[token("|=")]
    PipeEq,
    #[token("^=")]
    CaretEq,
    #[token("<<=")]
    ShlEq,
    #[token(">>=")]
    ShrEq,
    #[token("->")]
    Arrow,
    #[token("<-")]
    LeftArrow,
    #[token("=>")]
    FatArrow,
    #[token("..")]
    DotDot,
    #[token("..=")]
    DotDotEq,
    #[token("...")]
    DotDotDot,
    #[token("#")]
    Hash,
    #[token("@!")]
    AtExcl,
    #[token("?")]
    Question,
    #[token("<:")]
    SubType,
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
    #[token(";")]
    Semicolon,
    #[token("$")]
    Dollar,
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
        loop {
            let token = self.inner.next()?;
            let span = self.inner.span();
            match token {
                Ok(Token::BlockComment) => continue,  // 跳过块注释
                Ok(tok) => return Some(Ok((span.start, tok, span.end))),
                Err(_) => return Some(Err(format!(
                    "未知字符: '{}'",
                    &self.inner.source()[span.start..span.end]
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let source = "func add(a: Int64, b: Int64): Int64 { return a + b }";
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

        assert_eq!(tokens[3], Token::StringLit(StringOrInterpolated::Plain("hello world".to_string())));
    }

    #[test]
    fn test_string_escape() {
        let source = r#"let s = "a\nb\tc\"d\\e""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(
            tokens[3],
            Token::StringLit(StringOrInterpolated::Plain("a\nb\tc\"d\\e".to_string()))
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
        let source = "func // 注释\n add(a: Int64, b: Int64): Int64 { return a + b }";
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

    #[test]
    fn test_string_interpolation() {
        let source = r#"let s = "Hello, ${name}!""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        match &tokens[3] {
            Token::StringLit(StringOrInterpolated::Interpolated(parts)) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], StringPart::Literal("Hello, ".to_string()));
                assert_eq!(parts[1], StringPart::Interpolation("name".to_string()));
                assert_eq!(parts[2], StringPart::Literal("!".to_string()));
            }
            _ => panic!("Expected interpolated string"),
        }
    }

    #[test]
    fn test_string_interpolation_escape() {
        let source = r#"let s = "Price: \$100""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        // \$ should be escaped to $, so this is a plain string
        assert_eq!(tokens[3], Token::StringLit(StringOrInterpolated::Plain("Price: $100".to_string())));
    }

    #[test]
    fn test_unescape_string_all_escapes() {
        // 直接测试 unescape_string
        assert_eq!(super::unescape_string("hello\\nworld"), "hello\nworld");
        assert_eq!(super::unescape_string("tab\\there"), "tab\there");
        assert_eq!(super::unescape_string("quote\\\"inside"), "quote\"inside");
        assert_eq!(super::unescape_string("back\\\\slash"), "back\\slash");
    }

    #[test]
    fn test_unescape_unknown_sequence() {
        // 未知转义如 \x => 保留 \x
        assert_eq!(super::unescape_string("test\\xend"), "test\\xend");
    }

    #[test]
    fn test_unescape_trailing_backslash() {
        // 尾部 \ 应保留
        assert_eq!(super::unescape_string("end\\"), "end\\");
    }

    #[test]
    fn test_unescape_no_escape() {
        assert_eq!(super::unescape_string("hello world"), "hello world");
    }

    #[test]
    fn test_unescape_empty() {
        assert_eq!(super::unescape_string(""), "");
    }

    #[test]
    fn test_process_multiline_string_basic() {
        let s = "\n    hello\n    world\n    ";
        let result = super::process_multiline_string(s);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_process_multiline_string_empty() {
        assert_eq!(super::process_multiline_string(""), "");
    }

    #[test]
    fn test_process_multiline_string_no_indent() {
        let s = "\nhello\nworld\n";
        let result = super::process_multiline_string(s);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_string_interpolation_with_nested_braces() {
        // ${expr} 中包含嵌套 {} 的情况
        let source = r#"let s = "value: ${if true { 1 } else { 0 }}""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        // 应该能正常解析
        assert!(tokens.len() >= 4);
    }

    #[test]
    fn test_string_interpolation_with_embedded_string() {
        // ${expr} 中包含内嵌字符串
        let source = r#"let s = "hello ${name + "!"} world""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert!(tokens.len() >= 4);
    }

    #[test]
    fn test_lexer_error_handling_tokens() {
        // 确保 try, catch, throw, finally tokens 正确识别
        let source = "try catch throw finally";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Try);
        assert_eq!(tokens[1], Token::Catch);
        assert_eq!(tokens[2], Token::Throw);
        assert_eq!(tokens[3], Token::Finally);
    }

    #[test]
    fn test_lexer_type_tokens() {
        let source = "Int8 Int16 Int32 Int64 UInt8 UInt16 UInt32 UInt64 Float32 Float64 Bool Rune String Unit";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::TypeInt8);
        assert_eq!(tokens[1], Token::TypeInt16);
        assert_eq!(tokens[2], Token::TypeInt32);
        assert_eq!(tokens[3], Token::TypeInt64);
        assert_eq!(tokens[4], Token::TypeUInt8);
        assert_eq!(tokens[5], Token::TypeUInt16);
        assert_eq!(tokens[6], Token::TypeUInt32);
        assert_eq!(tokens[7], Token::TypeUInt64);
        assert_eq!(tokens[8], Token::TypeFloat32);
        assert_eq!(tokens[9], Token::TypeFloat64);
        assert_eq!(tokens[10], Token::TypeBool);
        assert_eq!(tokens[11], Token::TypeRune);
        assert_eq!(tokens[12], Token::TypeString);
        assert_eq!(tokens[13], Token::TypeUnit);
    }

    #[test]
    fn test_lexer_keyword_tokens() {
        let source = "package import as is open abstract sealed override extend interface prop foreign";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Package);
        assert_eq!(tokens[1], Token::Import);
        assert_eq!(tokens[2], Token::As);
        assert_eq!(tokens[3], Token::Is);
        assert_eq!(tokens[4], Token::Open);
        assert_eq!(tokens[5], Token::Abstract);
        assert_eq!(tokens[6], Token::Sealed);
        assert_eq!(tokens[7], Token::Override);
        assert_eq!(tokens[8], Token::Extend);
        assert_eq!(tokens[9], Token::Interface);
        assert_eq!(tokens[10], Token::Prop);
        assert_eq!(tokens[11], Token::Foreign);
    }

    #[test]
    fn test_lexer_operator_tokens() {
        let source = "** ?? |> ~> << >> & | ^ ~ !";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::StarStar);
        assert_eq!(tokens[1], Token::QuestionQuestion);
        assert_eq!(tokens[2], Token::Pipeline);
        assert_eq!(tokens[3], Token::Composition);
        assert_eq!(tokens[4], Token::Shl);
        assert_eq!(tokens[5], Token::Shr);
        assert_eq!(tokens[6], Token::And);
        assert_eq!(tokens[7], Token::Pipe);
        assert_eq!(tokens[8], Token::Caret);
        assert_eq!(tokens[9], Token::Tilde);
        assert_eq!(tokens[10], Token::Bang);
    }

    #[test]
    fn test_lexer_float32_literal() {
        let source = "3.14f 2.0f";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        match &tokens[0] {
            Token::Float32(v) => assert!((*v - 3.14_f32).abs() < 0.001),
            _ => panic!("Expected Float32, got {:?}", tokens[0]),
        }
    }

    #[test]
    fn test_lexer_char_literal() {
        let source = "'A' 'Z'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::CharLit('A'));
        assert_eq!(tokens[1], Token::CharLit('Z'));
    }

    #[test]
    fn test_lexer_hex_literal() {
        let source = "0xFF 0x0F 0xABCD";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Integer(255));
        assert_eq!(tokens[1], Token::Integer(15));
        assert_eq!(tokens[2], Token::Integer(0xABCD));
    }
}
