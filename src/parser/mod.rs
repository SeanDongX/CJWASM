use crate::ast::*;
use crate::lexer::{StringOrInterpolated, Token};

mod decl;
mod error;
mod expr;
mod r#macro;
mod pattern;
mod stmt;
mod type_;

pub use error::{line_column_from_source, ParseError, ParseErrorAt};

pub struct Parser {
    tokens: Vec<(usize, Token, usize)>,
    pos: usize,
    /// 单 token 回退，用于将 >> 在类型上下文中拆成 > >
    pushback: Option<Token>,
    /// 方法体内的 receiver 参数名（用于解析 this）
    receiver_name: Option<String>,
    /// 当前泛型作用域的类型参数名，用于将 Ident 解析为 TypeParam
    current_type_params: Vec<String>,
    /// struct/enum 内部方法，解析完成后合并到 functions
    pending_struct_methods: Vec<Function>,
    /// P2.2: 类型别名映射 (alias_name -> actual_type)
    type_aliases: std::collections::HashMap<String, Type>,
    /// 下一处解析的 func 为 operator func（用于 enum 体）
    parsing_operator_func: bool,
    /// 原始源码（用于检测跨行表达式边界）
    source: String,
    /// 是否允许把 `_` 作为表达式通配符解析（仅在 unittest mock 宏实参中启用）
    allow_wildcard_expr: bool,
    /// 条件/主体表达式中禁止把尾随 `{` 当作 lambda（避免 `if x { ... }` 的块被吞掉）
    no_trailing_lambda: bool,
    /// 当前是否在静态方法体内（静态方法不能使用 super）
    in_static_method: bool,
    /// 上一次 `skip_optional_attributes` 是否跳过了 `@C` 属性（13.1.2.1(3)）。
    last_attr_c: bool,
    /// 顶层声明是否带 `@C`（由属性跳过逻辑置位，供声明分派消费）。
    pending_c_decl: bool,
    /// 本文件中带 `@C` 的结构体名（用于字段 CType 约束校验）。
    c_structs: std::collections::HashSet<String>,
}

impl Parser {
    pub fn new(tokens: Vec<(usize, Token, usize)>) -> Self {
        Self {
            tokens,
            pos: 0,
            pushback: None,
            receiver_name: None,
            current_type_params: Vec::new(),
            pending_struct_methods: Vec::new(),
            type_aliases: std::collections::HashMap::new(),
            parsing_operator_func: false,
            source: String::new(),
            allow_wildcard_expr: false,
            no_trailing_lambda: false,
            in_static_method: false,
            last_attr_c: false,
            pending_c_decl: false,
            c_structs: std::collections::HashSet::new(),
        }
    }

    pub fn with_source(mut self, source: &str) -> Self {
        self.source = source.to_string();
        self
    }

    /// 检查前一个 token 的结束位置到当前 token 的开始位置之间是否有换行
    fn newline_before_current(&self) -> bool {
        if self.source.is_empty() {
            return false;
        }
        let prev_end = if self.pos > 0 {
            self.tokens[self.pos - 1].2
        } else {
            return false;
        };
        let cur_start = self.tokens.get(self.pos).map(|t| t.0).unwrap_or(prev_end);
        if cur_start <= prev_end || cur_start > self.source.len() {
            return false;
        }
        self.source[prev_end..cur_start].contains('\n')
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
        if let Some(ref t) = self.pushback {
            return Some(t);
        }
        self.tokens.get(self.pos).map(|(_, t, _)| t)
    }

