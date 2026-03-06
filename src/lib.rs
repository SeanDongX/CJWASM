//! CJWasm：仓颉语言到 WebAssembly 编译器库。

pub mod ast;
pub mod cjpm;
pub mod codegen;
pub mod lexer;
pub mod memory;
pub mod metadata;
pub mod monomorph;
pub mod optimizer;
pub mod parser;
pub mod pipeline;
pub mod sema;
pub mod typeck;