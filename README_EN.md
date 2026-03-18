[中文说明](README.md)

# CJWasm

A Cangjie-to-WebAssembly compiler frontend written in Rust.

CJWasm compiles Cangjie source code directly into WASM bytecode without an intermediate IR. The generated `.wasm` files can run in wasmtime, browsers, and other WebAssembly runtimes.

## Project Status

- ✅ **Coverage**: **80.15%** line coverage / 89.93% region coverage
- ✅ **Tests**: 1,317 tests passing (672 unit + 631 integration + 14 stdlib)
- ✅ **Examples**: 37/37 examples passing (100%), 0 WASM validation errors
- ✅ **Code Size**: ~47,200 lines of Rust including tests
- ✅ **AST Nodes**: 91 nodes (Expr: 47, Stmt: 15, Pattern: 10, Type: 19)
- ✅ **Completion**: ~98%

## Features

### Core Language
- **Complete type system**: Int8~Int64, UInt8~UInt64, Float16/32/64, Bool, Rune, String, Array, Tuple, Option, Result
- **Object-oriented features**: structs, classes (including inheritance and abstract/sealed), interfaces with default implementations, properties (`prop`), extension methods (`extend`)
- **Generics**: generic functions, structs, classes, enums, constraints, multiple bounds, `where` clauses
- **Pattern matching**: `match`, enum destructuring, struct destructuring, tuple destructuring, `if-let`, `while-let`, guards
- **Error handling**: `try-catch-finally`, `throws`, `Result<T, E>` / `Option<T>`, `?`, null-coalescing `??`

### Advanced Features
- **Collections**: ArrayList, HashMap, HashSet with full support for get/put/remove/contains/size
- **Lambda and closures**: closure expressions, function types, trailing closure syntax
- **Operator overloading**: custom operators such as `op_add`, `op_sub`
- **Type conversions**: `as`, `is`, automatic coercions (Bool ↔ Int64, i32 ↔ i64)
- **Ranges**: `start..end`, `start..=end`, open-ended ranges such as `arr[..end]`, `arr[start..]`
- **String interpolation**: `"Hello, ${name}!"`
- **Optional chaining**: `obj?.method()?.field`
- **Conditional compilation**: `@When[os == "Windows"]`

### Memory and Runtime
- **Memory management**: free-list allocator + reference counting + mark-sweep GC
- **Module system**: multi-file compilation, automatic `import` dependency resolution, cjpm project support
- **Standard library**: math, string operations, formatting, sorting, time/random via WASI
- **Built-in test assertions**: `@Assert(a, b)` / `@Expect(a, b)`

### Compile-Time Optimizations
- **Type inference**: local variables, globals, method return types
- **Type coercion**: automatic conversion instructions (i32 ↔ i64, Bool ↔ Int64)
- **Constant folding**
- **Dead code elimination**

## Quick Start

### Install

```bash
# Clone and build
git clone https://gitcode.com/SeanXDO/CJWasm
cd cjwasm
cargo build --release
```

### Use with a cjpm Project

CJWasm supports the Cangjie package-manager layout and can compile directly from `cjpm.toml`:

```bash
# Initialize a new project
cjwasm init myproject
cd myproject

# Build the project (reads cjpm.toml and discovers .cj files under src/)
cjwasm build

# Specify output
cjwasm build -o app.wasm

# Verbose output
cjwasm build -v

# Run the result
wasmtime run --invoke main target/wasm/myproject.wasm
```

Generated project layout:

```text
myproject/
├── cjpm.toml
└── src/
    └── main.cj
```

### Compile Directly

```bash
# Compile a single file
cjwasm tests/examples/hello.cj

# Specify output
cjwasm tests/examples/hello.cj -o hello.wasm

# Compile multiple files
cjwasm main.cj lib.cj -o app.wasm
```

### Run WASM

```bash
wasmtime run --invoke main hello.wasm
```

### Test

```bash
# Interactive test menu
./scripts/run_test.sh

# Or choose a test level directly
./scripts/run_test.sh 1
./scripts/run_test.sh 2
./scripts/run_test.sh 3
./scripts/run_test.sh 4
./scripts/run_test.sh 5

# Individual commands
cargo test
./scripts/system_test.sh
./scripts/coverage.sh
./scripts/coverage.sh --html
```

## Language Example

