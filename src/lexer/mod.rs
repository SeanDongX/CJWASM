use crate::ast::IntSuffix;
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

/// 解析十六进制浮点字面量（仓颉规范 1.3：0x<hex>(.<hex>)?[pP]<sign><digits>）
///
/// 例如 `0x1.8p3` = 1.5 * 2^3 = 12.0，`0x1p-2` = 0.25，`0x.1p1` = 0.125。
/// 下划线分隔符在解析前移除；尾数不允许为空。
fn parse_hex_float_literal(s: &str) -> Option<f64> {
    let s: String = s.chars().filter(|c| *c != '_').collect();
    let (mantissa, exp_text) = s.split_once(['p', 'P'])?;
    let exp: i32 = exp_text.parse().ok()?;
    let mantissa = mantissa
        .strip_prefix("0x")
        .or_else(|| mantissa.strip_prefix("0X"))?;
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((a, b)) => (a, b),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    let mut value = 0.0f64;
    for c in int_part.chars() {
        value = value * 16.0 + c.to_digit(16)? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        value += c.to_digit(16)? as f64 * scale;
        scale /= 16.0;
    }
    Some(value * 2f64.powi(exp))
}

/// 解析十进制浮点字面量（已移除下划线）。支持前导点形式 `.5` / `.8e1`。
fn parse_decimal_float(s: &str) -> Option<f64> {
    if s.starts_with('.') {
        format!("0{}", s).parse::<f64>().ok()
    } else {
        s.parse::<f64>().ok()
    }
}

/// 解析整数字面量正文（已移除下划线，且不含类型后缀）为 i64。
///
/// 超出 i64 但可表示为 u64 的无符号字面量按位折叠为 i64（与既有行为一致）。
fn parse_integer_literal(body: &str) -> Option<i64> {
    if body.len() >= 2 && (body.starts_with("0x") || body.starts_with("0X")) {
        i64::from_str_radix(&body[2..], 16)
            .ok()
            .or_else(|| u64::from_str_radix(&body[2..], 16).ok().map(|v| v as i64))
    } else if body.starts_with("0o") || body.starts_with("0O") {
        i64::from_str_radix(&body[2..], 8)
            .ok()
            .or_else(|| u64::from_str_radix(&body[2..], 8).ok().map(|v| v as i64))
    } else if body.starts_with("0b") || body.starts_with("0B") {
        i64::from_str_radix(&body[2..], 2)
            .ok()
            .or_else(|| u64::from_str_radix(&body[2..], 2).ok().map(|v| v as i64))
    } else {
        body.parse::<i64>()
            .ok()
            .or_else(|| body.parse::<u64>().ok().map(|v| v as i64))
    }
}

/// 解析十进制 Float32 字面量（已移除下划线）。支持前导点形式 `.5` / `.8e1`。
fn parse_decimal_float32(s: &str) -> Option<f32> {
    if s.starts_with('.') {
        format!("0{}", s).parse::<f32>().ok()
    } else {
        s.parse::<f32>().ok()
    }
}

/// 十进制字面量不能有前导零（规范 1.3.1/1.3.2 负例：`01`、`01.0`）。
///
/// 单个 `0`、`0.x` 分数部分、以及 `0x`/`0o`/`0b` 前缀均合法。
fn decimal_leading_zero(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 1 && b[0] == b'0' && b[1].is_ascii_digit()
}

/// 结合上下文判定前导零是否非法：紧跟在 `.` 之后视为前导点浮点的分数部分
/// （如 `.00e0` 被词法拆成 `.` + `00e0`），合法；其余前导零非法。
fn leading_zero_invalid(lex: &logos::Lexer<Token>, s: &str) -> bool {
    if !decimal_leading_zero(s) {
        return false;
    }
    !matches!(
        lex.source()[..lex.span().start].chars().next_back(),
        Some('.')
    )
}

/// 前导点浮点（`.5`）后紧跟标识符字符或另一个 `.` 时视为非法字面量
/// （如 `.00xp0`、`.0.0`），返回 true 让词法器报错（规范 1.3.2 负样例）。
fn leading_dot_malformed(lex: &logos::Lexer<Token>) -> bool {
    if !lex.slice().starts_with('.') {
        return false;
    }
    matches!(
        lex.remainder().chars().next(),
        Some(c) if c == '.' || c == '_' || c.is_alphanumeric()
    )
}

/// 浮点字面量后紧跟 `.` + 数字（如 `1e0.0`、`0x0p0.0` 的小数点跟在指数后）
/// 视为非法字面量（规范 1.3.2 负样例），返回 true 让词法器报错。
/// 仅当紧跟的是 `.` 后接数字时报错；`.toString()` 等 `.标识符` 后缀不受影响。
fn trailing_dot_malformed(lex: &logos::Lexer<Token>) -> bool {
    let mut chars = lex.remainder().chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('.'), Some(c)) if c.is_ascii_digit()
    )
}

/// 整数字面量后紧跟 `.e<数字>` / `.E<数字>`（可带负号）视为非法的浮点形式
/// （规范 1.3.2 负例：`1.e2`、`123.e2`、`4142.e-5`——小数点后缺少数位）。
///
/// 只匹配 `.` + e/E + 可选 `-` + 数字，`42.toString`、`1.5` 等不受影响。
fn dot_exponent_malformed(lex: &logos::Lexer<Token>) -> bool {
    let mut it = lex.remainder().chars();
    if it.next() != Some('.') {
        return false;
    }
    match it.next() {
        Some('e') | Some('E') => {
            let mut rest = it;
            if rest.clone().next() == Some('-') {
                rest.next();
            }
            matches!(rest.next(), Some(c) if c.is_ascii_digit())
        }
        _ => false,
    }
}

/// 跳过（并消耗）一个仓颉块注释，支持任意层嵌套（规范 1.5(2)）。
///
/// `/*` 已由 token 匹配消耗，remainder() 为注释正文起点。返回 Skip 时
/// logos 会同时消耗 bump() 推进的部分。
fn lex_block_comment(lex: &mut logos::Lexer<Token>) -> logos::FilterResult<(), ()> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut depth = 1usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                lex.bump(i);
                return logos::FilterResult::Skip;
            }
        } else {
            i += 1;
        }
    }
    logos::FilterResult::Error(())
}

/// 解码单个字符的合法转义（不含 `\u{...}`，其单独校验）。
/// 规范 1.3.6：合法 EscapeSeq 为 `\n \r \t \b \f \v \0 \' \" \\ \$`。
fn decode_simple_escape(next: u8) -> Option<char> {
    match next {
        b'n' => Some('\n'),
        b't' => Some('\t'),
        b'r' => Some('\r'),
        b'b' => Some('\u{08}'),
        b'f' => Some('\u{0C}'),
        b'v' => Some('\u{0B}'),
        b'0' => Some('\0'),
        b'\'' => Some('\''),
        b'"' => Some('"'),
        b'\\' => Some('\\'),
        b'$' => Some('$'),
        _ => None,
    }
}

