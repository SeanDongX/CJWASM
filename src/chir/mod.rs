//! CHIR (Cangjie High-level IR) - 中间表示层
//!
//! CHIR 是 AST 和 WASM 之间的中间表示，提供完整的类型信息和符号解析。
//!
//! 架构: AST → CHIR → WASM
//!
//! CHIR 的优势:
//! - 每个表达式都有完整的类型信息（AST 类型 + WASM 类型）
//! - 所有符号都已解析（函数索引、局部变量索引、字段偏移）
//! - 类型转换显式表示，便于生成正确的 WASM 指令
//! - 便于添加优化 Pass

pub mod types;
pub mod builder;
pub mod type_inference;
pub mod lower_expr;
pub mod lower_stmt;
pub mod lower;
pub mod optimize;

pub use types::*;
pub use builder::CHIRBuilder;
pub use type_inference::TypeInferenceContext;
pub use lower::{lower_program, lower_function};