```cangjie
// Functions and recursion
func fibonacci(n: Int64): Int64 {
    if (n <= 1) { return n }
    return fibonacci(n - 1) + fibonacci(n - 2)
}

// Generic struct
struct Pair<T, U> {
    var first: T;
    var second: U;
    init(first: T, second: U) {
        this.first = first
        this.second = second
    }
}

// Classes and inheritance
open class Animal {
    var name: Int64;
    init(name: Int64) { this.name = name }
    func speak(): Int64 { return 0 }
}

class Dog <: Animal {
    init(name: Int64) { super(name) }
    override func speak(): Int64 { return 1 }
}

// Enums and pattern matching
enum Shape {
    | Circle(Int64)
    | Rectangle(Int64, Int64)
}

func area(s: Shape): Int64 {
    match (s) {
        case Shape.Circle(r) => r * r * 3,
        case Shape.Rectangle(w, h) => w * h
    }
}

// Error handling
func safeDivide(a: Int64, b: Int64): Result<Int64, String> {
    if (b == 0) { return Err("division by zero") }
    return Ok(a / b)
}

// Test assertions
main(): Int64 {
    let fib = fibonacci(10)
    @Assert(fib, 55)

    let result = safeDivide(10, 2) ?? 0
    @Assert(result, 5)

    return fib + result
}
```

More examples are available under [`tests/examples/`](tests/examples/).

## Test Coverage

### Example Tests (37/37 Passing)

| Category | Examples | Status |
|------|------|------|
| **Basic syntax** | hello.cj, variables.cj, functions.cj, recursion.cj | ✅ |
| **Control flow** | if_else.cj, loops.cj, match.cj, pattern_matching.cj | ✅ |
| **Data structures** | arrays.cj, tuples.cj, structs.cj, enums.cj | ✅ |
| **OOP** | classes.cj, inheritance.cj, interfaces.cj, properties.cj | ✅ |
| **Generics** | generics.cj, generic_constraints.cj, generic_advanced.cj | ✅ |
| **Error handling** | option_result.cj, try_catch.cj, error_propagation.cj | ✅ |
| **Collections** | p3_collections.cj, p4_collections.cj | ✅ |
| **Advanced features** | lambda.cj, closures.cj, operator_overload.cj, type_conversion.cj | ✅ |
| **Strings** | strings.cj, string_interpolation.cj, string_methods.cj | ✅ |
| **Stdlib** | std_math.cj, std_features.cj, type_methods.cj | ✅ |
| **P6 features** | p6_new_features.cj | ✅ |
| **Multi-file** | multifile/ | ✅ |

### Code Coverage

Total line coverage is **80.15%** and region coverage is **89.93%**.

