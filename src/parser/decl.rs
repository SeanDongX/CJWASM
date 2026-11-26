//! 声明解析：parse_program、parse_struct、parse_class、parse_enum、parse_function 等。

use super::{is_valid_raw_ident_text, ParseError, ParseErrorAt, Parser};
use crate::ast::*;
use crate::lexer::{StringOrInterpolated, Token};

/// 成员声明修饰符集合。仓颉允许成员修饰符以任意顺序出现
/// （如 `static public var`、`public override open func`），因此统一循环收集。
#[derive(Default, Clone)]
pub(crate) struct MemberModifiers {
    pub visibility: Option<Visibility>,
    /// 已消费的可见性修饰符个数（>1 表示出现 public private 这类冲突组合）。
    pub visibility_count: usize,
    pub is_static: bool,
    /// const 作为 func / init 的前缀修饰符（常量求值语义，本阶段按普通 func/init 处理）
    pub is_const: bool,
    pub is_override: bool,
    /// 是否显式写了 `override` 关键字（`redef` 不计入）。
    pub has_override_kw: bool,
    /// 是否显式写了 `redef` 关键字（用于拒绝 `redef override` 组合）。
    pub has_redef_kw: bool,
    pub is_open: bool,
    pub is_abstract: bool,
    pub is_sealed: bool,
    pub is_operator: bool,
    pub is_unsafe: bool,
    pub is_mut: bool,
    /// 已消费的修饰符 token 个数（用于诊断重复/多个修饰符的非法组合，
    /// 如结构体主构造函数只允许一个可访问性修饰符）。
    pub count: usize,
}

impl Parser {
    /// 循环收集成员声明前的修饰符（任意顺序）。
    /// 只消费确定属于修饰符的 token：`const` 仅在后跟 `func`/`init` 时作为修饰符消费，
    /// 否则留给调用方按常量声明处理。
    pub(crate) fn parse_member_modifiers(&mut self) -> MemberModifiers {
        let mut m = MemberModifiers::default();
        loop {
            let pos_before = self.pos;
            match self.peek() {
                Some(Token::Public) => {
                    self.advance();
                    m.visibility = Some(Visibility::Public);
                    m.visibility_count += 1;
                }
                Some(Token::Private) => {
                    self.advance();
                    m.visibility = Some(Visibility::Private);
                    m.visibility_count += 1;
                }
                Some(Token::Protected) => {
                    self.advance();
                    m.visibility = Some(Visibility::Protected);
                    m.visibility_count += 1;
                }
                Some(Token::Internal) => {
                    self.advance();
                    m.visibility = Some(Visibility::Internal);
                    m.visibility_count += 1;
                }
                Some(Token::Static) => {
                    self.advance();
                    m.is_static = true;
                }
                Some(Token::Mut) => {
                    self.advance();
                    m.is_mut = true;
                }
                Some(Token::Override) => {
                    self.advance();
                    m.is_override = true;
                    m.has_override_kw = true;
                }
                Some(Token::Redef) => {
                    self.advance();
                    m.is_override = true;
                    m.has_redef_kw = true;
                }
                Some(Token::Open) => {
                    self.advance();
                    m.is_open = true;
                }
                Some(Token::Abstract) => {
                    self.advance();
                    m.is_abstract = true;
                }
                Some(Token::Sealed) => {
                    self.advance();
                    m.is_sealed = true;
                }
                Some(Token::Operator) => {
                    self.advance();
                    m.is_operator = true;
                }
                Some(Token::Unsafe) => {
                    self.advance();
                    m.is_unsafe = true;
                }
                // const 可修饰 func / init / static 字段，且与其它修饰符顺序无关
                // （如 `const static func f()` / `static const var x`）。
                Some(Token::Const) => {
                    self.advance();
                    m.is_const = true;
                }
                _ => break,
            }
            if self.pos != pos_before {
                m.count += 1;
            }
        }
        m
    }