/// 校验 `\u{HEX...}` 转义（`pos` 指向反斜杠，`bytes[pos+1] == 'u'`）。
/// 合法形态：`\u{` + 1..=8 个十六进制数字 + `}`，且码点在 Unicode 标量范围内。
fn validate_unicode_escape(remainder: &str, bytes: &[u8], pos: usize) -> bool {
    // 必须紧跟 `u{`
    if bytes.get(pos + 1) != Some(&b'u') || bytes.get(pos + 2) != Some(&b'{') {
        return false;
    }
    let mut j = pos + 3;
    while j < bytes.len() && bytes[j] != b'}' {
        if !bytes[j].is_ascii_hexdigit() {
            return false;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return false; // 缺少右花括号
    }
    let hex = &remainder[pos + 3..j];
    if hex.is_empty() || hex.len() > 8 {
        return false;
    }
    match u32::from_str_radix(hex, 16) {
        Ok(v) => v <= 0x10FFFF,
        Err(_) => false,
    }
}

/// 解析多行字符串 """..."""，支持转义（`\"` `\\` `\$` `\n` `\t`）。
///
/// 终止条件是未被反斜杠转义的 `"""`；`\"""` 中的 `\"` 是转义引号，不构成终止。
/// `${...}` 按插值处理（与普通字符串一致）；无插值时返回 Ok(Plain)。
fn lex_multiline_string(lex: &mut logos::Lexer<Token>) -> Option<Result<String, Vec<StringPart>>> {
    let remainder = lex.remainder();
    let bytes = remainder.as_bytes();
    let mut pos = 0usize;
    let mut current = String::new();
    let mut parts: Vec<StringPart> = Vec::new();
    let mut has_interpolation = false;

    // 规范 1.3.6：多行字符串的起始 `"""` 之后必须紧跟行终止符
    // （`""""""`、`"""123"""` 等无换行的形态均非法）。
    match bytes.first() {
        Some(&b'\n') | Some(&b'\r') => {}
        _ => return None,
    }

    while pos < bytes.len() {
        // 终止符 """（到达此处时一定未被转义，因为转义引号已在下面消耗）
        if bytes[pos] == b'"'
            && pos + 2 < bytes.len()
            && bytes[pos + 1] == b'"'
            && bytes[pos + 2] == b'"'
        {
            lex.bump(pos + 3);
            if !has_interpolation {
                return Some(Ok(process_multiline_string(&current)));
            }
            if !current.is_empty() {
                parts.push(StringPart::Literal(process_multiline_string(&current)));
            }
            return Some(Err(parts));
        }

        let c = bytes[pos];
        if c == b'\\' && pos + 1 < bytes.len() {
            let next = bytes[pos + 1];
            if next == b'u' {
                // \u{...} 结构校验；通过后按原样保留（与既有行为一致）
                if !validate_unicode_escape(remainder, bytes, pos) {
                    return None;
                }
                current.push('\\');
                current.push('u');
                pos += 2;
            } else if let Some(ch) = decode_simple_escape(next) {
                current.push(ch);
                pos += 2;
            } else {
                // 未知转义（如 \1 \a \^ \ ）在多行字符串中非法（规范 1.3.6）
                return None;
            }
        } else if c == b'$' && pos + 1 < bytes.len() && bytes[pos + 1] == b'{' {
            // 插值开始 ${
            has_interpolation = true;
            if !current.is_empty() {
                parts.push(StringPart::Literal(process_multiline_string(&current)));
                current.clear();
            }
            pos += 2;
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
            if expr_text.trim().is_empty() {
                return None; // 规范 1.3.6：插值表达式不能为空（`${}` 非法）
            }
            parts.push(StringPart::Interpolation(expr_text.to_string()));
            pos += 1; // 跳过 }
        } else {
            // 按 UTF-8 字符复制
            let ch = remainder[pos..].chars().next()?;
            current.push(ch);
            pos += ch.len_utf8();
        }
    }
    None // 未找到结束引号
}

/// 词法分析入口：多行字符串（可能含 ${...} 插值）
fn lex_any_multiline_string(
    lex: &mut logos::Lexer<Token>,
) -> logos::FilterResult<StringOrInterpolated, ()> {
    match lex_multiline_string(lex) {
        Some(Ok(s)) => logos::FilterResult::Emit(StringOrInterpolated::Plain(s)),
        Some(Err(parts)) => logos::FilterResult::Emit(StringOrInterpolated::Interpolated(parts)),
        None => logos::FilterResult::Error(()),
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
            // 转义字符（规范 1.3.6 合法集）
            let next = bytes[pos + 1];
            if next == b'u' {
                // \u{...} 结构校验；通过后按原样保留（与既有行为一致）
                if !validate_unicode_escape(remainder, bytes, pos) {
                    return None;
                }
                current_literal.push('\\');
                current_literal.push('u');
                pos += 2;
            } else if let Some(ch) = decode_simple_escape(next) {
                current_literal.push(ch);
                pos += 2;
            } else {
                // 非法转义序列（如 \% \A \1 \ ）（规范 1.3.6）
                return None;
            }
        } else if c == b'\n' || c == b'\r' {
            // 单行字符串不能跨行（规范 1.3.6 / 1.3.7 负例）
            return None;
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
                // 规范 2.1.6.2(5)：单行字符串的插值表达式内不允许换行
                // （多行字符串 """...""" 的同类表达式由 lex_multiline_string 处理）。
                if bytes[pos] == b'\n' || bytes[pos] == b'\r' {
                    return None;
                }
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
            if expr_text.trim().is_empty() {
                return None; // 规范 1.3.6：插值表达式不能为空（`${}` 非法）
            }
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
            // 按 UTF-8 解码整个字符，逐字节 `as char` 会把多字节字符
            // 误当作 Latin-1 重新编码（mojibake），导致 size()/输出翻倍。
            let ch = remainder[pos..].chars().next()?;
            current_literal.push(ch);
            pos += ch.len_utf8();
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
                    // next 可能是多字节字符的首字节：按字符解码，
                    // 保持 pos 落在 UTF-8 字符边界，避免后续切片 panic。
                    current_literal.push('\\');
                    let esc = remainder[pos + 1..].chars().next().unwrap_or(next as char);
                    current_literal.push(esc);
                    pos += 1 + esc.len_utf8();
                    continue;
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
            // 按 UTF-8 解码整个字符，避免多字节编码 mojibake（同 lex_string）
            let ch = remainder[pos..].chars().next()?;
            current_literal.push(ch);
            pos += ch.len_utf8();
        }
    }
    None
}

