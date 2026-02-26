//! 解析期错误类型与集中诊断文案，便于 i18n 与统一风格。

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
        write!(
            f,
            "{} (字节偏移 {}-{})",
            self.error, self.byte_start, self.byte_end
        )
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

/// 构造「意外的 token」错误
pub fn unexpected_token(tok: Token, expected: impl Into<String>) -> ParseError {
    ParseError::UnexpectedToken(tok, expected.into())
}

/// 构造「意外的输入结束」错误
pub fn unexpected_eof() -> ParseError {
    ParseError::UnexpectedEof
}