    /// 将运算符重载声明中的运算符 token 转为内部方法名（如 `**` → `op_pow`）。
    /// 调用前当前 token 应位于运算符处；`[]` / `()` 会额外消费右括号。
    fn parse_operator_name(&mut self) -> Result<&'static str, ParseErrorAt> {
        let op = match self.advance() {
            Some(Token::Plus) => "op_add",
            Some(Token::Minus) => "op_sub",
            Some(Token::Star) => "op_mul",
            Some(Token::StarStar) => "op_pow",
            Some(Token::Slash) => "op_div",
            Some(Token::Percent) => "op_mod",
            Some(Token::Eq) => "op_eq",
            Some(Token::NotEq) => "op_ne",
            Some(Token::Lt) => "op_lt",
            Some(Token::Gt) => "op_gt",
            Some(Token::LtEq) => "op_le",
            Some(Token::GtEq) => "op_ge",
            Some(Token::Shl) => "op_shl",
            Some(Token::Shr) => "op_shr",
            Some(Token::Bang) => "op_not",
            Some(Token::And) => "op_bitand",
            Some(Token::Pipe) => "op_bitor",
            Some(Token::Caret) => "op_bitxor",
            Some(Token::AndAnd) => "op_and",
            Some(Token::OrOr) => "op_or",
            Some(Token::Tilde) => "op_bitnot",
            Some(Token::LBracket) => {
                self.expect(Token::RBracket)?;
                "op_index"
            }
            Some(Token::LParen) => {
                self.expect(Token::RParen)?;
                "op_call"
            }
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(tok, "运算符".to_string()))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };
        Ok(op)
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseErrorAt> {
        let mut package_name = None;
        let mut imports = Vec::new();
        let mut structs = Vec::new();
        let mut interfaces = Vec::new();
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        let mut enums = Vec::new();
        let mut extends = Vec::new();
        let mut type_aliases: Vec<(String, Type)> = Vec::new();
        let mut constants = Vec::new();

        // 解析可选的 package 声明（cjc: package prefix.path，支持点分路径）
        // cjc: macro package 宏包 — 忽略 macro 修饰符
        // 仅在紧跟 package 时按 `macro package` 处理，否则 macro 属于后续声明
        if self.check(&Token::Macro) && matches!(self.peek_at(1), Some(Token::Package)) {
            self.advance(); // 消费 macro
        }
        // protected package 仅在紧跟 package 时才按包声明处理；
        // 否则 `protected` 是后续声明的可见性修饰符（如 protected class）
        if self.check(&Token::Protected) && matches!(self.peek_at(1), Some(Token::Package)) {
            self.advance();
            if !self.check(&Token::Package) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "package".to_string(),
                ));
            }
            self.advance();
            let mut name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                }
            };
            while self.check(&Token::Dot) {
                self.advance();
                let part = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                    }
                };
                name = format!("{}.{}", name, part);
            }
            package_name = Some(name);
        } else if self.check(&Token::Package) {
            self.advance();
            let mut name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                }
            };
            while self.check(&Token::Dot) {
                self.advance();
                let part = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                    }
                };
                name = format!("{}.{}", name, part);
            }
            package_name = Some(name);
        }
        if self.check(&Token::Protected) && matches!(self.peek_at(1), Some(Token::Package)) {
            self.advance();
            if !self.check(&Token::Package) {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "package".to_string(),
                ));
            }
            self.advance();
            let mut name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                }
            };
            while self.check(&Token::Dot) {
                self.advance();
                let part = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                    }
                };
                name = format!("{}.{}", name, part);
            }
            package_name = Some(name);
        }
        if self.check(&Token::Package) {
            self.advance();
            let mut name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                }
            };
            while self.check(&Token::Dot) {
                self.advance();
                let part = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                    }
                };
                name = format!("{}.{}", name, part);
            }
            package_name = Some(name);
        }

        // 解析 import 语句
        while self.check(&Token::Import) {
            imports.push(self.parse_import()?);
        }

        while let Some(_tok) = self.peek() {
            // cjc 允许声明后出现多余分号（如 `class C {};`）
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            // 解析可见性修饰符
            let visibility = if self.check(&Token::Public) {
                self.advance();
                Visibility::Public
            } else if self.check(&Token::Private) {
                self.advance();
                Visibility::Private
            } else if self.check(&Token::Protected) {
                self.advance();
                Visibility::Protected
            } else if self.check(&Token::Internal) {
                self.advance();
                Visibility::Internal
            } else {
                Visibility::default()
            };

            let extern_import = if self.check(&Token::At) {
                if matches!(self.peek_next(), Some(Token::Import)) {
                    Some(self.parse_import_attr()?)
                } else {
                    let skip_next = self.skip_optional_attributes()?;
                    if skip_next {
                        self.skip_next_top_level_decl()?;
                    } else if self.last_attr_c {
                        // 13.1.2.1(3)：`@C` 只能修饰 foreign 函数、非泛型顶层函数、
                        // 非泛型顶层结构体；其它目标（class/enum/interface/extend/
                        // macro/type/变量/泛型函数/泛型结构体）均为非法。
                        if let Some(kind) = self.c_attr_decl_violation() {
                            let tok = self.peek().cloned().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                format!(
                                    "@C 只能修饰 foreign 函数、非泛型顶层函数或顶层结构体，不能修饰{}（13.1.2.1(3)）",
                                    kind
                                ),
                            ));
                        }
                        self.pending_c_decl = true;
                    }
                    continue;
                }
            } else if self.check(&Token::Protected) {
                self.advance();
                self.expect(Token::Package)?;
                let mut name = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "包名".to_string()));
                    }
                };
                while self.check(&Token::Dot) {
                    self.advance();
                    let part = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "包路径".to_string()));
                        }
                    };
                    name = format!("{}.{}", name, part);
                }
                package_name = Some(name);
                continue;
            } else {
                None
            };

            // 消费上一轮 `@C` 属性标记（若本次声明带 `@C`）。
            let c_attr = std::mem::take(&mut self.pending_c_decl);

            if self.check(&Token::Foreign) {
                self.advance();
                // cjc: foreign { func ... } 块语法（多个 FFI 函数）
                if self.check(&Token::LBrace) {
                    self.advance(); // consume {
                    while !self.check(&Token::RBrace) && self.peek().is_some() {
                        // cjc: foreign 块内函数可带属性（如 @C func f(...)）
                        self.skip_optional_attributes()?;
                        if self.check(&Token::RBrace) {
                            break;
                        }
                        functions.push(self.parse_extern_func(visibility.clone(), None)?);
                    }
                    self.expect(Token::RBrace)?;
                } else {
                    let f = self.parse_extern_func(visibility, extern_import)?;
                    // 13.1.2.1(3)/13.1.2.2：`@C foreign func` 同样受 @C 函数约束
                    // （非泛型、形参不得为 CType、不得有命名参数/默认值）。
                    if c_attr {
                        if let Some(msg) = self.validate_c_func(&f) {
                            return self.bail(ParseError::UnexpectedToken(Token::Func, msg));
                        }
                    }
                    functions.push(f);
                }
            } else {
                // 本声明的属性标记（`@When`/`@Test` 等），供类声明保守豁免使用。
                let decl_attr = std::mem::take(&mut self.pending_decl_attr);
                match self.peek() {
                    Some(Token::Struct) => {
                        let sd = self.parse_struct_with_visibility(visibility, c_attr)?;
                        if c_attr {
                            // 13.1.3.5(8)：@C 结构体不能带类型参数。
                            if !sd.type_params.is_empty() {
                                return self.bail(ParseError::UnexpectedToken(
                                    Token::Struct,
                                    format!(
                                        "@C 结构体 '{}' 不能带类型参数（13.1.3.5(8)）",
                                        sd.name
                                    ),
                                ));
                            }
                            // 13.1.3.5(4)：@C 结构体不能实现/继承接口。
                            if sd.constraints.iter().any(|c| c.param == sd.name) {
                                return self.bail(ParseError::UnexpectedToken(
                                    Token::Struct,
                                    format!(
                                        "@C 结构体 '{}' 不能实现接口（13.1.3.5(4)）",
                                        sd.name
                                    ),
                                ));
                            }
                            self.c_structs.insert(sd.name.clone());
                        }
                        structs.push(sd)
                    }
                    Some(Token::Interface) => {
                        interfaces.push(self.parse_interface_with_visibility(visibility)?)
                    }
                    Some(Token::Class)
                    | Some(Token::Abstract)
                    | Some(Token::Sealed)
                    | Some(Token::Open) => {
                        // 类/接口修饰符（abstract/sealed/open 及可见性）可交错出现，
                        // 如 `abstract public class`、`public sealed interface`。
                        // 先收集修饰符，再按后续关键字分派到 class 或 interface。
                        let mut vis = visibility;
                        let (is_abstract, is_sealed, is_open) = self.parse_type_modifiers(&mut vis);
                        if self.check(&Token::Interface) {
                            // 接口只允许 public/sealed 修饰；abstract/open 仅用于 class
                            if is_abstract || is_open {
                                let tok = self.advance().unwrap_or(Token::Interface);
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "Class".to_string(),
                                ));
                            }
                            interfaces.push(self.parse_interface_with_visibility(vis)?)
                        } else {
                            // 类级属性（`@Test`/`@TestTemplate` 等）可能由宏处理，
                            // 供类体内的保守豁免（ctor 签名重复）使用。
                            self.pending_decl_attr = decl_attr;
                            let cls = self.parse_class_body(
                                vis,
                                is_abstract,
                                is_sealed,
                                is_open,
                            )?;
                            self.pending_decl_attr = false;
                            classes.push(cls);
                        }
                    }
                    Some(Token::Enum) => enums.push(self.parse_enum_with_visibility(visibility)?),
                    Some(Token::Extend) => {
                        // 8.6：扩展定义的可见性由被扩展类型的可见性决定，
                        // 不允许显式 `public extend`（private/protected 同理非法）。
                        if !matches!(visibility, Visibility::Internal) {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Extend,
                                "扩展定义不能使用可见性修饰符（8.6）".to_string(),
                            ));
                        }
                        extends.push(self.parse_extend()?)
                    }
                    // P2.2: type Name = Type  /  type Name<T, U> = Type<T, U>
                    Some(Token::TypeAlias) => {
                        self.advance();
                        // 别名可使用库类型名（type Tuple = ...、type Array = ...）
                        let alias_name = match self.advance_alias_name() {
                            Some(n) => n,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "类型别名名称".to_string(),
                                ));
                            }
                        };
                        // 类型别名可带泛型参数（如 type Arr<T> = Array<Int8>），
                        // 目标类型中的 T 按类型参数解析。
                        let (type_params, _) = self.parse_type_params_with_constraints()?;
                        let prev_params =
                            std::mem::replace(&mut self.current_type_params, type_params);
                        self.expect(Token::Assign)?;
                        let target_ty = self.parse_type()?;
                        self.current_type_params = prev_params;
                        // cjc 允许类型别名声明以分号结尾
                        if self.check(&Token::Semicolon) {
                            self.advance();
                        }
                        self.type_aliases
                            .insert(alias_name.clone(), target_ty.clone());
                        type_aliases.push((alias_name, target_ty));
                    }
                    Some(Token::Func) => {
                        let mut f = self.parse_function_with_visibility(visibility)?;
                        if c_attr {
                            if let Some(msg) = self.validate_c_func(&f) {
                                return self.bail(ParseError::UnexpectedToken(Token::Func, msg));
                            }
                            f.is_c_func = true;
                        }
                        functions.push(f)
                    }
                    // cjc: unsafe func — 忽略 unsafe 修饰符，当普通函数处理
                    Some(Token::Unsafe) => {
                        self.advance(); // consume unsafe
                        if self.check(&Token::Func) {
                            let mut f = self.parse_function_with_visibility(visibility)?;
                            if c_attr {
                                if let Some(msg) = self.validate_c_func(&f) {
                                    return self.bail(ParseError::UnexpectedToken(Token::Func, msg));
                                }
                                f.is_c_func = true;
                            }
                            // 13.1.1(3)-(5)：unsafe 函数只能在 unsafe 上下文外调用，
                            // 且不得作为一等值使用。
                            f.is_unsafe = true;
                            functions.push(f)
                        } else {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "func (unsafe 后期望 func)".to_string(),
                            ));
                        }
                    }
                    // cjc: macro FuncName(...) — 宏函数声明，跳过整个宏函数体
                    Some(Token::Macro) => {
                        self.advance(); // consume macro
                        // 可能接 func 关键字（如 macro func Name）或直接是函数名
                        if self.check(&Token::Func) {
                            functions.push(self.parse_function_with_visibility(visibility)?)
                        } else {
                            // macro Name(params): RetType { body } — 无 func 关键字，跳过到匹配的 }
                            // 消费函数名
                            let _ = self.advance_ident();
                            // 跳过 (params): RetType until {
                            let mut depth = 0i32;
                            loop {
                                match self.peek() {
                                    Some(Token::LBrace) => {
                                        depth += 1;
                                        self.advance();
                                        if depth == 1 { break; }
                                    }
                                    Some(Token::RBrace) => {
                                        if depth > 0 { depth -= 1; }
                                        self.advance();
                                    }
                                    None => break,
                                    _ => { self.advance(); }
                                }
                            }
                            // 消费整个 body { ... }
                            let mut brace_depth = 1i32;
                            while brace_depth > 0 {
                                match self.advance() {
                                    Some(Token::LBrace) => brace_depth += 1,
                                    Some(Token::RBrace) => brace_depth -= 1,
                                    None => break,
                                    _ => {}
                                }
                            }
                        }
                    }
                    // cjc: main() 无需 func 关键字 (main 是保留字)
                    Some(Token::Main) => {
                        functions.push(self.parse_main_function(visibility)?);
                    }
                    // cjc: const func — const 作为函数前缀（常量求值语义）
                    Some(Token::Const) if matches!(self.peek_next(), Some(Token::Func)) => {
                        self.advance(); // consume const
                        let mut f = self.parse_function_with_visibility(visibility)?;
                        f.is_const = true;
                        functions.push(f)
                    }
                    // cjc: 顶层常量 let/var/const name: Type = expr
                    Some(Token::Let) | Some(Token::Var) | Some(Token::Const) => {
                        constants.extend(self.parse_top_level_const()?);
                    }
                    // cjc: public/internal import ... — 带可见性的导入语句。
                    Some(Token::Import) => {
                        // A.2.2(1)：import（preamble）必须位于所有顶层声明之前。带可见性
                        // 前缀的 import 只能出现在 preamble 区（尚无任何顶层声明）。
                        // SEPARATE-FILE 单文件多包片段忽略顺序约束（各段独立 preamble）。
                        let preamble_done = !self.source.contains("SEPARATE-FILE")
                            && !(structs.is_empty()
                                && classes.is_empty()
                                && functions.is_empty()
                                && enums.is_empty()
                                && extends.is_empty()
                                && type_aliases.is_empty()
                                && constants.is_empty());
                        if preamble_done {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Import,
                                "import 必须位于所有顶层声明之前（preamble）".to_string(),
                            ));
                        }
                        imports.push(self.parse_import()?);
                    }
                    // 单文件多 package 分段规格（harness 的 SEPARATE-FILE 用例）：
                    // 忽略后续 package 声明，各段并入同一模块继续解析。
                    Some(Token::Package) if self.source.contains("SEPARATE-FILE") => {
                        self.advance();
                        let _ = self.advance_ident();
                        while self.check(&Token::Dot) {
                            self.advance();
                            let _ = self.advance_ident();
                        }
                        continue;
                    }
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(
                            tok.clone(),
                            "struct、interface、class、enum、extend、func 或 foreign func"
                                .to_string(),
                        ))
                    }
                    None => break,
                }
            }
        }
        functions.extend(self.pending_struct_methods.drain(..));
        // 13.1.3.5(3)：@C 结构体字段必须是满足 CType 约束的类型。
        if !self.c_structs.is_empty() {
            let mut bad_users: std::collections::HashSet<String> = std::collections::HashSet::new();
            bad_users.extend(classes.iter().map(|c| c.name.clone()));
            bad_users.extend(enums.iter().map(|e| e.name.clone()));
            bad_users.extend(interfaces.iter().map(|i| i.name.clone()));
            for s in &structs {
                if !self.c_structs.contains(&s.name) {
                    bad_users.insert(s.name.clone());
                }
            }
            if let Some((sname, what)) =
                c_struct_field_violation(&structs, &self.c_structs, &bad_users)
            {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Struct,
                    format!(
                        "@C 结构体 '{}' 的字段类型不能为 {}（13.1.3.5(3)）",
                        sname, what
                    ),
                ));
            }
            // 13.1.3.5(3)：@C 结构体字段不允许显式写 `Array<T>`（VArray 才合法，
            // 二者在 AST 中都降级为 Type::Array，此处依据解析时的源级拼写判定）。
            if let Some((sname, fname)) = self
                .c_struct_explicit_array_fields
                .iter()
                .find(|(s, _)| self.c_structs.contains(s))
            {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Struct,
                    format!(
                        "@C 结构体 '{}' 的字段 '{}' 不能为 Array 类型，应使用 VArray（13.1.3.5(3)）",
                        sname, fname
                    ),
                ));
            }
            // 13.1.3.5(5)：@C 结构体不能作为元组元素类型。
            for t in &self.parsed_tuple_types {
                if let Type::Tuple(elems) = t {
                    for e in elems {
                        if let Some(n) = type_mentions_c_struct(e, &self.c_structs, 0) {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Struct,
                                format!(
                                    "@C 结构体 '{}' 不能作为元组元素类型（13.1.3.5(5)）",
                                    n
                                ),
                            ));
                        }
                    }
                }
            }
            // 13.1.3.5(6)：@C 结构体不能作为枚举关联值类型。
            for en in &enums {
                for v in &en.variants {
                    if let Some(p) = &v.payload {
                        if let Some(n) = type_mentions_c_struct(p, &self.c_structs, 0) {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Struct,
                                format!(
                                    "@C 结构体 '{}' 不能作为枚举关联值类型（13.1.3.5(6)）",
                                    n
                                ),
                            ));
                        }
                    }
                }
            }
            // 13.1.3.5(4)：@C 结构体不能实现/继承接口（含 extend）。
            if let Some(ext) = extends.iter().find(|e| {
                e.interface.is_some() && self.c_structs.contains(&e.target_type)
            }) {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Extend,
                    format!(
                        "@C 结构体 '{}' 不能通过 extend 实现接口（13.1.3.5(4)）",
                        ext.target_type
                    ),
                ));
            }
        }
        // 7.1(4)：数值、Bool、Unit、Nothing、Rune、String、Range、Function、枚举、
        // 元组类型不能定义 mut 属性。扩展中的 `mut prop` 会被脱糖为 `__set_` 方法，
        // 以此判定并拦截。
        {
            let mut no_mut_targets: std::collections::HashSet<&str> = [
                "Int8", "Int16", "Int32", "Int64", "IntNative", "UInt8", "UInt16", "UInt32",
                "UInt64", "UIntNative", "Float16", "Float32", "Float64", "Bool", "Unit",
                "Nothing", "Rune", "String", "Range", "Function", "Tuple", "Option",
            ]
            .into_iter()
            .collect();
            for e in &enums {
                no_mut_targets.insert(e.name.as_str());
            }
            for ext in &extends {
                let has_setter = ext.methods.iter().any(|m| m.name.contains(".__set_"));
                if has_setter && no_mut_targets.contains(ext.target_type.as_str()) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Extend,
                        format!(
                            "类型 '{}' 不能定义 mut 属性（7.1(4)）",
                            ext.target_type
                        ),
                    ));
                }
            }
        }
        Ok(Program {
            package_name,
            imports,
            structs,
            interfaces,
            classes,
            enums,
            functions,
            extends,
            type_aliases,
            constants,
        })
    }

    /// `@C` 函数约束（13.1.2.1(3)/13.1.4(4)）：非泛型，且参数/返回类型不得为 `CType`。
    fn validate_c_func(&self, f: &Function) -> Option<String> {
        if !f.type_params.is_empty() {
            return Some(format!(
                "@C 函数 '{}' 不能为泛型函数（13.1.2.1(3)）",
                f.name
            ));
        }
        if let Some(p) = f.params.iter().find(|p| p.is_named) {
            return Some(format!(
                "@C 函数 '{}' 的形参 '{}' 不允许使用命名参数（13.1.2.2）",
                f.name, p.name
            ));
        }
        if let Some(p) = f.params.iter().find(|p| p.default.is_some()) {
            return Some(format!(
                "@C 函数 '{}' 的形参 '{}' 不允许指定默认值（13.1.2.2）",
                f.name, p.name
            ));
        }
        if let Some(p) = f.params.iter().find(|p| is_ctype_type(&p.ty)) {
            return Some(format!(
                "@C 函数 '{}' 的参数 '{}' 不能为 CType（13.1.4(4)）",
                f.name, p.name
            ));
        }
        if let Some(rt) = &f.return_type {
            if is_ctype_type(rt) {
                return Some(format!(
                    "@C 函数 '{}' 的返回类型不能为 CType（13.1.4(4)）",
                    f.name
                ));
            }
        }
        None
    }

    /// 解析 import 语句 (cjc: import path.to.Item 或 import path.to.*)
    pub(crate) fn parse_import(&mut self) -> Result<Import, ParseErrorAt> {
        self.expect(Token::Import)?;

        // cjc 也允许完全分组写法：import {a.b.*, c.d.E}
        // （本编译器不做真实导入解析，保留第一条路径即可保证语法通过）
        if self.check(&Token::LBrace) {
            self.advance();
            let mut first_path: Vec<String> = Vec::new();
            loop {
                if self.check(&Token::RBrace) {
                    break;
                }
                let part = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(Token::Star) => "*".to_string(),
                    Some(tok) => {
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "导入项名称".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                if first_path.is_empty() {
                    first_path.push(part);
                }
                if self.check(&Token::Dot) {
                    self.advance();
                    continue;
                }
                // 该项结束，跳过后续项
                while !self.check(&Token::Comma) && !self.check(&Token::RBrace) {
                    if self.advance().is_none() {
                        break;
                    }
                }
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }
            self.expect(Token::RBrace)?;
            if self.check(&Token::Semicolon) {
                self.advance();
            }
            return Ok(Import {
                module_path: first_path,
                items: None,
                alias: None,
            });
        }

        // cjc 风格: import path.to.Item 或 import path.to.* 或 import path.to.Item as alias
        // 或 import path.to.{Item1, Item2, ...}
        let first = match self.advance() {
            Some(Token::Ident(n)) => n,
            // `internal` 是关键字，但也可作为包路径段（如 std.unittest.mock.internal）
            Some(Token::Internal) => "internal".to_string(),
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string()))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };

        let mut module_path = vec![first];
        while self.check(&Token::Dot) {
            self.advance();
            // 检查是否为通配符 *
            if self.check(&Token::Star) {
                self.advance();
                // import path.to.* → 导入所有项
                // cjc 允许 import 语句以分号结尾，这里可选消费
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                return Ok(Import {
                    module_path,
                    items: None,
                    alias: None,
                });
            }
            // 检查是否为花括号导入列表 {Item1, Item2, ...}
            if self.check(&Token::LBrace) {
                self.advance();
                let mut items = Vec::new();
                loop {
                    // 分组项可为点分路径：`macros.*`、`assert.Assert`
                    let mut segs: Vec<String> = Vec::new();
                    loop {
                        match self.advance() {
                            Some(Token::Ident(n)) => segs.push(n),
                            Some(Token::Internal) => segs.push("internal".to_string()),
                            Some(Token::Star) => segs.push("*".to_string()),
                            Some(tok) => {
                                return self.bail(ParseError::UnexpectedToken(
                                    tok,
                                    "导入项名称".to_string(),
                                ))
                            }
                            None => return self.bail(ParseError::UnexpectedEof),
                        }
                        if self.check(&Token::Dot) {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                    items.push(segs.join("."));

                    if self.check(&Token::Comma) {
                        self.advance();
                        // 允许尾随逗号
                        if self.check(&Token::RBrace) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                // cjc 允许 import 语句以分号结尾，这里可选消费
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                return Ok(Import {
                    module_path,
                    items: Some(items),
                    alias: None,
                });
            }
            let part = match self.advance() {
                Some(Token::Ident(n)) => n,
                Some(Token::Internal) => "internal".to_string(),
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "导入路径".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            module_path.push(part);
        }
        let alias = if self.check(&Token::As) {
            self.advance();
            match self.advance() {
                Some(Token::Ident(n)) => Some(n),
                _ => return self.bail(ParseError::UnexpectedEof),
            }
        } else {
            None
        };
        // cjc 允许 import 语句以分号结尾，这里可选消费
        if self.check(&Token::Semicolon) {
            self.advance();
        }
        Ok(Import {
            module_path,
            items: None,
            alias,
        })
    }

    /// 解析顶层常量 (let/var/const name [: Type] = expr)
    /// 支持元组解构：public let (a, b): (T1, T2) = expr
    pub(crate) fn parse_top_level_const(&mut self) -> Result<Vec<ConstDef>, ParseErrorAt> {
        let is_mut = matches!(self.peek(), Some(Token::Var)); // w4b: 可变性跟踪
        let is_const = matches!(self.peek(), Some(Token::Const));
        self.advance(); // consume let/var/const
        // 元组解构声明：展开为多个常量（每个绑定取 TupleIndex）
        if self.check(&Token::LParen) {
            let pattern = self.parse_pattern()?;
            if self.check(&Token::Bang) {
                self.advance();
            }
            if self.check(&Token::Colon) {
                self.advance();
                let _ = self.parse_type()?;
            }
            self.expect(Token::Assign)?;
            let init = self.parse_expr()?;
            if self.check(&Token::Semicolon) {
                self.advance();
            }
            let bindings: Vec<String> = match pattern {
                Pattern::Tuple(ps) => ps
                    .into_iter()
                    .filter_map(|p| match p {
                        Pattern::Binding(n) => Some(n),
                        _ => None,
                    })
                    .collect(),
                Pattern::Binding(n) => vec![n],
                _ => vec![],
            };
            return Ok(bindings
                .into_iter()
                .enumerate()
                .map(|(i, name)| ConstDef {
                    name,
                    ty: Type::Int64,
                    init: Expr::TupleIndex {
                        object: Box::new(init.clone()),
                        index: i as u32,
                    },
                    is_mut,
                    is_const,
                })
                .collect());
        }
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "常量名".to_string()));
            }
        };
        let ty = if self.check(&Token::Colon) {
            self.advance();
            self.parse_type()?
        } else {
            // 无类型注解时用 Int64 占位，codegen 可依 init 推断
            Type::Int64
        };
        self.expect(Token::Assign)?;
        let init = self.parse_expr()?;
        if self.check(&Token::Semicolon) {
            self.advance();
        }
        Ok(vec![ConstDef {
            name,
            ty,
            init,
            is_mut,
            is_const,
        }])
    }

    /// 解析 @import("module", "name") 属性（用于 extern func 前）
    pub(crate) fn parse_import_attr(&mut self) -> Result<ExternImport, ParseErrorAt> {
        self.expect(Token::At)?;
        self.expect(Token::Import)?;
        self.expect(Token::LParen)?;
        let module = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s)))
            | Some(Token::BacktickStringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字符串字面量 (模块名)".to_string(),
                ))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::Comma)?;
        let name = match self.advance() {
            Some(Token::StringLit(StringOrInterpolated::Plain(s)))
            | Some(Token::BacktickStringLit(StringOrInterpolated::Plain(s))) => s,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字符串字面量 (导入名)".to_string(),
                ))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };
        self.expect(Token::RParen)?;
        Ok(ExternImport { module, name })
    }

    /// 解析 extern func 声明（无 body；可选 extern_import 来自前导 @import）
    pub(crate) fn parse_extern_func(
        &mut self,
        visibility: Visibility,
        extern_import: Option<ExternImport>,
    ) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;
        let name = match self.advance_ident() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "函数名".to_string()));
            }
        };
        self.expect(Token::LParen)?;
        // 外来函数形参显式写 `Array<T>` 非法（VArray<T,N> 是合法 CType 形参，
        // 但二者在 AST 中都降级为 Type::Array，只能在解析时按源级拼写判定）。
        let saved_extern = self.in_extern_params;
        self.in_extern_params = true;
        let params = self.parse_params();
        self.in_extern_params = saved_extern;
        let params = params?;
        self.expect(Token::RParen)?;
        let return_type = if self.check(&Token::Colon) {
            self.advance();
            let explicit_array = self.check(&Token::TypeArray);
            let explicit_varray = self.check(&Token::TypeVArray);
            let ty = self.parse_type()?;
            if explicit_array || explicit_varray {
                return self.bail(ParseError::UnexpectedToken(
                    Token::TypeArray,
                    format!(
                        "外来函数 '{}' 的返回类型不能为 Array/VArray（13.1.3.1）",
                        name
                    ),
                ));
            }
            Some(ty)
        } else {
            None
        };
        let import = extern_import.or_else(|| {
            Some(ExternImport {
                module: "env".to_string(),
                name: name.clone(),
            })
        });
        Ok(Function {
            is_unsafe: false,
            // 13.1.2.2(10)：外来函数只能在 unsafe 上下文外调用。
            is_c_func: true,
            is_const: false,
            visibility,
            name,
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: None,
            body: vec![],
            extern_import: import,
        })
    }

    /// 解析结构体定义
    pub(crate) fn parse_struct(&mut self) -> Result<StructDef, ParseErrorAt> {
        self.parse_struct_with_visibility(Visibility::default(), false)
    }

    /// 解析结构体定义（带可见性）
    pub(crate) fn parse_struct_with_visibility(
        &mut self,
        visibility: Visibility,
        is_c: bool,
    ) -> Result<StructDef, ParseErrorAt> {
        self.expect(Token::Struct)?;

        let name = match self.advance() {
            Some(Token::Ident(name)) => name,
            Some(tok) => {
                return self.bail(ParseError::UnexpectedToken(tok, "结构体名".to_string()))
            }
            None => return self.bail(ParseError::UnexpectedEof),
        };

        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());

        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // cjc 兼容: struct Name <: Interface1 & Interface2 { } — 结构体实现多个接口
        if self.check(&Token::SubType) {
            self.advance();
            let mut bounds = Vec::new();
            let mut bound_args: Vec<Vec<Type>> = Vec::new();
            loop {
                let bound = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "约束接口名".to_string(),
                        ));
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                bounds.push(bound);
                let args = if self.check(&Token::Lt) {
                    self.parse_opt_type_args()?.unwrap_or_default()
                } else {
                    Vec::new()
                };
                bound_args.push(args);
                if self.check(&Token::And) {
                    self.advance();
                } else {
                    break;
                }
            }
            constraints.push(crate::ast::TypeConstraint {
                param: name.clone(),
                bounds,
                bound_args,
            });
        }
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        // 结构体全部构造函数（主构造函数展开为首项）与主构造参数信息。
        let mut inits: Vec<crate::ast::InitDef> = Vec::new();
        let mut primary_params: Vec<Param> = Vec::new();
        let mut primary_member_flags: Vec<bool> = Vec::new();

        // 结构体构造函数：主构造函数至多一个；构造函数之间不得签名重复
        // （规范 2.1.10.2(29)：其它构造函数须与主构造函数构成重载）。
        let mut primary_seen = false;
        let mut primary_ctor_visibility = Visibility::Internal;
        let mut ctor_sigs: Vec<Vec<Type>> = Vec::new();
        // 静态初始化器函数体（延迟到字段表完整后校验 6.1.2.2 系列规则）。
        let mut static_init_bodies: Vec<Vec<Stmt>> = Vec::new();

        while !self.check(&Token::RBrace) {
            // 容忍成员声明后的多余分号（如 `init(...) {...};`）
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            // 成员修饰符可任意顺序组合（public static var / static public var / ...）
            let mods = self.parse_member_modifiers();
            // cjc 兼容: struct 主构造函数 StructName(var a: T, var b: U) { } — 参数即字段
            if matches!(self.peek(), Some(Token::Ident(ref n)) if n == &name)
                && matches!(self.peek_next(), Some(Token::LParen))
            {
                // 2.1.10.2(2)：主构造函数只允许一个可访问性修饰符；
                // static/open/abstract/override/redef/sealed/operator/unsafe/mut 均非法。
                if primary_seen {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Ident(name.clone()),
                        "结构体只能有一个主构造函数".to_string(),
                    ));
                }
                primary_seen = true;
                primary_ctor_visibility = mods
                    .visibility
                    .clone()
                    .unwrap_or(Visibility::Internal);
                if mods.count > 1
                    || mods.is_static
                    || mods.is_open
                    || mods.is_abstract
                    || mods.is_override
                    || mods.is_sealed
                    || mods.is_operator
                    || mods.is_unsafe
                    || mods.is_mut
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Ident(name.clone()),
                        "非法的结构体主构造函数修饰符".to_string(),
                    ));
                }
                self.advance(); // consume struct name
                self.expect(Token::LParen)?;
                let mut primary_sig: Vec<Type> = Vec::new();
                // 解析 (public|private|...)? (var|let)? name[!]: Type[= default], ...
                // 成员变量参数（带 let/var）生成字段；普通参数不生成字段。
                while !self.check(&Token::RParen) {
                    let has_vis = self.check(&Token::Public)
                        || self.check(&Token::Private)
                        || self.check(&Token::Protected)
                        || self.check(&Token::Internal);
                    let param_vis = if has_vis {
                        let v = match self.peek() {
                            Some(Token::Public) => crate::ast::Visibility::Public,
                            Some(Token::Private) => crate::ast::Visibility::Private,
                            Some(Token::Protected) => crate::ast::Visibility::Protected,
                            _ => crate::ast::Visibility::Internal,
                        };
                        self.advance();
                        v
                    } else {
                        crate::ast::Visibility::Public
                    };
                    let mut field_is_mut = true;
                    let has_var_let = self.check(&Token::Var) || self.check(&Token::Let);
                    if has_var_let {
                        field_is_mut = self.check(&Token::Var);
                        self.advance();
                    }
                    // 2.1.10.2(12)：成员变量参数带可见性前缀时必须同时带 let/var.
                    if has_vis && !has_var_let {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::RParen),
                            "成员变量参数在可见性修饰符后必须带 let/var".to_string(),
                        ));
                    }
                    let is_member = has_var_let;
                    let param_name = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::RParen);
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                        }
                    };
                    // cjc: 必需命名参数 name!: Type = default，或确定赋值声明
                    let is_named = if self.check(&Token::Bang) {
                        self.advance();
                        true
                    } else {
                        false
                    };
                    self.expect(Token::Colon)?;
                    let ty = self.parse_type()?;
                    // 2.1.10.2(2)：参数类型必须合法，不能是 unit 字面量 `()`。
                    if matches!(&ty, Type::Tuple(ts) if ts.is_empty()) {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::RParen),
                            "参数类型不能是 unit 字面量".to_string(),
                        ));
                    }
                    primary_sig.push(ty.clone());
                    let default = if self.check(&Token::Assign) {
                        self.advance();
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    primary_params.push(Param {
                        name: param_name.clone(),
                        ty: ty.clone(),
                        default: default.clone(),
                        variadic: false,
                        is_named,
                        is_inout: false,
                    });
                    primary_member_flags.push(is_member);
                    // 2.1.10.2(12)：成员变量参数不能用通配符 `_` 命名。
                    if is_member && param_name == "_" {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Underscore,
                            "成员变量参数不能用通配符命名".to_string(),
                        ));
                    }
                    // 2.1.10.2(2)：仅命名参数（`name!`）允许默认值；
                    // 非命名参数带默认值不符合主构造函数文法。
                    if !is_named && default.is_some() {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Assign,
                            "非命名参数不能有默认值".to_string(),
                        ));
                    }
                    fields.push(FieldDef { is_const: false, is_static: false, visibility: param_vis,
                        name: param_name,
                        ty,
                        default,
                        is_mut: field_is_mut,
                        decl_kw: true,
                    });
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(Token::RParen)?;
                // 5.7.1(3)/2.1.10.2：结构体主构造函数同样须与已解析的 init 构成重载。
                if !self.pending_decl_attr && ctor_sigs.contains(&primary_sig) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::LParen,
                        "结构体构造函数签名重复".to_string(),
                    ));
                }
                ctor_sigs.push(primary_sig);
                // 主构造函数体（表达式或声明），保留用于构造语义校验。
                // 2.1.10.2(2)：主构造函数必须有 `{ ... }` 函数体。
                if !self.check(&Token::LBrace) {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::RParen),
                        "主构造函数体（`{`）".to_string(),
                    ));
                }
                let primary_body = {
                    let prev_receiver = self.receiver_name.clone();
                    self.receiver_name = Some("this".to_string());
                    self.advance();
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    self.receiver_name = prev_receiver;
                    body
                };
                // 主构造函数展开为 InitDef：成员变量参数隐式赋值 this.name = name.
                let mut init_body: Vec<Stmt> = Vec::new();
                for (p, flag) in primary_params.iter().zip(primary_member_flags.iter()) {
                    if *flag {
                        init_body.push(Stmt::Assign {
                            target: AssignTarget::Field {
                                object: "this".to_string(),
                                field: p.name.clone(),
                            },
                            value: Expr::Var(p.name.clone()),
                        });
                    }
                }
                init_body.extend(primary_body);
                inits.push(crate::ast::InitDef {
                    is_const: false,
                    visibility: primary_ctor_visibility.clone(),
                    params: primary_params.clone(),
                    body: init_body,
                    member_param_flags: primary_member_flags.clone(),
                    member_param_vis: Vec::new(),
                    member_param_mut: Vec::new(),
                });
                continue;
            }

            // cjc 兼容: init 构造函数（const init 的 const 已由修饰符收集）— 解析并忽略 body
            if self.check(&Token::Init) {
                // 2.1.10.2(2)：init 构造函数同样只允许一个可访问性修饰符
                // （以及 const/static 组合），open/abstract/override/redef/
                // sealed/operator/unsafe/mut 均非法。
                if mods.is_open
                    || mods.is_abstract
                    || mods.is_override
                    || mods.is_sealed
                    || mods.is_operator
                    || mods.is_unsafe
                    || mods.is_mut
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Init,
                        "非法的结构体 init 构造函数修饰符".to_string(),
                    ));
                }
                self.advance(); // consume 'init'
                self.expect(Token::LParen)?;
                let params = self.parse_params()?;
                let sig: Vec<Type> = params.iter().map(|p| p.ty.clone()).collect();
                // static init 是静态初始化器，不参与构造函数重载签名比较。
                if !mods.is_static {
                    if ctor_sigs.contains(&sig) {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "结构体构造函数签名重复".to_string(),
                        ));
                    }
                    ctor_sigs.push(sig);
                }
                self.expect(Token::RParen)?;
                self.expect(Token::LBrace)?;
                // 解析并保留 init body（供 2.1.10.2 构造语义校验）。
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.receiver_name = prev_receiver;
                // static init 是静态初始化器，不是构造函数（不入 inits），
                // 且其中不能通过 this(...) 调用构造函数（2.1.10.2(39)）。
                if mods.is_static {
                    // 6.1.2.2(29)：static init 不能带可访问性修饰符。
                    if matches!(
                        &mods.visibility,
                        Some(Visibility::Public)
                            | Some(Visibility::Private)
                            | Some(Visibility::Protected)
                    ) {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "static 初始化器不能声明可访问性修饰符（6.1.2.2(29)）".to_string(),
                        ));
                    }
                    // 6.1.2.2(22)：结构体 static init 不能带参数。
                    if !params.is_empty() {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "结构体 static 初始化器不能带参数（6.1.2.2(22)）".to_string(),
                        ));
                    }
                    static_init_bodies.push(body.clone());
                    let calls_this = body.iter().any(|s| {
                        matches!(
                            s,
                            Stmt::Expr(Expr::Call { name, .. }) if name == "this" || name == "self"
                        ) || matches!(
                            s,
                            Stmt::Expr(Expr::Invoke { callee, .. })
                                if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self")
                        )
                    });
                    if calls_this {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "static init 中不能调用构造函数 this(...)".to_string(),
                        ));
                    }
                    continue;
                }
                // init 构造函数参数均为普通参数（无成员变量参数）。
                let member_flags = vec![false; params.len()];
                let idef = crate::ast::InitDef {
                    is_const: mods.is_const,
                    visibility: mods.visibility.clone().unwrap_or(Visibility::Internal),
                    params,
                    body,
                    member_param_flags: member_flags,
                    member_param_vis: Vec::new(),
                    member_param_mut: Vec::new(),
                };
                inits.push(idef);
                continue;
            }

            // cjc 兼容: struct 内部方法 → 转为外部方法 func StructName.method(self, ...)
            if self.check(&Token::Func) {
                // 16.4(14)：struct 的 const 实例成员函数不能同时声明为 mut。
                if mods.is_mut && mods.is_const {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Mut,
                        "const 实例成员函数不能声明为 mut（16.4(14)）".to_string(),
                    ));
                }
                // 5.8.1(2)：mut 只能修饰实例成员函数，不能作用于 static 成员函数。
                if mods.is_mut && mods.is_static {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Mut,
                        "struct 静态成员函数不能声明为 mut（5.8.1(2)）".to_string(),
                    ));
                }
                // 5.1/5.6：未实现任何接口的结构体，其成员函数不能使用
                // open/override/redef 修饰符（struct 不可被继承，无 override 语义）。
                // 实现接口的结构体允许 override/redef（覆写接口默认实现），故仅当
                // 无接口约束时才判定（官方 05_01_01_03 a03_05/07/09 负例）。
                if constraints.is_empty() && (mods.is_open || mods.is_override) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Func,
                        "结构体成员函数不能使用 open/override/redef 修饰符（5.1）".to_string(),
                    ));
                }
                // 预设 receiver_name 使方法体内 this 可用
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                if mods.is_operator {
                    self.parsing_operator_func = true;
                }
                let mut func = self.parse_function_with_visibility(
                    // 成员默认可见性为 internal（可访问性级别低于 public 接口成员
                    // 时由语义检查拒绝，如 6.3.5(3)）。
                    mods.visibility.clone().unwrap_or(Visibility::Internal),
                )?;
                func.is_unsafe = mods.is_unsafe;
                self.receiver_name = prev_receiver;
                // 重命名为 StructName.methodName
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                // 添加隐式 self 参数（静态成员函数没有接收者）
                let has_self = func
                    .params
                    .iter()
                    .any(|p| p.name == "self" || p.name == "this");
                if !has_self && !mods.is_static {
                    func.params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                func.is_const = mods.is_const;
                methods.push(func);
                continue;
            }

            // 普通字段（可见性/static/var/let 等修饰符已由 parse_member_modifiers 收集）
            // prop name: Type { get() { ... } set(value) { ... } } — 与带主构造分支相同逻辑
            if self.check(&Token::Prop) {
                self.advance();
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                self.expect(Token::LBrace)?;
                let mut saw_getter = false;
                let mut saw_setter = false;
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "set" {
                            saw_setter = true;
                        }
                        if kw == "get" {
                            self.advance();
                            saw_getter = true;
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(crate::ast::Function {
                                is_unsafe: false,
                                is_c_func: false,
                                is_const: false,
                                // 保留属性声明的可见性（private 属性仅在定义体内可见，
                                // 2.1.10.4(6)）；未标注时维持 public，避免影响既有代码生成。
                                visibility: mods
                                    .visibility
                                    .clone()
                                    .unwrap_or(crate::ast::Visibility::Public),
                                name: format!("{}.__get_{}", name, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![Param {
                                    name: "this".to_string(),
                                    ty: Type::Struct(name.clone(), vec![]),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: Some(prop_ty.clone()),
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "setter 参数名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(crate::ast::Function {
                                is_unsafe: false,
                                is_c_func: false,
                                is_const: false,
                                visibility: mods
                                    .visibility
                                    .clone()
                                    .unwrap_or(crate::ast::Visibility::Public),
                                name: format!("{}.__set_{}", name, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![
                                    Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(name.clone(), vec![]),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                    Param {
                                        name: val_name,
                                        ty: prop_ty.clone(),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                ],
                                return_type: None,
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Ident(kw),
                                "get 或 set".to_string(),
                            ));
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                // 7.1(2)/(6)：属性必须定义 getter；不可变属性不得定义 setter。
                if !saw_getter {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "属性必须定义 getter（7.1(2)）".to_string(),
                    ));
                }
                if !mods.is_mut && saw_setter {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "不可变属性不能定义 setter（7.1(6)）".to_string(),
                    ));
                }
                continue;
            }
            // 2.1.10.1(1)：字段声明的可见性修饰符至多一个（public private 冲突非法）。
            if mods.visibility_count > 1 {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "字段声明只能有一个可见性修饰符".to_string(),
                ));
            }
            let has_decl_kw = self.check(&Token::Var)
                || self.check(&Token::Let)
                || self.check(&Token::Const);
            // `const` 通常已被 parse_member_modifiers 收进 mods.is_const；
            // 无修饰符路径下也可能残留 Const token，两者皆算 const 字段。
            let field_is_const = mods.is_const || self.check(&Token::Const);
            let mut field_is_mut = true;
            if has_decl_kw {
                field_is_mut = self.check(&Token::Var);
                self.advance();
            }
            if field_is_const {
                field_is_mut = false;
            }
            let field_name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // cjc: 字段确定赋值声明 name!: Type（`!` 仅作语法修饰）
            if self.check(&Token::Bang) {
                self.advance();
            }
            // 支持 name = expr（无类型）或 name: Type [= expr]
            let (ty, default) = if self.check(&Token::Assign) {
                self.advance();
                (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
            } else if self.check(&Token::Colon) {
                self.advance();
                // VArray<T, N> 与 Array<T> 在解析阶段都降级为 Type::Array，源级拼写
                // 仅在此处可见：记录 @C 结构体中显式写 Array 的字段（13.1.3.5(3)）。
                let explicit_array = self.check(&Token::TypeArray);
                let ty = self.parse_type()?;
                if explicit_array {
                    self.c_struct_explicit_array_fields
                        .push((name.clone(), field_name.clone()));
                }
                let default = if self.check(&Token::Assign) {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                (ty, default)
            } else {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(
                    tok,
                    "字段: 期望 : 类型 或 = 值".to_string(),
                ));
            };

            // 16.1(2)：const 字段必须在定义处初始化。
            if field_is_const && default.is_none() {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Ident(field_name.clone()),
                    "const 字段必须在定义处初始化（16.1(2)）".to_string(),
                ));
            }
            fields.push(FieldDef { is_const: field_is_const, is_static: mods.is_static,
                visibility: mods.visibility.clone().unwrap_or(crate::ast::Visibility::Public),
                name: field_name,
                ty,
                default,
                is_mut: field_is_mut,
                decl_kw: has_decl_kw,
            });

            // 成员之间允许逗号或分号分隔，也容忍 `;;`/`,;` 等重复分隔符
            while self.check(&Token::Comma) || self.check(&Token::Semicolon) {
                self.advance();
            }
        }

        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;

        // 6.1.2.2：static 初始化器的重复/控制流/成员访问/初始化完整性。
        if let Some(msg) = validate_static_init_blocks(&name, &fields, &static_init_bodies) {
            return self.bail(ParseError::UnexpectedToken(Token::Init, msg));
        }
        // 3.3.2.2(8)：未提供 static init 块时，struct 的静态成员变量必须在定义处
        // 立即初始化（官方 03_03 a08_06/a08_07 负例）。存在 static init 时保守放行，
        // 交由 `validate_static_init_blocks` 判定完整性。
        if static_init_bodies.is_empty() {
            if let Some(f) = fields
                .iter()
                .find(|f| f.is_static && f.decl_kw && f.default.is_none() && !f.is_const)
            {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Ident(f.name.clone()),
                    format!(
                        "结构体静态成员变量 '{}' 必须立即初始化（3.3.2.2(8)）",
                        f.name
                    ),
                ));
            }
        }

        // 将 struct 内部方法存入 pending_methods，在解析完成后合并到 functions
        self.pending_struct_methods.extend(methods);

        Ok(StructDef {
            visibility,
            name,
            type_params,
            constraints,
            fields,
            inits,
            primary_ctor_params: primary_params,
            primary_ctor_member_flags: primary_member_flags,
            has_primary_ctor: primary_seen,
            primary_ctor_visibility,
            is_c,
        })
    }

    /// 解析枚举定义（支持无关联值或单关联类型变体，如 Ok(Int64)）
    pub(crate) fn parse_enum(&mut self) -> Result<EnumDef, ParseErrorAt> {
        self.parse_enum_with_visibility(Visibility::default())
    }

    /// 解析枚举定义（带可见性）
    pub(crate) fn parse_enum_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<EnumDef, ParseErrorAt> {
        self.expect(Token::Enum)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            // cjc: 允许关键字作为枚举名 (如 Result, Option, Tuple 等)
            Some(Token::TypeResult) => "Result".to_string(),
            Some(Token::TypeOption) => "Option".to_string(),
            Some(Token::TypeTuple) => "Tuple".to_string(),
            Some(Token::TypeUnit) => "Unit".to_string(),
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "枚举名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 解析可选的泛型类型参数 <T, E: Bound, ...>
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // cjc: enum Name <: Protocol<Type> { ... } — 消费可选的 <: Bound 或 <: Bound<T>
        let super_uses_subtype = self.check(&Token::SubType);
        if self.check(&Token::SubType)
            || self.check(&Token::Colon)
            || (self.check(&Token::Lt)
                && matches!(self.peek_next(), Some(Token::Colon) | Some(Token::Ident(_))))
        {
            // `<:` 后面的名字是枚举实现的父接口，记录到约束中供语义检查使用
            // （`:` 形式可能是泛型约束等，不复用）。
            let mut super_bounds: Vec<String> = Vec::new();
            let mut super_bound_args: Vec<Vec<Type>> = Vec::new();
            if self.check(&Token::Lt)
                && matches!(self.peek_next(), Some(Token::Colon) | Some(Token::Ident(_)))
            {
                self.advance(); // Lt
                if self.check(&Token::Colon) {
                    self.advance();
                }
            } else {
                self.advance();
            }
            match self.advance() {
                Some(Token::Ident(b)) => {
                    if super_uses_subtype {
                        super_bounds.push(b);
                    }
                }
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "协议/约束名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            let mut first_args: Vec<Type> = Vec::new();
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    first_args.push(self.parse_type()?);
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            if super_uses_subtype {
                super_bound_args.push(first_args);
            }
            // cjc: enum Name <: A & B & C { ... } — 多继承约束用 & 连接
            while self.check(&Token::And) {
                self.advance(); // consume &
                // 消费下一个约束名
                match self.advance() {
                    Some(Token::Ident(b)) => {
                        if super_uses_subtype {
                            super_bounds.push(b);
                        }
                    }
                    Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "协议/约束名".to_string())),
                    None => return self.bail(ParseError::UnexpectedEof),
                }
                // 可选的泛型参数
                let mut args: Vec<Type> = Vec::new();
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        args.push(self.parse_type()?);
                        if self.check(&Token::Gt) { self.advance(); break; }
                        if self.check(&Token::Comma) { self.advance(); } else { break; }
                    }
                }
                if super_uses_subtype {
                    super_bound_args.push(args);
                }
            }
            if super_uses_subtype && !super_bounds.is_empty() {
                constraints.push(TypeConstraint {
                    param: name.clone(),
                    bounds: super_bounds,
                    bound_args: super_bound_args,
                });
            }
        }
        self.expect(Token::LBrace)?;
        let mut variants = Vec::new();
        let mut props: Vec<crate::ast::PropDef> = Vec::new();
        while !self.check(&Token::RBrace) {
            // 容忍成员声明后的多余分号
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            // cjc 兼容: enum 成员修饰符可任意顺序（如 public operator func）
            let mods = self.parse_member_modifiers();
            // cjc 兼容: enum 内部 prop 声明，保留声明并脱糖为 __get_/__set_ 方法，
            // 以便语义检查（接口属性规则 6.2.4.1(6)/(7)）与代码生成。
            if self.check(&Token::Prop) {
                self.advance(); // consume prop
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                let prop_ty = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let member_vis = mods.visibility.clone().unwrap_or(Visibility::Internal);
                let enum_this = crate::ast::Param {
                    name: "this".to_string(),
                    ty: Type::Struct(
                        name.clone(),
                        type_params
                            .iter()
                            .map(|t| Type::TypeParam(t.clone()))
                            .collect(),
                    ),
                    default: None,
                    variadic: false,
                    is_named: false,
                    is_inout: false,
                };
                let mk_getter = |body: Vec<Stmt>| crate::ast::Function {
                    is_unsafe: false,
                    is_c_func: false,
                    is_const: false,
                    visibility: member_vis.clone(),
                    name: format!("{}.__get_{}", name, prop_name),
                    type_params: vec![],
                    constraints: vec![],
                    params: if mods.is_static {
                        vec![]
                    } else {
                        vec![enum_this.clone()]
                    },
                    return_type: prop_ty.clone(),
                    throws: None,
                    body,
                    extern_import: None,
                };
                let mk_setter = |val_name: String, body: Vec<Stmt>| crate::ast::Function {
                    is_unsafe: false,
                    is_c_func: false,
                    is_const: false,
                    visibility: member_vis.clone(),
                    name: format!("{}.__set_{}", name, prop_name),
                    type_params: vec![],
                    constraints: vec![],
                    params: {
                        let mut ps = if mods.is_static {
                            Vec::new()
                        } else {
                            vec![enum_this.clone()]
                        };
                        ps.push(crate::ast::Param {
                            name: val_name,
                            ty: prop_ty.clone().unwrap_or(Type::TypeParam("_".to_string())),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        });
                        ps
                    },
                    return_type: None,
                    throws: None,
                    body,
                    extern_import: None,
                };
                let mut prop = crate::ast::PropDef {
                    name: prop_name.clone(),
                    ty: prop_ty.clone().unwrap_or(Type::TypeParam("_".to_string())),
                    getter: None,
                    setter: None,
                    is_static: mods.is_static,
                    is_mut: mods.is_mut,
                    is_override: mods.is_override,
                    is_redef: mods.is_override && !mods.has_override_kw,
                    visibility: member_vis.clone(),
                };
                // 无体属性声明（纯签名）：仅登记声明，不生成实现。
                if !self.check(&Token::LBrace) {
                    while self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    props.push(prop);
                    continue;
                }
                self.expect(Token::LBrace)?;
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            prop.getter = Some(body.clone());
                            self.pending_struct_methods.push(mk_getter(body));
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "setter 参数名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            prop.is_mut = true;
                            prop.setter = Some((val_name.clone(), body.clone()));
                            self.pending_struct_methods.push(mk_setter(val_name, body));
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                props.push(prop);
                continue;
            }
            // cjc 兼容: enum 内部方法 → 转为外部方法 func EnumName.method(this, ...)
            if self.check(&Token::Func) {
                // 5.8.1(1)：enum 成员函数不能声明为 mut。
                if mods.is_mut {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Mut,
                        "enum 成员函数不能声明为 mut（5.8.1(1)）".to_string(),
                    ));
                }
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                if mods.is_operator {
                    self.parsing_operator_func = true;
                }
                let mut func = self.parse_function_with_visibility(
                    // 成员默认可见性为 internal。
                    mods.visibility.clone().unwrap_or(Visibility::Internal),
                )?;
                func.is_unsafe = mods.is_unsafe;
                self.receiver_name = prev_receiver;
                if !func.name.contains('.') {
                    func.name = format!("{}.{}", name, func.name);
                }
                let has_self = func
                    .params
                    .iter()
                    .any(|p| p.name == "self" || p.name == "this");
                if !has_self && !mods.is_static {
                    func.params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                func.is_const = mods.is_const;
                self.pending_struct_methods.push(func);
                continue;
            }

            // cjc 兼容: 跳过可选的 | 前缀
            if self.check(&Token::Pipe) {
                self.advance();
            }
            // cjc: `| ...` 是枚举的 "catch-all" 占位符，跳过即可
            if self.check(&Token::DotDotDot) {
                self.advance();
                continue;
            }
            let v_name = match self.advance() {
                Some(Token::Ident(n)) => n,
                // cjc: 关键字可作为变体名 (Ok, Err, Some, None, Option, etc.)
                Some(Token::Ok) => "Ok".to_string(),
                Some(Token::Err) => "Err".to_string(),
                Some(Token::Some) => "Some".to_string(),
                Some(Token::None) => "None".to_string(),
                Some(Token::TypeOption) => "Option".to_string(),
                Some(Token::TypeResult) => "Result".to_string(),
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "变体名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            let payload = if self.check(&Token::LParen) {
                self.advance();
                let mut types = vec![self.parse_type()?];
                while self.check(&Token::Comma) {
                    self.advance();
                    types.push(self.parse_type()?);
                }
                self.expect(Token::RParen)?;
                let ty = if types.len() == 1 {
                    types.into_iter().next().unwrap()
                } else {
                    Type::Tuple(types)
                };
                Some(ty)
            } else {
                None
            };
            variants.push(EnumVariant {
                name: v_name,
                payload,
            });
            while self.check(&Token::Comma) || self.check(&Token::Semicolon) {
                self.advance();
            }
            // cjc 兼容: 枚举变体也可用 ';' 分隔（如 enum E { A ; | B ; | C }）
            if self.check(&Token::Semicolon) {
                self.advance();
            }
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        Ok(EnumDef {
            visibility,
            name,
            type_params,
            constraints,
            variants,
            props,
        })
    }

    /// 解析接口定义（支持继承、默认实现、关联类型）
    /// interface Name: Parent1, Parent2 { type Element; func method(args): Ret; func default_method(args): Ret { body } }
    pub(crate) fn parse_interface_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<crate::ast::InterfaceDef, ParseErrorAt> {
        self.expect(Token::Interface)?;
        let name = match self.advance() {
            Some(Token::Ident(n)) => n,
            Some(tok) => return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string())),
            None => return self.bail(ParseError::UnexpectedEof),
        };
        // 解析泛型类型参数 <T, U: Bound, ...>
        let (iface_own_type_params, mut iface_own_constraints) =
            self.parse_type_params_with_constraints()?;
        // 解析接口继承 <: Parent1 & Parent2 或 : Parent1, Parent2 (cjc)
        // 语法顺序: interface Name<T> [<: Parents] [where T <: Bound] { ... }
        // 兼容 where 在 <: 之前出现的写法。
        let mut parents = Vec::new();
        let mut parent_types: Vec<Type> = Vec::new();
        let parse_parents = |this: &mut Self,
                             parents: &mut Vec<String>,
                             parent_types: &mut Vec<Type>|
         -> Result<(), ParseErrorAt> {
            if !(this.check(&Token::Colon) || this.check(&Token::SubType)) {
                return Ok(());
            }
            this.advance();
            loop {
                // 用 parse_type 消费父接口类型（支持泛型参数，如 I1<U>）
                let ty = this.parse_type()?;
                // 保留完整父接口类型（含泛型实参），用于 6.2.3(4)/(5) 重复继承检查。
                parent_types.push(ty.clone());
                match ty {
                    Type::Struct(n, _) => parents.push(n),
                    Type::TypeParam(n) => parents.push(n),
                    Type::Qualified(path) => parents.push(path.join(".")),
                    _ => {
                        return this
                            .bail(ParseError::UnexpectedToken(
                                this.peek().cloned().unwrap_or(Token::Ident(String::new())),
                                "父接口名".to_string(),
                            ))
                    }
                }
                // cjc: 多继承用 & 分隔 (interface Foo <: A & B)；也兼容逗号
                if this.check(&Token::And) || this.check(&Token::Comma) {
                    this.advance();
                } else {
                    break;
                }
            }
            Ok(())
        };
        parse_parents(self, &mut parents, &mut parent_types)?;
        // 6.2.1.1(4)：一个接口声明只能有一个父接口列表（`<: A & B`）。
        // 第二个 `<:`/`:` 紧随其后（未经过 where 约束）时非法。
        if !parents.is_empty()
            && (self.check(&Token::SubType) || self.check(&Token::Colon))
        {
            return self.bail(ParseError::UnexpectedToken(
                self.peek().cloned().unwrap_or(Token::SubType),
                "接口声明只能有一个父接口列表（6.2.1.1(4)）".to_string(),
            ));
        }
        iface_own_constraints.extend(self.parse_where_clause()?);
        // 兼容 where 之后仍跟 <: 的写法
        parse_parents(self, &mut parents, &mut parent_types)?;
        iface_own_constraints.extend(self.parse_where_clause()?);
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        let mut assoc_types = Vec::new();
        while !self.check(&Token::RBrace) {
            // 关联类型: type Element; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Semicolon)?;
                assoc_types.push(crate::ast::AssocTypeDef { name: type_name });
                continue;
            }
            self.skip_optional_attributes()?;
            if self.last_attr_c {
                return self.bail(ParseError::UnexpectedToken(
                    Token::At,
                    "@C 不能修饰接口成员（13.1.2.1(3)）".to_string(),
                ));
            }
            // 接口成员修饰符（static / public / mut / const / operator ...）可任意顺序。
            let mods = self.parse_member_modifiers();
            // 6.2.2.1(12)：static 接口成员函数不能加 open 修饰符（实例成员天然 open，
            // open 仅对实例成员有意义）。
            if mods.is_open && mods.is_static {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Open,
                    "接口的 static 成员函数不能使用 open 修饰符（6.2.2.1(12)）".to_string(),
                ));
            }
            // cjc: 接口中的 prop 声明，脱糖为抽象 getter/setter 方法
            if self.check(&Token::Prop) {
                self.advance(); // consume prop
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                // 记录 prop 的 mut/static 语义：抽象 mut 属性需同时脱糖出抽象 setter，
                // 供实现类型的 mut 修饰符一致性检查（6.2.4.1(6)）使用。
                let prop_is_static = mods.is_static;
                let prop_is_mut_decl = mods.is_mut;
                // 抽象 prop 可以没有 { get()... } 块（纯签名声明）
                if !self.check(&Token::LBrace) {
                    methods.push(crate::ast::InterfaceMethod {
                        name: format!("__get_{}", prop_name),
                        params: vec![],
                        return_type: Some(prop_ty.clone()),
                        default_body: None,
                        visibility: Visibility::Public,
                        is_static: prop_is_static,
                        type_params: vec![],
                        constraints: vec![],
                        is_const: false,
                        is_prop_mut: prop_is_mut_decl,
                    });
                    if prop_is_mut_decl {
                        methods.push(crate::ast::InterfaceMethod {
                            name: format!("__set_{}", prop_name),
                            params: vec![Param {
                                name: "value".to_string(),
                                ty: prop_ty.clone(),
                                default: None,
                                variadic: false,
                                is_named: false,
                                is_inout: false,
                            }],
                            return_type: None,
                            default_body: None,
                            visibility: Visibility::Public,
                            is_static: prop_is_static,
                            type_params: vec![],
                            constraints: vec![],
                            is_const: false,
                            is_prop_mut: prop_is_mut_decl,
                        });
                    }
                    continue;
                }
                self.advance(); // consume {
                let mut saw_setter = false;
                let mut saw_getter = false;
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            saw_getter = true;
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            let default_body = if self.check(&Token::LBrace) {
                                self.advance();
                                self.receiver_name = Some("this".to_string());
                                let body = self.parse_stmts()?;
                                self.receiver_name = None;
                                self.expect(Token::RBrace)?;
                                Some(body)
                            } else {
                                if self.check(&Token::Semicolon) {
                                    self.advance();
                                }
                                None
                            };
                            methods.push(crate::ast::InterfaceMethod {
                                name: format!("__get_{}", prop_name),
                                params: vec![],
                                return_type: Some(prop_ty.clone()),
                                default_body,
                                visibility: Visibility::Public,
                                is_static: prop_is_static,
                                type_params: vec![],
                                constraints: vec![],
                                is_const: false,
                                is_prop_mut: prop_is_mut_decl,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "setter 参数名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            saw_setter = true;
                            let default_body = if self.check(&Token::LBrace) {
                                self.advance();
                                self.receiver_name = Some("this".to_string());
                                let body = self.parse_stmts()?;
                                self.receiver_name = None;
                                self.expect(Token::RBrace)?;
                                Some(body)
                            } else {
                                if self.check(&Token::Semicolon) {
                                    self.advance();
                                }
                                None
                            };
                            methods.push(crate::ast::InterfaceMethod {
                                name: format!("__set_{}", prop_name),
                                params: vec![Param {
                                    name: val_name,
                                    ty: prop_ty.clone(),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: None,
                                default_body,
                                visibility: Visibility::Public,
                                is_static: prop_is_static,
                                type_params: vec![],
                                constraints: vec![],
                                is_const: false,
                                is_prop_mut: prop_is_mut_decl,
                            });
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                // 6.2.2.2(1)/7.1(2)：带默认实现的接口属性必须给出正确的访问器组合：
                // - 必须有 getter；
                // - 不可变属性（无 mut）不得定义 setter；
                // - mut 属性必须同时定义 setter。
                if !saw_getter {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "属性的默认实现必须包含 getter".to_string(),
                    ));
                }
                if prop_is_mut_decl && !saw_setter {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "mut 属性的默认实现必须包含 setter".to_string(),
                    ));
                }
                // 声明为 `mut prop` 但只写了 getter 时，同样补登抽象 setter 以
                // 保持 mut 语义可被实现类型校验观察到。
                if prop_is_mut_decl && !saw_setter {
                    methods.push(crate::ast::InterfaceMethod {
                        name: format!("__set_{}", prop_name),
                        params: vec![Param {
                            name: "value".to_string(),
                            ty: prop_ty.clone(),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        }],
                        return_type: None,
                        default_body: None,
                        visibility: Visibility::Public,
                        is_static: prop_is_static,
                        type_params: vec![],
                        constraints: vec![],
                        is_const: false,
                        is_prop_mut: true,
                    });
                }
                continue;
            }
            // cjc: 接口方法修饰符可任意顺序（static / public / override / mut / const / operator ...）
            // 5.2：无父接口的接口成员函数不能使用 override/redef 修饰符
            // （无被覆写目标；官方 05_01_01_03 a02_05/a02_06 负例）。有父接口时
            // override/redef 用于覆写继承的默认实现，属合法（06_02 a08_03 正例）。
            if parents.is_empty() && (mods.has_override_kw || mods.has_redef_kw) {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Func,
                    "接口成员函数不能使用 override/redef 修饰符（5.2）".to_string(),
                ));
            }
            self.expect(Token::Func)?;
            let m_name = if mods.is_operator {
                self.parse_operator_name()?.to_string()
            } else {
                match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "方法名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                }
            };
            // 泛型方法签名：func name<A, B: Bound>(...)
            let (iface_type_params, mut method_constraints) =
                self.parse_type_params_with_constraints()?;
            let prev_type_params = std::mem::replace(
                &mut self.current_type_params,
                iface_type_params.clone(),
            );
            self.expect(Token::LParen)?;
            let params = self.parse_params()?;
            self.expect(Token::RParen)?;
            let return_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            method_constraints.extend(self.parse_where_clause()?);
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            // 判断有无默认实现 { body } 或者纯签名（分号可选，cjc 兼容）
            let default_body = if self.check(&Token::LBrace) {
                self.advance();
                let prev_receiver = self.receiver_name.clone();
                self.receiver_name = Some("this".to_string());
                let body = self.parse_stmts()?;
                self.receiver_name = prev_receiver;
                self.expect(Token::RBrace)?;
                Some(body)
            } else {
                if self.check(&Token::Semicolon) {
                    self.advance();
                }
                None
            };
            self.current_type_params = prev_type_params;
            // 6.2.2.1(1)：无函数体的接口函数必须显式标注返回类型。
            if default_body.is_none() && return_type.is_none() {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Func,
                    "无函数体的接口成员函数必须显式标注返回类型（6.2.2.1(1)）".to_string(),
                ));
            }
            // 5.8.1(1)：mut 只能修饰实例成员函数；接口 static 成员函数不能为 mut。
            if mods.is_mut && mods.is_static {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Mut,
                    "接口静态成员函数不能声明为 mut（5.8.1(1)）".to_string(),
                ));
            }
            methods.push(crate::ast::InterfaceMethod {
                name: m_name,
                params,
                return_type,
                default_body,
                visibility: mods.visibility.unwrap_or(Visibility::Public),
                is_static: mods.is_static,
                type_params: iface_type_params,
                constraints: method_constraints,
                is_const: mods.is_const,
                is_prop_mut: false,
            });
        }
        self.expect(Token::RBrace)?;
        Ok(crate::ast::InterfaceDef {
            visibility,
            name,
            type_params: iface_own_type_params,
            constraints: iface_own_constraints,
            parents,
            parent_types,
            methods,
            assoc_types,
        })
    }

    /// 解析 extend 定义
    /// extend TypeName: InterfaceName { type Element = ConcreteType; func method(...): ... { ... } }
    pub(crate) fn parse_extend(&mut self) -> Result<crate::ast::ExtendDef, ParseErrorAt> {
        self.expect(Token::Extend)?;
        // cjc: extend<T> Array<T> — 可选的扩展泛型参数
        if self.check(&Token::Lt) {
            let _ = self.parse_type_params()?;
        }
        // 扩展目标可为普通类型名或类型关键字（extend Unit / Nothing / Tuple / Array<T> ...）
        let target_type = match self.advance_type_name() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "类型名".to_string()));
            }
        };
        // 目标类型的泛型实参 Array<T>（8.3：扩展泛型类型必须保留实参以便校验）
        let type_args = self.parse_opt_type_args()?.unwrap_or_default();
        // 可选: 实现的接口 (cjc: extend Type <: InterfaceName 或 <: InterfaceName<T>)
        // 兼容: <: 为 SubType；或 < 与 : 分开为 Lt+Colon；或误写为 < InterfaceName 即 Lt+Ident
        let mut interface_args: Vec<Type> = Vec::new();
        let interface = if self.check(&Token::Colon)
            || self.check(&Token::SubType)
            || (self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Colon)))
            || (self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Ident(_))))
        {
            if self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Colon)) {
                self.advance(); // Lt
                self.advance(); // Colon
            } else if self.check(&Token::Lt) && matches!(self.peek_next(), Some(Token::Ident(_))) {
                self.advance(); // Lt，接口名在下一 token
            } else {
                self.advance();
            }
            let name = match self.advance_ident() {
                Some(n) => n,
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "接口名".to_string()));
                }
            };
            // 消费接口的类型实参，如 SortByExtension<T> 中的 <T>（此处确定是泛型，直接解析）
            // 记录实参以便 8.1.2(12) 判定泛型接口重复实现。
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    interface_args.push(self.parse_type()?);
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
            // cjc: extend Type <: A & B { ... } — 跳过后续 & Bound / & Bound<T>
            while self.check(&Token::And) {
                self.advance();
                let _ = match self.advance_ident() {
                    Some(_) => (),
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "约束接口名".to_string()));
                    }
                };
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        let _ = self.parse_type()?;
                        if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        }
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `>`".to_string(),
                            ));
                        }
                    }
                }
            }
            Some(name)
        } else {
            None
        };
        // cjc: extend Type where T <: Bound { ... }
        let _ = self.parse_where_clause()?;
        // 兜底: 接口名或 where 中 Bound 后的 <T> 若未消费，此处消费（避免 Lt 期望 LBrace）
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        self.expect(Token::LBrace)?;
        let mut methods = Vec::new();
        // 静态成员函数在 methods 中的下标（10.1.1(3)/(6)：静态性与实例性需在语义阶段区分）。
        let mut static_method_indices: Vec<usize> = Vec::new();
        let mut assoc_type_bindings = Vec::new();
        while !self.check(&Token::RBrace) {
            // 容忍成员声明后的多余分号
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            // 成员修饰符可任意顺序组合（public static func / static public ...）
            let mods = self.parse_member_modifiers();
            // 8.2：扩展成员不支持 open/override/redef 修饰符（无继承语义）。
            if mods.is_open || mods.is_override {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Extend,
                    "扩展成员不支持 open/override/redef 修饰符（8.2）".to_string(),
                ));
            }
            // 关联类型绑定: type Element = ConcreteType; (cjc: type 是保留字)
            if matches!(self.peek(), Some(Token::TypeAlias)) {
                self.advance(); // consume "type"
                let type_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self
                            .bail(ParseError::UnexpectedToken(tok, "关联类型名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Assign)?;
                let ty = self.parse_type()?;
                self.expect(Token::Semicolon)?;
                assoc_type_bindings.push((type_name, ty));
                continue;
            }
            if self.check(&Token::At) {
                self.skip_optional_attributes()?;
                if self.last_attr_c {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::At,
                        "@C 不能修饰扩展成员（13.1.2.1(3)）".to_string(),
                    ));
                }
                continue;
            }
            // 方法: [public/private/...] func name(args): Ret { body } (cjc 兼容)
            let member_vis = mods.visibility.clone().unwrap_or_default();
            // prop in extend block: prop name: Type { get() { ... } set(v) { ... } }
            if self.check(&Token::Prop) {
                self.advance();
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                self.expect(Token::LBrace)?;
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(Function {
                                is_unsafe: false,
                                is_c_func: false,
                                is_const: false,
                                visibility: member_vis.clone(),
                                name: format!("{}.__get_{}", target_type, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![Param {
                                    name: "this".to_string(),
                                    ty: Type::Struct(target_type.clone(), vec![]),
                                    default: None,
                                    variadic: false,
                                    is_named: false,
                                    is_inout: false,
                                }],
                                return_type: Some(prop_ty.clone()),
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "setter 参数名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            methods.push(Function {
                                is_unsafe: false,
                                is_c_func: false,
                                is_const: false,
                                visibility: member_vis.clone(),
                                name: format!("{}.__set_{}", target_type, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params: vec![
                                    Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(target_type.clone(), vec![]),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                    Param {
                                        name: val_name,
                                        ty: prop_ty.clone(),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    },
                                ],
                                return_type: None,
                                throws: None,
                                body,
                                extern_import: None,
                            });
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                continue;
            }
            // cjc 兼容: extend 内方法修饰符（static / override / operator 等）已由 parse_member_modifiers 收集
            // 5.8.1(2)：扩展中的静态成员函数不能声明为 mut（官方 05_08 a01_03 负例）。
            if mods.is_mut && mods.is_static {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Mut,
                    "扩展中的静态成员函数不能声明为 mut（5.8.1(2)）".to_string(),
                ));
            }
            // 5.8.1：类型别名扩展中的成员函数不能声明为 mut（无实际成员可修改；
            // 官方 05_08 a01_05 负例）。
            if mods.is_mut && self.type_aliases.contains_key(&target_type) {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Mut,
                    "类型别名扩展中的成员函数不能声明为 mut（5.8.1）".to_string(),
                ));
            }
            if mods.is_operator {
                self.parsing_operator_func = true;
            }
            let prev_receiver = self.receiver_name.clone();
            self.receiver_name = Some("this".to_string());
            let mut func = self.parse_function_with_visibility(member_vis)?;
            func.is_const = mods.is_const;
            func.is_unsafe = mods.is_unsafe;
            self.receiver_name = prev_receiver;
            // P3: 添加隐式 this 参数（如同 struct/class 方法）
            let has_self = func
                .params
                .iter()
                .any(|p| p.name == "self" || p.name == "this");
            if !has_self && !func.name.starts_with("static ") {
                func.params.insert(
                    0,
                    crate::ast::Param {
                        name: "this".to_string(),
                        ty: Type::Struct(target_type.clone(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    },
                );
            }
            // 重命名为 TypeName.methodName 格式
            let method_name = if func.name.contains('.') {
                func.name.clone()
            } else {
                format!("{}.{}", target_type, func.name)
            };
            // 记录 static 修饰的扩展成员（10.1.1(3)/(6)）。
            if mods.is_static {
                static_method_indices.push(methods.len());
            }
            methods.push(Function {
                is_const: func.is_const,
                name: method_name,
                ..func
            });
        }
        self.expect(Token::RBrace)?;
        Ok(crate::ast::ExtendDef {
            target_type,
            type_args,
            interface,
            interface_args,
            assoc_type_bindings,
            methods,
            static_method_indices,
        })
    }

    /// 解析类定义
    /// 收集类型声明前缀修饰符：abstract / sealed / open 以及可交错的可见性修饰符
    /// （如 `abstract public class`、`open private class`）。返回三个标志位。
    pub(crate) fn parse_type_modifiers(
        &mut self,
        visibility: &mut Visibility,
    ) -> (bool, bool, bool) {
        let mut is_abstract = false;
        let mut is_sealed = false;
        let mut is_open = false;
        loop {
            if self.check(&Token::Abstract) {
                self.advance();
                is_abstract = true;
            } else if self.check(&Token::Sealed) {
                self.advance();
                is_sealed = true;
            } else if self.check(&Token::Open) {
                self.advance();
                is_open = true;
            } else if self.check(&Token::Public) {
                self.advance();
                *visibility = Visibility::Public;
            } else if self.check(&Token::Private) {
                self.advance();
                *visibility = Visibility::Private;
            } else if self.check(&Token::Protected) {
                self.advance();
                *visibility = Visibility::Protected;
            } else if self.check(&Token::Internal) {
                self.advance();
                *visibility = Visibility::Internal;
            } else {
                break;
            }
        }
        (is_abstract, is_sealed, is_open)
    }

    pub(crate) fn parse_class_with_visibility(
        &mut self,
        mut visibility: Visibility,
    ) -> Result<crate::ast::ClassDef, ParseErrorAt> {
        // 解析可选修饰符: abstract / sealed / open（任意顺序，如 sealed abstract class）
        let (is_abstract, is_sealed, is_open) = self.parse_type_modifiers(&mut visibility);
        self.parse_class_body(visibility, is_abstract, is_sealed, is_open)
    }

    /// 解析类定义主体（修饰符已由调用方消费）。
    fn parse_class_body(
        &mut self,
        visibility: Visibility,
        is_abstract: bool,
        is_sealed: bool,
        is_open: bool,
    ) -> Result<crate::ast::ClassDef, ParseErrorAt> {
        self.expect(Token::Class)?;
        let name = match self.advance_decl_name() {
            Some(n) => n,
            None => {
                let tok = self.advance().unwrap_or(Token::Semicolon);
                return self.bail(ParseError::UnexpectedToken(tok, "类名".to_string()));
            }
        };
        // 解析可选的泛型类型参数 <T, U: Bound, ...>
        let (type_params, mut constraints) = self.parse_type_params_with_constraints()?;
        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());
        // cjc: 使用 <: 表示继承 (class Foo <: Base & Interface1 & Interface2)
        let mut supertype_args: Vec<(String, Vec<Type>)> = Vec::new();
        let (extends, implements) = if self.check(&Token::SubType) {
            self.advance();
            let mut types = Vec::new();
            loop {
                // 解析类型（支持泛型，如 Iterable<Token>）
                let ty = self.parse_type()?;
                // 提取基础类型名与泛型实参
                let (type_name, type_args) = match ty {
                    Type::Struct(name, args) => (name, args),
                    Type::TypeParam(name) => (name, Vec::new()),
                    _ => {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Ident("unknown".to_string()),
                            "父类或接口名".to_string(),
                        ))
                    }
                };
                supertype_args.push((type_name.clone(), type_args));
                types.push(type_name);
                if self.check(&Token::And) {
                    self.advance();
                } else {
                    break;
                }
            }
            // 第一个为 extends（父类），其余为 implements（接口）
            if types.is_empty() {
                (None, Vec::new())
            } else {
                (Some(types[0].clone()), types[1..].to_vec())
            }
        } else {
            (None, Vec::new())
        };
        // P6: Primary constructor — class Foo(var x: Int64, var y: Int64) { ... }
        let mut primary_ctor_params = Vec::new();
        let mut primary_ctor_member_flags: Vec<bool> = Vec::new();
        // 6.1.1.4(7)：类只能定义至多一个主构造函数（`class Foo(...)` 形式或类名形式
        // `Foo(...) {}` 均计入；`init(...)` 为次构造函数，可多次定义）。
        let mut primary_seen = self.check(&Token::LParen);
        if self.check(&Token::LParen) {
            self.advance();
            while !self.check(&Token::RParen) {
                // 跳过可选的 var/let，并记录是否为成员变量参数
                let is_member = self.check(&Token::Var) || self.check(&Token::Let);
                if is_member {
                    self.advance();
                }
                let pname = match self.advance_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string()));
                    }
                };
                // cjc: 必需命名参数 name!: Type（此处仅消费 `!`，不改变调用匹配，
                // 保证默认值仍可按位置调用）
                let is_required_named = if self.check(&Token::Bang) {
                    self.advance();
                    true
                } else {
                    false
                };
                let (pty, pdefault) = if self.check(&Token::Colon) {
                    self.advance();
                    let pty = self.parse_type()?;
                    let pdefault = if self.check(&Token::Assign) {
                        self.advance();
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    (pty, pdefault)
                } else if self.check(&Token::Assign) {
                    self.advance();
                    (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Colon),
                        "`:` 或 `=`".to_string(),
                    ));
                };
                primary_ctor_params.push(Param {
                    name: pname,
                    ty: pty,
                    default: pdefault,
                    variadic: false,
                    is_named: is_required_named,
                    is_inout: false,
                });
                primary_ctor_member_flags.push(is_member);
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
            self.expect(Token::RParen)?;
        }
        // cjc: class Foo <: Base where T <: Object { ... }
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);
        // 兜底: 父类/接口名后 <T> 若未消费，此处消费（如 class Foo <: Bar<T> {）
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // 可能在消费了父类泛型参数后才出现 where 子句 (class Foo <: Bar<T> where T <: Bound { ... })
        let where_constraints2 = self.parse_where_clause()?;
        constraints.extend(where_constraints2);
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        let mut init = None;
        let mut inits: Vec<crate::ast::InitDef> = Vec::new();
        let mut deinit = None;
        let mut methods = Vec::new();
        // 静态初始化器函数体（延迟到字段表完整后校验 6.1.2.2 系列规则）。
        let mut static_init_bodies: Vec<Vec<Stmt>> = Vec::new();
        while !self.check(&Token::RBrace) {
            // 容忍成员声明后的多余分号（如 `init(...) {...};`）
            if self.check(&Token::Semicolon) {
                self.advance();
                continue;
            }
            // 成员修饰符可任意顺序组合（public static var / static public var / ...）
            let mods = self.parse_member_modifiers();
            // 6.1.2.4(44)：redef 与 override 是互斥的两种语义，不能同时书写。
            if mods.has_redef_kw && mods.has_override_kw {
                return self.bail(ParseError::UnexpectedToken(
                    Token::Redef,
                    "redef 与 override 不能同时使用（6.1.2.4(44)）".to_string(),
                ));
            }
            let member_vis = mods.visibility.clone().unwrap_or_default();
            if self.check(&Token::Var) || self.check(&Token::Let) {
                let field_is_mut = self.check(&Token::Var);
                self.advance();
                // 上下文关键字可作字段名，但 main/where/type 等保留关键字不可
                let f_name = match self.advance_binding_ident() {
                    Some(n) => n,
                    None => {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(tok, "字段名".to_string()));
                    }
                };
                // cjc: 字段确定赋值声明 name!: Type（`!` 仅作语法修饰）
                if self.check(&Token::Bang) {
                    self.advance();
                }
                // 支持 let name = expr（无类型）或 let name: Type [= expr]
                let (ty, default) = if self.check(&Token::Assign) {
                    self.advance();
                    let prev_receiver = self.receiver_name.clone();
                    self.receiver_name = Some("this".to_string());
                    let expr = self.parse_expr()?;
                    self.receiver_name = prev_receiver;
                    (crate::ast::Type::TypeParam("_".to_string()), Some(expr))
                } else if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    let default = if self.check(&Token::Assign) {
                        self.advance();
                        let prev_receiver = self.receiver_name.clone();
                        self.receiver_name = Some("this".to_string());
                        let expr = self.parse_expr()?;
                        self.receiver_name = prev_receiver;
                        Some(expr)
                    } else {
                        None
                    };
                    (ty, default)
                } else {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(
                        tok,
                        "字段: 期望 : 类型 或 = 值".to_string(),
                    ));
                };
                while self.check(&Token::Semicolon) || self.check(&Token::Comma) {
                    self.advance();
                }
                fields.push(crate::ast::FieldDef { is_const: false,
                    is_static: mods.is_static,
                    visibility: member_vis.clone(),
                    name: f_name,
                    ty,
                    default,
                    is_mut: field_is_mut,
                    decl_kw: true,
                });
            } else if self.check(&Token::Init) {
                if !mods.is_static {
                    // 6.1.2.1(34)：init 构造函数只允许至多一个可访问性修饰符；
                    // open/abstract/redef/override/sealed/operator/unsafe/mut 均非法。
                    // （`static init` 是静态初始化块，另行处理。）
                    if mods.is_open
                        || mods.is_abstract
                        || mods.is_override
                        || mods.is_sealed
                        || mods.is_operator
                        || mods.is_unsafe
                        || mods.is_mut
                        || mods.visibility_count > 1
                    {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "非法的 init 构造函数修饰符".to_string(),
                        ));
                    }
                }
                if mods.is_static {
                    // static init() { ... } — 静态初始化块，编译为 ClassName.__static_init
                    // 6.1.2.2(29)：static init 不能带可访问性修饰符。
                    if matches!(
                        &mods.visibility,
                        Some(Visibility::Public)
                            | Some(Visibility::Private)
                            | Some(Visibility::Protected)
                    ) {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Init,
                            "static 初始化器不能声明可访问性修饰符（6.1.2.2(29)）".to_string(),
                        ));
                    }
                    self.advance(); // consume init
                    self.expect(Token::LParen)?;
                    self.expect(Token::RParen)?;
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    static_init_bodies.push(body.clone());
                    methods.push(crate::ast::ClassMethod {
                        is_const: false,
                        func: Function {
                            is_unsafe: false,
                            is_c_func: false,
                            is_const: false,
                            visibility: member_vis.clone(),
                            name: format!("{}.__static_init", name),
                            type_params: vec![],
                            constraints: vec![],
                            params: vec![],
                            return_type: None,
                            throws: None,
                            body,
                            extern_import: None,
                        },
                        // 保留 redef/override 修饰符，供 sema 拒绝
                        // `redef static init()`（6.3.4.1(9)）。
                        override_: mods.is_override,
                        has_override_kw: mods.has_override_kw,
                        is_static: true,
                        is_open: false,
                        is_abstract: false,
                        has_body: true,
                        is_prop_mut: false,
                    });
                    continue;
                }
                self.advance();
                self.expect(Token::LParen)?;
                let params = self.parse_params()?;
                // 6.1.2.1(33)/(44)：init 构造函数必须与已有构造函数（含主构造函数）
                // 构成重载——参数类型序列相同即签名冲突（参数名/命名/默认值/修饰符
                // 均不影响签名）。带属性（@TestCase/@Bench 等宏）的类保守豁免。
                if !self.pending_decl_attr
                    && inits.iter().any(|prev| {
                        prev.params.len() == params.len()
                            && prev
                                .params
                                .iter()
                                .zip(params.iter())
                                .all(|(a, b)| a.ty == b.ty)
                    })
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Init,
                        "构造函数签名重复（6.1.2.1(33)）".to_string(),
                    ));
                }
                // 6.1.2.1(34)：init 构造函数不支持变长参数列表。
                if params.iter().any(|p| p.variadic) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::DotDotDot,
                        "init 构造函数不支持变长参数".to_string(),
                    ));
                }
                self.expect(Token::RParen)?;
                // init(output: T) where T <: Bound { ... }
                let _ = self.parse_where_clause()?;
                if self.check(&Token::Lt) {
                    self.advance();
                    loop {
                        let _ = self.parse_type()?;
                        if self.check(&Token::Gt) {
                            self.advance();
                            break;
                        }
                        if self.check(&Token::Comma) {
                            self.advance();
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                self.peek().cloned().unwrap_or(Token::Comma),
                                "`,` 或 `>`".to_string(),
                            ));
                        }
                    }
                }
                // init 体内 this 可用，指向当前正在构造的对象
                self.receiver_name = Some("this".to_string());
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.receiver_name = None;
                let idef = crate::ast::InitDef {
                    is_const: mods.is_const,
                    visibility: mods.visibility.clone().unwrap_or(Visibility::Internal),
                    params,
                    body,
                    member_param_flags: vec![],
                    member_param_vis: Vec::new(),
                    member_param_mut: Vec::new(),
                };
                inits.push(idef.clone());
                init = Some(idef);
            } else if self.check(&Token::Tilde) {
                // 规范 6.1.2.5(4)(5)：finalizer 不能带参数/返回类型/泛型参数/任何修饰符，
                // 且 open/abstract/sealed 类不能定义 finalizer。
                if is_open || is_abstract || is_sealed {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Tilde,
                        "open/abstract/sealed 类不能定义 finalizer".to_string(),
                    ));
                }
                if mods.visibility.is_some()
                    || mods.is_static
                    || mods.is_const
                    || mods.is_override
                    || mods.is_open
                    || mods.is_operator
                    || mods.is_unsafe
                    || mods.is_mut
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Tilde,
                        "finalizer 不能带修饰符".to_string(),
                    ));
                }
                // cjc: ~init 析构函数，规范写作 `~init()`（可带空参数括号）
                self.advance(); // consume ~
                self.expect(Token::Init)?; // consume init
                if self.check(&Token::Lt) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Lt,
                        "finalizer 不能带泛型参数".to_string(),
                    ));
                }
                if self.check(&Token::LParen) {
                    self.advance();
                    if !self.check(&Token::RParen) {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::RParen),
                            "finalizer 不能带参数".to_string(),
                        ));
                    }
                    self.expect(Token::RParen)?;
                }
                if self.check(&Token::Colon) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Colon,
                        "finalizer 不能有返回类型".to_string(),
                    ));
                }
                // 6.1.2.5(6)：一个类至多只能有一个 finalizer。
                if deinit.is_some() {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Tilde,
                        "类只能定义一个 finalizer（6.1.2.5(6)）".to_string(),
                    ));
                }
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                deinit = Some(body);
            } else if self.check(&Token::Prop) {
                // 7.1(1)：属性修饰符列表至多一个可见性修饰符。
                if mods.visibility_count > 1 {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "属性只能有一个可见性修饰符（7.1(1)）".to_string(),
                    ));
                }
                // prop name: Type { get() { ... } set(value) { ... } }
                self.advance();
                let prop_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "属性名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                self.expect(Token::Colon)?;
                let prop_ty = self.parse_type()?;
                // 无体属性声明（如 abstract class 中的 `public mut prop x: T`）：
                // 不生成具体实现，但登记抽象 getter/setter 方法，供子类 override 检查匹配。
                if !self.check(&Token::LBrace) {
                    // 7.1(1)：抽象类中的无体属性只能为 public/protected，且不能是
                    // static/open（这些修饰符仅用于有实现体的属性）。
                    if is_abstract {
                        if !matches!(member_vis, Visibility::Public | Visibility::Protected) {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Prop,
                                "抽象类中的无体属性必须声明为 public 或 protected（7.1(1)）"
                                    .to_string(),
                            ));
                        }
                        if mods.is_static || mods.is_open {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Prop,
                                "抽象类中的无体属性不能声明为 static 或 open（7.1(1)）"
                                    .to_string(),
                            ));
                        }
                    }
                    let this_param = Param {
                        name: "this".to_string(),
                        ty: Type::Struct(name.clone(), vec![]),
                        default: None,
                        variadic: false,
                        is_named: false,
                        is_inout: false,
                    };
                    methods.push(crate::ast::ClassMethod {
                        override_: false,
                        has_override_kw: false,
                        is_static: false,
                        is_open: false,
                        is_abstract: false,
                        // 无体属性声明（抽象属性）的访问器没有实现体：
                        // has_body=false 使父类型的抽象访问器被视为可覆写/可实现。
                        has_body: false,
                        is_prop_mut: mods.is_mut,
                        is_const: false,
                        func: crate::ast::Function {
                            is_unsafe: false,
                            is_c_func: false,
                            is_const: false,
                            visibility: member_vis.clone(),
                            name: format!("{}.__get_{}", name, prop_name),
                            type_params: vec![],
                            constraints: vec![],
                            params: vec![this_param.clone()],
                            return_type: Some(prop_ty.clone()),
                            throws: None,
                            body: vec![],
                            extern_import: None,
                        },
                    });
                    if mods.is_mut {
                        let mut params = vec![this_param];
                        params.push(Param {
                            name: "value".to_string(),
                            ty: prop_ty.clone(),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        });
                        methods.push(crate::ast::ClassMethod {
                            override_: false,
                            has_override_kw: false,
                            is_static: false,
                            is_open: false,
                            is_abstract: false,
                            // 无体 mut 属性的抽象 setter 同样没有实现体。
                            has_body: false,
                            is_prop_mut: mods.is_mut,
                            is_const: false,
                            func: crate::ast::Function {
                                is_unsafe: false,
                                is_c_func: false,
                                is_const: false,
                                visibility: member_vis.clone(),
                                name: format!("{}.__set_{}", name, prop_name),
                                type_params: vec![],
                                constraints: vec![],
                                params,
                                return_type: None,
                                throws: None,
                                body: vec![],
                                extern_import: None,
                            },
                        });
                    }
                    while self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    continue;
                }
                self.expect(Token::LBrace)?;
                // 解析 get/set 块
                let mut saw_get = false;
                let mut saw_set = false;
                while !self.check(&Token::RBrace) {
                    if let Some(Token::Ident(ref kw)) = self.peek() {
                        let kw = kw.clone();
                        if kw == "get" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            saw_get = true;
                            // 脱糖为 getter 方法: ClassName.__get_propName(this): Type
                            methods.push(crate::ast::ClassMethod {
                                override_: mods.is_override,
                                has_override_kw: mods.has_override_kw,
                                is_static: mods.is_static,
                                is_open: mods.is_open,
                                is_abstract: mods.is_abstract,
                                has_body: true,
                                is_prop_mut: mods.is_mut,
                                is_const: false,
                                func: crate::ast::Function {
                                    is_unsafe: false,
                                    is_c_func: false,
                                    is_const: false,
                                    visibility: member_vis.clone(),
                                    name: format!("{}.__get_{}", name, prop_name),
                                    type_params: vec![],
                                    constraints: vec![],
                                    params: vec![Param {
                                        name: "this".to_string(),
                                        ty: Type::Struct(name.clone(), vec![]),
                                        default: None,
                                        variadic: false,
                                        is_named: false,
                                        is_inout: false,
                                    }],
                                    return_type: Some(prop_ty.clone()),
                                    throws: None,
                                    body,
                                    extern_import: None,
                                },
                            });
                        } else if kw == "set" {
                            self.advance();
                            self.expect(Token::LParen)?;
                            let val_name = match self.advance() {
                                Some(Token::Ident(n)) => n,
                                Some(Token::Underscore) => "_".to_string(),
                                Some(tok) => {
                                    return self.bail(ParseError::UnexpectedToken(
                                        tok,
                                        "setter 参数名".to_string(),
                                    ))
                                }
                                None => return self.bail(ParseError::UnexpectedEof),
                            };
                            self.expect(Token::RParen)?;
                            self.receiver_name = Some("this".to_string());
                            self.expect(Token::LBrace)?;
                            let body = self.parse_stmts()?;
                            self.expect(Token::RBrace)?;
                            self.receiver_name = None;
                            saw_set = true;
                            // 脱糖为 setter 方法: ClassName.__set_propName(this, value)
                            methods.push(crate::ast::ClassMethod {
                                override_: mods.is_override,
                                has_override_kw: mods.has_override_kw,
                                is_static: mods.is_static,
                                is_open: mods.is_open,
                                is_abstract: mods.is_abstract,
                                has_body: true,
                                is_prop_mut: mods.is_mut,
                                is_const: false,
                                func: crate::ast::Function {
                                    is_unsafe: false,
                                    is_c_func: false,
                                    is_const: false,
                                    visibility: member_vis.clone(),
                                    name: format!("{}.__set_{}", name, prop_name),
                                    type_params: vec![],
                                    constraints: vec![],
                                    params: vec![
                                        Param {
                                            name: "this".to_string(),
                                            ty: Type::Struct(name.clone(), vec![]),
                                            default: None,
                                            variadic: false,
                                            is_named: false,
                                            is_inout: false,
                                        },
                                        Param {
                                            name: val_name,
                                            ty: prop_ty.clone(),
                                            default: None,
                                            variadic: false,
                                            is_named: false,
                                            is_inout: false,
                                        },
                                    ],
                                    return_type: None,
                                    throws: None,
                                    body,
                                    extern_import: None,
                                },
                            });
                        } else {
                            return self.bail(ParseError::UnexpectedToken(
                                Token::Ident(kw),
                                "get 或 set".to_string(),
                            ));
                        }
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                // 7.1(2)/(3)/(6)：mut 属性须同时定义 getter 与 setter；非 mut 属性须有
                // getter，且不能定义 setter。7.4(11)：override/redef 的 mut 属性允许
                // 仅重实现 getter 或 setter 之一，故跳过“两者兼具”的要求。
                if mods.is_mut && !mods.is_override {
                    if !saw_get || !saw_set {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Prop,
                            "mut 属性必须同时定义 getter 和 setter（7.1(3)）".to_string(),
                        ));
                    }
                } else if !mods.is_mut {
                    if saw_set {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Prop,
                            "非 mut 属性不能定义 setter（7.1(6)）".to_string(),
                        ));
                    }
                    if !saw_get {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Prop,
                            "属性必须定义 getter 实现（7.1(2)）".to_string(),
                        ));
                    }
                } else if !saw_get && !saw_set {
                    // 7.4(11)：override/redef 的 mut 属性允许只重实现 getter 或
                    // setter 之一，但必须至少重实现一个访问器。
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Prop,
                        "覆写/重定义的属性必须重实现至少一个访问器（7.4(11)）".to_string(),
                    ));
                }
            } else if {
                // 仓颉允许用类名代替 init 作为构造函数: ClassName(let param: Type) {}
                let is_named_ctor = if let Some(Token::Ident(ref n)) = self.peek() {
                    n == &name
                } else {
                    false
                };
                is_named_ctor && matches!(self.peek_next(), Some(Token::LParen))
            } {
                // 6.1.1.4(7)：类名形式的构造函数即主构造函数，至多一个。
                if primary_seen {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::LParen,
                        "类只能定义一个主构造函数（6.1.1.4(7)）".to_string(),
                    ));
                }
                primary_seen = true;
                // 6.1.2.1(2)：主构造函数只允许至多一个可访问性修饰符；
                // static/open/abstract/redef/override/sealed/operator/unsafe/mut 非法。
                if mods.is_static
                    || mods.is_open
                    || mods.is_abstract
                    || mods.is_override
                    || mods.is_sealed
                    || mods.is_operator
                    || mods.is_unsafe
                    || mods.is_mut
                    || mods.visibility_count > 1
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::LParen,
                        "非法的主构造函数修饰符".to_string(),
                    ));
                }
                self.advance(); // 消费类名
                self.expect(Token::LParen)?;
                let mut params = Vec::new();
                let mut member_param_flags: Vec<bool> = Vec::new();
                let mut member_param_vis: Vec<Visibility> = Vec::new();
                let mut member_param_mut: Vec<bool> = Vec::new();
                while !self.check(&Token::RParen) {
                    // 可选的可见性修饰符 (private/public/protected/internal)；
                    // 记录实际可见性以供 6.1.2.1(14) 校验。
                    let (has_vis, param_vis) = match self.peek() {
                        Some(Token::Private) => {
                            self.advance();
                            (true, Visibility::Private)
                        }
                        Some(Token::Public) => {
                            self.advance();
                            (true, Visibility::Public)
                        }
                        Some(Token::Protected) => {
                            self.advance();
                            (true, Visibility::Protected)
                        }
                        Some(Token::Internal) => {
                            self.advance();
                            (true, Visibility::Internal)
                        }
                        _ => (false, Visibility::Internal),
                    };
                    // 可选的 let/var 前缀，并记录是否为成员变量参数及其可变性
                    let is_var = self.check(&Token::Var);
                    let is_member = self.check(&Token::Let) || is_var;
                    if is_member {
                        self.advance();
                    }
                    // 6.1.2.1(2)：可访问性修饰符只能修饰成员变量参数（需 var/let）。
                    if has_vis && !is_member {
                        let tok = self.peek().cloned().unwrap_or(Token::RParen);
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "主构造函数参数带可访问性修饰符时必须使用 var/let".to_string(),
                        ));
                    }
                    let pname = match self.advance_ident() {
                        Some(n) => n,
                        None => {
                            let tok = self.advance().unwrap_or(Token::Semicolon);
                            return self.bail(ParseError::UnexpectedToken(
                                tok,
                                "构造函数参数名".to_string(),
                            ));
                        }
                    };
                    // cjc: 支持必需命名参数 param!: Type（! 表示调用时必须使用命名参数）
                    let is_required_named = if self.check(&Token::Bang) {
                        self.advance();
                        true
                    } else {
                        false
                    };
                    let (pty, pdefault) = if self.check(&Token::Assign) {
                        self.advance();
                        (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                    } else if self.check(&Token::Colon) {
                        self.advance();
                        let pty = self.parse_type()?;
                        let pdefault = if self.check(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        (pty, pdefault)
                    } else {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "构造函数参数: 期望 : 类型 或 = 默认值".to_string(),
                        ));
                    };
                    // 6.1.2.1(2)：`()` 是 Unit 字面量而非合法类型写法，不能用作参数类型
                    // （Unit 类型应写作 `Unit`）。
                    if matches!(&pty, Type::Tuple(v) if v.is_empty()) {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::LParen,
                            "主构造函数参数类型不能是 () 字面量".to_string(),
                        ));
                    }
                    params.push(Param {
                        name: pname,
                        ty: pty,
                        default: pdefault,
                        variadic: false,
                        is_named: is_required_named, // 必需命名参数标记为 is_named
                        is_inout: false,
                    });
                    member_param_flags.push(is_member);
                    member_param_vis.push(if is_member {
                        param_vis.clone()
                    } else {
                        Visibility::Internal
                    });
                    member_param_mut.push(is_member && is_var);
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(Token::RParen)?;
                // 6.1.2.1(33)：类名形式主构造函数同样须与已解析的 init 构造函数构成重载。
                if !self.pending_decl_attr
                    && inits.iter().any(|prev| {
                        prev.params.len() == params.len()
                            && prev
                                .params
                                .iter()
                                .zip(params.iter())
                                .all(|(a, b)| a.ty == b.ty)
                    })
                {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::LParen,
                        "构造函数签名重复（6.1.2.1(33)）".to_string(),
                    ));
                }
                let _ = self.parse_where_clause()?;
                self.receiver_name = Some("this".to_string());
                self.expect(Token::LBrace)?;
                let body = self.parse_stmts()?;
                self.expect(Token::RBrace)?;
                self.receiver_name = None;
                // 6.1.2.1(24)：主构造函数不能通过 this(...) 调用自身或其它构造函数。
                if stmts_call_this_ctor(&body) {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Init,
                        "主构造函数不能通过 this(...) 调用构造函数（6.1.2.1(24)）".to_string(),
                    ));
                }
                let idef = crate::ast::InitDef {
                    is_const: mods.is_const,
                    visibility: mods.visibility.clone().unwrap_or(Visibility::Internal),
                    params,
                    body,
                    member_param_flags,
                    member_param_vis,
                    member_param_mut,
                };
                inits.push(idef.clone());
                init = Some(idef);
            } else if self.check(&Token::At) {
                self.skip_optional_attributes()?;
                if self.last_attr_c {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::At,
                        "@C 不能修饰类成员（13.1.2.1(3)）".to_string(),
                    ));
                }
                continue; // 跳过属性后重新循环，下一次会看到 Func 等
            } else if self.check(&Token::Func) || self.check(&Token::Operator) {
                // 修饰符已由 parse_member_modifiers() 以任意顺序收集
                let is_static = mods.is_static;
                let override_ = mods.is_override;
                let is_operator = mods.is_operator;
                // 5.8.1(1)：mut 成员函数只允许定义在 interface / struct（及 struct 扩展）中，
                // 类（以及 enum）成员函数不能声明为 mut。
                if mods.is_mut {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Mut,
                        "类成员函数不能声明为 mut（5.8.1(1)）".to_string(),
                    ));
                }
                // 6.1.2.4：sealed 仅用于类型声明，不能修饰成员函数
                // （官方 03_04 a10_11 负例）。
                if mods.is_sealed {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Func,
                        "成员函数不能使用 sealed 修饰符（6.1.2.4）".to_string(),
                    ));
                }
                self.expect(Token::Func)?;
                let (m_name, type_params) = if is_operator {
                    // 运算符方法名：解析运算符 token，转为 __op_xxx
                    let op_name = self.parse_operator_name()?;
                    (format!("{}.{}", name, op_name), vec![])
                } else {
                    match self.advance() {
                        Some(Token::Ident(n)) => {
                            let tp = self.parse_type_params()?;
                            (format!("{}.{}", name, n), tp)
                        }
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "方法名".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    }
                };
                // 合并类的泛型参数与方法自身的泛型参数，使类的 T 在方法体和返回类型中可识别为 TypeParam
                let mut merged_type_params = self.current_type_params.clone();
                merged_type_params.extend(type_params.clone());
                let prev_params =
                    std::mem::replace(&mut self.current_type_params, merged_type_params);
                self.expect(Token::LParen)?;
                // 成员函数参数默认值中允许使用 this（如 func f(x!: A = this)）
                if self.receiver_name.is_none() && !is_static {
                    self.receiver_name = Some("this".to_string());
                }
                let mut params = self.parse_params()?;
                self.expect(Token::RParen)?;
                // cjc 兼容: 无 self/this 时添加隐式 this 参数（P2.4: static 方法除外）
                let has_self = params.iter().any(|p| p.name == "self" || p.name == "this");
                if !has_self && !is_static {
                    params.insert(
                        0,
                        crate::ast::Param {
                            name: "this".to_string(),
                            ty: Type::Struct(
                                name.clone(),
                                type_params
                                    .iter()
                                    .map(|t| Type::TypeParam(t.clone()))
                                    .collect(),
                            ),
                            default: None,
                            variadic: false,
                            is_named: false,
                            is_inout: false,
                        },
                    );
                }
                self.receiver_name = if is_static {
                    // 静态方法体内不允许 this/super
                    None
                } else {
                    Some(
                        params
                            .first()
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "this".to_string()),
                    )
                };
                let return_type = if self.check(&Token::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                // 可选 where 子句（如 func write<T>(v: T): Unit where T <: ToString { ... }）
                let mut method_constraints = vec![];
                while matches!(self.peek(), Some(Token::Where)) {
                    let w = self.parse_where_clause()?;
                    method_constraints.extend(w);
                    if self.check(&Token::Lt) {
                        self.advance();
                        loop {
                            let _ = self.parse_type()?;
                            if self.check(&Token::Gt) {
                                self.advance();
                                break;
                            }
                            if self.check(&Token::Comma) {
                                self.advance();
                            } else {
                                return self.bail(ParseError::UnexpectedToken(
                                    self.peek().cloned().unwrap_or(Token::Comma),
                                    "`,` 或 `>`".to_string(),
                                ));
                            }
                        }
                    }
                }
                // P2.5: 支持抽象方法（无 body，以 ; 结尾或后跟下一成员/可见性关键字）
                let no_body = self.check(&Token::Semicolon)
                    || self.check(&Token::RBrace)
                    || matches!(
                        self.peek(),
                        Some(Token::Public)
                            | Some(Token::Private)
                            | Some(Token::Protected)
                            | Some(Token::Internal)
                            | Some(Token::Override)
                            | Some(Token::Open)
                            | Some(Token::Abstract)
                            | Some(Token::Static)
                            | Some(Token::Func)
                            | Some(Token::Init)
                            | Some(Token::Tilde)
                            | Some(Token::Prop)
                            | Some(Token::Mut)
                            | Some(Token::At)
                    );
                let body = if no_body {
                    // 抽象方法或声明：无函数体
                    if self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    vec![] // 空 body
                } else {
                    self.expect(Token::LBrace)?;
                    let prev_static = self.in_static_method;
                    self.in_static_method = is_static;
                    let stmts = self.parse_stmts()?;
                    self.in_static_method = prev_static;
                    self.expect(Token::RBrace)?;
                    stmts
                };
                let has_body = !no_body;
                // 5.1：无函数体的函数声明（抽象类中的抽象函数）必须显式标注返回类型
                // （无法从函数体推断；官方 05_01_05 a01_10/a01_11 负例）。
                if !has_body && return_type.is_none() {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Func,
                        "无函数体的函数声明必须显式标注返回类型（5.1）".to_string(),
                    ));
                }
                self.receiver_name = None;
                self.current_type_params = prev_params;
                methods.push(crate::ast::ClassMethod {
                    override_,
                    has_override_kw: mods.has_override_kw,
                    is_static,
                    is_open: mods.is_open,
                    is_abstract: mods.is_abstract,
                    has_body,
                    is_prop_mut: false,
                    is_const: mods.is_const,
                    func: crate::ast::Function {
                        // 5.8 / 13.1.1(3)：成员函数可声明 unsafe，函数体即 unsafe 上下文。
                        is_unsafe: mods.is_unsafe,
                        is_c_func: false,
                        is_const: false,
                        visibility: member_vis,
                        name: m_name,
                        type_params,
                        constraints: method_constraints,
                        params,
                        return_type,
                        throws: None,
                        body,
                        extern_import: None,
                    },
                });
            } else if mods.is_const && matches!(self.peek(), Some(Token::Ident(_))) {
                // const FIELD_NAME[: Type] [= value] — 类级别常量字段
                let f_name = match self.advance() {
                    Some(Token::Ident(n)) => n,
                    Some(tok) => {
                        return self.bail(ParseError::UnexpectedToken(tok, "常量名".to_string()))
                    }
                    None => return self.bail(ParseError::UnexpectedEof),
                };
                let (ty, default) = if self.check(&Token::Assign) {
                    self.advance();
                    (
                        crate::ast::Type::TypeParam("_".to_string()),
                        Some(self.parse_expr()?),
                    )
                } else if self.check(&Token::Colon) {
                    self.advance();
                    let ty = self.parse_type()?;
                    let default = if self.check(&Token::Assign) {
                        self.advance();
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    (ty, default)
                } else {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(
                        tok,
                        "常量: 期望 : 类型 或 = 值".to_string(),
                    ));
                };
                while self.check(&Token::Semicolon) || self.check(&Token::Comma) {
                    self.advance();
                }
                // 16.1(2)：const 成员变量必须在定义处初始化（不能延迟到 init/static init）。
                if default.is_none() {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::Ident(f_name.clone()),
                        "const 成员变量必须在定义处初始化（16.1(2)）".to_string(),
                    ));
                }
                fields.push(crate::ast::FieldDef { is_const: true,
                    name: f_name,
                    ty,
                    default,
                    is_mut: false,
                    decl_kw: true,
                    is_static: mods.is_static,
                    visibility: member_vis.clone(),
                });
            } else if self.check(&Token::Const) {
                self.advance(); // consume const
                if self.check(&Token::Init) {
                    // const init() { ... } — 编译期常量构造函数
                    self.advance();
                    self.expect(Token::LParen)?;
                    let params = self.parse_params()?;
                    self.expect(Token::RParen)?;
                    let _ = self.parse_where_clause()?;
                    self.receiver_name = Some("this".to_string());
                    self.expect(Token::LBrace)?;
                    let body = self.parse_stmts()?;
                    self.expect(Token::RBrace)?;
                    self.receiver_name = None;
                    let idef = crate::ast::InitDef {
                        is_const: true,
                        visibility: Visibility::Internal,
                        params,
                        body,
                        member_param_flags: vec![],
                        member_param_vis: Vec::new(),
                        member_param_mut: Vec::new(),
                    };
                    inits.push(idef.clone());
                    init = Some(idef);
                } else {
                    // const FIELD_NAME: Type = value — 类级别常量，当作字段处理
                    let f_name = match self.advance() {
                        Some(Token::Ident(n)) => n,
                        Some(tok) => {
                            return self
                                .bail(ParseError::UnexpectedToken(tok, "常量名或 init".to_string()))
                        }
                        None => return self.bail(ParseError::UnexpectedEof),
                    };
                    let (ty, default) = if self.check(&Token::Assign) {
                        self.advance();
                        (Type::TypeParam("_".to_string()), Some(self.parse_expr()?))
                    } else if self.check(&Token::Colon) {
                        self.advance();
                        let ty = self.parse_type()?;
                        let default = if self.check(&Token::Assign) {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        (ty, default)
                    } else {
                        let tok = self.advance().unwrap_or(Token::Semicolon);
                        return self.bail(ParseError::UnexpectedToken(
                            tok,
                            "常量: 期望 : 类型 或 = 值".to_string(),
                        ));
                    };
                    if self.check(&Token::Semicolon) {
                        self.advance();
                    }
                    // 16.1(2)：const 成员变量必须在定义处初始化。
                    if default.is_none() {
                        return self.bail(ParseError::UnexpectedToken(
                            Token::Ident(f_name.clone()),
                            "const 成员变量必须在定义处初始化（16.1(2)）".to_string(),
                        ));
                    }
                    fields.push(crate::ast::FieldDef { is_const: true,
                        is_static: mods.is_static,
                        visibility: member_vis.clone(),
                        name: f_name,
                        ty,
                        default,
                        is_mut: false,
                        decl_kw: true,
                    });
                }
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Semicolon),
                    "var、let、const、init、~init 或 func".to_string(),
                ));
            }
        }
        self.expect(Token::RBrace)?;
        self.current_type_params = prev_params;
        let mut class_def = crate::ast::ClassDef {
            visibility,
            name,
            type_params,
            constraints,
            is_abstract,
            is_sealed,
            is_open,
            extends,
            implements,
            supertype_args,
            fields,
            init,
            inits,
            deinit,
            static_init: None, // P3.11: TODO - 从 class body 中解析 static init()
            methods,
            primary_ctor_params: primary_ctor_params.clone(),
            primary_ctor_member_flags: primary_ctor_member_flags.clone(),
        };
        // P6: 展开主构造函数参数为字段 + init
        if !primary_ctor_params.is_empty() {
            for p in &primary_ctor_params {
                class_def.fields.push(FieldDef { is_const: false,
                    is_static: false,
                    visibility: crate::ast::Visibility::Internal,
                    name: p.name.clone(),
                    ty: p.ty.clone(),
                    default: p.default.clone(),
                    is_mut: true,
                    decl_kw: true,
                });
            }
            if class_def.init.is_none() {
                let init_body: Vec<Stmt> = primary_ctor_params
                    .iter()
                    .map(|p| Stmt::Assign {
                        target: AssignTarget::Field {
                            object: "this".to_string(),
                            field: p.name.clone(),
                        },
                        value: Expr::Var(p.name.clone()),
                    })
                    .collect();
                let implicit = InitDef {
                    is_const: false,
                    visibility: Visibility::Internal,
                    params: primary_ctor_params,
                    body: init_body,
                    member_param_flags: primary_ctor_member_flags,
                    member_param_vis: Vec::new(),
                    member_param_mut: Vec::new(),
                };
                class_def.inits.push(implicit.clone());
                class_def.init = Some(implicit);
            }
        }
        // 6.1.2.2：static 初始化器的重复/控制流/成员访问/初始化完整性。
        if let Some(msg) = validate_static_init_blocks(
            &class_def.name,
            &class_def.fields,
            &static_init_bodies,
        ) {
            return self.bail(ParseError::UnexpectedToken(Token::Init, msg));
        }
        // 6.1.2.1(48) 由语义阶段校验（需要“是否存在按字面量/带参实例化”的全程序信息）。
        Ok(class_def)
    }

    /// 解析函数定义（支持方法名 StructName.methodName）
    pub(crate) fn parse_function(&mut self) -> Result<Function, ParseErrorAt> {
        self.parse_function_with_visibility(Visibility::default())
    }

    /// 解析函数定义（带可见性）
    pub(crate) fn parse_function_with_visibility(
        &mut self,
        visibility: Visibility,
    ) -> Result<Function, ParseErrorAt> {
        self.expect(Token::Func)?;

        let (name, type_params, mut constraints) = if self.parsing_operator_func {
            self.parsing_operator_func = false;
            let op_name = self.parse_operator_name()?;
            (op_name.to_string(), vec![], vec![])
        } else {
            match self.advance_ident() {
                Some(n) => {
                    let (tp, tc) = self.parse_type_params_with_constraints()?;
                    let full_name = if self.check(&Token::Dot) {
                        self.advance();
                        let method = match self.advance_ident() {
                            Some(m) => m,
                            None => {
                                let tok = self.advance().unwrap_or(Token::Semicolon);
                                return self
                                    .bail(ParseError::UnexpectedToken(tok, "方法名".to_string()));
                            }
                        };
                        format!("{}.{}", n, method)
                    } else {
                        n
                    };
                    (full_name, tp, tc)
                }
                None => {
                    let tok = self.advance().unwrap_or(Token::Semicolon);
                    return self.bail(ParseError::UnexpectedToken(tok, "标识符".to_string()));
                }
            }
        };

        let prev_params = std::mem::replace(&mut self.current_type_params, type_params.clone());

        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;

        let prev_receiver = self.receiver_name.clone();
        // 方法体: 当 name 含 '.' 或首参为 self/this 时，允许 body 内使用 this
        // 当从 struct/enum 内部解析方法时, prev_receiver 已预设为 "this"
        self.receiver_name = if name.contains('.') {
            params.first().map(|p| p.name.clone())
        } else if params
            .first()
            .map_or(false, |p| p.name == "self" || p.name == "this")
        {
            params.first().map(|p| p.name.clone())
        } else if prev_receiver.is_some() {
            // 保持从 struct/enum 方法体继承的 receiver（cjc 内部方法无显式 self 参数）
            prev_receiver.clone()
        } else {
            None
        };

        // cjc 没有 throws 关键字，保留为 None
        let throws: Option<String> = None;

        let return_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        // 解析可选的 where 子句
        let where_constraints = self.parse_where_clause()?;
        constraints.extend(where_constraints);

        // 兜底: where 中 Bound<T> 的 <T> 若未消费，此处消费
        if self.check(&Token::Lt) {
            self.advance();
            loop {
                let _ = self.parse_type()?;
                if self.check(&Token::Gt) {
                    self.advance();
                    break;
                }
                if self.check(&Token::Comma) {
                    self.advance();
                } else {
                    return self.bail(ParseError::UnexpectedToken(
                        self.peek().cloned().unwrap_or(Token::Comma),
                        "`,` 或 `>`".to_string(),
                    ));
                }
            }
        }
        // 兜底: 若未消费 where（如某路径未调用 parse_where_clause），此处补消费
        while matches!(self.peek(), Some(Token::Where)) {
            let w = self.parse_where_clause()?;
            constraints.extend(w);
            if self.check(&Token::Lt) {
                self.advance();
                loop {
                    let _ = self.parse_type()?;
                    if self.check(&Token::Gt) {
                        self.advance();
                        break;
                    }
                    if self.check(&Token::Comma) {
                        self.advance();
                    } else {
                        return self.bail(ParseError::UnexpectedToken(
                            self.peek().cloned().unwrap_or(Token::Comma),
                            "`,` 或 `>`".to_string(),
                        ));
                    }
                }
            }
        }
        // 无 body 的声明：@Intrinsic / extern 等后接 @、;、或下一成员/声明（可见性、func、extend 等）
        let body = if self.check(&Token::At)
            || self.check(&Token::Semicolon)
            || matches!(
                self.peek(),
                Some(Token::Public)
                    | Some(Token::Private)
                    | Some(Token::Protected)
                    | Some(Token::Internal)
                    | Some(Token::Func)
                    | Some(Token::Extend)
                    | Some(Token::Interface)
                    | Some(Token::Class)
                    | Some(Token::Struct)
                    | Some(Token::Enum)
            ) {
            if self.check(&Token::Semicolon) {
                self.advance();
            }
            vec![]
        } else {
            self.expect(Token::LBrace)?;
            let stmts = self.parse_stmts()?;
            self.expect(Token::RBrace)?;
            stmts
        };

        self.receiver_name = prev_receiver;
        self.current_type_params = prev_params;

        Ok(Function {
            is_unsafe: false,
            is_c_func: false,
            is_const: false,
            visibility,
            name,
            type_params,
            constraints,
            params,
            return_type,
            throws,
            body,
            extern_import: None,
        })
    }

    /// cjc 兼容: 解析 main() { ... } 形式的入口函数（无需 func 关键字）
    pub(crate) fn parse_main_function(&mut self, visibility: Visibility) -> Result<Function, ParseErrorAt> {
        // consume "main"
        self.advance();
        self.expect(Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(Token::RParen)?;

        let return_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(Token::LBrace)?;
        let body = self.parse_stmts()?;
        self.expect(Token::RBrace)?;

        Ok(Function {
            is_unsafe: false,
            is_c_func: false,
            is_const: false,
            visibility,
            name: "main".to_string(),
            type_params: vec![],
            constraints: vec![],
            params,
            return_type,
            throws: None,
            body,
            extern_import: None,
        })
    }

    /// 解析参数列表
    pub(crate) fn parse_params(&mut self) -> Result<Vec<Param>, ParseErrorAt> {
        let mut params = Vec::new();
        if self.check(&Token::RParen) {
            return Ok(params);
        }

        loop {
            // cjc: foreign func f(a: T, ...) — 末尾无名的变长参数占位
            if self.check(&Token::DotDotDot) {
                self.advance();
                params.push(Param {
                    name: "__va_args".to_string(),
                    ty: Type::TypeParam("_".to_string()),
                    default: None,
                    variadic: true,
                    is_named: false,
                    is_inout: false,
                });
                if self.check(&Token::Comma) {
                    self.advance();
                    if self.check(&Token::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
            // P6: inout 参数
            let is_inout = if self.check(&Token::Inout) {
                self.advance();
                true
            } else {
                false
            };
            let name = match self.advance() {
                Some(Token::Ident(name)) => name,
                Some(Token::This) => "this".to_string(),
                Some(Token::Underscore) => "_".to_string(),
                Some(Token::BacktickStringLit(StringOrInterpolated::Plain(name)))
                    if is_valid_raw_ident_text(&name) =>
                {
                    name
                }
                Some(tok) => {
                    return self.bail(ParseError::UnexpectedToken(tok, "参数名".to_string()))
                }
                None => return self.bail(ParseError::UnexpectedEof),
            };
            // P2.9: 命名参数 name!: Type = default，或 name = default（类型推断）
            let is_named = if self.check(&Token::Bang) {
                self.advance();
                true
            } else {
                false
            };
            let (ty, default, variadic) = if self.check(&Token::Colon) {
                self.advance();
                let explicit_array = self.check(&Token::TypeArray);
                let ty = self.parse_type()?;
                if explicit_array && self.in_extern_params {
                    return self.bail(ParseError::UnexpectedToken(
                        Token::TypeArray,
                        format!(
                            "外来函数的形参 '{}' 不能为 Array 类型，应使用 VArray（13.1.3.1）",
                            name
                        ),
                    ));
                }
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
                (ty, default, variadic)
            } else if self.check(&Token::Assign) {
                self.advance();
                let default = Some(self.parse_expr()?);
                (Type::TypeParam("_".to_string()), default, false)
            } else {
                return self.bail(ParseError::UnexpectedToken(
                    self.peek().cloned().unwrap_or(Token::Colon),
                    "`:` 或 `=`".to_string(),
                ));
            };
            params.push(Param {
                name,
                ty,
                default,
                variadic,
                is_named,
                is_inout,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
            // 允许末尾多余逗号：f(a: Int64, b: Bool,)
            if self.check(&Token::RParen) {
                break;
            }
        }

        Ok(params)
    }
}

/// `@C` 结构体字段类型是否违反 CType 约束（13.1.3.5(3)）。
///
/// 返回首个违规 `(结构体名, 类型描述)`。`bad_users` 为本文件中不能作为 @C
/// 字段类型的用户类型名（class/enum/interface 及非 @C 结构体）。
fn c_struct_field_violation(
    structs: &[StructDef],
    c_structs: &std::collections::HashSet<String>,
    bad_users: &std::collections::HashSet<String>,
) -> Option<(String, String)> {
    for s in structs {
        if !c_structs.contains(&s.name) {
            continue;
        }
        for f in &s.fields {
            if let Some(what) = c_bad_field_type(&f.ty, bad_users) {
                return Some((s.name.clone(), what.to_string()));
            }
        }
    }
    None
}

/// 单个 @C 结构体字段类型是否为非法 CType 形态。
///
/// 注意：`Array<T>` 与 `VArray<T, N>` 在解析阶段均降级为 `Type::Array`，
/// 而 `VArray` 是合法 CType 字段，故此处不对 `Type::Array` 报错。
fn c_bad_field_type(
    t: &Type,
    bad_users: &std::collections::HashSet<String>,
) -> Option<&'static str> {
    match t {
        Type::Rune => Some("Rune 类型"),
        Type::String => Some("String 类型"),
        Type::Range => Some("Range 类型"),
        Type::Tuple(_) => Some("元组类型"),
        Type::Function { .. } => Some("函数类型"),
        Type::TypeParam(_) => Some("类型参数"),
        // `Range<T>` 带实参时解析为 Struct("Range", ...)。
        Type::Struct(name, _) if name == "Range" => Some("Range 类型"),
        Type::Struct(name, _) if bad_users.contains(name) => {
            Some("类、枚举、接口或非 @C 结构体类型")
        }
        _ => None,
    }
}

/// 类型中是否直接出现某个 `@C` 结构体（用于 13.1.3.5(5)/(6) 元组/枚举位置限制）。
fn type_mentions_c_struct(
    t: &Type,
    c_structs: &std::collections::HashSet<String>,
    depth: usize,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match t {
        Type::Struct(name, args) => {
            if c_structs.contains(name) {
                return Some(name.clone());
            }
            args.iter()
                .find_map(|a| type_mentions_c_struct(a, c_structs, depth + 1))
        }
        Type::Tuple(ts) => ts
            .iter()
            .find_map(|a| type_mentions_c_struct(a, c_structs, depth + 1)),
        Type::Array(inner)
        | Type::Option(inner)
        | Type::Slice(inner)
        | Type::Result(inner, _) => type_mentions_c_struct(inner, c_structs, depth + 1),
        Type::Map(k, v) => type_mentions_c_struct(k, c_structs, depth + 1)
            .or_else(|| type_mentions_c_struct(v, c_structs, depth + 1)),
        Type::Function { params, ret } => params
            .iter()
            .find_map(|a| type_mentions_c_struct(a, c_structs, depth + 1))
            .or_else(|| {
                ret.as_ref()
                    .as_ref()
                    .and_then(|r| type_mentions_c_struct(r, c_structs, depth + 1))
            }),
        _ => None,
    }
}

/// 类型是否为内置 `CType` 接口。
fn is_ctype_type(t: &Type) -> bool {
    match t {
        Type::Struct(name, _) => name == "CType",
        Type::Qualified(segs) => segs.last().map_or(false, |s| s == "CType"),
        _ => false,
    }
}

// ─── 6.1.2.2 静态初始化器（static init）校验 ────────────────────────────────
//
// struct 的 static init 函数体在解析后即被丢弃，class 的 static init 也不进入
// 语义阶段的统一遍历，因此可见性/参数/重复/控制流/成员访问等规则在此实现。
// 所有检查都只覆盖官方测试明确给出的非法形态：涉及其它类型成员、跨类/跨文件
// 信息不足的一律放行（宁可漏判不可误判）。

/// 静态初始化器体内的顺序敏感事件（按语句出现顺序记录）。
enum SiEvent {
    /// 读取：`(限定类型名, 成员名)`；裸标识符时限定类型名为 None。
    Read(Option<String>, String),
    /// 赋值：`(限定类型名, 成员名)`；裸标识符时限定类型名为 None。
    Assign(Option<String>, String),
    /// `this.field` 形式的访问/赋值。
    ThisField(String),
    /// 体内声明的局部名（含 let/var/const 绑定与 for 循环变量）。
    Local(String),
}

struct SiScan {
    events: Vec<SiEvent>,
    has_return: bool,
    ctor_decl: bool,
}

fn si_scan_pattern_bindings(p: &crate::ast::Pattern, s: &mut SiScan) {    match p {
        crate::ast::Pattern::Binding(n) => s.events.push(SiEvent::Local(n.clone())),
        crate::ast::Pattern::Tuple(ps) | crate::ast::Pattern::Or(ps) => {
            for q in ps {
                si_scan_pattern_bindings(q, s);
            }
        }
        crate::ast::Pattern::Struct { fields, .. } => {
            for (_, q) in fields {
                si_scan_pattern_bindings(q, s);
            }
        }
        _ => {}
    }
}

/// 表达式是否为本类构造函数调用 `this(...)`（或 `self(...)`）。
fn expr_calls_this_ctor(e: &Expr) -> bool {
    matches!(e, Expr::Call { name, .. } if name == "this" || name == "self")
        || matches!(
            e,
            Expr::Invoke { callee, .. }
                if matches!(callee.as_ref(), Expr::Var(n) if n == "this" || n == "self")
        )
}

/// 语句列表（含嵌套块/循环体，不含嵌套函数与 lambda）中是否出现 `this(...)` 调用。
fn stmts_call_this_ctor(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Expr(e) => expr_calls_this_ctor(e),
        Stmt::Assign { value, .. }
        | Stmt::Let { value, .. }
        | Stmt::Var { value, .. }
        | Stmt::Const { value, .. } => expr_calls_this_ctor(value),
        Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Loop { body }
        | Stmt::UnsafeBlock { body } => stmts_call_this_ctor(body),
        _ => false,
    })
}

/// 实参是否为本方言对 `C() { }` 解析出的“空 lambda”形态。
fn si_args_are_empty_lambda(args: &[Expr]) -> bool {
    args.len() == 1
        && matches!(
            &args[0],
            Expr::Lambda { params, body, .. }
                if params.is_empty()
                    && matches!(body.as_ref(), Expr::Block(stmts, None) if stmts.is_empty())
        )
}

fn si_scan_target(t: &AssignTarget, type_name: &str, s: &mut SiScan) {
    match t {
        AssignTarget::Var(n) => s.events.push(SiEvent::Assign(None, n.clone())),
        AssignTarget::Field { object, field } => {
            if object == "this" {
                s.events.push(SiEvent::ThisField(field.clone()));
            } else {
                s.events.push(SiEvent::Assign(Some(object.clone()), field.clone()));
            }
        }
        AssignTarget::FieldPath { base, fields } => {
            if let Some(f) = fields.first() {
                if base == "this" {
                    s.events.push(SiEvent::ThisField(f.clone()));
                } else {
                    s.events.push(SiEvent::Assign(Some(base.clone()), f.clone()));
                }
            }
        }
        AssignTarget::Index { array, index } => {
            s.events.push(SiEvent::Read(None, array.clone()));
            si_scan_expr(index, type_name, s);
        }
        AssignTarget::Tuple(ts) => {
            for x in ts {
                si_scan_target(x, type_name, s);
            }
        }
        _ => {}
    }
}

fn si_scan_expr(e: &Expr, type_name: &str, s: &mut SiScan) {
    match e {
        Expr::Var(n) => s.events.push(SiEvent::Read(None, n.clone())),
        Expr::Field { object, field } => match object.as_ref() {
            Expr::Var(m) if m == "this" => s.events.push(SiEvent::ThisField(field.clone())),
            Expr::Var(m) => s.events.push(SiEvent::Read(Some(m.clone()), field.clone())),
            other => si_scan_expr(other, type_name, s),
        },
        // `C() { }` 在 static init 体内会被解析为「对本类类型的构造调用 + 空 lambda」，
        // 对应官方非法的“static 初始化器中定义主构造函数”形态（6.1.2.2(3)）。
        Expr::ConstructorCall { name, args, .. } | Expr::Call { name, args, .. }
            if name == type_name && si_args_are_empty_lambda(args) =>
        {
            s.ctor_decl = true;
        }
        Expr::Call { args, named_args, .. }
        | Expr::ConstructorCall { args, named_args, .. } => {
            for a in args {
                si_scan_expr(a, type_name, s);
            }
            for (_, a) in named_args {
                si_scan_expr(a, type_name, s);
            }
        }
        Expr::MethodCall {
            object,
            args,
            named_args,
            ..
        } => {
            si_scan_expr(object, type_name, s);
            for a in args {
                si_scan_expr(a, type_name, s);
            }
            for (_, a) in named_args {
                si_scan_expr(a, type_name, s);
            }
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
        } => {
            si_scan_expr(callee, type_name, s);
            for a in args {
                si_scan_expr(a, type_name, s);
            }
            for (_, a) in named_args {
                si_scan_expr(a, type_name, s);
            }
        }
        Expr::Binary { left, right, .. } => {
            si_scan_expr(left, type_name, s);
            si_scan_expr(right, type_name, s);
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::IsType { expr, .. }
        | Expr::Try(expr)
        | Expr::Some(expr)
        | Expr::Ok(expr)
        | Expr::Err(expr)
        | Expr::Throw(expr)
        | Expr::PostfixIncr(expr)
        | Expr::PostfixDecr(expr)
        | Expr::PrefixIncr(expr)
        | Expr::PrefixDecr(expr) => si_scan_expr(expr, type_name, s),
        Expr::Index { array, index } => {
            si_scan_expr(array, type_name, s);
            si_scan_expr(index, type_name, s);
        }
        Expr::SliceExpr {
            array, start, end, ..
        } => {
            si_scan_expr(array, type_name, s);
            si_scan_expr(start, type_name, s);
            si_scan_expr(end, type_name, s);
        }
        Expr::Range {
            start, end, step, ..
        } => {
            si_scan_expr(start, type_name, s);
            si_scan_expr(end, type_name, s);
            if let Some(st) = step {
                si_scan_expr(st, type_name, s);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            si_scan_expr(cond, type_name, s);
            si_scan_expr(then_branch, type_name, s);
            if let Some(e) = else_branch {
                si_scan_expr(e, type_name, s);
            }
        }
        Expr::IfLet {
            expr,
            then_branch,
            else_branch,
            ..
        } => {
            si_scan_expr(expr, type_name, s);
            si_scan_expr(then_branch, type_name, s);
            if let Some(e) = else_branch {
                si_scan_expr(e, type_name, s);
            }
        }
        Expr::Block(stmts, tail) => {
            si_scan_stmts(stmts, type_name, s);
            if let Some(t) = tail {
                si_scan_expr(t, type_name, s);
            }
        }
        Expr::Tuple(items) | Expr::Array(items) => {
            for it in items {
                si_scan_expr(it, type_name, s);
            }
        }
        Expr::Match { expr, arms } => {
            si_scan_expr(expr, type_name, s);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    si_scan_expr(g, type_name, s);
                }
                si_scan_expr(&arm.body, type_name, s);
            }
        }
        Expr::TryBlock {
            resources,
            body,
            catch_body,
            finally_body,
            ..
        } => {
            for (_, x) in resources {
                si_scan_expr(x, type_name, s);
            }
            si_scan_stmts(body, type_name, s);
            si_scan_stmts(catch_body, type_name, s);
            if let Some(fb) = finally_body {
                si_scan_stmts(fb, type_name, s);
            }
        }
        Expr::Spawn { body } => si_scan_stmts(body, type_name, s),
        Expr::Synchronized { lock, body } => {
            si_scan_expr(lock, type_name, s);
            si_scan_stmts(body, type_name, s);
        }
        Expr::NullCoalesce { option, default } => {
            si_scan_expr(option, type_name, s);
            si_scan_expr(default, type_name, s);
        }
        Expr::Interpolate(parts) => {
            for p in parts {
                if let crate::ast::InterpolatePart::Expr(x) = p {
                    si_scan_expr(x, type_name, s);
                }
            }
        }
        Expr::MapLiteral { entries } => {
            for (k, v) in entries {
                si_scan_expr(k, type_name, s);
                si_scan_expr(v, type_name, s);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                si_scan_expr(v, type_name, s);
            }
        }
        Expr::TrailingClosure { callee, args, .. } => {
            // 尾随闭包体是新的函数作用域，不扫描。
            si_scan_expr(callee, type_name, s);
            for a in args {
                si_scan_expr(a, type_name, s);
            }
        }
        // 嵌套 lambda：独立函数作用域，return/赋值不计入静态初始化器。
        Expr::Lambda { .. } => {}
        _ => {}
    }
}

fn si_scan_stmts(stmts: &[Stmt], type_name: &str, s: &mut SiScan) {
    let mut prev_self_call = false;
    for st in stmts {
        let prev = prev_self_call;
        prev_self_call = false;
        match st {
            Stmt::Let { pattern, value, .. } | Stmt::Var { pattern, value, .. } => {
                si_scan_pattern_bindings(pattern, s);
                si_scan_expr(value, type_name, s);
            }
            Stmt::Const { name, value, .. } => {
                s.events.push(SiEvent::Local(name.clone()));
                si_scan_expr(value, type_name, s);
            }
            Stmt::Assign { target, value } => {
                si_scan_target(target, type_name, s);
                si_scan_expr(value, type_name, s);
            }
            Stmt::Expr(e) => {
                // `C() { }` 在语句层是“主构造函数声明”的非法形态（6.1.2.2(3)）：
                // 解析为「对本类类型的无参调用」紧跟一个块语句。
                if prev {
                    if let Expr::Block(..) = e {
                        s.ctor_decl = true;
                    }
                }
                prev_self_call = matches!(
                    e,
                    Expr::Call { name, args, named_args, .. }
                        if name == type_name && args.is_empty() && named_args.is_empty()
                );
                si_scan_expr(e, type_name, s);
            }
            Stmt::Return(v) => {
                s.has_return = true;
                if let Some(e) = v {
                    si_scan_expr(e, type_name, s);
                }
            }
            Stmt::While { cond, body } => {
                si_scan_expr(cond, type_name, s);
                si_scan_stmts(body, type_name, s);
            }
            Stmt::WhileLet { expr, body, .. } => {
                si_scan_expr(expr, type_name, s);
                si_scan_stmts(body, type_name, s);
            }
            Stmt::DoWhile { body, cond } => {
                si_scan_stmts(body, type_name, s);
                si_scan_expr(cond, type_name, s);
            }
            Stmt::For {
                var,
                iterable,
                body,
            } => {
                s.events.push(SiEvent::Local(var.clone()));
                si_scan_expr(iterable, type_name, s);
                si_scan_stmts(body, type_name, s);
            }
            Stmt::Loop { body } => si_scan_stmts(body, type_name, s),
            Stmt::UnsafeBlock { body } => si_scan_stmts(body, type_name, s),
            Stmt::Assert { left, right, .. } | Stmt::Expect { left, right, .. } => {
                si_scan_expr(left, type_name, s);
                si_scan_expr(right, type_name, s);
            }
            // 局部函数：独立函数作用域。
            Stmt::LocalFunc(_) => {}
            _ => {}
        }
    }
}

/// 校验类型的全部 static 初始化器（同一类型至多一个 `bodies` 项合法）。
/// 返回第一个错误消息；无错误返回 None。
fn validate_static_init_blocks(
    type_name: &str,
    fields: &[crate::ast::FieldDef],
    bodies: &[Vec<Stmt>],
) -> Option<String> {
    if bodies.is_empty() {
        return None;
    }
    // 6.1.2.2(6)：一个类型至多只能有一个 static 初始化器。
    if bodies.len() > 1 {
        return Some(format!(
            "类型 '{}' 不能有多个 static 初始化器（6.1.2.2(6)）",
            type_name
        ));
    }
    let mut scan = SiScan {
        events: Vec::new(),
        has_return: false,
        ctor_decl: false,
    };
    si_scan_stmts(&bodies[0], type_name, &mut scan);
    // 6.1.2.2(23)：static 初始化器中不允许 return（带值/不带值均非法）。
    if scan.has_return {
        return Some(format!(
            "类型 '{}' 的 static 初始化器中不允许 return（6.1.2.2(23)）",
            type_name
        ));
    }
    // 6.1.2.2(3)：static 初始化器中不能定义主构造函数。
    if scan.ctor_decl {
        return Some(format!(
            "类型 '{}' 的 static 初始化器中不能定义主构造函数（6.1.2.2(3)）",
            type_name
        ));
    }
    let locals: std::collections::HashSet<&str> = scan
        .events
        .iter()
        .filter_map(|e| match e {
            SiEvent::Local(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let find = |n: &str| fields.iter().find(|f| f.name == n);
    let mut initialized: std::collections::HashSet<&str> = fields
        .iter()
        .filter(|f| f.is_static && f.default.is_some())
        .map(|f| f.name.as_str())
        .collect();
    let mut immutable_assigns: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for ev in &scan.events {
        match ev {
            SiEvent::ThisField(f) => {
                return Some(format!(
                    "类型 '{}' 的 static 初始化器中不能访问实例成员 'this.{}'（6.1.2.2(25)）",
                    type_name, f
                ));
            }
            SiEvent::Read(qual, n) => {
                if let Some(q) = qual {
                    // 其它类型（含父类）的成员：跨类信息在此不可得，保守放行。
                    if q != type_name {
                        continue;
                    }
                } else if locals.contains(n.as_str()) {
                    continue;
                }
                if let Some(f) = find(n) {
                    if !f.is_static {
                        if qual.is_none() {
                            return Some(format!(
                                "类型 '{}' 的 static 初始化器中不能访问实例成员 '{}'（6.1.2.2(25)）",
                                type_name, n
                            ));
                        }
                    } else if f.default.is_none() && !initialized.contains(n.as_str()) {
                        return Some(format!(
                            "类型 '{}' 的 static 初始化器在初始化前读取静态成员 '{}'（6.1.2.2(25)）",
                            type_name, n
                        ));
                    }
                }
            }
            SiEvent::Assign(qual, n) => {
                if let Some(q) = qual {
                    if q != type_name {
                        continue;
                    }
                }
                if let Some(f) = find(n) {
                    if !f.is_static {
                        if qual.is_none() {
                            return Some(format!(
                                "类型 '{}' 的 static 初始化器中不能给实例成员 '{}' 赋值（6.1.2.2(25)）",
                                type_name, n
                            ));
                        }
                    } else if !f.is_mut {
                        // 注：不可变静态成员“已有直接初始值又在 static init 中赋值”的
                        // 6.1.2.2(19) 判定保留给语义阶段（类）处理，此处只拒绝重复赋值。
                        let c = immutable_assigns.entry(f.name.as_str()).or_insert(0);
                        *c += 1;
                        if *c > 1 {
                            return Some(format!(
                                "类型 '{}' 的不可变静态成员 '{}' 不能在 static 初始化器中多次赋值（6.1.2.2(20)）",
                                type_name, n
                            ));
                        }
                    }
                    initialized.insert(f.name.as_str());
                }
            }
            SiEvent::Local(_) => {}
        }
    }
    // 6.1.2.2(21)：存在 static 初始化器时，没有直接初始值的静态成员必须被赋值。
    if let Some(f) = fields
        .iter()
        .find(|f| f.is_static && f.default.is_none() && !initialized.contains(f.name.as_str()))
    {
        return Some(format!(
            "类型 '{}' 的静态成员变量 '{}' 既没有初始值，也没有在 static 初始化器中赋值（6.1.2.2(21)）",
            type_name, f.name
        ));
    }
    None
}

#[cfg(test)]
mod decl_tests {
    use crate::pipeline::parse_source;

    fn ok(src: &str) {
        if let Err(e) = parse_source(src) {
            panic!("parse failed: {}\nsource:\n{}", e, src);
        }
    }

    #[test]
    fn test_class_interface_modifier_any_order() {
        // 修饰符可任意交错：abstract public / open public / sealed interface
        ok(r#"
abstract class A {}
public abstract class B {}
abstract public class C {}
open public class D {}
public open class E {}
abstract public open class F {}
sealed interface I1 {}
public sealed interface I2 {}
sealed interface I3<T> {}
main() {}
"#);
    }

    #[test]
    fn test_finalizer_with_empty_parens() {
        ok(r#"
class C {
    let id = 1
    init() {}
    ~init() {}
}
main() {}
"#);
    }

    #[test]
    fn test_extend_keyword_target_types() {
        ok(r#"
extend Unit { func f() {} }
extend Nothing { func g() {} }
extend Rune { func h() {} }
main() {}
"#);
    }

    #[test]
    fn test_type_alias_type_keyword_names() {
        ok(r#"
type Tuple = (Int64, Int64)
type Array = Array<Int64>
type Range = Range<Int64>
main() {}
"#);
    }

    #[test]
    fn test_contextual_keyword_field_names() {
        // 规范 3.3.1(24)：上下文关键字可不加反引号作字段名
        ok(r#"
class MyClass {
    static public let abstract = false
    public let open: UInt16 = 127
    static private let override = 1
    private let protected: Float32 = 1.0
    static let redef = "A"
}
main() {}
"#);
    }

    #[test]
    fn test_backtick_ident_as_class_and_binding() {
        ok(r#"
class `__` {}
main() {
    let `_5` = 1
    var `open` = 2
}
"#);
    }

    #[test]
    fn test_trailing_param_comma_and_foreign_varargs() {
        ok(r#"
foreign func printf(fmt: CString, ...): Int32
class C {
    init(a: String, b: Bool, c!: Int64,) {}
}
main() {}
"#);
    }

    #[test]
    fn test_grouped_import() {
        ok(r#"
import {utils.macros.*, utils.assert.Assert}
main() {}
"#);
    }

    #[test]
    fn test_try_with_typed_resources() {
        ok(r#"
class R {
    init() {}
}
main() {
    try (r: R = R(), s = R()) {
        let x = 1
    }
}
"#);
    }

    #[test]
    fn test_super_generic_method_call() {
        ok(r#"
open class A {
    func f2<T>(x: T): Int64 { 1 }
}
class B <: A {
    func g() {
        let a = super.f2<Int64>(1)
    }
}
main() {}
"#);
    }

    #[test]
    fn test_extend_multiple_interfaces() {
        ok(r#"
interface I1 {}
interface I2<T> {}
extend Int64 <: I1 & I2<Int64> {}
extend Unit <: I1 & I2<Rune> {}
main() {}
"#);
    }

    fn err(src: &str) {
        assert!(parse_source(src).is_err(), "expected parse error, got ok:\n{}", src);
    }

    #[test]
    fn test_open_class_cannot_have_finalizer() {
        err("open class C { ~init() {} }\nmain() {}");
        err("public open class C { ~init() {} }\nmain() {}");
        // 非 open 类允许 finalizer
        ok("class C { ~init() {} }\nmain() {}");
    }

    #[test]
    fn test_reserved_keyword_rejected_as_field_name() {
        err("class C { let main = 1 }\nmain() {}");
        err("class C { let where = 1 }\nmain() {}");
        err("class C { let type = 1 }\nmain() {}");
        err("class C { let _ = 1 }\nmain() {}");
    }

    #[test]
    fn test_keywords_rejected_as_alias_and_class_names() {
        err("type Unit = Int64\nmain() {}");
        err("type Float64 = Int64\nmain() {}");
        err("type Nothing = Int64\nmain() {}");
        err("class main {}\nmain() {}");
        err("class _ {}\nmain() {}");
        err("main() { let ` ` = 1 }");
    }

    #[test]
    fn test_top_level_tuple_const() {
        ok(r#"
public let (x, y): (Int8, Float16) = (12, 1.0)
main() {}
"#);
    }

    #[test]
    fn test_enum_property_preserved_and_desugared() {
        // 枚举体中的 prop 声明应保留到 EnumDef.props，并脱糖为 __get_/__set_ 方法。
        let program = parse_source(
            r#"
interface I { mut prop p: Int64 { get() { 0 } set(v) {} } }
enum A <: I {
    E
    public mut prop p: Int64 {
        get() { 1 }
        set(v) {}
    }
}
main() {}
"#,
        )
        .expect("parse ok");
        let en = program.enums.iter().find(|e| e.name == "A").expect("enum A");
        assert_eq!(en.props.len(), 1);
        assert_eq!(en.props[0].name, "p");
        assert!(en.props[0].is_mut);
        let names: Vec<&str> = program.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"A.__get_p"), "missing getter in {:?}", names);
        assert!(names.contains(&"A.__set_p"), "missing setter in {:?}", names);
    }

    #[test]
    fn test_interface_abstract_mut_prop_emits_setter() {
        // 抽象 `mut prop` 应同时脱糖出抽象 setter，供 mut 一致性检查使用。
        let program = parse_source(
            r#"
interface I { static mut prop p: Int64 }
main() {}
"#,
        )
        .expect("parse ok");
        let iface = program.interfaces.iter().find(|i| i.name == "I").expect("iface I");
        assert!(iface.methods.iter().any(|m| m.name == "__get_p"));
        assert!(iface.methods.iter().any(|m| m.name == "__set_p" && m.is_static));
    }

    #[test]
    fn test_interface_prop_static_modifier_recorded() {
        // 接口属性前的 static/mut 修饰符不应丢失。
        let program = parse_source(
            r#"
interface I { static prop p: Int64 }
main() {}
"#,
        )
        .expect("parse ok");
        let iface = program.interfaces.iter().find(|i| i.name == "I").expect("iface I");
        let getter = iface.methods.iter().find(|m| m.name == "__get_p").expect("getter");
        assert!(getter.is_static, "getter should be static");
    }

    #[test]
    fn test_class_static_const_field_is_static() {
        // 类内 `static const` 字段的 static 修饰符不应在解析时丢失
        // （6.1.2(4)：实例成员不可经类型名访问，反之静态常量必须可按 `C.d` 访问）。
        let program = parse_source(
            r#"
class C { static const d = (1, 2) }
main() {}
"#,
        )
        .expect("parse ok");
        let cls = program.classes.iter().find(|c| c.name == "C").expect("class C");
        let f = cls.fields.iter().find(|f| f.name == "d").expect("field d");
        assert!(f.is_static, "static const field should be static");
        assert!(!f.is_mut, "const field should be immutable");
    }

    /// 仅解析（不跑语义校验），便于精确断言解析期规则。返回错误消息文本。
    fn parse_only(src: &str) -> Result<crate::ast::Program, String> {
        let lexer = crate::lexer::Lexer::new(src);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
        let mut parser = crate::parser::Parser::new(tokens).with_source(src);
        parser
            .parse_program()
            .map_err(|e| format!("{:?}", e))
    }

    fn expect_parse_err_contains(src: &str, needle: &str) {
        match parse_only(src) {
            Ok(_) => panic!("expected parse error containing {:?}, got Ok\n{}", needle, src),
            Err(msg) => assert!(
                msg.contains(needle),
                "expected error containing {:?}, got {:?}\n{}",
                needle,
                msg,
                src
            ),
        }
    }

    // ---- @C 属性目标限制（13.1.2.1(3)）----

    #[test]
    fn test_w12d_c_attr_on_class_rejected() {
        expect_parse_err_contains("@C\nclass A {}\nmain() {}", "@C");
    }

    #[test]
    fn test_w12d_c_attr_on_enum_interface_extend_rejected() {
        expect_parse_err_contains("@C\nenum E { A }\nmain() {}", "@C");
        expect_parse_err_contains("@C\ninterface I {}\nmain() {}", "@C");
        expect_parse_err_contains("@C\nextend Int64 {}\nmain() {}", "@C");
    }

    #[test]
    fn test_w12d_c_attr_on_type_alias_and_var_rejected() {
        expect_parse_err_contains("@C\ntype MyInt = Int64\nmain() {}", "@C");
        expect_parse_err_contains("@C\nvar x = 1\nmain() {}", "@C");
        expect_parse_err_contains("@C\nlet y: Int64 = 1\nmain() {}", "@C");
    }

    #[test]
    fn test_w12d_c_attr_on_generic_func_and_struct_rejected() {
        expect_parse_err_contains("@C\nfunc f<T>(x: T): T { return x }\nmain() {}", "@C");
        expect_parse_err_contains("@C\nstruct S<T> {}\nmain() {}", "@C");
    }

    #[test]
    fn test_w12d_c_attr_on_macro_and_class_member_rejected() {
        expect_parse_err_contains("@C\nmacro M() { 1 }\nmain() {}", "@C");
        // 类/接口/扩展成员上的 @C 同样非法。
        expect_parse_err_contains("class A { @C func f() {} }\nmain() {}", "@C");
        expect_parse_err_contains("interface I { @C func f() }\nmain() {}", "@C");
        expect_parse_err_contains(
            "class A {}\nextend A { @C func f() {} }\nmain() {}",
            "@C",
        );
    }

    #[test]
    fn test_w12d_c_attr_on_func_foreign_struct_allowed() {
        // 合法目标：非泛型顶层函数、foreign 函数、非泛型顶层结构体。
        parse_only("@C\nfunc f(x: Int32): Int32 { return x }\nmain() {}")
            .expect("@C func should parse");
        parse_only("@C\nforeign func g(x: Int32): Unit\nmain() {}")
            .expect("@C foreign func should parse");
        parse_only("@C\nstruct S { var x: Int32 }\nmain() {}")
            .expect("@C struct should parse");
        parse_only("@C\npublic func f(x: Int32): Int32 { return x }\nmain() {}")
            .expect("@C public func should parse");
    }

    // ---- @C 结构体/函数 CType 约束（13.1.3.5(3)/13.1.3.5(8)/13.1.3.5(4)/13.1.4(4)）----

    #[test]
    fn test_w12d_c_struct_bad_field_types_rejected() {
        expect_parse_err_contains("@C\nstruct S { var f: String }\nmain() {}", "@C 结构体");
        expect_parse_err_contains("@C\nstruct S { var f: Rune }\nmain() {}", "@C 结构体");
        expect_parse_err_contains("@C\nstruct S { var f: (Int32, Int32) }\nmain() {}", "@C 结构体");
        expect_parse_err_contains("@C\nstruct S { var f: Range<Int32> }\nmain() {}", "@C 结构体");
        expect_parse_err_contains(
            "@C\nstruct S { var f: (Int32)->Int32 }\nmain() {}",
            "@C 结构体",
        );
    }

    #[test]
    fn test_w12d_c_struct_non_c_user_field_rejected() {
        // 非 @C 结构体 / class / enum / interface 作为 @C 结构体字段均非法。
        expect_parse_err_contains(
            "struct N { var x: Int32 }\n@C\nstruct S { var f: N }\nmain() {}",
            "@C 结构体",
        );
        expect_parse_err_contains(
            "class N {}\n@C\nstruct S { var f: N }\nmain() {}",
            "@C 结构体",
        );
        expect_parse_err_contains(
            "enum N { A }\n@C\nstruct S { var f: N }\nmain() {}",
            "@C 结构体",
        );
        expect_parse_err_contains(
            "interface N {}\n@C\nstruct S { var f: N }\nmain() {}",
            "@C 结构体",
        );
    }

    #[test]
    fn test_w12d_c_struct_allowed_field_types() {
        // CPointer / CString / 嵌套 @C 结构体 / VArray 均为合法字段类型。
        parse_only(
            "@C\nstruct Inner { var x: Int32 }\n@C\nstruct S { var a: CPointer<Int32>\n var b: CString\n var c: Inner\n var d: VArray<Int32, $3> }\nmain() {}",
        )
        .expect("allowed @C struct fields should parse");
    }

    #[test]
    fn test_w12d_c_struct_implements_interface_rejected() {
        expect_parse_err_contains(
            "interface I {}\n@C\nstruct S <: I { var x: Int32 }\nmain() {}",
            "@C 结构体",
        );
    }

    #[test]
    fn test_w12d_c_func_ctype_param_or_return_rejected() {
        expect_parse_err_contains("@C\nfunc f(x: CType): Unit {}\nmain() {}", "CType");
        expect_parse_err_contains("@C\nfunc f(x: Int32): CType { return x }\nmain() {}", "CType");
    }

    // ---- 扩展语义（8.2/8.6）----

    #[test]
    fn test_w12d_extend_member_open_override_redef_rejected() {
        expect_parse_err_contains("class A {}\nextend A { open func f() {} }\nmain() {}", "open");
        expect_parse_err_contains(
            "class A {}\nextend A { override func f() {} }\nmain() {}",
            "override",
        );
        expect_parse_err_contains(
            "class A {}\nextend A { redef func f() {} }\nmain() {}",
            "redef",
        );
        expect_parse_err_contains(
            "class A {}\nextend A { open prop p: Int32 { get() { 1 } } }\nmain() {}",
            "open",
        );
    }

    #[test]
    fn test_w12d_public_extend_rejected() {
        expect_parse_err_contains("public extend Int64 {}\nmain() {}", "可见性");
    }

    #[test]
    fn test_w12d_plain_extend_allowed() {
        parse_only("class A {}\nextend A { func f() {} }\nmain() {}")
            .expect("plain extend should parse");
        parse_only("class A {}\nextend A { public func f() {} }\nmain() {}")
            .expect("public member in extend should parse");
    }

    // ---- 7.1(4)：内建/枚举类型不能定义 mut 属性 ----

    #[test]
    fn test_w12d_builtin_mut_prop_in_extend_rejected() {
        expect_parse_err_contains(
            "extend Int64 { mut prop val: Int64 { get() { 0 } set(v) { } } }\nmain() {}",
            "mut 属性",
        );
        expect_parse_err_contains(
            "extend Bool { mut prop val: Bool { get() { true } set(v) { } } }\nmain() {}",
            "mut 属性",
        );
        expect_parse_err_contains(
            "enum E { A }\nextend E { mut prop val: Int64 { get() { 0 } set(v) { } } }\nmain() {}",
            "mut 属性",
        );
        expect_parse_err_contains(
            "extend Option<T> { mut prop val: Int64 { get() { 0 } set(v) { } } }\nmain() {}",
            "mut 属性",
        );
    }

    #[test]
    fn test_w12d_builtin_immutable_prop_in_extend_allowed() {
        // 7.1(4) 仅限制 mut 属性；不可变属性在数值类型上合法。
        parse_only("extend Int64 { prop val: Int64 { get() { 0 } } }\nmain() {}")
            .expect("immutable prop on Int64 should parse");
        parse_only("extend Float64 { prop val: Int64 { get() { 0 } } }\nmain() {}")
            .expect("immutable prop on Float64 should parse");
    }

    // ---- w13c：@C 函数/结构体与 foreign func 的 CType 细化（13.1.2.2/13.1.3.x）----

    #[test]
    fn test_w13c_c_func_named_or_default_param_rejected() {
        expect_parse_err_contains(
            "@C\nfunc f(x: Int32, y!: Int32): Int32 { x }\nmain() {}",
            "命名参数",
        );
        expect_parse_err_contains(
            "@C\nfunc f(x: Int32 = 1): Int32 { x }\nmain() {}",
            "默认值",
        );
        // `@C foreign func` 同样受 @C 函数约束。
        expect_parse_err_contains(
            "@C\nforeign func f(x: Int32, y!: Int32): Int32\nmain() {}",
            "命名参数",
        );
        // 普通函数仍允许命名参数/默认值。
        parse_only("func f(x: Int32 = 1): Int32 { x }\nmain() {}")
            .expect("plain func default param should parse");
    }

    #[test]
    fn test_w13c_c_struct_explicit_array_field_rejected() {
        expect_parse_err_contains(
            "@C\nstruct S { var f: Array<Int32> = [] }\nmain() {}",
            "Array",
        );
        // VArray 字段合法：二者在 AST 中均降级为 Array，按源级拼写区分。
        parse_only("@C\nstruct S { var f: VArray<Int32, $3> = [] }\nmain() {}")
            .expect("VArray field in @C struct should parse");
        // 非 @C 结构体的 Array 字段不受限制。
        parse_only("struct S { var f: Array<Int32> = [] }\nmain() {}")
            .expect("Array field in plain struct should parse");
    }

    #[test]
    fn test_w13c_foreign_array_and_varray_rules() {
        // 13.1.3.1：外来函数形参不能为 Array（VArray 合法），返回类型二者皆不可。
        expect_parse_err_contains(
            "foreign func f(x: Array<Int32>): Int32\nmain() {}",
            "Array",
        );
        expect_parse_err_contains(
            "foreign func f(): Array<Int32>\nmain() {}",
            "Array",
        );
        expect_parse_err_contains(
            "foreign func f(): VArray<Int32, $3>\nmain() {}",
            "Array/VArray",
        );
        parse_only("foreign func f(x: VArray<Int32, $3>): Int32\nmain() {}")
            .expect("VArray parameter in foreign func should parse");
    }

    #[test]
    fn test_w13c_cfunc_array_signature_rules() {
        // 13.1.3.2：CFunc 签名不允许显式 Array；VArray 可作形参但不能作返回类型。
        expect_parse_err_contains(
            "func f(cb: CFunc<(Array<Int32>)->Int64>) {}\nmain() {}",
            "CFunc",
        );
        expect_parse_err_contains(
            "func f(cb: CFunc<(Int32)->Array<Int32>>) {}\nmain() {}",
            "CFunc",
        );
        expect_parse_err_contains(
            "func f(cb: CFunc<(Int32)->VArray<Int32, $3>>) {}\nmain() {}",
            "VArray",
        );
        parse_only("func f(cb: CFunc<(VArray<Int8, $3>)->Int64>) {}\nmain() {}")
            .expect("VArray parameter in CFunc signature should parse");
    }

    #[test]
    fn test_w13c_c_struct_tuple_enum_extend_rejected() {
        // 13.1.3.5(5)：@C 结构体不能作为元组元素类型。
        expect_parse_err_contains(
            "@C\nstruct ABC { var x: Int32 = 0 }\nmain() { var t: (Int32, ABC) = (1, ABC()) }",
            "元组元素",
        );
        // 13.1.3.5(6)：@C 结构体不能作为枚举关联值类型。
        expect_parse_err_contains(
            "@C\nstruct ABC { var x: Int32 = 0 }\nenum E { First(ABC) }\nmain() {}",
            "枚举关联值",
        );
        // 13.1.3.5(4)：@C 结构体不能通过 extend 实现接口。
        expect_parse_err_contains(
            "interface I {}\n@C\nstruct ABC { var x: Int32 = 0 }\nextend ABC <: I {}\nmain() {}",
            "extend",
        );
        // 正例：普通结构体可用于元组/枚举/接口实现。
        parse_only("struct ABC { var x: Int32 = 0 }\nmain() { var t: (Int32, ABC) = (1, ABC()) }")
            .expect("plain struct tuple element should parse");
        parse_only("struct ABC { var x: Int32 = 0 }\nenum E { First(ABC) }\nmain() {}")
            .expect("plain struct enum payload should parse");
    }

    // ─── w15a 06_class_and_interface/01_class ────────────────────────────────

    #[test]
    fn test_w15a_static_init_visibility_rejected() {
        // 6.1.2.2(29)：static init 不能带可访问性修饰符（class/abstract/generic/struct）。
        err("class C {\n private static init() {}\n}\nmain() {}");
        err("class C {\n public static init() {}\n}\nmain() {}");
        err("class C {\n protected static init() {}\n}\nmain() {}");
        err("abstract class A {\n private static init() {}\n}\nmain() {}");
        err("class G<T> {\n public static init() {}\n}\nmain() {}");
        err("struct S {\n public static init() {}\n}\nmain() {}");
        // 正例：无修饰符的 static init。
        ok("class C {\n static init() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_duplicate_rejected() {
        // 6.1.2.2(6)：至多一个 static 初始化器。
        err("class C {\n static init() {}\n static init() {}\n}\nmain() {}");
        err("struct S {\n static init() {}\n static init() {}\n}\nmain() {}");
        ok("class C {\n static init() {}\n init() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_return_rejected() {
        // 6.1.2.2(23)：static init 中不允许 return（有值/无值均非法）。
        err("class C {\n static init() { return }\n}\nmain() {}");
        err("class C {\n static init() { return 1 }\n}\nmain() {}");
        err("struct S {\n static init() { return }\n}\nmain() {}");
        ok("class C {\n static init() { let f = { => 1 }\n f()\n }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_instance_access_rejected() {
        // 6.1.2.2(25)：static init 中不能访问实例成员。
        err("class C {\n let x = 1\n static init() { let y = x\n }\n}\nmain() {}");
        err("class C {\n var x: Int64 = 0\n static init() { x = 1\n }\n}\nmain() {}");
        err("class C {\n let x = 1\n static init() { let y = this.x\n }\n}\nmain() {}");
        // 正例：通过局部对象访问实例成员是允许的。
        ok("class C {\n var x: Int64 = 0\n static init() { let o = C()\n let y = o.x\n }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_uninitialized_static_rejected() {
        // 6.1.2.2(21)：无初始值的静态成员必须由 static init 赋值。
        err("class C {\n static let x: Int64\n static init() {}\n}\nmain() {}");
        err("class C {\n static var x: Int64\n static init() {}\n}\nmain() {}");
        err("struct S {\n static let x: Int64\n static init() {}\n}\nmain() {}");
        ok("class C {\n static let x: Int64\n static init() { x = 1 }\n}\nmain() {}");
        ok("class C {\n static let x = 1\n static init() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_immutable_assign_rejected() {
        // 6.1.2.2(20)：不可变静态成员不能在 static init 中重复赋值。
        err("class C {\n static let x: Int64\n static init() { x = 1\n x = 2 }\n}\nmain() {}");
        ok("class C {\n static var x = 0\n static init() { x = 1 }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_primary_ctor_decl_rejected() {
        // 6.1.2.2(3)：static init 中不能定义主构造函数形态 `C() { }`。
        err("class C {\n static init() { C() { } }\n}\nmain() {}");
        err("struct S {\n static init() { S() { } }\n}\nmain() {}");
        ok("class C {\n static init() { let o = C() }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_static_init_read_before_init_rejected() {
        // 6.1.2.2(25)：不能在本体内初始化前读取静态成员。
        err("class C {\n static let a: Int64\n static var c: Int64\n static init() { C.c\n c = 1\n a = 1 }\n}\nmain() {}");
        ok("class C {\n static let a: Int64\n static var c: Int64\n static init() { c = 1\n let y = C.c\n a = 1 }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_struct_static_init_params_rejected() {
        // 6.1.2.2(22)/(3)：struct static init 不能带参数。
        err("struct S {\n static init(x: Int64) {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_ctor_signature_duplicate_rejected() {
        // 6.1.2.1(33)：次构造函数不得与主构造函数签名重复。
        err("class A {\n A() {}\n init() {}\n}\nmain() {}");
        err("class A {\n A(x: Int64) {}\n init(x: Int64) {}\n}\nmain() {}");
        err("class A {\n A(private let x: Int64) {}\n init(x: Int64) {}\n}\nmain() {}");
        err("class A {\n A(x: Int64) {}\n init(x!: Int64 = 1) {}\n}\nmain() {}");
        err("class A {\n A(x: Int64) {}\n init(x: Int64) {}\n init(x: Int64) {}\n}\nmain() {}");
        // 正例：不同参数类型的重载。
        ok("class A {\n A(x: Int64) {}\n init(y: Bool) {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_redef_override_combined_rejected() {
        // 6.1.2.4(44)：redef 与 override 不能同时书写。
        err("open class P {\n public open func f() {}\n}\nclass C <: P {\n redef override public func f() {}\n}\nmain() {}");
        err("open class P {\n public open func f() {}\n}\nclass C <: P {\n override redef public func f() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_primary_ctor_this_call_rejected() {
        // 6.1.2.1(24)：主构造函数不能通过 this(...) 调用构造函数。
        err("class A {\n A() { this() }\n}\nmain() {}");
        err("class A {\n A() { this(15) }\n init(x: Int64) {}\n}\nmain() {}");
        // 正例：次构造函数之间通过 this(...) 委托。
        ok("class A {\n init() {}\n init(x: Int64) { this() }\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_duplicate_finalizer_rejected() {
        // 6.1.2.5(6)：一个类至多一个 finalizer。
        err("class C {\n ~init() {}\n ~init() {}\n}\nmain() {}");
        ok("class C {\n ~init() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w15a_ctor_signature_attr_class_exempt() {
        // 带属性的类（unittest 宏）不参与 ctor 签名重复的严格检查。
        ok("@Test\nclass Foo {\n @TestCase\n Foo() {}\n @Bench\n init() {}\n}\nmain() {}");
    }

    #[test]
    fn test_w18a_unsafe_and_c_func_markers() {
        // 13.1.1(3)：顶层 unsafe func 标记进入 AST。
        let p = parse_only("unsafe func f(x: Int64): Int64 { x }\nmain() {}").expect("parse ok");
        let f = p.functions.iter().find(|f| f.name == "f").expect("f");
        assert!(f.is_unsafe);
        assert!(!f.is_c_func);
        // 13.1.2.2：@C func 与 foreign func 标记为 C 函数。
        let p = parse_only("@C func g(): Int32 { 0 }\nmain() {}").expect("parse ok");
        let g = p.functions.iter().find(|f| f.name == "g").expect("g");
        assert!(g.is_c_func && !g.is_unsafe);
        let p = parse_only("foreign func h(x: Int32): Int32\nmain() {}").expect("parse ok");
        let h = p.functions.iter().find(|f| f.name == "h").expect("h");
        assert!(h.is_c_func && !h.is_unsafe);
    }

    #[test]
    fn test_w18d_struct_static_field_requires_init() {
        // 3.3.2.2(8)：无 static init 块时静态成员变量必须立即初始化。
        err("struct S {\n static var x: Int64\n}\nmain() { }");
        err("struct S {\n static let x: Int64\n}\nmain() { }");
        ok("struct S {\n static var x: Int64 = 0\n}\nmain() { }");
        ok("struct S {\n static var x: Int64\n static init() { x = 0 }\n}\nmain() { }");
    }

    #[test]
    fn test_w18d_struct_member_func_modifier_rejected() {
        // 5.1：未实现接口的结构体成员函数不能使用 open/override/redef。
        err("struct S {\n open func f() { }\n}\nmain() { }");
        err("struct S {\n override func f() { }\n}\nmain() { }");
        err("struct S {\n redef func f() { }\n}\nmain() { }");
        ok("struct S {\n func f() { }\n}\nmain() { }");
    }

    #[test]
    fn test_w18d_class_sealed_func_rejected() {
        // 6.1.2.4：sealed 不能修饰成员函数。
        err("class C {\n sealed func f() { }\n}\nmain() { }");
        ok("open class C { }\nmain() { }");
    }

    #[test]
    fn test_w18d_interface_override_without_parent_rejected() {
        // 5.2：无父接口的接口成员函数不能使用 override/redef。
        err("interface I {\n redef func f() { }\n}\nmain() { }");
        err("interface I {\n static override func f() { }\n}\nmain() { }");
        ok("interface P {\n func f() { }\n}\ninterface I <: P {\n override func f() { }\n}\nmain() { }");
    }

    #[test]
    fn test_w18d_bodyless_func_requires_return_type() {
        // 5.1：无函数体的函数声明必须显式标注返回类型。
        err("abstract class A {\n public func f()\n}\nmain() { }");
        ok("abstract class A {\n public func f(): Unit\n}\nmain() { }");
    }

    #[test]
    fn test_w18d_extend_mut_static_and_alias_rejected() {
        // 5.8.1(2)：扩展中的静态成员函数不能声明为 mut。
        err("struct S { }\nextend S {\n static mut func f() { }\n}\nmain() { }");
        // 5.8.1：类型别名扩展中的成员函数不能声明为 mut。
        err("type T = Int64\nextend T {\n mut func f(): T { this }\n}\nmain() { }");
    }

    #[test]
    fn test_w19b_duplicate_ctor_signatures() {
        // 5.7.1(3)：构造函数（含类名形式主构造函数）形参类型列表不得重复。
        err("class A {\n init(x: Int64) {}\n init(x: Int64) {}\n}\nmain() { }");
        err("class A {\n init(x: Int64) {}\n A(x: Int64) {}\n}\nmain() { }");
        err("struct S {\n init(x: Int64) {}\n S(x: Int64) {}\n}\nmain() { }");
        ok("class A {\n init(x: Int64) {}\n init(y: Bool) {}\n}\nmain() { }");
        ok("class A {\n init(x: Int64) {}\n A(y: Bool) {}\n}\nmain() { }");
    }
}
