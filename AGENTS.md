# CJWasm Agent Guidelines

This file provides guidelines for agentic coding assistants working on the CJWasm codebase.

## Build, Lint, and Test Commands

### Build
```bash
cargo build                    # Debug build
cargo build --release          # Release build (optimized)
```

### Test
```bash
cargo test                     # Run all unit/integration tests (388 tests)
cargo test <test_name>         # Run a single test (e.g., cargo test test_compile_hello_snippet)
cargo test --lib               # Run library tests only
cargo test --tests             # Run integration tests only

# Interactive test menu
./scripts/run_test.sh          # Show menu, select test level 1-5
./scripts/run_test.sh 1        # Unit tests only
./scripts/run_test.sh 2        # System tests (compile .cj examples)
./scripts/run_test.sh 3        # Performance tests
./scripts/run_test.sh 4        # Unit + System tests
./scripts/run_test.sh 5        # All tests

# System tests
./scripts/system_test.sh       # Compile and run all .cj examples, verify output
```

### Benchmarks
```bash
cargo bench                    # Run Criterion benchmarks
./scripts/benchmark.sh         # CJWasm vs CJC performance comparison
./scripts/benchmark.sh --quick # Quick benchmark mode
```

### Coverage
```bash
./scripts/coverage.sh           # Generate coverage report
./scripts/coverage.sh --html   # HTML report at target/llvm-cov/html/
```

## Code Style Guidelines

### Module Organization
- Each major component has its own module: `ast`, `lexer`, `parser`, `codegen`, `optimizer`, `monomorph`, `memory`, `cjpm`, `pipeline`
- Use `pub mod` declarations in `src/lib.rs` to expose modules
- Module-level documentation with `//!` at the top of each file

### Naming Conventions
- **Types/Enums/Structs**: PascalCase (e.g., `Type`, `Program`, `CodeGen`, `ParseError`)
- **Functions/Methods**: snake_case (e.g., `optimize_program`, `fold_expr`, `emit_alloc_func`)
- **Constants**: SCREAMING_SNAKE_CASE (e.g., `HEAP_BASE`, `PAGE_SIZE`, `ALLOC_HEADER_SIZE`)
- **Variables/Fields**: snake_case (e.g., `func_types`, `heap_ptr`, `current_type_params`)
- **Type parameters**: Single uppercase letter (e.g., `T`, `U`, `E`)

### Imports and Ordering
```rust
// 1. External crates
use logos::Logos;
use wasm_encoder::{Instruction, ValType};
use thiserror::Error;

// 2. Standard library
use std::collections::HashMap;
use std::path::Path;

// 3. Internal modules
use crate::ast::{Expr, Stmt, Type};
use crate::lexer::Token;
```

### Error Handling
- Use `thiserror` derive macro for error types
- Custom error structs should include location information (byte offsets)
- Return `Result<T, Error>` for fallible operations
- Use `?` operator for error propagation
- Panic only for truly unrecoverable errors (e.g., type conversion failures)

```rust
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("unexpected token: {0:?}, expected: {1}")]
    UnexpectedToken(Token, String),
    #[error("unexpected end of input")]
    UnexpectedEof,
}
```

### Types and Enums
- Use `#[derive(Debug, Clone, PartialEq, Eq, Hash)]` for AST types
- Use `#[derive(Debug, Clone)]` for enums with non-hashable fields
- Prefer `Option<T>` and `Result<T, E>` over null/error codes
- Use `Box<Type>` for recursive type definitions

### Documentation
- Module-level docs: `//! Module description`
- Public items: `/// Item description`
- Include examples in doc comments where appropriate
- Document memory layout for WASM codegen (offsets, sizes, alignment)

### Code Patterns
- **AST transformations**: Use mutable references for in-place optimization
- **WASM codegen**: Use `wasm_encoder` crate, don't generate raw bytes
- **String handling**: Use `unescape_string()` and `process_multiline_string()` helpers
- **Memory layout**: Document heap object headers, alignment, and offsets
- **HashMap lookups**: Use `.expect()` with descriptive messages for required keys

### Testing
- Unit tests in `src/` files, integration tests in `tests/`
- Test helper functions: `compile_source()`, `assert_valid_wasm()`
- Test names: `test_<feature>_<scenario>` (e.g., `test_compile_arithmetic`)
- Use `assert_eq!`, `assert!` macros, avoid `unwrap()` in tests

### WASM Codegen Specifics
- Memory layout: `[block_size: i32][refcount: i32][user_data...]`
- All allocations aligned to 8 bytes
- Use `mem_arg(offset, align)` helper for memory instructions
- Global indices: 0=heap_ptr, 1=free_list_head, 2+=runtime functions
- IO buffer reserved at addresses 0-127

### Cursor/Copilot Rules
- For Cangjie language questions, use context7 MCP with `yolomao/cangjiecorpus-mirror` repository
- Use `cjpm init`, `cjpm build`, `cjpm run`, `cjpm test` for Cangjie project workflows

## Architecture Notes

### Compilation Pipeline
1. **Lexer** (logos): Source → Token stream
2. **Parser** (recursive descent): Tokens → AST
3. **Optimizer**: Constant folding, dead code elimination
4. **Monomorphizer**: Generic instantiation
5. **CodeGen** (wasm-encoder): AST → WASM bytecode

### Key Data Structures
- `ast::Type`: All Cangjie types (primitives, composite, generics)
- `ast::Program`/`ast::Function`: AST nodes
- `codegen::CodeGen`: Main compiler state with HashMaps for functions, types, strings
- `codegen::ClassInfo`: Runtime class info with vtable and inheritance layout

### Memory Management
- Free List Allocator (malloc/free)
- Reference Counting (RC) with automatic inc/dec
- Mark-Sweep GC for cycle detection
- 8-byte header: [block_size][refcount]

## Dependencies
- **logos**: Lexer generation
- **wasm-encoder**: WASM bytecode encoding
- **thiserror**: Error handling
- **ariadne**: Error diagnostics
- **toml + serde**: cjpm.toml parsing
- **criterion** (dev): Benchmarking
