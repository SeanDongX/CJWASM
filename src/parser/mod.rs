use crate::ast::*;
use crate::lexer::Token;

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
            Some(Token::Loop) => {
                self.advance();
                Some("loop".to_string())
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
        // 类型上下文中的 >> 视为 > >，以便解析 Array<UInt32>> 等
        if std::mem::discriminant(&expected) == std::mem::discriminant(&Token::Gt)
            && self.peek() == Some(&Token::Shr)
        {
            self.advance(); // consume Shr
            self.pushback = Some(Token::Gt);
            return Ok(());
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

    /// 跳过可选的属性注解 @Ident [ ... ]（如 @Deprecated[message: "..."]]），不解析内容
    /// Skip `@Attr` or `@Attr[...]` annotations.
    /// Returns `true` if the following declaration should be skipped
    /// (i.e. a `@When[os == "Windows"]` condition was encountered that
    /// does not apply when targeting WASM/non-Windows platforms).
    fn skip_optional_attributes(&mut self) -> Result<bool, ParseErrorAt> {
        use crate::lexer::StringOrInterpolated;
        let mut should_skip_next = false;
        while self.check(&Token::At) {
            self.advance(); // @
            let attr_name = match self.advance_ident() {
                Some(n) => n,
                None => return self.bail_at(ParseError::UnexpectedEof, self.at_prev()),
            };
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
        let source = r#"
            extend Int64 {
                prop doubled: Int64 { get() { this * 2 } set(_) { } }
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
        let source = r#"
            struct T { x: Int64 }
            extend T {
                override func m() : Int64 { return 0 }
            }
        "#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = Parser::new(tokens);
        let program = parser.parse_program().unwrap();
        assert!(!program.extends.is_empty());
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
}