/// 词法分析入口：反引号字符串
fn lex_any_backtick_string(
    lex: &mut logos::Lexer<Token>,
) -> logos::FilterResult<StringOrInterpolated, ()> {
    match lex_backtick_string(lex) {
        Some(Ok(s)) => logos::FilterResult::Emit(StringOrInterpolated::Plain(s)),
        Some(Err(parts)) => logos::FilterResult::Emit(StringOrInterpolated::Interpolated(parts)),
        None => logos::FilterResult::Error(()),
    }
}

/// 解析 #"..."#, ##"..."##, ###"..."### 定界符原始字符串
fn lex_hash_raw_string(lex: &mut logos::Lexer<Token>) -> logos::FilterResult<String, ()> {
    let slice = lex.slice();
    let hash_count = slice.len() - 1; // 去掉末尾的 "
    let remainder = lex.remainder();
    // 构造结束标记: " + N个#
    let end_marker_char = '"';
    let mut pos = 0;
    let bytes = remainder.as_bytes();
    while pos < bytes.len() {
        if bytes[pos] == end_marker_char as u8 {
            // 检查后面是否有 hash_count 个 #
            let end_start = pos + 1;
            if end_start + hash_count <= bytes.len() {
                let hashes = &remainder[end_start..end_start + hash_count];
                if hashes.chars().all(|c| c == '#') {
                    let content = &remainder[..pos];
                    lex.bump(pos + 1 + hash_count); // 跳过内容 + " + ###
                    return logos::FilterResult::Emit(content.to_string());
                }
            }
        }
        pos += 1;
    }
    logos::FilterResult::Error(())
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
// 规范 A.1.2(1)：WS := [ 	]，另含 \r\n
#[logos(skip r"[ \t\r\n\x0C]+")] // 跳过空白（含换页符 U+000C）
#[logos(skip r"//[^\n]*")] // 跳过单行注释
pub enum Token {
    // 块注释 /* ... */：支持嵌套（规范 1.5(2)），由 callback 手工计数深度后跳过
    #[token("/*", lex_block_comment)]
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
    #[token("Option", priority = 1)]
    TypeOption,
    #[token("Result", priority = 1)]
    TypeResult,
    #[token("Slice", priority = 1)]
    TypeSlice,
    #[token("Map", priority = 1)]
    TypeMap,

    // Float64：小数、科学计数法（无后缀）、前导点小数或十六进制浮点（规范 1.3）
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE]-?[0-9][0-9_]*)?|[0-9][0-9_]*[eE]-?[0-9][0-9_]*|0[xX]([0-9a-fA-F][0-9a-fA-F_]*(\.[0-9a-fA-F][0-9a-fA-F_]*)?|\.[0-9a-fA-F][0-9a-fA-F_]*)[pP]-?[0-9][0-9_]*", |lex| {
        if leading_dot_malformed(lex) || trailing_dot_malformed(lex) {
            return None;
        }
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        if s.starts_with("0x") || s.starts_with("0X") {
            // 十六进制浮点：0x1.8p3 / 0xA.BP-10 / 0x.1p1（十六进制无类型后缀）
            parse_hex_float_literal(&s)
        } else if leading_zero_invalid(lex, &s) {
            None // 前导零非法（`01.0`）
        } else {
            parse_decimal_float(&s)
        }
    })]
    Float(f64),

    // Float64 带 f64 后缀
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?f64|[0-9][0-9_]*[eE]-?[0-9][0-9_]*f64|[0-9][0-9_]*f64", priority = 4, callback = |lex| {
        if leading_dot_malformed(lex) || trailing_dot_malformed(lex) {
            return None;
        }
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        let s = s.trim_end_matches("f64");
        if leading_zero_invalid(lex, s) {
            return None;
        }
        parse_decimal_float(s)
    })]
    Float64Suffix(f64),

    // Float32：f、f32 或 f16 后缀（Float16 在 WASM 层同为 f32，值按 f32 解析）
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?(f32|f16)|[0-9][0-9_]*[eE]-?[0-9][0-9_]*(f32|f16)|[0-9][0-9_]*(f32|f16)|[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?f|[0-9][0-9_]*[eE]-?[0-9][0-9_]*f|[0-9][0-9_]*f", priority = 3, callback = |lex| {
        if leading_dot_malformed(lex) || trailing_dot_malformed(lex) {
            return None;
        }
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        let s = s.trim_end_matches("f32").trim_end_matches("f16").trim_end_matches('f');
        if leading_zero_invalid(lex, s) {
            return None;
        }
        parse_decimal_float32(s)
    })]
    Float32(f32),

    // 整数字面量（无类型后缀）。带类型后缀的形态由下面的 `IntegerSuffixed`
    // 变体按最长匹配优先识别。
    #[regex(r"(0[xX][0-9a-fA-F][0-9a-fA-F_]*|0[oO][0-7][0-7_]*|0[bB][01][01_]*|[0-9][0-9_]*)", |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        if leading_zero_invalid(lex, &s) {
            return None; // 前导零非法（`01`）
        }
        if dot_exponent_malformed(lex) {
            return None; // `.e2` / `.E-5` 等小数点后缺少数位（`1.e2`）
        }
        parse_integer_literal(&s)
    })]
    Integer(i64),

    /// 进制前缀整数字面量（0x/0o/0b）：值与 `Integer` 相同，但携带“进制前缀”
    /// 信息——十进制整数字面量可经字面量协调赋给 Float 注解，进制前缀形式
    /// 不可以（官方负例 a06_47 / 2.1.11 a11_02/03/04）。
    #[regex(r"(0[xX][0-9a-fA-F][0-9a-fA-F_]*|0[oO][0-7][0-7_]*|0[bB][01][01_]*)", priority = 6, callback = |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        if leading_zero_invalid(lex, &s) {
            return None;
        }
        parse_integer_literal(&s)
    })]
    IntegerRadix(i64),

    // 整数字面量带 Cangjie 类型后缀（i8/i16/i32/i64/u8/u16/u32/u64）。
    // 保留后缀类型供语义分析判定字面量的具体整型；代码生成与 `Integer` 等价。
    #[regex(r"(0[xX][0-9a-fA-F][0-9a-fA-F_]*|0[oO][0-7][0-7_]*|0[bB][01][01_]*|[0-9][0-9_]*)(i8|i16|i32|i64|u8|u16|u32|u64)", priority = 5, callback = |lex| {
        let s: String = lex.slice().chars().filter(|c| *c != '_').collect();
        let suffixes = [
            ("i8", IntSuffix::I8),
            ("i16", IntSuffix::I16),
            ("i32", IntSuffix::I32),
            ("i64", IntSuffix::I64),
            ("u8", IntSuffix::U8),
            ("u16", IntSuffix::U16),
            ("u32", IntSuffix::U32),
            ("u64", IntSuffix::U64),
        ];
        for (suf, ty) in suffixes {
            if s.ends_with(suf) {
                let body = &s[..s.len() - suf.len()];
                if leading_zero_invalid(lex, body) {
                    return None; // 前导零非法（`01u8`）
                }
                if dot_exponent_malformed(lex) {
                    return None; // `1u8.e2` 小数点后缺少数位
                }
                if let Some(v) = parse_integer_literal(body) {
                    return Some((v, ty));
                }
            }
        }
        None
    })]
    IntegerSuffixed((i64, IntSuffix)),

    // 字节字面量 b'x' (Cangjie 语法，支持转义，含 \u{XX} 十六进制字节)
    #[regex(r#"b'[^'\\]'|b'\\[ntrbfv0'"\\]'|b'\\u\{[0-9a-fA-F]{1,2}\}'"#, |lex| {
        let s = lex.slice();
        let inner = &s[2..s.len()-1]; // 去除 b' 和 '
        if inner.starts_with('\\') {
            match inner.chars().nth(1) {
                Some('n') => Some(b'\n'),
                Some('t') => Some(b'\t'),
                Some('r') => Some(b'\r'),
                Some('b') => Some(0x08),
                Some('f') => Some(0x0C),
                Some('v') => Some(0x0B),
                Some('\\') => Some(b'\\'),
                Some('0') => Some(0),
                Some('\'') => Some(b'\''),
                Some('"') => Some(b'"'),
                Some('$') => Some(b'$'),
                Some('u') => {
                    // \u{XX}：十六进制字节（0x00..=0xFF）
                    let hex = &inner[3..inner.len()-1];
                    u32::from_str_radix(hex, 16).ok().and_then(|v| u8::try_from(v).ok())
                }
                _ => None,
            }
        } else {
            inner.chars().next().map(|c| c as u8)
        }
    })]
    ByteLit(u8),

    // 字符字面量 'a' (支持转义 '\n' '\t' '\\' '\'' '\0' '\u{XXXX}' 及 \f \v \b \" \$)
    #[regex(r#"'[^'\\]'|'\\[ntrbfv0'"\\$u]'|'\\u\{[0-9a-fA-F]+\}'"#, |lex| {
        let s = lex.slice();
        let inner = &s[1..s.len()-1]; // 去除引号
        if inner.starts_with('\\') {
            match inner.chars().nth(1) {
                Some('n') => Some('\n'),
                Some('t') => Some('\t'),
                Some('r') => Some('\r'),
                Some('b') => Some('\u{08}'),
                Some('f') => Some('\u{0C}'),
                Some('v') => Some('\u{0B}'),
                Some('\\') => Some('\\'),
                Some('0') => Some('\0'),
                Some('\'') => Some('\''),
                Some('"') => Some('"'),
                Some('$') => Some('$'),
                Some('u') => {
                    // \u{XXXX} unicode escape
                    let hex = &inner[3..inner.len()-1]; // extract hex digits from \u{XXXX}
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                }
                _ => None,
            }
        } else {
            inner.chars().next()
        }
    })]
    CharLit(char),

    // Rune 字面量 r'.' (仓颉语法：r'x' 为原始字符，不处理转义)
    #[regex(r#"r'[^'\\]'|r'\\[ntrbfv0'"\\$u]'|r'\\u\{[0-9a-fA-F]+\}'"#, |lex| {
        let s = lex.slice();
        let inner = &s[2..s.len()-1]; // 去除 r' 和 '
        if inner.starts_with('\\') {
            match inner.chars().nth(1) {
                Some('n') => Some('\n'),
                Some('t') => Some('\t'),
                Some('r') => Some('\r'),
                Some('b') => Some('\u{08}'),
                Some('f') => Some('\u{0C}'),
                Some('v') => Some('\u{0B}'),
                Some('\\') => Some('\\'),
                Some('0') => Some('\0'),
                Some('\'') => Some('\''),
                Some('"') => Some('"'),
                Some('$') => Some('$'),
                Some('u') => {
                    // \u{XXXX} unicode escape
                    let hex = &inner[3..inner.len()-1];
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                }
                _ => None,
            }
        } else {
            inner.chars().next()
        }
    })]
    RuneLit(char),

    // 多字符或空单引号字符串 '...'（cjc 兼容: 0 或 2+ 字符时视为 String）
    #[regex(r"''|'[^'\\]{2,}'|'([^'\\]|\\.){2,}'", priority = 1, callback = |lex| {
        let s = lex.slice();
        if s == "''" {
            return Some(String::new());
        }
        let inner = &s[1..s.len()-1];
        Some(unescape_string(inner))
    })]
    SingleQuoteStringLit(String),

    #[token("true")]
    True,
    #[token("false")]
    False,

    // 多行字符串 """..."""（strip 公共缩进；支持转义与 ${...} 插值）
    #[token(r#"""""#, lex_any_multiline_string)]
    MultiLineStringLit(StringOrInterpolated),

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

    // 标识符（Cangjie 规范允许 ASCII 字母/数字/下划线；兼容官方测试集中的非 ASCII 字母标识符）
    #[regex(r"[_\p{L}\p{Nl}][_\p{L}\p{N}]*", |lex| lex.slice().to_string())]
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
    // 定界符原始字符串 #"..."#, ##"..."##, ###"..."### (不处理转义/插值)
    #[regex(r#"#+""#, lex_hash_raw_string)]
    HashRawStringLit(String),
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
                Ok(Token::BlockComment) => continue, // 跳过块注释
                // 原始标识符：`name`（反引号内为合法标识符时视为 Ident，而非反引号字符串）
                Ok(Token::BacktickStringLit(StringOrInterpolated::Plain(ref s)))
                    if is_raw_identifier(s) =>
                {
                    return Some(Ok((span.start, Token::Ident(s.clone()), span.end)));
                }
                // 规范 1.3.4：反引号仅用于原始标识符（内容必须是合法标识符或关键字，
                // 语法 `'_'* Letter (Letter|'_'|Digit)*`）。内容不合法（如 `a--`、
                // `type A=Int64`、`_`、带插值）即为词法错误，不得回退为字符串字面量。
                Ok(Token::BacktickStringLit(_)) => {
                    return Some(Err(format!(
                        "反引号内不是合法的原始标识符: '{}'",
                        &self.inner.source()[span.start..span.end]
                    )));
                }
                Ok(tok) => {
                    // 数字字面量不能直接紧贴标识符字符：
                    // `0a12345`（非法十进制前缀）、`654f32`/`0f16`（纯整数加浮点后缀）
                    // 均为规范负例。十六进制浮点可带 f16/f32/f64 后缀，
                    // 本词法器把后缀留给后续标识符，故对含 p/P 的十六进制浮点跳过。
                    if matches!(
                        tok,
                        Token::Integer(_)
                            | Token::IntegerSuffixed(_)
                            | Token::Float(_)
                            | Token::Float32(_)
                            | Token::Float64Suffix(_)
                    ) {
                        let prev = self.inner.source()[..span.start].chars().next_back();
                        // 相邻数字字面量（无任何分隔符）非法：如八进制中混入 8/9
                        // （`0O1779u8` → `0O177` + `9u8`），或指数后紧贴小数（`1e0.0`）。
                        if matches!(prev, Some(p) if p.is_ascii_digit()) {
                            return Some(Err(format!(
                                "无效的数字字面量: 相邻数字 '{}'",
                                self.inner.slice()
                            )));
                        }
                        let float_suffix = matches!(
                            tok,
                            Token::Float32(_) | Token::Float64Suffix(_)
                        );
                        if let Some(msg) = invalid_numeric_tail(
                            self.inner.slice(),
                            self.inner.remainder(),
                            prev,
                            float_suffix,
                        ) {
                            return Some(Err(msg));
                        }
                    }
                    return Some(Ok((span.start, tok, span.end)));
                }
                Err(_) => {
                    return Some(Err(format!(
                        "未知字符: '{}'",
                        &self.inner.source()[span.start..span.end]
                    )))
                }
            }
        }
    }
}