| Module | Line | Region |
|------|----------|------------|
| **ast/** | 98.3% | 100% |
| **lexer/** | 90.6% | 98.7% |
| **parser/** | 73.5% | 93.5% |
| **optimizer/** | 96.0% | 96.1% |
| **monomorph/** | 86.3% | 80.2% |
| **codegen/** | 76.5% | 78.5% |
| **chir/** | 91.4% | 97.5% |
| **typeck/** | 95.2% | 100% |
| **sema/** | 94.1% | 97.9% |
| **metadata/** | 93.8% | 97.5% |
| **pipeline/** | 90.2% | 95.2% |
| **memory/** | 100% | 100% |

```bash
./scripts/coverage.sh
./scripts/coverage.sh --html
```

### Unit and Integration Tests (1,317 Passing)

```bash
cargo test
```

| Category | Count | Notes |
|------|--------|------|
| **Lexing** | 229 | token recognition, keywords, operators |
| **Parsing** | 190 | declarations, statements, expressions, types, patterns |
| **Type checking** | 7 | symbol resolution, inference |
| **Semantic analysis** | 36 | expression inference, function analysis |
| **CHIR lowering** | ~65 | AST → CHIR |
| **Code generation** | 42 | WASM generation, macro compilation |
| **Monomorphization** | 12 | generic instantiation |
| **Optimizer** | 36 | constant folding, DCE |
| **Pipeline** | 35 | end-to-end compilation |
| **Metadata** | 31 | stdlib type metadata |
| **Integration** | 631 | full source → WASM pipeline |
| **Stdlib** | 14 | std module compile validation |

### Known Limitations

1. **`std/` package WASM validation**: large packages with 97 stdlib files compile, but the generated WASM still has type mismatch issues during validation for complex nested generic types such as `Map<K, Tuple<Array<...>>>`.
2. **Generic monomorphization**: some generic methods still use stubs instead of complete monomorphization.
3. **Macro system**: macro expansion is still limited.

## Project Layout

```text
cjwasm/
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── cjpm.rs
│   ├── lexer/
│   ├── parser/
│   ├── ast/
│   ├── optimizer/
│   ├── monomorph/
│   ├── codegen/
│   ├── memory.rs
│   └── pipeline.rs
├── tests/examples/
├── tests/fixtures/
├── benches/
├── scripts/
├── docs/
└── Cargo.toml
```

## Compilation Pipeline

```text
Cangjie source (.cj)
    │
    ▼
  Lexer (logos)         lexical analysis → token stream
    │
    ▼
  Parser                recursive descent → AST (including @Assert/@Expect)
    │
    ▼
  Optimizer             constant folding / dead code elimination
    │
    ▼
  Monomorphizer         generic instantiation
    │
    ▼
  CodeGen (wasm-encoder) AST → WASM bytecode (including WASI runtime helpers)
    │
    ▼
  .wasm                 runnable in wasmtime / browsers
```

## Supported Cangjie Syntax

| Category | Feature | Status |
|------|------|------|
| **Primitive types** | Int8~64, UInt8~64, Float16/32/64, Bool, Rune, String, `const` | ✅ |
| **Composite types** | Array, Tuple, Option, Result, Struct, Enum, Class, Range, Slice, Map | ✅ |
| **Functions** | definitions, recursion, default args, variadic args, named args, lambda, trailing closure, inout, function types | ✅ |
| **Generics** | generic functions/structs/classes/enums, constraints, multiple bounds, `where`, specialization | ✅ |
| **OOP** | classes, inheritance, abstract/sealed, primary constructors, interfaces, default impls, `prop`, operator funcs, `extend`, static init | ✅ |
| **Control flow** | if/else, while, do-while, for-in, loop, break/continue, match, while-let, spawn, synchronized | ✅ |
| **Pattern matching** | enum destructuring, struct destructuring, guards, nested patterns, type patterns, `is` | ✅ |
| **Error handling** | try-catch-finally, try-with-resources, throws, Result/Option, `?`, `??` | ✅ |
| **Modules** | import, multi-file compilation, visibility modifiers | ✅ |
| **Memory** | heap allocation, RC, mark-sweep GC, free-list allocator | ✅ |
| **Operators** | arithmetic, comparison, logical, bitwise, power `**`, `!in`, interpolation, `as`, `?.`, overloads | ✅ |
| **Collections** | HashMap, HashSet, ArrayList, LinkedList, ArrayStack | ✅ |
| **Concurrency stubs** | spawn, synchronized, AtomicInt64/AtomicBool, Mutex/ReentrantMutex | ✅ |
| **Stdlib** | math, string ops, array helpers, `now()` | ✅ |
| **Formatting** | `Int64.format`, `Float64.format`, `toString()` | ✅ |
| **WASI** | println/print, time, random, sort, exit | ✅ |
| **Testing** | `@Assert`, `@Expect`, single-argument boolean form, automatic type coercion | ✅ |
| **External interfaces** | `extern func` + `@import("module", "name")` | ✅ |

For the full grammar, see [`docs/spec.md`](docs/spec.md).

## Benchmarks

Comparison against the native Cangjie compiler (`cjc`):

| Metric | cjwasm | cjc | Ratio |
|------|--------|-----|------|
| Compile speed (small) | ~1ms | ~700ms | **~36x faster** |
| Compile speed (medium) | ~2ms | ~900ms | **~7x faster** |
| Output size (small) | 1.2 KB | 1020 KB | **845x smaller** |
| Output size (medium) | 1.9 KB | 1021 KB | **543x smaller** |

> CJWasm is a lightweight frontend that only emits WASM bytecode. `cjc` is a full native compiler that produces executables bundled with the standard library. The two tools have different goals.

```bash
./scripts/benchmark.sh
./scripts/benchmark.sh --quick
./scripts/benchmark.sh --runtime
cargo bench
```

Outputs:
- Comparison report: `benches/report.html`
- Criterion report: `target/criterion/report/index.html`

## Scripts

| Script | Purpose | Usage |
|------|------|------|
| `run_test.sh` | test runner with interactive levels | `./scripts/run_test.sh [1-5]` |
| `system_test.sh` | compile and run example `.cj` files | `./scripts/system_test.sh [--verbose]` |
| `conformance_diff.sh` | compare Conformance compile results between `cjc` and `cjwasm` (with harness report diff) | `./scripts/conformance_diff.sh [--tests <path>] [--level <n>]` |
| `benchmark.sh` | CJWasm vs CJC benchmarking | `./scripts/benchmark.sh [--quick]` |
| `coverage.sh` | coverage report generation | `./scripts/coverage.sh [--html]` |

## Development

### Build and Test

```bash
cargo build --release
cargo test
./scripts/run_test.sh
./scripts/coverage.sh
./scripts/coverage.sh --html
```

### Dependencies

| Dependency | Purpose |
|------|------|
| [logos](https://crates.io/crates/logos) | lexer generation |
| [wasm-encoder](https://crates.io/crates/wasm-encoder) | WASM bytecode encoding |
| [thiserror](https://crates.io/crates/thiserror) | error derives |
| [ariadne](https://crates.io/crates/ariadne) | compiler diagnostics |
| [toml](https://crates.io/crates/toml) + [serde](https://crates.io/crates/serde) | `cjpm.toml` parsing |
| [criterion](https://crates.io/crates/criterion) | benchmarks |

### Optional Runtime Tools

```bash
brew install wasmtime
brew install hyperfine
brew install wabt
```

## License

Apache 2.0