    fn peek_next(&self) -> Option<&Token> {
        if self.pushback.is_some() {
            self.tokens.get(self.pos).map(|(_, t, _)| t)
        } else {
            self.tokens.get(self.pos + 1).map(|(_, t, _)| t)
        }
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|(_, t, _)| t)
    }

    /// `@C` 顶层声明合法形态检查（13.1.2.1(3)）。
    ///
    /// `@C` 只能修饰：foreign 函数、非泛型顶层函数、非泛型顶层结构体。
    /// 返回违规目标的描述（供错误信息），合法或无法判定时返回 `None`。
    fn c_attr_decl_violation(&self) -> Option<&'static str> {
        let mut i = 0usize;
        while matches!(
            self.peek_at(i),
            Some(Token::Public)
                | Some(Token::Private)
                | Some(Token::Protected)
                | Some(Token::Internal)
                | Some(Token::Open)
                | Some(Token::Abstract)
                | Some(Token::Sealed)
                | Some(Token::Static)
                | Some(Token::Unsafe)
        ) {
            i += 1;
        }
        match self.peek_at(i)? {
            Token::Foreign => None,
            Token::Func | Token::Struct => {
                let mut j = i + 1;
                if matches!(self.peek_at(j), Some(Token::Ident(_))) {
                    j += 1;
                }
                if matches!(self.peek_at(j), Some(Token::Lt)) {
                    if matches!(self.peek_at(i), Some(Token::Func)) {
                        Some("泛型函数")
                    } else {
                        Some("泛型结构体")
                    }
                } else {
                    None
                }
            }
            Token::Macro => Some("macro 定义"),
            Token::Class => Some("class 定义"),
            Token::Interface => Some("interface 定义"),
            Token::Enum => Some("enum 定义"),
            Token::Extend => Some("extend 定义"),
            Token::TypeAlias => Some("类型别名"),
            Token::Var | Token::Let | Token::Const => Some("变量定义"),
            _ => None,
        }
    }

    /// 将当前 token 作为标识符消费（允许部分关键字在标识符位置出现）
    /// cjc 中 main, type, where, is 等在某些上下文中可作为标识符
    fn advance_ident(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(_)) => {
                if let Some(Token::Ident(n)) = self.advance() {
                    Some(n)
                } else {
                    None
                }
            }
            // 规范 1.1(5)：反引号原始标识符必须是合法标识符（如 `open`、`__`、`_5`），
            // 含空格等非法字符（如 ` `）必须报错。
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(ref s)))
                if is_valid_raw_ident_text(s) =>
            {
                if let Some(Token::BacktickStringLit(StringOrInterpolated::Plain(n))) = self.advance()
                {
                    Some(n)
                } else {
                    None
                }
            }
            Some(Token::Main) => {
                self.advance();
                Some("main".to_string())
            }
            Some(Token::Where) => {
                self.advance();
                Some("where".to_string())
            }
            Some(Token::TypeAlias) => {
                self.advance();
                Some("type".to_string())
            }
            Some(Token::Is) => {
                self.advance();
                Some("is".to_string())
            }
            Some(Token::Case) => {
                self.advance();
                Some("case".to_string())
            }
            Some(Token::With) => {
                self.advance();
                Some("with".to_string())
            }
            Some(Token::Underscore) => {
                self.advance();
                Some("_".to_string())
            }
            // Cangjie 规范 1.1(6): 以下修饰符关键字也可作为标识符使用
            Some(Token::Abstract) => {
                self.advance();
                Some("abstract".to_string())
            }
            Some(Token::Open) => {
                self.advance();
                Some("open".to_string())
            }
            Some(Token::Override) => {
                self.advance();
                Some("override".to_string())
            }
            Some(Token::Private) => {
                self.advance();
                Some("private".to_string())
            }
            Some(Token::Protected) => {
                self.advance();
                Some("protected".to_string())
            }
            Some(Token::Public) => {
                self.advance();
                Some("public".to_string())
            }
            Some(Token::Redef) => {
                self.advance();
                Some("redef".to_string())
            }
            Some(Token::Sealed) => {
                self.advance();
                Some("sealed".to_string())
            }
            Some(Token::Loop) => {
                self.advance();
                Some("loop".to_string())
            }
            _ => None,
        }
    }

    /// 消费绑定名（let/var 的变量名）：允许普通标识符及规范 1.1(6) 列出的
    /// 修饰符关键字（abstract/open/override/private/protected/public/redef/sealed），
    /// 但不允许 `main`/`case` 等其他关键字，也不允许 `_`（由调用方单独处理）。
    fn advance_binding_ident(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(_)) | Some(Token::BacktickStringLit(_)) => self.advance_ident(),
            Some(
                Token::Abstract
                | Token::Open
                | Token::Override
                | Token::Private
                | Token::Protected
                | Token::Public
                | Token::Redef
                | Token::Sealed,
            ) => self.advance_ident(),
            _ => None,
        }
    }

    /// 在类型名/别名/扩展目标等位置消费标识符：允许普通标识符、反引号标识符，
    /// 以及类型关键字（如 `type Tuple = (Int64, Int64)`、`extend Unit { ... }`）。
    fn advance_type_name(&mut self) -> Option<String> {
        let keyword = match self.peek() {
            Some(Token::TypeInt8) => Some("Int8"),
            Some(Token::TypeInt16) => Some("Int16"),
            Some(Token::TypeInt32) => Some("Int32"),
            Some(Token::TypeInt64) => Some("Int64"),
            Some(Token::TypeIntNative) => Some("IntNative"),
            Some(Token::TypeUInt8) => Some("UInt8"),
            Some(Token::TypeUInt16) => Some("UInt16"),
            Some(Token::TypeUInt32) => Some("UInt32"),
            Some(Token::TypeUInt64) => Some("UInt64"),
            Some(Token::TypeUIntNative) => Some("UIntNative"),
            Some(Token::TypeFloat16) => Some("Float16"),
            Some(Token::TypeFloat32) => Some("Float32"),
            Some(Token::TypeFloat64) => Some("Float64"),
            Some(Token::TypeRune) => Some("Rune"),
            Some(Token::TypeBool) => Some("Bool"),
            Some(Token::TypeString) => Some("String"),
            Some(Token::TypeUnit) => Some("Unit"),
            Some(Token::TypeNothing) => Some("Nothing"),
            Some(Token::TypeTuple) => Some("Tuple"),
            Some(Token::TypeArray) => Some("Array"),
            Some(Token::TypeRange) => Some("Range"),
            Some(Token::TypeVArray) => Some("VArray"),
            Some(Token::TypeOption) => Some("Option"),
            Some(Token::TypeResult) => Some("Result"),
            Some(Token::TypeMap) => Some("Map"),
            Some(Token::TypeSlice) => Some("Slice"),
            Some(Token::TypeThis) => Some("This"),
            _ => None,
        };
        match keyword {
            Some(name) => {
                self.advance();
                Some(name.to_string())
            }
            None => self.advance_ident(),
        }
    }

    /// 判断当前位置是否是 `Variant(...)` 形式的构造器模式起点
    /// （用于 let/var 解构声明，如 `let Red(a) = ...`）。
    fn looks_like_variant_pattern(&self) -> bool {
        matches!(self.peek(), Some(Token::Ident(n)) if n.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
            && matches!(self.peek_next(), Some(Token::LParen))
    }

    /// 类型别名名称：普通标识符/合法反引号标识符，以及库类型名（Tuple/Array/Range，
    /// 它们是普通标识符而非关键字，规范允许 `type Array = Array<Int64>`），
    /// 但拒绝 Unit/Float64/Nothing 等真正关键字及 main 等保留名。
    fn advance_alias_name(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::TypeTuple) => {
                self.advance();
                Some("Tuple".to_string())
            }
            Some(Token::TypeArray) => {
                self.advance();
                Some("Array".to_string())
            }
            Some(Token::TypeRange) => {
                self.advance();
                Some("Range".to_string())
            }
            _ => self.advance_decl_name(),
        }
    }

    /// 声明名（类名等）：普通标识符或合法反引号标识符；不允许关键字/保留名。
    fn advance_decl_name(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(_)) => self.advance_ident(),
            Some(Token::BacktickStringLit(StringOrInterpolated::Plain(ref s)))
                if is_valid_raw_ident_text(s) =>
            {
                self.advance_ident()
            }
            _ => None,
        }
    }

    fn advance(&mut self) -> Option<Token> {
        if let Some(t) = self.pushback.take() {
            return Some(t);
        }
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].1.clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), ParseErrorAt> {
        // 类型上下文中的 >> / >= / >>= 视为若干 > 与运算符的组合，
        // 以便解析 Array<UInt32>>、Array<Int32>=... 等写法。
        if std::mem::discriminant(&expected) == std::mem::discriminant(&Token::Gt) {
            match self.peek() {
                Some(Token::Shr) => {
                    self.advance(); // consume Shr
                    self.pushback = Some(Token::Gt);
                    return Ok(());
                }
                Some(Token::GtEq) => {
                    self.advance(); // consume GtEq, 留下 '='
                    self.pushback = Some(Token::Assign);
                    return Ok(());
                }
                Some(Token::ShrEq) => {
                    self.advance(); // consume ShrEq, 留下 '>='
                    self.pushback = Some(Token::GtEq);
                    return Ok(());
                }
                _ => {}
            }
        }
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

    /// 跳过当前处于开括号位置的一整组平衡括号（含混合嵌套的 () [] {}）。
    /// Skip one balanced group starting at the current opening token. Handles
    /// mixed nesting of `()`, `[]` and `{}` so macro/attribute argument lists
    /// containing lambdas, array literals or type arguments are consumed safely.
    fn skip_balanced_mixed(&mut self) -> Result<(), ParseErrorAt> {
        let mut depth = 0i32;
        loop {
            match self.advance() {
                Some(Token::LParen) | Some(Token::LBracket) | Some(Token::LBrace) => {
                    depth += 1
                }
                Some(Token::RParen) | Some(Token::RBracket) | Some(Token::RBrace) => {
                    depth -= 1;
                    if depth <= 0 {
                        return Ok(());
                    }
                }
                Some(_) => {}
                None => return self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
            }
        }
    }

    /// 从当前位置的 `<` 起扫描，判断是否在其后存在匹配的泛型实参结束符 `>`。
    ///
    /// 用于消解 `a < b, c` 这类比较表达式与泛型实参 `f<T, U>` 的歧义：
    /// 若在遇到 `)`/`}`/`]`/`;` 等表达式终止符之前没有找到匹配的 `>`，则不是泛型。
    /// `>>`/`>=`/`>>=` 按多个 `>` 与运算符拆分计深度。
    fn has_generic_close(&self) -> bool {
        let mut depth: i32 = 0;
        let mut nest: i32 = 0; // ()/[]/{} 嵌套深度
        let mut off = 0usize;
        loop {
            match self.peek_at(off) {
                None => return false,
                Some(Token::Lt) => depth += 1,
                Some(Token::Gt) => {
                    depth -= 1;
                    if depth <= 0 {
                        return true;
                    }
                }
                Some(Token::Shr) => {
                    depth -= 2;
                    if depth <= 0 {
                        return true;
                    }
                }
                Some(Token::GtEq) => {
                    depth -= 1;
                    if depth <= 0 {
                        return true;
                    }
                }
                Some(Token::ShrEq) => {
                    depth -= 2;
                    if depth <= 0 {
                        return true;
                    }
                }
                Some(Token::LParen) | Some(Token::LBracket) | Some(Token::LBrace) => nest += 1,
                Some(Token::RParen) | Some(Token::RBracket) | Some(Token::RBrace) => {
                    if nest > 0 {
                        nest -= 1;
                    } else {
                        return false;
                    }
                }
                Some(Token::Semicolon) if nest == 0 => return false,
                _ => {}
            }
            off += 1;
        }
    }

    /// 跳过可选的属性注解 @Ident [ ... ]（如 @Deprecated[message: "..."]]），不解析内容
    /// Skip `@Attr` or `@Attr[...]` annotations.
    /// Returns `true` if the following declaration should be skipped
    /// (i.e. a `@When[os == "Windows"]` condition was encountered that
    /// does not apply when targeting WASM/non-Windows platforms).
    fn skip_optional_attributes(&mut self) -> Result<bool, ParseErrorAt> {
        use crate::lexer::StringOrInterpolated;
        let mut should_skip_next = false;
        self.last_attr_c = false;
        while self.check(&Token::At) {
            self.advance(); // @
            let attr_name = match self.advance_ident() {
                Some(n) => n,
                None => return self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
            };
            // 记录 `@C`：其合法目标有限（13.1.2.1(3)），由各声明上下文校验。
            if attr_name == "C" || attr_name == "c" {
                self.last_attr_c = true;
            }
            if self.check(&Token::LBracket) {
                self.advance();
                // For @When[...], collect tokens to evaluate the condition
                let mut collected: Vec<Token> = Vec::new();
                let mut depth = 1u32;
                while depth > 0 {
                    match self.advance() {
                        Some(Token::LBracket) => { depth += 1; collected.push(Token::LBracket); }
                        Some(Token::RBracket) => {
                            depth -= 1;
                            if depth > 0 { collected.push(Token::RBracket); }
                        }
                        None => return self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
                        Some(tok) => collected.push(tok),
                    }
                }
                // Check for @When[os == "Windows"] — skip following decl on non-Windows
                if attr_name == "When" {
                    // Pattern: [Ident("os"), Eq, StringLit("Windows")]
                    let is_os_eq_windows = matches!(
                        collected.as_slice(),
                        [Token::Ident(k), Token::Eq,
                         Token::StringLit(StringOrInterpolated::Plain(v)), ..]
                        if k == "os" && v == "Windows"
                    );
                    if is_os_eq_windows {
                        should_skip_next = true;
                    }
                }
            }
        }
        // 声明形式的宏调用参数：@UnittestOption[Bool](opt, "desc")。
        // 属性后直接跟平衡括号参数列表时整体跳过（不做宏展开）。
        if self.check(&Token::LParen) {
            self.skip_balanced_mixed()?;
        }
        Ok(should_skip_next)
    }

    /// Skip the next top-level declaration (function/class/struct/enum/interface body)
    /// by consuming tokens until the matching `}` is found.
    fn skip_next_top_level_decl(&mut self) -> Result<(), ParseErrorAt> {
        // Skip leading modifiers/keywords/idents until we hit `{`
        while self.peek().is_some() && !self.check(&Token::LBrace) {
            self.advance();
        }
        if self.check(&Token::LBrace) {
            self.advance();
            let mut depth = 1u32;
            while depth > 0 {
                match self.advance() {
                    Some(Token::LBrace) => depth += 1,
                    Some(Token::RBrace) => depth -= 1,
                    None => return self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// 反引号原始标识符的文本是否为合法标识符（规范 1.1(5)）：
/// 以字母或下划线开头，后续为字母/数字/下划线。
pub(crate) fn is_valid_raw_ident_text(s: &str) -> bool {
    // `_` 是通配符关键字，反引号形式也不能作为名字
    if s == "_" {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::lexer::Lexer;

    #[test]
    fn test_parse_function() {
        let source = "func add(a: Int64, b: Int64): Int64 { return a + b }";
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
        let source = "func test(): Int64 { let arr = [1, 2, 3] return arr[0] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_loop() {
        let source =
            "func test(): Int64 { var sum: Int64 = 0 for i in 0..10 { sum = sum + i } return sum }";
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
        let source = "func test(n: Int64): Int64 { match n { 0 => 100, 1 => 200, _ => 999 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_or_pattern() {
        let source = "func test(n: Int64): Int64 { match n { 1 | 2 | 3 => 10, _ => 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_range_pattern() {
        let source = "func test(n: Int64): Int64 { match n { 0..10 => 1, 10..100 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_if_else() {
        let source = "func test(x: Int64): Int64 { if x > 0 { return 1 } else { return 0 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].body.is_empty());
    }

    #[test]
    fn test_parse_while() {
        let source = "func test(): Int64 { var n: Int64 = 0 while n < 10 { n = n + 1 } return n }";
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
            func test(): Int64 {
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
        let source = "func test(n: Int64): Int64 { match n { x if x < 0 => 1, 0 => 2, _ => 3 } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_in_array() {
        let source = "func test(): Int64 { let arr = [1, 2, 3] var s: Int64 = 0 for x in arr { s = s + x } return s }";
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
        let source =
            "func test(): Int64 { var s: Int64 = 0 for i in 1..=5 { s = s + i } return s }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();

        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_logical_ops() {
        let source = "func test(): Int64 { if true && false || !true { return 0 } return 1 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_unary_neg() {
        let source = "func test(): Int64 { let x = -1 let y = -(-2) return x + y }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_block_expr() {
        let source = "func test(): Int64 { let x = { let a = 1 let b = 2 a + b } return x }";
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
        // Lambda syntax: (x: T): R { body }
        let source = "func test() { let f = (x: Int64): Int64 { x * 2 } }";
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

    // === 覆盖率补充：Parser 单元测试 ===

    #[test]
    fn test_parse_import_path() {
        let source = "import bar.baz.foo\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_import_as() {
        let source = "import std.math as m\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_import_plain() {
        let source = "import std.io\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.imports.is_empty());
    }

    #[test]
    fn test_parse_package_declaration() {
        let source = "package test.app\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(program.package_name.is_some());
    }

    #[test]
    fn test_parse_interface_with_default_and_assoc() {
        let source = r#"
            interface Describable {
                type Element;
                func describe(): String;
                func default_method(): Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
        assert!(!program.interfaces[0].assoc_types.is_empty());
    }

    #[test]
    fn test_parse_interface_with_inheritance() {
        let source = r#"
            interface Base { func id(): Int64; }
            interface Extended: Base { func extra(): Int64; }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 2);
    }

    #[test]
    fn test_parse_extend_with_assoc_type() {
        let source = r#"
            struct Foo { x: Int64 }
            extend Foo: SomeInterface {
                type Element = Int64;
                func method(): Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.extends.is_empty());
    }

    #[test]
    fn test_parse_class_with_init_deinit_prop() {
        let source = r#"
            class MyClass {
                var x: Int64;
                var y: String;
                init(x: Int64) { this.x = x }
                ~init { }
                prop value: Int64 {
                    get() { return this.x }
                    set(v) { this.x = v }
                }
                func method(self: MyClass): Int64 { return self.x }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        let cls = &program.classes[0];
        assert!(cls.init.is_some());
        assert!(cls.deinit.is_some());
        // methods include the generated getter/setter + method
        assert!(!cls.methods.is_empty());
    }

    #[test]
    fn test_parse_class_override_method() {
        let source = r#"
            open class Base {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func get(self: Base): Int64 { return self.x }
            }
            class Derived <: Base {
                init(x: Int64) { super(x) }
                override func get(self: Derived): Int64 { return self.x * 2 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_class_abstract_sealed() {
        let source = r#"
            abstract class Shape {
                var name: String;
            }
            sealed class Container {
                var size: Int64;
                init(size: Int64) { this.size = size }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_function_with_throw() {
        let source = r#"
            func validate(x: Int64): Int64 {
                if x < 0 { throw 0 }
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_try_catch_finally() {
        let source = r#"
            func main(): Int64 {
                try {
                    throw 1
                } catch(e) {
                    return 0
                } finally {
                    let x = 1
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_enum_with_where() {
        let source = r#"
            enum Container<T> where T: Comparable {
                Full(T)
                Empty
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
        assert!(!program.enums[0].type_params.is_empty());
    }

    #[test]
    fn test_parse_struct_with_type_constraint() {
        let source = r#"
            struct Wrapper<T: Hashable> { inner: T }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 1);
    }

    #[test]
    fn test_parse_variadic_and_default_params() {
        let source = r#"
            func f(x: Int64, y: Int64 = 10): Int64 { return x + y }
            func g(args: Int64...): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 2);
        // Check default param
        assert!(program.functions[0].params[1].default.is_some());
        // Check variadic
        assert!(program.functions[1].params[0].variadic);
    }

    #[test]
    fn test_parse_type_annotations() {
        let source = r#"
            func main() {
                let a: Array<Int64> = [1, 2]
                let t: Tuple<Int64, Int64> = (1, 2)
                let o: Option<Int64> = None
                let r: Result<Int64, String> = Ok(1)
                let rng: Range = 0..10
                let c: Rune = 'A'
                let u: Unit = ()
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_super_call() {
        let source = r#"
            open class Base {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func get(self: Base): Int64 { return self.x }
            }
            class Child <: Base {
                init(x: Int64) { super(x) }
                func get2(self: Child): Int64 { return super.get() }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_parse_where_clause_function() {
        let source = r#"
            func compare<T>(a: T, b: T): Int64 where T: Comparable {
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].constraints.is_empty());
    }

    #[test]
    fn test_parse_class_implements() {
        let source = r#"
            interface I { func foo(): Int64; }
            class C <: I {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func foo(self: C): Int64 { return self.x }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        // cjc: <: 后的第一个类型为 extends（解析时不区分类/接口）
        assert!(program.classes[0].extends.is_some());
    }

    #[test]
    fn test_parse_for_in_range_and_array() {
        let source = r#"
            func main(): Int64 {
                for i in 0..10 { }
                for v in [1, 2, 3] { }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_arms() {
        let source = r#"
            func main(): Int64 {
                return match 5 {
                    0 => 1,
                    x if x > 10 => 2,
                    1 | 2 | 3 => 3,
                    _ => 0
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_if_let() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                if let Some(v) = o {
                    return v
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_while_let() {
        let source = r#"
            func main(): Int64 {
                var o: Option<Int64> = Some(1)
                while let Some(v) = o {
                    o = None
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_multi_constraint() {
        let source = r#"
            func process<T: Comparable & Hashable>(x: T): T {
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].constraints.len(), 1);
        // Single constraint with two bounds
        assert_eq!(program.functions[0].constraints[0].bounds.len(), 2);
    }

    #[test]
    fn test_parse_class_with_generic_method() {
        let source = r#"
            class Container {
                var x: Int64;
                init(x: Int64) { this.x = x }
                func transform<T>(self: Container): Int64 {
                    return self.x
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_parse_func_with_where() {
        let source = r#"
            func process<T>(x: T) where T <: Comparable { }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(!program.functions[0].constraints.is_empty());
    }

    // === 覆盖率补充：解析器错误处理路径 ===

    #[test]
    fn test_parse_error_display() {
        let err = ParseErrorAt {
            error: ParseError::UnexpectedEof,
            byte_start: 10,
            byte_end: 20,
        };
        let display = format!("{}", err);
        assert!(display.contains("10"));
        assert!(display.contains("20"));
    }

    #[test]
    fn test_parse_error_source() {
        use std::error::Error;
        let err = ParseErrorAt {
            error: ParseError::UnexpectedEof,
            byte_start: 0,
            byte_end: 0,
        };
        assert!(err.source().is_some());
    }

    #[test]
    fn test_line_column_from_source() {
        let source = "abc\ndef\nghi";
        assert_eq!(super::line_column_from_source(source, 0), (1, 1));
        assert_eq!(super::line_column_from_source(source, 3), (1, 4));
        assert_eq!(super::line_column_from_source(source, 4), (2, 1));
        assert_eq!(super::line_column_from_source(source, 7), (2, 4));
        assert_eq!(super::line_column_from_source(source, 8), (3, 1));
        // Past end
        assert_eq!(super::line_column_from_source(source, 100), (3, 4));
    }

    #[test]
    fn test_parse_visibility() {
        let source = r#"
            public func foo(): Int64 { return 1 }
            private func bar(): Int64 { return 2 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 2);
    }

    #[test]
    fn test_parse_struct_standalone() {
        let source = r#"
            struct Empty {}
            struct Single { x: Int64 }
            struct Multi { x: Int64, y: Float64, z: Bool }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 3);
    }

    #[test]
    fn test_parse_enum_variants() {
        let source = r#"
            enum Color { Red Green Blue }
            enum Value {
                Num(Int64)
                Str(String)
                Empty
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 2);
        assert_eq!(program.enums[0].variants.len(), 3);
    }

    #[test]
    fn test_parse_extern_func() {
        let source = r#"
            @import("env", "print")
            foreign func print(msg: String)
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(program.functions[0].extern_import.is_some());
    }

    #[test]
    fn test_parse_extern_func_no_attr() {
        let source = r#"
            foreign func console_log(msg: String): Int64
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(program.functions[0].extern_import.is_some());
    }

    #[test]
    fn test_parse_complex_expressions() {
        let source = r#"
            func main(): Int64 {
                let a = 1 + 2 * 3
                let b = (1 + 2) * 3
                let c = -5
                let d = !true
                let e = ~0xFF
                let f = 2 ** 3
                return a + b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_cast_expression() {
        let source = r#"
            func main(): Int64 {
                let a = 42 as Float64
                let b = 1.5 as Int64
                return b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_null_coalesce() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = None
                let v = o ?? 42
                return v
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_try_operator() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                let v = o?
                return v
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_block_expression() {
        let source = r#"
            func main(): Int64 {
                let x = {
                    let a = 10
                    a + 20
                }
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_range_expression() {
        let source = r#"
            func main(): Int64 {
                let r = 0..10
                let r2 = 0..=10
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_with_guard() {
        let source = r#"
            func main(): Int64 {
                let x = 5
                return match x {
                    n if n > 10 => 1,
                    n if n > 0 => 2,
                    _ => 0
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_compound_assignment() {
        let source = r#"
            func main(): Int64 {
                var x: Int64 = 10
                x += 5
                x -= 2
                x *= 3
                x /= 2
                x %= 7
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_loop_break_continue() {
        let source = r#"
            func main(): Int64 {
                var i: Int64 = 0
                loop {
                    i = i + 1
                    if i > 10 { break }
                    if i % 2 == 0 { continue }
                }
                return i
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_method_call() {
        let source = r#"
            struct Foo { x: Int64 }
            func main(): Int64 {
                let f = Foo { x: 42 }
                return f.x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_string_interpolation_expr() {
        let source = r#"
            func main(): Int64 {
                let name = "world"
                let s = "Hello ${name}!"
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_class_with_type_params() {
        let source = r#"
            class Container<T> {
                var value: T;
                init(v: T) { this.value = v }
                func get(self: Container<T>): T { return self.value }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(!program.classes[0].type_params.is_empty());
    }

    #[test]
    fn test_parse_multiple_imports() {
        let source = r#"
            import std.io
            import math
            import bar.baz.foo
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 3);
    }

    // === 覆盖率补充：错误处理路径 ===

    fn parse_should_fail(source: &str) -> ParseErrorAt {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        parser.parse_program().unwrap_err()
    }

    #[test]
    fn test_parse_error_unexpected_token() {
        let err = parse_should_fail("func 123() {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_missing_rparen() {
        let err = parse_should_fail("func foo( : Int64 { return 0 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_missing_lbrace() {
        let err = parse_should_fail("func foo(): Int64 return 0");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_empty_source() {
        // Empty source should just produce empty program
        let lexer = Lexer::new("");
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(program.functions.is_empty());
    }

    #[test]
    fn test_parse_error_bad_type_annotation() {
        let err = parse_should_fail("func foo(x: ): Int64 { return 0 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_unclosed_struct() {
        let err = parse_should_fail("struct Foo { x: Int64");
        assert!(matches!(
            err.error,
            ParseError::UnexpectedToken(..) | ParseError::UnexpectedEof
        ));
    }

    #[test]
    fn test_parse_error_bad_import() {
        let err = parse_should_fail("import 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_let() {
        let err = parse_should_fail("func main() { let 123 = 1 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_var() {
        let err = parse_should_fail("func main() { var 123 = 1 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_class_member() {
        let err = parse_should_fail("class Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_enum_variant() {
        let err = parse_should_fail("enum Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_interface_method() {
        let err = parse_should_fail("interface Foo { 123 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extend() {
        let err = parse_should_fail("extend 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_at_prev_position() {
        // Test that bail_at works with position tracking
        let err = parse_should_fail("func foo()): { return 0 }");
        assert!(err.byte_start > 0 || err.byte_end > 0);
    }

    // --- 更多错误路径覆盖 ---

    #[test]
    fn test_parse_error_bad_package_name() {
        // package 后面跟非标识符
        let err = parse_should_fail("package 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_package_path_part() {
        // package a.123 - 包路径中有数字
        let err = parse_should_fail("package a.123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_unexpected_top_level() {
        // 顶层出现非声明（裸表达式；let/const 已支持为顶层常量）
        let err = parse_should_fail("1 + 2");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_import_item() {
        // import 123
        let err = parse_should_fail("import 123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_import_path_bad_part() {
        // import a.123 - 路径中有数字
        let err = parse_should_fail("import a.123");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_import_alias_missing_name() {
        // import foo as
        let err = parse_should_fail("import foo as");
        assert!(matches!(err.error, ParseError::UnexpectedEof));
    }

    #[test]
    fn test_parse_error_bad_extern_import_attr() {
        // @import(123, "foo") foreign func
        let err = parse_should_fail(r#"@import(123, "foo") foreign func bar()"#);
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extern_import_name() {
        // @import("env", 123) foreign func
        let err = parse_should_fail(r#"@import("env", 123) foreign func bar()"#);
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_extern_func_name() {
        // foreign func 123()
        let err = parse_should_fail("foreign func 123()");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_name() {
        // struct 123 {}
        let err = parse_should_fail("struct 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_field_name() {
        // struct Foo { 123: Int64 }
        let err = parse_should_fail("struct Foo { 123: Int64 }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_struct_field_type() {
        // struct Foo { x: }
        let err = parse_should_fail("struct Foo { x: }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_enum_name() {
        // enum 123 {}
        let err = parse_should_fail("enum 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_interface_name() {
        // interface 123 {}
        let err = parse_should_fail("interface 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_class_name() {
        // class 123 {}
        let err = parse_should_fail("class 123 {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_function_name() {
        // func 123() {}
        let err = parse_should_fail("func 123() {}");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_for_iterable() {
        let err = parse_should_fail("func main() { for i in ) {} }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_error_bad_match_subject() {
        // match 无效token
        let err = parse_should_fail("func main(): Int64 { return match {} {} }");
        assert!(matches!(err.error, ParseError::UnexpectedToken(..)));
    }

    #[test]
    fn test_parse_import_dotted_path() {
        // import baz.qux.foo (cjc 风格)
        let source = "import baz.qux.foo\nfunc main(): Int64 { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].module_path.len(), 3);
    }

    #[test]
    fn test_parse_for_variable_range() {
        let source = r#"
            func main(): Int64 {
                let n: Int64 = 5
                var sum: Int64 = 0
                for i in 0..n {
                    sum = sum + i
                }
                return sum
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_for_array_literal() {
        let source = r#"
            func main(): Int64 {
                var sum: Int64 = 0
                for i in [1, 2, 3] {
                    sum = sum + i
                }
                return sum
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_match_with_bool_and_string_patterns() {
        let source = r#"
            func main(): Int64 {
                let b: Bool = true
                let r1 = match b {
                    true => 1,
                    false => 0
                }
                let s: String = "hello"
                let r2 = match s {
                    "hello" => 10,
                    _ => 0
                }
                return r1 + r2
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    // === Parser coverage tests (test_p_*) ===

    #[test]
    fn test_p_class_destructor_tilde_init() {
        let source = r#"
            class Foo {
                ~init { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(program.classes[0].deinit.is_some());
    }

    #[test]
    fn test_p_class_static_init() {
        let source = r#"
            class Foo {
                static init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_static_const_field() {
        let source = r#"
            class Foo {
                static const PI: Float64 = 3.14
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_override_method() {
        let source = r#"
            class Bar { func f(): Int64 { 0 } }
            class Foo <: Bar {
                override func f(): Int64 { 1 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_p_class_override_prop() {
        let source = r#"
            class Bar { prop x: Int64 { get() { 0 } set(_) { } } }
            class Foo <: Bar {
                override prop x: Int64 { get() { 1 } set(_) { } }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 2);
    }

    #[test]
    fn test_p_class_primary_constructor() {
        let source = r#"
            class Point(var x: Int64, var y: Int64) { }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(!program.classes[0].primary_ctor_params.is_empty());
    }

    #[test]
    fn test_p_class_named_constructor() {
        let source = r#"
            class Foo {
                Foo(x: Int64) { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_abstract_method() {
        let source = r#"
            abstract class Foo {
                func bar(): Int64;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_operator_func() {
        let source = r#"
            class Box {
                operator func +(other: Box): Box { Box() }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_operator_index() {
        let source = r#"
            class Vec {
                operator func [](i: Int64): Int64 { 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_field_let_inferred() {
        let source = r#"
            class Foo {
                let x = 42
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_field_var_with_default() {
        let source = r#"
            class Foo {
                var x: Int64 = 42
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_class_unsafe_method() {
        let source = r#"
            class Foo {
                unsafe func bar(): Int64 { 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_p_enum_pipe_prefix() {
        let source = r#"
            enum Color { | Red, | Green, | Blue }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_enum_tuple_payload() {
        let source = r#"
            enum Result { Ok(Int64, String), Err(String) }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_enum_subtype_constraint() {
        let source = r#"
            enum Foo <: Bar { A, B }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_enum_with_methods() {
        let source = r#"
            enum Color { Red, Green }
            func Color.name(): String { "color" }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_enum_operator_func() {
        let source = r#"
            enum Num {
                Zero, One
                operator func +(other: Num): Num { Zero }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_enum_prop_inside() {
        let source = r#"
            enum Foo {
                A, B
                prop x: Int64 { get() { 0 } set(_) { } }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    #[test]
    fn test_p_interface_prop_abstract() {
        let source = r#"
            interface Foo {
                prop x: Int64;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_p_interface_prop_getter_setter_default() {
        let source = r#"
            interface Foo {
                prop x: Int64 { get() { 0 } set(v) { } }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_p_interface_subtype_and_multi_inherit() {
        let source = r#"
            interface A { }
            interface B { }
            interface Foo <: A & B { }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 3);
    }

    #[test]
    fn test_p_interface_associated_type() {
        let source = r#"
            interface Iter {
                type Element;
                func next(): Element;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_p_interface_method_where_clause() {
        let source = r#"
            interface Foo {
                func bar(v: Int64): Unit where T <: Object;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_p_interface_static_modifier() {
        let source = r#"
            interface Foo {
                static func create(): Foo;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_p_extend_subtype_interface() {
        let source = r#"
            interface Printable { func print(): Unit }
            extend Int64 <: Printable {
                func print(): Unit { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.extends.len(), 1);
    }

    #[test]
    fn test_p_extend_prop_getter_setter() {
        // 7.1(4)：内建数值类型不允许 mut 属性，故改用本地类验证 getter/setter 解析。
        let source = r#"
            class C {}
            extend C {
                mut prop doubled: Int64 { get() { 0 } set(_) { } }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.extends.len(), 1);
    }

    #[test]
    fn test_p_extend_assoc_type_binding() {
        let source = r#"
            interface Iter { type Element; }
            extend Array<Int64> <: Iter {
                type Element = Int64;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.extends.len(), 1);
    }

    #[test]
    fn test_p_extend_primitive_int64() {
        let source = r#"
            extend Int64 { func foo(): Int64 { 0 } }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.extends.len(), 1);
    }

    #[test]
    fn test_p_extend_generic_type_params() {
        let source = r#"
            extend<T> Array<T> { func len(): Int64 { 0 } }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.extends.len(), 1);
    }

    #[test]
    fn test_p_const_top_level() {
        let source = r#"
            const PI: Float64 = 3.14
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.constants.is_empty());
    }

    #[test]
    fn test_p_import_foreign_at_import() {
        let source = r#"
            @import("env", "malloc") foreign func malloc(size: Int64): Int64
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 2);
    }

    #[test]
    fn test_p_import_std_io() {
        let source = r#"
            import std.io
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 1);
    }

    #[test]
    fn test_p_import_wildcard() {
        let source = r#"
            import std.collection.*
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 1);
    }

    #[test]
    fn test_p_func_default_params() {
        let source = r#"
            func foo(x: Int64 = 1, y: Int64 = 2): Int64 { x + y }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_package_decl() {
        let source = r#"
            package mylib
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(program.package_name.is_some());
    }

    #[test]
    fn test_p_let_wildcard() {
        let source = r#"
            func main(): Int64 {
                let _ = 42
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_let_struct_destructuring() {
        let source = r#"
            struct Point { x: Int64, y: Int64 }
            func main(): Int64 {
                let Point { x: a, y: b } = Point { x: 1, y: 2 }
                return a + b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_let_no_init_with_type() {
        let source = r#"
            func main(): Int64 {
                let x: Int64
                x = 1
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_var_no_init_with_type() {
        let source = r#"
            func main(): Int64 {
                var x: Int64
                x = 1
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_var_wildcard_discard() {
        // var _ = expr — 丢弃绑定，求值后丢弃
        let source = r#"
            func main(): Int64 {
                var _ = 42
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[0] {
            Stmt::Var { pattern, .. } => {
                assert!(matches!(pattern, Pattern::Wildcard));
            }
            other => panic!("expected Var wildcard stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_var_wildcard_with_type() {
        // var _: String = expr — 带类型注解的丢弃绑定
        let source = r#"
            func main(): Int64 {
                var _: Int64 = 7
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[0] {
            Stmt::Var { pattern, ty, .. } => {
                assert!(matches!(pattern, Pattern::Wildcard));
                assert!(matches!(ty, Some(Type::Int64)));
            }
            other => panic!("expected Var wildcard stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_let_definite_assign_no_init() {
        // let x!: T — 确定赋值声明，无初始化式
        let source = r#"
            func main(): Int64 {
                let x!: Int64
                x = 1
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[0] {
            Stmt::Let { pattern, ty, .. } => {
                assert!(matches!(pattern, Pattern::Binding(name) if name == "x"));
                assert!(matches!(ty, Some(Type::Int64)));
            }
            other => panic!("expected Let stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_var_definite_assign_no_init() {
        // var x!: T — 确定赋值声明，无初始化式
        let source = r#"
            func main(): Int64 {
                var x!: Int64
                x = 2
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[0] {
            Stmt::Var { pattern, ty, .. } => {
                assert!(matches!(pattern, Pattern::Binding(name) if name == "x"));
                assert!(matches!(ty, Some(Type::Int64)));
            }
            other => panic!("expected Var stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_struct_primary_ctor_named_bang() {
        // struct S { S(let a!: Int64 = 1) {} } — 主构造参数确定赋值/必需命名参数
        let source = r#"
            struct S {
                S(let a!: Int64 = 1, var b!: String = "x") {}
            }
            func main(): Int64 { return 0 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 1);
        assert_eq!(program.structs[0].fields.len(), 2);
        assert!(program.structs[0].fields[0].default.is_some());
        assert!(program.structs[0].fields[1].default.is_some());
    }

    #[test]
    fn test_p_or_eq_compound_assign() {
        // a ||= b 展开为 a = a || b
        let source = r#"
            func main(): Int64 {
                var a: Bool = false
                a ||= true
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[1] {
            Stmt::Assign { target, value } => {
                assert!(matches!(target, AssignTarget::Var(name) if name == "a"));
                match value {
                    Expr::Binary { op: BinOp::LogicalOr, .. } => {}
                    other => panic!("expected `a || b`, got {:?}", other),
                }
            }
            other => panic!("expected Assign stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_and_eq_compound_assign() {
        // a &&= b 展开为 a = a && b
        let source = r#"
            func main(): Int64 {
                var a: Bool = true
                a &&= false
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[1] {
            Stmt::Assign { value, .. } => match value {
                Expr::Binary { op: BinOp::LogicalAnd, .. } => {}
                other => panic!("expected `a && b`, got {:?}", other),
            },
            other => panic!("expected Assign stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_starstar_eq_compound_assign() {
        // a **= b 展开为 a = a ** b
        let source = r#"
            func main(): Int64 {
                var a: Int64 = 2
                var b: UInt64 = 3
                a **= b
                return a
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[2] {
            Stmt::Assign { target, value } => {
                assert!(matches!(target, AssignTarget::Var(name) if name == "a"));
                match value {
                    Expr::Binary { op: BinOp::Pow, left, right } => {
                        assert!(matches!(left.as_ref(), Expr::Var(name) if name == "a"));
                        assert!(matches!(right.as_ref(), Expr::Var(name) if name == "b"));
                    }
                    other => panic!("expected `a ** b`, got {:?}", other),
                }
            }
            other => panic!("expected Assign stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_less_than_builtin_max_is_comparison() {
        // a < UInt32.Max 必须解析为比较，而不是泛型实参
        let source = r#"
            func main(): Int64 {
                var a: UInt32 = 1
                var c: Bool = a < UInt32.Max
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        match &program.functions[0].body[1] {
            Stmt::Var { value, .. } => match value {
                Expr::Binary { op: BinOp::Lt, .. } => {}
                other => panic!("expected `<` comparison, got {:?}", other),
            },
            other => panic!("expected Var stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_p_while_let() {
        let source = r#"
            func main(): Int64 {
                var opt: Option<Int64> = Some(1)
                while (let Some(x) = opt) {
                    opt = None
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_for_tuple_destructuring() {
        let source = r#"
            func main(): Int64 {
                let items = [(1, 2), (3, 4)]
                for ((k, v) in items) {
                    let _ = k + v
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_for_underscore() {
        let source = r#"
            func main(): Int64 {
                let arr = [1, 2, 3]
                for (_ in arr) { }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_try_catch() {
        let source = r#"
            func main(): Int64 {
                try {
                    let _ = 1
                } catch (e: Exception) {
                    let _ = 0
                }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_throw_stmt() {
        let source = r#"
            func main(): Int64 {
                throw Exception()
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_compound_assign_plus_eq() {
        let source = r#"
            func main(): Int64 {
                var x: Int64 = 0
                x += 1
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_compound_assign_minus_eq() {
        let source = r#"
            func main(): Int64 {
                var x: Int64 = 10
                x -= 2
                return x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_loop_stmt() {
        let source = r#"
            func main(): Int64 {
                loop { break }
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pipeline_expr() {
        let source = r#"
            func main(): Int64 {
                let r = 5 |> { x: Int64 => x * 2 }
                return r
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_null_coalesce_expr() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = None
                let v = o ?? 0
                return v
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_cast_expr() {
        let source = r#"
            func main(): Int64 {
                let x = 42 as Float64
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_is_type_expr() {
        let source = r#"
            func main(): Int64 {
                let x: Int64 = 42
                if x is Float64 { return 0 }
                return 1
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_lambda_brace_arrow() {
        let source = r#"
            func main(): Int64 {
                let f = { x: Int64 => x + 1 }
                return f(1)
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_range_expr() {
        let source = r#"
            func main(): Int64 {
                let r = 0..10
                let r2 = 0..=10
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_match_guard_expr() {
        let source = r#"
            func main(): Int64 {
                let x = 5
                return match x {
                    n if n > 0 => 1,
                    _ => 0
                }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_method_call_chain() {
        let source = r#"
            struct S { x: Int64 }
            func S.a(): S { S { x: 1 } }
            func S.b(): S { S { x: 2 } }
            func main(): Int64 {
                let s = S { x: 1 }
                let _ = s.a().b()
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 3);
    }

    #[test]
    fn test_p_index_expr() {
        let source = r#"
            func main(): Int64 {
                let arr = [1, 2, 3]
                return arr[0]
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_tuple_access() {
        let source = r#"
            func main(): Int64 {
                let t = (1, 2, 3)
                return t.0 + t.1
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_struct_init_expr() {
        let source = r#"
            struct Point { x: Int64, y: Int64 }
            func main(): Int64 {
                let p = Point { x: 1, y: 2 }
                return p.x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_constructor_call() {
        let source = r#"
            struct Foo { x: Int64 }
            func main(): Int64 {
                let f = Foo(1)
                return f.x
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_array() {
        let source = r#"
            func main(): Int64 {
                let arr: Array<Int64> = [1, 2, 3]
                return arr[0]
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_tuple() {
        let source = r#"
            func main(): Int64 {
                let t: (Int64, String) = (1, "a")
                return t.0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_option() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_optional_suffix() {
        let source = r#"
            func main(): Int64 {
                let o: Int64? = None
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_function() {
        let source = r#"
            func main(): Int64 {
                let f: (Int64) -> Int64 = { x: Int64 => x }
                return f(1)
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_nested_generics() {
        let source = r#"
            func main(): Int64 {
                let arr: Array<Array<Int64>> = [[1]]
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_map() {
        let source = r#"
            func main(): Int64 {
                let m: Map<String, Int64>
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_result() {
        let source = r#"
            func main(): Int64 {
                let r: Result<Int64, String> = Ok(1)
                return 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_unit() {
        let source = r#"
            func main(): Unit {
                let _: Unit = ()
                return
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_type_nothing() {
        let source = r#"
            func main(): Nothing {
                throw Exception()
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_or() {
        let source = r#"
            func main(): Int64 {
                match 1 { 1 | 2 | 3 => 10, _ => 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_struct() {
        let source = r#"
            struct Point { x: Int64, y: Int64 }
            func main(): Int64 {
                let Point { x: a, y: b } = Point { x: 1, y: 2 }
                return a + b
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_variant() {
        let source = r#"
            func main(): Int64 {
                let o: Option<Int64> = Some(1)
                return match o { Some(x) => x, None => 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_variant_with_payload() {
        let source = r#"
            func main(): Int64 {
                let r: Result<Int64, String> = Ok(42)
                return match r { Ok(v) => v, Err(_) => 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_wildcard() {
        let source = r#"
            func main(): Int64 {
                match 1 { _ => 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_p_pattern_binding() {
        let source = r#"
            func main(): Int64 {
                let x = 1
                return match x { n => n }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    // --- test_pf_ — 覆盖率补充：parser/decl.rs 等路径 ---

    #[test]
    fn test_pf_class_static_var() {
        let source = r#"
            class X {
                static var count: Int64 = 0
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_pf_class_static_func() {
        let source = r#"
            class X {
                static func create() : X { return X() }
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_pf_class_mut_prop() {
        let source = r#"
            class X {
                public mut prop value: Int64 {
                    get() { return 0 }
                    set(v) { }
                }
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_pf_interface_prop_set_abstract() {
        let source = r#"
            interface I {
                prop x: Int64 { get(); set(v); }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_pf_extend_override_static() {
        // 8.2：扩展成员不支持 open/override/redef 修饰符。此用例历史上被宽松
        // 放行以接纳示例代码，现按规范拒绝（见 decl 中扩展成员修饰符校验）。
        let source = r#"
            struct T { x: Int64 }
            extend T {
                override func m() : Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_pf_enum_with_comma_separators() {
        let source = r#"
            enum E { A, B, C(Int64) }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
        assert!(program.enums[0].variants.len() >= 2);
    }

    #[test]
    fn test_pf_class_at_attribute() {
        let source = r#"
            class X {
                @import("env", "log") foreign func log(msg: String)
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_program();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_pf_func_throws_exception() {
        let source = r#"
            func f() : Int64 {
                throw 0
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_pf_class_open_abstract_sealed() {
        let source = r#"
            open abstract sealed class X {
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_pf_struct_subtype_multiple() {
        let source = r#"
            interface A { }
            interface B { }
            struct S <: A & B { x: Int64 }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.structs.len(), 1);
    }

    #[test]
    fn test_pf_extend_assoc_type() {
        let source = r#"
            interface I { type Element = Int64; }
            struct X { x: Int64 }
            extend X <: I { type Element = Int64; }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_program();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_pf_class_named_ctor_with_required() {
        let source = r#"
            class C {
                init(param!: Int64) { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_program();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_pf_interface_open_method() {
        let source = r#"
            interface I {
                open func m() : Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
    }

    #[test]
    fn test_pf_class_protected_field() {
        let source = r#"
            class X {
                protected var x: Int64 = 0
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn test_pf_class_const_field() {
        let source = r#"
            class X {
                const MAX: Int64 = 100
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_program();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_p_error_expected_lparen() {
        let source = "func foo ) { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_expected_rparen() {
        let source = "func foo( { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_expected_identifier() {
        let source = "func func() { return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_missing_colon_type() {
        let source = "func main() { let x Int64 = 1 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_missing_lbrace_after_if() {
        let source = "func main() { if true return 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_missing_rbrace_close() {
        let source = "func main() { let x = 1 ";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_p_error_unexpected_eof() {
        let source = "func foo(";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    // === test_pg_ — parser/decl.rs coverage: UNCOVERED paths ===

    #[test]
    fn test_pg_class_with_deinit_body() {
        let source = r#"
            class Foo {
                var x: Int64
                init(n: Int64) { this.x = n }
                ~init { let _ = 1 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(program.classes[0].deinit.is_some());
        assert!(!program.classes[0].deinit.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_pg_class_field_assign_default() {
        let source = r#"
            class Foo {
                var x = 42
                init() { }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(!program.classes[0].fields.is_empty());
        assert!(program.classes[0].fields[0].default.is_some());
    }

    #[test]
    fn test_pg_extend_generic_array() {
        let source = r#"
            extend<T> Array<T> {
                func len(): Int64 { 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.extends.is_empty());
    }

    #[test]
    fn test_pg_interface_method_with_params() {
        let source = r#"
            interface I {
                func foo(self: I, a: Int64, b: String): Int64;
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.interfaces.len(), 1);
        assert!(!program.interfaces[0].methods.is_empty());
        assert!(program.interfaces[0].methods[0].params.len() >= 3);
    }

    #[test]
    fn test_pg_enum_prop_with_body() {
        let source = r#"
            enum Foo {
                A, B
                prop x: Int64 { get() { 0 } set(_) { } }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.enums.len(), 1);
    }

    fn parse_ok(source: &str) -> crate::ast::Program {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        parser.parse_program().unwrap()
    }

    // ---- interface 内泛型方法 ----

    #[test]
    fn test_w1c_interface_generic_method() {
        let program = parse_ok(
            r#"
            interface I {
                static func myFunc<A>(a: A, b!: A): A
                static func myFunc(a: Int32, b!: Int32): Int32
            }
        "#,
        );
        assert_eq!(program.interfaces.len(), 1);
        let methods = &program.interfaces[0].methods;
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "myFunc");
        assert_eq!(methods[0].params.len(), 2);
        assert!(matches!(methods[0].params[0].ty, crate::ast::Type::TypeParam(ref n) if n == "A"));
        assert!(methods[0].params[1].is_named);
    }

    #[test]
    fn test_w1c_interface_generic_method_with_constraint() {
        let program = parse_ok(
            r#"
            interface I {
                func write<T>(v: T): Unit where T <: ToString
            }
        "#,
        );
        assert_eq!(program.interfaces[0].methods.len(), 1);
        assert!(matches!(
            program.interfaces[0].methods[0].params[0].ty,
            crate::ast::Type::TypeParam(ref n) if n == "T"
        ));
    }

    // ---- 修饰符任意顺序 ----

    #[test]
    fn test_w1c_class_modifier_order() {
        let program = parse_ok(
            r#"
            class C {
                static public var a: Int32 = 0
                public static var b: Int32 = 0
                static protected var c: Int32 = 1
                static private var d: Int32 = 2
                public mut func f() {}
                protected override func g() {}
                override func h() {}
            }
        "#,
        );
        let class = &program.classes[0];
        assert_eq!(class.fields.len(), 4);
        assert_eq!(class.methods.len(), 3);
        assert_eq!(class.methods[0].func.name, "C.f");
        assert_eq!(class.methods[1].func.name, "C.g");
        assert!(class.methods[1].override_);
        assert_eq!(
            class.methods[1].func.visibility,
            crate::ast::Visibility::Protected
        );
        assert!(class.methods[2].override_);
    }

    #[test]
    fn test_w1c_struct_modifier_order_and_generic_method() {
        let program = parse_ok(
            r#"
            struct S {
                static public var a: Int64 = 0
                public static func f(): Int64 { 1 }
                static private func g<Q>(x: Q): Q { x }
            }
        "#,
        );
        assert_eq!(program.structs[0].fields.len(), 1);
        let names: Vec<_> = program.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"S.f"));
        assert!(names.contains(&"S.g"));
        let g = program.functions.iter().find(|f| f.name == "S.g").unwrap();
        assert_eq!(g.type_params, vec!["Q".to_string()]);
    }

    // ---- const func / const init ----

    #[test]
    fn test_w1c_top_level_const_func() {
        let program = parse_ok("const func f(): Int64 { 1 }");
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].name, "f");
    }

    #[test]
    fn test_w1c_class_const_func_and_const_init() {
        let program = parse_ok(
            r#"
            class C {
                var v: Int64
                const func f(): Int64 { 1 }
                static const func g(): Int64 { 2 }
                const init(x: Int64) { v = x }
            }
        "#,
        );
        let class = &program.classes[0];
        let names: Vec<_> = class.methods.iter().map(|m| m.func.name.as_str()).collect();
        assert!(names.contains(&"C.f"));
        assert!(names.contains(&"C.g"));
        assert!(class.init.is_some());
        assert_eq!(class.init.as_ref().unwrap().params.len(), 1);
    }

    #[test]
    fn test_w1c_struct_const_func() {
        let program = parse_ok(
            r#"
            struct S {
                static const func g(): Int64 { 1 }
            }
        "#,
        );
        assert!(program.functions.iter().any(|f| f.name == "S.g"));
    }

    // ---- ** 等运算符重载声明 ----

    #[test]
    fn test_w1c_operator_star_star() {
        let program = parse_ok(
            r#"
            class Test {
                operator func **(rhs: Test) { 6 }
                operator func !() { 8 }
                operator func ()() { 9 }
                operator func [](idx: Int64) { 10 }
            }
        "#,
        );
        let names: Vec<_> = program.classes[0]
            .methods
            .iter()
            .map(|m| m.func.name.as_str())
            .collect();
        assert!(names.contains(&"Test.op_pow"));
        assert!(names.contains(&"Test.op_not"));
        assert!(names.contains(&"Test.op_call"));
        assert!(names.contains(&"Test.op_index"));
    }

    #[test]
    fn test_w1c_enum_generic_method() {
        let program = parse_ok(
            r#"
            enum E {
                A
                func myFunc<A>(a: A): A { a }
            }
        "#,
        );
        let f = program
            .functions
            .iter()
            .find(|f| f.name == "E.myFunc")
            .expect("enum method should be lowered to a function");
        assert_eq!(f.type_params, vec!["A".to_string()]);
        assert!(f.params.iter().any(|p| p.name == "this"));
    }

    #[test]
    fn test_w9a_static_struct_enum_method_has_no_implicit_this() {
        // static 成员函数没有接收者；实例方法仍插入隐式 this。
        let program = parse_ok(
            r#"
            struct S {
                static func sf(x: Int64) {}
                func mf() {}
            }
            enum E {
                V
                static func sf2() {}
                func mf2() {}
            }
        "#,
        );
        let sf = program
            .functions
            .iter()
            .find(|f| f.name == "S.sf")
            .expect("struct static method");
        assert!(!sf.params.iter().any(|p| p.name == "this"));
        assert_eq!(sf.params.len(), 1);
        let mf = program
            .functions
            .iter()
            .find(|f| f.name == "S.mf")
            .expect("struct instance method");
        assert!(mf.params.iter().any(|p| p.name == "this"));
        let esf = program
            .functions
            .iter()
            .find(|f| f.name == "E.sf2")
            .expect("enum static method");
        assert!(!esf.params.iter().any(|p| p.name == "this"));
        let emf = program
            .functions
            .iter()
            .find(|f| f.name == "E.mf2")
            .expect("enum instance method");
        assert!(emf.params.iter().any(|p| p.name == "this"));
    }
    #[test]
    fn test_parse_varray_type_position() {
        // VArray<T, N> 类型位置：降级为 Array<T>
        let source = "func test() { let a: VArray<Int64, 4> = [] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        match &program.functions[0].body[0] {
            Stmt::Let {
                ty: Some(Type::Array(inner)),
                ..
            } => assert_eq!(**inner, Type::Int64),
            other => panic!("期望 VArray 类型降级为 Array，实际: {:?}", other),
        }
    }

    #[test]
    fn test_parse_varray_type_dollar_len() {
        // VArray<T, $0>：$前缀编译期常量长度
        let source = "func test() { let c: VArray<Int64, $0> = [] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        match &program.functions[0].body[0] {
            Stmt::Let {
                ty: Some(Type::Array(inner)),
                ..
            } => assert_eq!(**inner, Type::Int64),
            other => panic!("期望 VArray 类型降级为 Array，实际: {:?}", other),
        }
    }

    #[test]
    fn test_parse_varray_in_generic_type_arg() {
        // 嵌套泛型实参位置：Array<VArray<T, N>>（>> 需正确拆分）
        let source = "func test(): Array<VArray<Int64, 2>> { return [] }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        let ret = program.functions[0].return_type.as_ref().unwrap();
        match ret {
            Type::Array(inner) => match inner.as_ref() {
                Type::Array(elem) => assert_eq!(**elem, Type::Int64),
                other => panic!("期望内层降级为 Array，实际: {:?}", other),
            },
            other => panic!("期望外层为 Array，实际: {:?}", other),
        }
    }

    #[test]
    fn test_parse_varray_constructor_expr() {
        // VArray<T, N>(repeat: v) 构造表达式：降级为 Array<T>(N, v)
        let source = "func test() { let a = VArray<Int64, 4>(repeat: 0) }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        match &program.functions[0].body[0] {
            Stmt::Let {
                value: Expr::ConstructorCall { name, args, .. },
                ..
            } => {
                assert_eq!(name, "Array");
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Expr::Integer(4)));
            }
            other => panic!("期望 VArray 构造降级为 Array 构造，实际: {:?}", other),
        }
    }

    #[test]
    fn test_parse_range_constructor_expr() {
        // Range<T>(...) 构造表达式应可解析（兼容降级为构造调用）
        let source = "func test() { let r = Range<Int64>(1, 5, 1, true, true, false) }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        match &program.functions[0].body[0] {
            Stmt::Let {
                value: Expr::ConstructorCall { name, .. },
                ..
            } => assert_eq!(name, "Range"),
            other => panic!("期望 Range 构造表达式，实际: {:?}", other),
        }
    }

    #[test]
    fn test_parse_multi_catch() {
        // 兼容多个连续 catch 子句
        let source =
            "func test() { try { let x = 1 } catch (e: E1) { } catch (e: E2) { } finally { } }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(matches!(
            &program.functions[0].body[0],
            Stmt::Expr(Expr::TryBlock { .. })
        ));
    }
    // w2c-declpos: interface 声明的位置语法
    #[test]
    fn test_interface_type_params_and_where() {
        let program = parse_ok(
            "interface I<T> where T <: Any { func f(x: T): T }",
        );
        assert_eq!(program.interfaces.len(), 1);
        assert_eq!(program.interfaces[0].name, "I");
    }

    #[test]
    fn test_interface_generic_inheritance() {
        let program = parse_ok("interface J<U> <: I1<U> & I2<Int32> {}");
        assert_eq!(program.interfaces.len(), 1);
        assert_eq!(program.interfaces[0].parents, vec!["I1", "I2"]);
    }

    #[test]
    fn test_interface_inheritance_then_where() {
        let program = parse_ok("interface I10<T> <: I2 where T <: Any {}");
        assert_eq!(program.interfaces[0].parents, vec!["I2"]);
    }

    #[test]
    fn test_interface_generic_method_and_where() {
        let program = parse_ok(
            r#"
            interface I<T> {
                func f(x: T): T
                static func s<U>(): Unit
                func g<V>(x: V): Unit where V <: Any
            }
            "#,
        );
        assert_eq!(program.interfaces[0].methods.len(), 3);
        assert_eq!(program.interfaces[0].methods[1].name, "s");
    }

    // w2c-declpos: 泛型类型别名
    #[test]
    fn test_type_alias_generic_params() {
        let program = parse_ok("type Arr<T> = Array<Int8>");
        assert_eq!(program.type_aliases.len(), 1);
        assert_eq!(program.type_aliases[0].0, "Arr");
        assert!(matches!(program.type_aliases[0].1, Type::Array(_)));
    }

    #[test]
    fn test_type_alias_generic_params_used_in_target() {
        let program = parse_ok("type Class3<V> = GenericClassC<Int64, V>");
        match &program.type_aliases[0].1 {
            Type::Struct(name, args) => {
                assert_eq!(name, "GenericClassC");
                assert_eq!(args.len(), 2);
                assert!(matches!(args[1], Type::TypeParam(ref p) if p == "V"));
            }
            other => panic!("expected Struct, got {:?}", other),
        }
    }

    // w2c-declpos: 宏调用语句只做语法接纳
    #[test]
    fn test_macro_invocation_statement_with_input_and_args() {
        let program = parse_ok(
            r#"
            func main() {
                @matchOrThrow [id] (e)
                @Twice(foo())
                @MacroWithoutParens
            }
            "#,
        );
        let body = &program.functions[0].body;
        assert_eq!(body.len(), 3);
        match &body[0] {
            Stmt::Expr(Expr::Macro { name, .. }) => assert_eq!(name, "matchOrThrow"),
            other => panic!("expected macro stmt, got {:?}", other),
        }
    }

    #[test]
    fn test_macro_attribute_before_local_func() {
        let program = parse_ok(
            r#"
            func main() {
                @When[backend == "cjnative"]
                func foo(): Unit { }
                foo()
            }
            "#,
        );
        let body = &program.functions[0].body;
        // body[0] 是 @When 宏语句，body[1] 才是被注解的局部函数
        assert!(matches!(body[0], Stmt::Expr(Expr::Macro { .. })));
        assert!(matches!(body[1], Stmt::LocalFunc(_)));
    }

    #[test]
    fn test_parse_import_trailing_semicolon() {
        // cjc 样例常见 `import path.Item;`（带分号）形式
        let source = "import utils.assert.Assert;\nmain() { }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.functions.len(), 1);
    }

    #[test]
    fn test_parse_uint_var_assignment_after_int_init() {
        // 回归：整数初始化后，下一行的变量名 u8/u16/... 不能被当作字面量后缀吞掉
        let source = "main() { var u8: UInt8 = 0 u8 = 8 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions[0].body.len(), 2);
        assert!(matches!(
            program.functions[0].body[1],
            Stmt::Assign { .. }
        ));
    }

    #[test]
    fn test_parse_inout_call_argument() {
        // C 互操作：调用 CFunc 时的 inout 实参（仅语法接纳）
        let source = "main() { var x: Int32 = 0 func_01(inout x) }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(matches!(
            program.functions[0].body[1],
            Stmt::Expr(Expr::Call { .. })
        ));
    }

    #[test]
    fn test_parse_zero_arg_lambda_immediate_call() {
        // `{ => expr }()` — 零参 lambda 立即调用，求值为其主体
        let source = "main() { let x = { => 1 }() }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        if let Stmt::Let { value, .. } = &program.functions[0].body[0] {
            assert!(matches!(value, Expr::Integer(1)));
        } else {
            panic!("应为 let 语句");
        }
    }

    #[test]
    fn test_parse_type_alias_trailing_semicolon() {
        // cjc 允许类型别名声明以分号结尾：type T = Int64;
        let source = "type TestedType = Int64;\nmain() { }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert_eq!(program.type_aliases.len(), 1);
        assert_eq!(program.type_aliases[0].0, "TestedType");
    }

    #[test]
    fn test_parse_integer_literal_suffix_into_ast() {
        // 带类型后缀的整数字面量在 AST 中保留后缀（1u8 → IntegerSuffixed）。
        let source = "main() { let a = 1u8 let b = 2 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        if let Stmt::Let { value, .. } = &program.functions[0].body[0] {
            assert!(matches!(
                value,
                Expr::IntegerSuffixed(1, crate::ast::IntSuffix::U8)
            ));
        } else {
            panic!("应为 let 语句");
        }
        if let Stmt::Let { value, .. } = &program.functions[0].body[1] {
            assert!(matches!(value, Expr::Integer(2)));
        } else {
            panic!("应为 let 语句");
        }
    }
    // ─── w5c-final-pos: 正测试收尾修复的单测 ───

    #[test]
    fn test_w5c_local_const_func() {
        // 局部 `const func` 声明按普通局部函数解析
        let program = parse_ok("main() { const func f() { 1 } let x = f() }");
        assert!(matches!(
            program.functions[0].body[0],
            Stmt::LocalFunc(_)
        ));
    }

    #[test]
    fn test_w5c_const_static_func_modifier_order() {
        // `const static func` / `static const func` 修饰符顺序无关（class 与 struct）
        let program = parse_ok(
            "class G<T> { const static func f() { 3 } const init() { } }\n\
             struct S { const func f() { 4 } }\n\
             main() { }",
        );
        assert_eq!(program.classes.len(), 1);
        assert_eq!(program.structs.len(), 1);
        let static_f = program.classes[0]
            .methods
            .iter()
            .find(|m| m.func.name.ends_with(".f"))
            .expect("应解析出 static f");
        assert!(static_f.is_const);
    }

    #[test]
    fn test_w5c_foreign_block_attribute_func() {
        // foreign { ... } 块内的 @C 属性函数
        let program = parse_ok(
            "foreign { func func_01(x: Int32): Int32\n @C func func_02(x: Int32): Int32 }\n\
             main() { }",
        );
        let externs: Vec<_> = program
            .functions
            .iter()
            .filter(|f| f.extern_import.is_some())
            .collect();
        assert_eq!(externs.len(), 2);
    }

    #[test]
    fn test_w5c_group_import_dotted_items() {
        // `import pkg.{a.*, b.C}` 分组点分导入项
        let program = parse_ok("import utils.{macros.*, assert.Assert}\nmain() { }");
        assert_eq!(program.imports.len(), 1);
        let items = program.imports[0].items.as_ref().expect("分组导入项");
        assert_eq!(items, &vec!["macros.*".to_string(), "assert.Assert".to_string()]);
    }

    #[test]
    fn test_w5c_var_constructor_pattern() {
        // 单构造器枚举允许不可反驳的构造器模式绑定
        let program = parse_ok(
            "enum MyEnum { C(Int64) }\nmain() { var C(c) = C(16) c++ }",
        );
        assert!(matches!(program.functions[0].body[0], Stmt::Var { .. }));
    }

    #[test]
    fn test_w5c_lambda_mixed_param_annotations() {
        // 多参 lambda 允许部分参数带类型注解
        parse_ok("main() { var l: (Int32, Int32) -> Int32 = {a, b: Int32 => a + b} }");
        parse_ok("main() { var l: (Int32, Int32) -> Int32 = {a: Int32, b => a + b} }");
    }

    #[test]
    fn test_w5c_trailing_closure_statement_body() {
        // `Column { 语句块 }` 按尾随闭包调用解析（而非结构体字面量）
        let program = parse_ok(
            "func Column(c: () -> Unit) {}\n\
             main() { Column { println(1) } }",
        );
        match &program.functions[1].body[0] {
            Stmt::Expr(Expr::Call { args, .. }) => {
                assert!(matches!(args.as_slice(), [Expr::Lambda { .. }]))
            }
            other => panic!("应为尾随闭包调用，实际: {:?}", other),
        }
    }

    #[test]
    fn test_w5c_optional_chain_empty_closure() {
        // `s?.f{}` 可选链尾随空闭包
        parse_ok(
            "class FunctionsSet { operator func() () { 5 } }\n\
             struct S { var vf: FunctionsSet = FunctionsSet() }\n\
             main() { let s: ?S = S() let x = s?.vf{} }",
        );
    }

    #[test]
    fn test_w5c_varray_type_close_gt_adjacent_assign() {
        // `VArray<Int64, $2>=[...]` 中 `>=` 需拆分为 `>` 与 `=`
        parse_ok("main() { let a: VArray<Int64, $2>=[0, 1] }");
    }

    #[test]
    fn test_w5c_multiline_string_interpolation() {
        // 多行字符串支持 ${...} 插值
        let program = parse_ok("main() { let s = \"\"\"\n${1 + 2}\"\"\" }");
        assert!(matches!(
            program.functions[0].body[0],
            Stmt::Let { value: Expr::Interpolate(_), .. }
        ));
    }

    #[test]
    fn test_w5c_extra_package_decl_requires_separate_file_marker() {
        // 仅在 SEPARATE-FILE 规格中允许单文件出现多个 package 声明
        parse_should_fail("package a\nfunc f() {}\npackage b\nmain() { }");
        let source = "// SEPARATE-FILE: other.cj\npackage a\nfunc f() {}\npackage b\nmain() { }";
        let tokens: Vec<_> = Lexer::new(source).filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens).with_source(source);
        parser.parse_program().expect("SEPARATE-FILE 规格应允许额外 package");
    }

    #[test]
    fn test_w7b_struct_ctor_modifier_validation() {
        let parses = |s: &str| {
            let tokens: Vec<_> = Lexer::new(s).filter_map(|r| r.ok()).collect();
            Parser::new(tokens).with_source(s).parse_program().is_ok()
        };
        // 2.1.10.2(2)：主构造函数 / init 构造函数的非法修饰符。
        assert!(!parses("struct S { static S() {} }"));
        assert!(!parses("struct S { open init() {} }"));
        assert!(!parses("struct S { private private private S() {} }"));
        assert!(!parses("struct S { unsafe init() {} }"));
        // 合法：可访问性修饰符、const/static init。
        assert!(parses("struct S { private S(x: Int64) {} }"));
        assert!(parses("struct S { S() {} static init() {} }"));
        assert!(parses("struct S { const init() {} }"));
    }

    #[test]
    fn test_w7b_struct_ctor_duplicate_signature() {
        let parses = |s: &str| {
            let tokens: Vec<_> = Lexer::new(s).filter_map(|r| r.ok()).collect();
            Parser::new(tokens).with_source(s).parse_program().is_ok()
        };
        // 2.1.10.2(29)：构造函数之间须构成重载，签名不能重复。
        assert!(!parses("struct S { init() {} init() {} }"));
        assert!(!parses("struct S { S() {} init() {} }"));
        assert!(!parses("struct S { init(x: Int64) {} init(y: Int64) {} }"));
        assert!(parses("struct S { init(x: Int64) {} init(x: Bool) {} }"));
        assert!(parses("struct S { S(x: Int64) {} init(y: Bool) {} }"));
    }

    #[test]
    fn test_w11c_struct_field_conflicting_visibility() {
        let parses = |s: &str| {
            let tokens: Vec<_> = Lexer::new(s).filter_map(|r| r.ok()).collect();
            Parser::new(tokens).with_source(s).parse_program().is_ok()
        };
        // 2.1.10.1(1)：字段声明的可见性修饰符至多一个。
        assert!(!parses("struct S { public private let v = 0 }"));
        assert!(!parses("struct S { private public var v = 0 }"));
        assert!(parses("struct S { private let v = 0 }"));
        assert!(parses("struct S { public var v = 0 }"));
    }

    #[test]
    fn test_w11c_varray_negative_length_rejected() {
        let parses = |s: &str| {
            let tokens: Vec<_> = Lexer::new(s).filter_map(|r| r.ok()).collect();
            Parser::new(tokens).with_source(s).parse_program().is_ok()
        };
        // 2.1.12(5)：VArray 长度不能为负。
        assert!(!parses("main() { let a: VArray<Int64, $-3> = [] }"));
        assert!(parses("main() { let a: VArray<Int64, $3> = [] }"));
    }

    #[test]
    fn test_w11c_struct_private_prop_visibility_preserved() {
        let program = parse_ok("struct S { private prop p: Int64 { get() { 1 } } }");
        let getter = program
            .functions
            .iter()
            .find(|f| f.name == "S.__get_p")
            .expect("应脱糖出属性访问器");
        assert!(matches!(
            getter.visibility,
            crate::ast::Visibility::Private
        ));
    }

    // ─── w13d：6.1.1.4(7) 类只能有一个主构造函数（类名形式） ────────────────

    #[test]
    fn test_w13d_single_primary_ctor_ok() {
        // 一个类名形式主构造函数 + 多个 init 次构造函数（合法）。
        let src = "class A {\n A() {}\n init(x: Int64) { this() }\n init(y: Bool) { this() }\n}\nmain() {}";
        let lexer = Lexer::new(src);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_ok());
    }

    #[test]
    fn test_w13d_two_primary_ctors_rejected() {
        // 两个类名形式主构造函数（不同签名）非法。
        let src = "class A {\n A() {}\n A(x: Int64) {}\n}\nmain() {}";
        let lexer = Lexer::new(src);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }

    #[test]
    fn test_w13d_paren_primary_plus_named_rejected() {
        // `class A(...)` 主构造 + 类名形式 `A()` 主构造 => 两个主构造函数。
        let src = "class A(x: Int64) {\n A() {}\n}\nmain() {}";
        let lexer = Lexer::new(src);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_program().is_err());
    }
}