/// 判断反引号内的文本是否构成一个原始标识符
///（标识符字符集：字母/数字/下划线，首字符非数字；关键字合法。
/// 官方正例 a01_11/a01_14 使用 `__`（纯下划线合法），负例 a06_10 `a--`、
/// a06_24 `type A=Int64` 含运算符/空格/等号非法——不要求“含字母”。
/// 唯一例外：`_`（单个下划线）是通配符模式，不能作为原始标识符
///（官方负例 a05_42 `var \`_\`: Int32 = 1`）。
fn is_raw_identifier(s: &str) -> bool {
    if s == "_" {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    s.chars().all(|c| c == '_' || c.is_alphanumeric())
}

/// 数字字面量非法形态检测。
///
/// 规范 1.3.1/1.3.2：十进制字面量不能以 `0` + 非前缀字母开头
/// （`0a12345`），整数部分不能直接接浮点后缀（`654f32`、`0f16`）。
/// 十六进制浮点允许 `f16/f32/f64` 后缀，词法正则未包含该后缀，
/// 因此字符串中含 `p/P` 的十六进制浮点跳过检查。
fn invalid_numeric_tail(
    slice: &str,
    remainder: &str,
    prev: Option<char>,
    float_suffix: bool,
) -> Option<String> {
    // 纯整数直接带浮点后缀（`654f32` / `0f16`）非法；前导点小数
    // （`.002f32`，此时前一个 token 是 `.`）是合法的分数部分形态。
    if float_suffix {
        let pure_int = !slice.contains('.')
            && !slice.contains('e')
            && !slice.contains('E')
            && !slice.contains('p')
            && !slice.contains('P');
        if pure_int && prev != Some('.') {
            return Some(format!(
                "无效的数字字面量: '{}'（浮点后缀不能用于整数）",
                slice
            ));
        }
    }
    let next = remainder.chars().next()?;
    if !(next.is_ascii_alphabetic() || next == '_') {
        return None;
    }
    let is_hex_float = (slice.starts_with("0x") || slice.starts_with("0X"))
        && (slice.contains('p') || slice.contains('P'));
    if is_hex_float {
        // 十六进制浮点不允许类型后缀（规范 1.3.2 负例 `0x0p0f16`）
        if remainder.starts_with("f16")
            || remainder.starts_with("f32")
            || remainder.starts_with("f64")
        {
            return Some(format!(
                "无效的数字字面量: '{}{}'（十六进制浮点不能带类型后缀）",
                slice, next
            ));
        }
        return None;
    }
    Some(format!("无效的数字字面量: '{}{}'", slice, next))
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

        assert_eq!(
            tokens[3],
            Token::StringLit(StringOrInterpolated::Plain("hello world".to_string()))
        );
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
    fn test_string_literal_utf8_bytes_preserved() {
        // 多字节字符必须原样保留（不被逐字节按 Latin-1 重新编码）
        let source = r#"let s = "中文🙏🏿""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(
            tokens[3],
            Token::StringLit(StringOrInterpolated::Plain("中文🙏🏿".to_string()))
        );
        if let Token::StringLit(StringOrInterpolated::Plain(s)) = &tokens[3] {
            // 中文 6 字节 + emoji 组合 8 字节
            assert_eq!(s.len(), 14);
        }
    }

    #[test]
    fn test_backtick_string_utf8_bytes_preserved() {
        // 规范 1.3.4：反引号仅作原始标识符。内容先经 lex_backtick_string 词法化
        // （必须 UTF-8 字节保真），再重解释为 Ident；多字节标识符不得 mojibake。
        let source = "let `中x文` = 1";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(tokens[1], Token::Ident("中x文".to_string()));
    }

    #[test]
    fn test_interpolated_string_utf8_literal_parts_preserved() {
        // 插值字符串的字面量部分同样不得 mojibake
        let source = "let s = \"前${word}后\"";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        match &tokens[3] {
            Token::StringLit(StringOrInterpolated::Interpolated(parts)) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], StringPart::Literal("前".to_string()));
                assert_eq!(parts[1], StringPart::Interpolation("word".to_string()));
                assert_eq!(parts[2], StringPart::Literal("后".to_string()));
            }
            other => panic!("expected interpolated string, got {other:?}"),
        }
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

        assert_eq!(
            tokens[3],
            Token::MultiLineStringLit(StringOrInterpolated::Plain(
                "hello\nworld".to_string()
            ))
        );
    }

    #[test]
    fn test_multiline_string_inline() {
        // 规范 1.3.6：`"""` 后必须紧跟换行
        let source = "let s = \"\"\"\nhello world\"\"\"";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();

        assert_eq!(
            tokens[3],
            Token::MultiLineStringLit(StringOrInterpolated::Plain(
                "hello world".to_string()
            ))
        );
    }

    #[test]
    fn test_multiline_string_interpolation() {
        // 多行字符串内的 ${...} 按插值解析（支持换行/分号）
        let source = "let s = \"\"\"\n${\"a\"}${b}\"\"\"";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        match &tokens[3] {
            Token::MultiLineStringLit(StringOrInterpolated::Interpolated(parts)) => {
                let exprs: Vec<&StringPart> = parts
                    .iter()
                    .filter(|p| matches!(p, StringPart::Interpolation(_)))
                    .collect();
                assert_eq!(exprs.len(), 2);
                assert_eq!(exprs[0], &StringPart::Interpolation("\"a\"".to_string()));
                assert_eq!(exprs[1], &StringPart::Interpolation("b".to_string()));
            }
            other => panic!("期望多行插值字符串，实际: {:?}", other),
        }
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
        assert_eq!(
            tokens[3],
            Token::StringLit(StringOrInterpolated::Plain("Price: $100".to_string()))
        );
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
        let source =
            "package import as is open abstract sealed override extend interface prop foreign";
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

    /// Float16 字面量后缀 f16（1.2f16 / 0.11f16）应整体作为一个浮点 token 消费，
    /// 不得把尾部数字拆成 Integer。
    #[test]
    fn test_lexer_float16_literal() {
        // 纯整数不能直接带浮点后缀（`3f16` 非法，见 1.3.2(3)），
        // 合法形态需带小数或指数部分。
        let source = "1.2f16 0.11f16 3e0f16";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens.len(), 3);
        match &tokens[0] {
            Token::Float32(v) => assert!((*v - 1.2_f32).abs() < 0.001),
            _ => panic!("Expected Float32 for f16 literal, got {:?}", tokens[0]),
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
        assert_eq!(tokens[0], Token::IntegerRadix(255));
        assert_eq!(tokens[1], Token::IntegerRadix(15));
        assert_eq!(tokens[2], Token::IntegerRadix(0xABCD));
    }

    #[test]
    fn test_uint64_large_literal() {
        let source = "18446744073709551615 0xFFFFFFFFFFFFFFFF 0x8000000000000000";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Integer(u64::MAX as i64));
        assert_eq!(tokens[1], Token::IntegerRadix(u64::MAX as i64));
        assert_eq!(tokens[2], Token::IntegerRadix(0x8000000000000000u64 as i64));
    }

    #[test]
    fn test_hash_raw_string() {
        let source = r####"#"hello"# ##"world"## ###"raw###string"###"####;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::HashRawStringLit("hello".to_string()));
        assert_eq!(tokens[1], Token::HashRawStringLit("world".to_string()));
        assert_eq!(tokens[2], Token::HashRawStringLit("raw###string".to_string()));
    }

    #[test]
    fn test_rune_literal() {
        let source = "r'A' r'\\n'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::RuneLit('A'));
        assert_eq!(tokens[1], Token::RuneLit('\n'));
    }

    #[test]
    fn test_block_comment_nested_two_levels() {
        // 规范 1.5(2)：/* /* inner */ */ 为单个注释，后续代码正常词法
        let source = "let a = 1 /* /* inner */ */ let b = 2";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Let);
        assert_eq!(tokens[1], Token::Ident("a".to_string()));
        assert_eq!(tokens[3], Token::Integer(1));
        // 注释整体被跳过，紧接着是 let b = 2
        assert_eq!(tokens[4], Token::Let);
        assert_eq!(tokens[5], Token::Ident("b".to_string()));
        assert_eq!(tokens[7], Token::Integer(2));
    }

    #[test]
    fn test_block_comment_nested_three_levels() {
        let source = "/* outer /* middle /* inner */ middle */ outer */ let x = 7";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Let);
        assert_eq!(tokens[1], Token::Ident("x".to_string()));
        assert_eq!(tokens[3], Token::Integer(7));
    }

    #[test]
    fn test_block_comment_nested_spans_whole_comment() {
        // 保证嵌套注释被完整消耗（span 覆盖到最外层 */ 之后）
        let source = "/* a /* b */ c */x";
        let lexer = Lexer::new(source);
        let toks: Vec<_> = lexer.collect();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0], Ok((17, Token::Ident("x".to_string()), 18)));
    }

    #[test]
    fn test_block_comment_unterminated_is_error() {
        let source = "let a = 1 /* /* no end";
        let lexer = Lexer::new(source);
        let toks: Vec<_> = lexer.collect();
        assert!(toks.iter().any(|t| t.is_err()));
    }

    #[test]
    fn test_rune_literal_unicode() {
        // CJK 字符（3 字节 UTF-8）与增补平面字符（4 字节 UTF-8）
        let source = "r'严' r'𐐷'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::RuneLit('严'));
        assert_eq!(tokens[1], Token::RuneLit('𐐷'));
    }

    #[test]
    fn test_rune_literal_unicode_escape() {
        let source = r"r'\u{4e25}' r'\u{10437}'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::RuneLit('严'));
        assert_eq!(tokens[1], Token::RuneLit('𐐷'));
    }

    #[test]
    fn test_char_literal_unicode() {
        let source = "'严' '𐐷'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::CharLit('严'));
        assert_eq!(tokens[1], Token::CharLit('𐐷'));
    }

    #[test]
    fn test_hex_float_literals() {
        // 0x1.8p3 = 1.5 * 2^3 = 12.0；0x1p-2 = 0.25；0x.1p1 = 0.125
        let source = "0x1.8p3 0x1p-2 0x.1p1";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        match tokens[0] {
            Token::Float(v) => assert!((v - 12.0).abs() < 1e-12, "got {}", v),
            ref other => panic!("Expected Float, got {:?}", other),
        }
        match tokens[1] {
            Token::Float(v) => assert!((v - 0.25).abs() < 1e-12, "got {}", v),
            ref other => panic!("Expected Float, got {:?}", other),
        }
        match tokens[2] {
            Token::Float(v) => assert!((v - 0.125).abs() < 1e-12, "got {}", v),
            ref other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_hex_float_uppercase_and_negative_exp() {
        // 0xA.BP-10 = (10 + 11/16) * 2^-10
        let source = "0xA.BP-10 0X1P4";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        match tokens[0] {
            Token::Float(v) => {
                let expected = (10.0 + 11.0 / 16.0) * 2f64.powi(-10);
                assert!((v - expected).abs() < 1e-15, "got {}", v);
            }
            ref other => panic!("Expected Float, got {:?}", other),
        }
        assert_eq!(tokens[1], Token::Float(16.0));
    }

    #[test]
    fn test_hex_float_with_underscores() {
        let source = "0x1_0.8p1";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        // 0x10.8 = 16.5，* 2 = 33.0
        assert_eq!(tokens[0], Token::Float(33.0));
    }

    #[test]
    fn test_hex_integer_not_confused_with_hex_float() {
        let source = "0x1F 0xABCD 0o17 0b1010";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerRadix(0x1F));
        assert_eq!(tokens[1], Token::IntegerRadix(0xABCD));
        assert_eq!(tokens[2], Token::IntegerRadix(0o17));
        assert_eq!(tokens[3], Token::IntegerRadix(0b1010));
    }

    #[test]
    fn test_integer_type_suffixes() {
        let source = "55i8 300i32 0b110111_i8 0x1Fi8 7u64";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerSuffixed((55, IntSuffix::I8)));
        assert_eq!(tokens[1], Token::IntegerSuffixed((300, IntSuffix::I32)));
        assert_eq!(tokens[2], Token::IntegerSuffixed((55, IntSuffix::I8)));
        assert_eq!(tokens[3], Token::IntegerSuffixed((31, IntSuffix::I8)));
        assert_eq!(tokens[4], Token::IntegerSuffixed((7, IntSuffix::U64)));
    }

    #[test]
    fn test_rune_unicode_escape() {
        // r'\u{26D4}' — Rune 字面量的 unicode 转义
        let source = "r'\\u{26D4}'";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::RuneLit('\u{26D4}'));
    }

    #[test]
    fn test_integer_type_suffix() {
        // 整数字面量的 Cangjie 类型后缀由独立的 IntegerSuffixed token 保留
        let source = "8i8 16u16 64i64 0x0Au8 0b101u32";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerSuffixed((8, IntSuffix::I8)));
        assert_eq!(tokens[1], Token::IntegerSuffixed((16, IntSuffix::U16)));
        assert_eq!(tokens[2], Token::IntegerSuffixed((64, IntSuffix::I64)));
        assert_eq!(tokens[3], Token::IntegerSuffixed((10, IntSuffix::U8)));
        assert_eq!(tokens[4], Token::IntegerSuffixed((5, IntSuffix::U32)));
    }

    #[test]
    fn test_integer_suffix_preserved_no_leak() {
        // 后缀 token 保留类型；无后缀整数仍为 Integer；相邻标识符不被吞并。
        let source = "1i64 2u8 x3 4";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerSuffixed((1, IntSuffix::I64)));
        assert_eq!(tokens[1], Token::IntegerSuffixed((2, IntSuffix::U8)));
        assert_eq!(tokens[2], Token::Ident("x3".to_string()));
        assert_eq!(tokens[3], Token::Integer(4));
    }

    #[test]
    fn test_float16_suffix() {
        // f16 后缀按 Float32 token 解析（WASM 中 Float16 亦为 f32）
        let source = "2.003E4f16 1.5f16";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        match tokens[0] {
            Token::Float32(v) => assert!((v - 20030.0).abs() < 1.0),
            ref other => panic!("Expected Float32, got {:?}", other),
        }
        match tokens[1] {
            Token::Float32(v) => assert!((v - 1.5).abs() < 1e-6),
            ref other => panic!("Expected Float32, got {:?}", other),
        }
    }

    #[test]
    fn test_raw_identifier_backtick() {
        // 规范 1.1(5)：`name` 为原始标识符，应产生 Ident
        let source = "var `A` = 1 func `f`() { 0 }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[1], Token::Ident("A".to_string()));
        assert_eq!(tokens[5], Token::Ident("f".to_string()));
    }

    #[test]
    fn test_raw_identifier_keyword_content() {
        // 反引号内可为关键字：`as`
        let source = "var `as` = 1";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[1], Token::Ident("as".to_string()));
    }

    #[test]
    fn test_raw_identifier_underscore_only_not_ident() {
        // 反引号内容必须是合法标识符字符集（官方正例 `__` 合法；
        // 运算符/空格/等号内容为词法错误，负例 a06_10/a06_24）。
        let source = "`__` `_` `a--` `type A=Int64`";
        let toks: Vec<_> = Lexer::new(source).filter_map(|r| r.ok()).collect();
        assert_eq!(toks.len(), 1, "仅 `__` 应成为标识符；`_` 是通配符不能作原始标识符");
        let n_err = Lexer::new(source).filter(|r| r.is_err()).count();
        assert_eq!(n_err, 3, "`_`/运算符/空格内容应产生词法错误");
    }

    #[test]
    fn test_unicode_identifier() {
        // 官方测试集要求接受非 ASCII 字母标识符（a04 正测试），含 Nl 类字符 〇
        let source = "var 仓颉 = 1 var 〇 = 2 var Venäjän = 3";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[1], Token::Ident("仓颉".to_string()));
        assert_eq!(tokens[5], Token::Ident("〇".to_string()));
        assert_eq!(tokens[9], Token::Ident("Venäjän".to_string()));
    }

    #[test]
    fn test_integer_uppercase_base_prefix() {
        // 规范 1.3.1：0O/0B/0X 大写前缀合法
        let source = "0O17 0B1010 0XFF";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerRadix(15));
        assert_eq!(tokens[1], Token::IntegerRadix(10));
        assert_eq!(tokens[2], Token::IntegerRadix(255));
    }

    #[test]
    fn test_float_exp_sign_only_minus_and_radix_forms() {
        // 官方：指数符号只允许 -（a06_42/fpl_2 负例拒绝 e+/p+）；
        // 进制前缀整数不能初始化 Float 注解（a06_47 由 parser 检查）。
        let source = "1e-1 1E-1 1e1 0x0p-1";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Float(0.1));
        assert_eq!(tokens[1], Token::Float(0.1));
        assert_eq!(tokens[2], Token::Float(10.0));
        assert_eq!(tokens[3], Token::Float(0.0)); // 0x0p-1 = 0 * 2^-1 = 0
        // 0x_0p0（前缀后下划线）、1e+1 / 0xA.BP+10（指数 + 号）、1e0.0
        //（小数点跟在指数后）均为非法字面量：lexer 对其产生 Err。
        let bad_src = "0x_0p0 1e+1 0xA.BP+10 1e0.0";
        let n_err = Lexer::new(bad_src).filter(|r| r.is_err()).count();
        assert!(n_err >= 3, "非法浮点形态未被拒绝, err={}", n_err);
    }

    #[test]
    fn test_integer_uppercase_prefix_with_suffix() {
        let source = "0O371u8 0B1111i64 0XFAu64";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::IntegerSuffixed((0o371, IntSuffix::U8)));
        assert_eq!(tokens[1], Token::IntegerSuffixed((15, IntSuffix::I64)));
        assert_eq!(tokens[2], Token::IntegerSuffixed((0xFA, IntSuffix::U64)));
    }

    #[test]
    #[test]
    fn test_float_leading_dot_with_suffix() {
        // 前导点带后缀拆分：.002f32 → Dot + Float32(词法值不含前导点，
        // 实际数值由解析器 try_leading_dot_float 从源码切片重建)
        let source = ".002f32";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::Dot);
        match tokens[1] {
            Token::Float32(_) => {}
            ref other => panic!("Expected Float32, got {:?}", other),
        }
    }

    #[test]

    #[test]
    fn test_rune_escape_fvb_dollar_quote() {
        // Rune 转义：\f \v \b \$ \"
        let source = r#"r'\f' r'\v' r'\b' r'\$' r'\"'"#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::RuneLit('\u{0C}'));
        assert_eq!(tokens[1], Token::RuneLit('\u{0B}'));
        assert_eq!(tokens[2], Token::RuneLit('\u{08}'));
        assert_eq!(tokens[3], Token::RuneLit('$'));
        assert_eq!(tokens[4], Token::RuneLit('"'));
    }

    #[test]
    fn test_byte_escape_unicode_and_controls() {
        // b'\u{A}' = 10，b'\u{AB}' = 171，b'\f' = 12，b'\"' = 34
        let source = r#"b'\u{A}' b'\u{AB}' b'\f' b'\"'"#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(tokens[0], Token::ByteLit(10));
        assert_eq!(tokens[1], Token::ByteLit(171));
        assert_eq!(tokens[2], Token::ByteLit(12));
        assert_eq!(tokens[3], Token::ByteLit(b'"'));
    }

    #[test]
    fn test_byte_escape_dollar_rejected() {
        // ByteEscapeSeq 不包含 \$：不应产生单个 ByteLit token（会拆成 Ident + CharLit，
        // 进而由语法分析阶段拒绝该非法程序）
        let lexer = Lexer::new(r"b'\$'");
        let toks: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert!(
            !toks.iter().any(|t| matches!(t, Token::ByteLit(_))),
            "b'\\$' must not be a byte literal, got {:?}",
            toks
        );
    }

    #[test]
    fn test_multiline_string_escaped_triple_quote() {
        // 多行字符串中 \""" 不应提前终止，转义后内容为 """（`"""` 后须先换行）
        let source = "let s = \"\"\"\nabc\\\"\"\" def\"\"\"";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).map(|(_, t, _)| t).collect();
        assert_eq!(
            tokens[3],
            Token::MultiLineStringLit(StringOrInterpolated::Plain(
                "abc\"\"\" def".to_string()
            ))
        );
    }
}

