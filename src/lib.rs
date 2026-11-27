//! CJWasm：仓颉语言到 WebAssembly 编译器库。

pub mod ast;
pub mod chir;
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

/// w31a：运行时检查模式开关（`--checked` CLI 选项）。
///
/// CLI 设置环境变量 `CJWASM_CHECKED=1` 后生效。开启后代码生成插入整数
/// 溢出检查、数组下标越界检查、`Assert.equals` 断言检查，并在 `_start`
/// 中把 `main` 的返回值作为进程退出码（仓颉运行期语义）。默认关闭，
/// 生产编译路径完全不变。
pub fn checked_mode() -> bool {
    std::env::var("CJWASM_CHECKED").is_ok()
}