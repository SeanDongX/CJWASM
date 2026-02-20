//! CJWasm：仓颉语言到 WebAssembly 编译器库。

pub mod ast;
pub mod cjpm;
pub mod codegen;
pub mod lexer;
pub mod macro_expand;
pub mod memory;
pub mod monomorph;
pub mod optimizer;
pub mod parser;
pub mod pipeline;
pub mod stdlib;