#[cfg(test)]
mod batch04_tests {
    use super::*;

    fn lex_ok(src: &str) -> Vec<Token> {
        let mut out = Vec::new();
        for t in Lexer::new(src) {
            match t {
                Ok((_, tok, _)) => out.push(tok),
                Err(e) => panic!("lex error: {} in {:?}", e, src),
            }
        }
        out
    }

    #[test]
    fn test_form_feed_is_whitespace() {
        // 规范 A.1.2(1)：WS 包含 U+000C
        let toks = lex_ok("let\u{000C}a\u{000C}=\u{000C}1");
        assert_eq!(toks[0], Token::Let);
        assert_eq!(toks[1], Token::Ident("a".to_string()));
        assert_eq!(toks[2], Token::Assign);
    }

    #[test]
    fn test_w7b_invalid_numeric_literal_forms() {
        // 1.3.1/1.3.2 负例：`0` + 非前缀字母、纯整数直接接浮点后缀。
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        assert!(err_count("0a12345") > 0);
        assert!(err_count("0F12") > 0);
        assert!(err_count("654f32") > 0);
        assert!(err_count("0f16") > 0);
        // 十六进制浮点不允许类型后缀（规范 1.3.2 负例 `0x0p0f16`）
        assert!(err_count("0x1p1f32") > 0);
        // 前导零非法（`01` / `01.0`）
        assert!(err_count("01") > 0);
        assert!(err_count("01.0") > 0);
        // 合法形态不受影响：前缀、类型后缀、小数/指数浮点、前导点小数、
        // 十六进制浮点（无后缀）。
        assert_eq!(
            err_count("0x1F 0b101 0o77 0i64 0.5f32 .5f16 1e3f64 0x1p1 0.0f16"),
            0
        );
        // 前导点浮点的分数部分前导零合法（`.00e0` 拆成 `.` + `00e0`）
        assert_eq!(err_count("let a = .00e0"), 0);
    }

    // ==== w9b: 词法边角（字符串转义 / 跨行 / 数字前导零）====

    #[test]
    fn test_w9b_invalid_string_escape_rejected() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        // 规范 1.3.6：非法转义序列
        assert!(err_count(r#"let s = "bad \%""#) > 0);
        assert!(err_count(r#"let s = "bad \A""#) > 0);
        assert!(err_count(r#"let s = "bad \1""#) > 0);
        assert!(err_count("let s = \"bad \\ \"") > 0); // `\ `（反斜杠+空格）
        // 正例：合法转义不报错
        assert_eq!(err_count(r#"let s = "a\n\t\"\\\$${x}""#), 0);
    }

    #[test]
    fn test_w9b_unicode_escape_structure() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        // 负例：空、超长、非十六进制
        assert!(err_count(r#"let s = "\u{}""#) > 0);
        assert!(err_count(r#"let s = "\u{00000006c}""#) > 0); // 9 位
        assert!(err_count(r#"let s = "\u{zz}""#) > 0);
        // 正例：1..=8 位十六进制
        assert_eq!(err_count(r#"let s = "\u{41}\u{A}\u{00061}""#), 0);
    }

    #[test]
    fn test_w9b_single_line_string_cannot_span_lines() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        assert!(err_count("let s = \"a\nb\"") > 0);
        assert!(err_count("let s = \"line\r\nnext\"") > 0);
        assert_eq!(err_count("let s = \"single line\"") , 0);
    }

    #[test]
    fn test_w9b_multiline_string_rules() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        // 负例：`"""` 后无换行
        assert!(err_count("let s = \"\"\"\"\"\"") > 0); // 六引号
        assert!(err_count("let s = \"\"\"123\"\"\"") > 0);
        // 负例：多行字符串内的非法转义
        assert!(err_count("let s = \"\"\"\nbad \\1\"\"\"") > 0);
        // 正例：以换行开头的多行字符串
        assert_eq!(err_count("let s = \"\"\"\nhello\"\"\""), 0);
    }

    #[test]
    fn test_w9b_empty_interpolation_rejected() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        assert!(err_count(r#"let s = "${}""#) > 0);
        assert!(err_count("let s = \"\"\"\n${}\n\"\"\"") > 0);
        // 正例
        assert_eq!(err_count(r#"let s = "${x + 1}""#), 0);
    }

    #[test]
    fn test_w9b_leading_zero_and_adjacent_numbers() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        assert!(err_count("let a = 01") > 0);
        assert!(err_count("let a = 01.0") > 0);
        // 八进制混入 8/9 → 被拆成相邻数字
        assert!(err_count("let a = 0O1779u8") > 0);
        // 正例
        assert_eq!(err_count("let a = 0"), 0);
        assert_eq!(err_count("let a = 0.5"), 0);
        assert_eq!(err_count("let a = 0O177"), 0);
    }

    #[test]
    fn test_w19a_dot_exponent_without_fraction() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        // 规范 1.3.2 负例：小数点后直接接指数。
        assert!(err_count("let a = 1.e2") > 0);
        assert!(err_count("let a = 123.e2") > 0);
        assert!(err_count("let a = 4142.e-5") > 0);
        // 正例：正常浮点与成员访问不影响。
        assert_eq!(err_count("let a = 1.5"), 0);
        assert_eq!(err_count("let a = 1.5e2"), 0);
        assert_eq!(err_count("let a = 42.toString") , 0);
    }

    #[test]
    fn test_w19a_single_line_interpolation_no_newline() {
        let err_count = |s: &str| Lexer::new(s).filter(|r| r.is_err()).count();
        // 2.1.6.2(5)：单行字符串的插值表达式内不允许换行。
        assert!(err_count("let a = \"x${\n1 + 2\n}\"") > 0);
        // 正例：同一行插值。
        assert_eq!(err_count("let a = \"x${1 + 2}\""), 0);
    }
}
