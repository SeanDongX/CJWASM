[中文说明](README.md)

# CJWasm

A Cangjie-to-WebAssembly compiler frontend written in Rust.

CJWasm compiles Cangjie source code into WASM bytecode: sources are parsed into an AST, lowered to the CHIR intermediate representation, optimized at the CHIR level (small-function inlining, redundant local elimination), and then emitted as a `.wasm` file that runs in wasmtime, browsers, and other WebAssembly runtimes.

## Project Status

- ✅ **Official Conformance Suite**: overall **99.1%** (positive tests 5,132/5,132 = **100%**, negative tests 19,832/20,069 = **98.8%**)
- ✅ **std API Suite**: **99.3%** (1,413/1,441; L1 **100%** / L2 99.2%, 99.8% excluding @Unittest cases; the 28 non-passing split into 10 official negatives that are correctly rejected + 18 structural skips)
- ✅ **Tests**: 2,188 tests passing (1,543 unit + 631 integration + 14 stdlib)
- ✅ **Examples**: 40/40 system tests passing (100%), 0 WASM validation errors
- ✅ **Code Size**: ~130,900 lines of Rust including tests
- ✅ **AST Nodes**: 117 nodes (Expr: 58, Stmt: 17, Pattern: 11, Type: 31)
- ✅ **CHIR Optimizer**: small-function inlining + redundant local elimination; 1.44x on wasmtime for the heavy benchmark (measured 2026-11-26)

## Official Test Suite Compatibility

Compatibility is measured against the official Cangjie test suite (`third_party/cangjie_test`, a git submodule):

| Suite | Scale | Pass criteria | Current |
|------|------|----------|----------|
| **Conformance compiler suite** | 29,705 test files across 17 spec chapters (25,201 executed after skipping 4,504 multi-file structural cases) | positive tests must compile; negative tests (`@Negative: yes`) must fail compilation | **99.1%** (positive **100%** / negative 98.8%) |
| **std API suite** | 1,441 test files across 33 std modules (L1 + L2) | source files must be accepted by the compiler | **99.3%** (L1 **100%** / L2 99.2%) |

During the full-compatibility sprint started on 2026-11-24, overall compatibility rose from a **49.1%** baseline (positive 60.1% / negative 46.3%) batch by batch:

```
49.1% ──▶ 65.1% ──▶ 79.3% ──▶ 91.2% ──▶ 93.7% ──▶ 95.3% ──▶ 96.0% ──▶ 97.0% ──▶ 98.1% ──▶ 99.1%
baseline  batch4   batch7   batch12  batch14  batch15  batch16  batch19  batch23  batch35
```

Red lines enforced on every merged batch: positive tests stay at 100%, std suite shows zero regression, all 2,188 cargo tests pass, and system tests stay at 40/40. The sprint runs a 4-agent parallel workflow (git-worktree isolated) implementing semantic checks in batches, with 30-minute heartbeat measurements.

The remaining 237 failing negative tests (237/20,069 ≈ 1.2%) are all classified: **160 "X" official contradictions** (144 in the chapter-13 CPointer conversion group + 16 metadata contradictions across chapters), **54 "L" type-system-depth items** (overload resolution / branch least-common-supertype / integer-match exhaustiveness / `as` compatibility, …), **8 "G" grammar/macro/name-resolution items**, **4 "R" runtime-mechanism items**, and **11 "W" multi-file package-structure pseudo-negatives**. Positive tests have zero failures; per-item classification and evidence are in `.compat-run/final-report/classification.md`.

Of the std suite's 1,441 files, the 28 non-passing split into two parts: 10 are official negative cases (macro_errors / invalid_syntax / foreignclass / strategy_wrong_target / out-of-range random integers, …) that the compiler **correctly rejects**, and 18 are structural skips (deriving / argOpt) because the corresponding cross-file/derived structure is absent.

To reproduce the measurement: `git submodule update --init third_party/cangjie_test`, then compile every suite file with `target/release/cjwasm` — positive tests must succeed, negative tests must fail (non-zero exit).

## Features

### Core Language
- **Complete type system**: Int8~Int64, UInt8~UInt64, Float16/32/64, Bool, Rune, String, Array, Tuple, Option, Result
- **Literal precision**: integer type suffixes (`i8`/`i16`/`i32`/`i64`/`u8`/`u16`/`u32`/`u64`), hexadecimal floats, nested block comments, UTF-8 escape whitelist
- **Object-oriented features**: structs, classes (inheritance, abstract/sealed/open, primary constructors, multiple inits, static initializers), interfaces with default implementations and generic methods, properties (`prop`/`mut prop`), extension methods (`extend`)
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
- **@C interoperability**: `@C` structs/foreign functions, `CFunc`/`CType` cross-language type rules (compile-time-checkable subset of ch. 13)

### Sprint additions: compile-time diagnostics and rules (batches 15–19)
- **Raw identifiers** — backtick identifiers (e.g. `` `type` ``) with reserved-word conflict handling
- **unsafe context checking** — `@C` / `foreign` / `CFunc` calls must occur inside an `unsafe` context, otherwise compilation fails
- **@C/CFunc/CType compile-time rules** — struct layout and cross-language parameter/return type constraints decidable at compile time
- **extend generic-argument validation** — arity and bound checks on `extend` declarations
- **Constructor member-parameter visibility and mutability** — `let` / `private` member parameters, named-parameter consistency, mutability rules
- **Mutable closure capture is not a first-class value** — closures capturing mutable variables may not escape as first-class values
- **String-interpolation type diagnostics** — type errors and undeclared calls inside interpolation expressions
- **Concurrency built-in type constraints** — compile-time constraints for Future / spawn / ThreadLocal / Atomic families
- **Enum semantics** — enums cannot be inherited; duplicate constructor-name detection
- **Literal details** — radix prefixes cannot initialize a `Float` annotation; float literal exponents allow only a negative sign

### Sprint additions: root-cause fixes and check families (batches 20–35)
- **Generic-constraint instantiation solving** — type-alias inherited constraints, explicit type-argument constraint solving at instance-method call sites, and interfaces used as type arguments must implement their abstract static functions (2.4.1(9) / 5.3.2(5) / 9.5.2(5))
- **Overload-resolution ambiguity** — multi-candidate equally-matching ambiguity: default-valued parameters excluded from specificity comparison (5.3.2(9)), named-argument literal-adaptation ambiguity (10.1.3.2), variadic-sugar ambiguity (5.3.4(11)), and generic-parameter-name-normalized identical signatures
- **`--checked` runtime check mode** — inserts integer-overflow / array-bounds / assertion `trap` checks on demand, used for runtime verdicts of `@Mode:run` negatives
- **AST root-cause variants** — `Type::VArray(elem, len)`, `Expr::FloatSuffixed`, `Expr::IntegerOverflow`, `Expr::GenericFunctionRef`, making four previously undecidable semantics (length consistency / float suffix / i64 overflow / generic function as value) decidable in one shot
- **Method arity and exhaustiveness check family** — call arity under-supply, selector-less enum-match exhaustiveness, scalar/enum least common supertype, `inout` argument checks

### Memory and Runtime
- **Memory management**: heap allocator (bump + free-list); the default CHIR backend is a bump-only allocator that never reclaims, with a `memory.grow` fallback when capacity runs out (initial 4 pages = 256 KB, max 256 pages = 16 MB; grow only triggers on paths that would otherwise be out-of-bounds, so default behavior is unchanged); RC / mark-sweep remain implemented-but-unwired stock code, designed around "one run per WASM instance"; long-running / very-large-memory programs still need a reclamation plan — the mid-term recommendation is WasmGC (see Known Limitations and `.compat-run/gc-research.md`)
- **Module system**: multi-file compilation, automatic `import` dependency resolution, cjpm project support
- **Standard library**: math, string operations, formatting, sorting, time/random via WASI
- **Built-in test assertions**: `@Assert(a, b)` / `@Expect(a, b)`

### Compile-Time Optimizations
- **Type inference**: local variables, globals, method return types
- **Type coercion**: automatic conversion instructions (i32 ↔ i64, Bool ↔ Int64)
- **Constant folding**
- **Dead code elimination**
- **CHIR small-function inlining**: call sites of pure single-expression functions (no side effects, non-recursive, ≤ 8 expression nodes) are replaced by the function body, removing call overhead
- **CHIR redundant local elimination**: `local.set` + `local.get` pairs for single-write/single-read adjacent pure bindings are eliminated

> CHIR optimizations are on by default; set `NO_CHIR_OPT=1` to disable them, or `NO_CHIR=1` to fall back to the legacy direct AST → WASM path.

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

// Classes and inheritance (an override needs the parent method to be public open)
open class Animal {
    var name: Int64;
    init(name: Int64) { this.name = name }
    public open func speak(): Int64 { return 0 }
}

class Dog <: Animal {
    init(name: Int64) { super(name) }
    public override func speak(): Int64 { return 1 }
}

// Enums and pattern matching
enum Shape {
    | Circle(Int64)
    | Square(Int64)
}

func area(s: Shape): Int64 {
    match (s) {
        case Shape.Circle(r) => r * r * 3
        case Shape.Square(s) => s * s
    }
}

// Error handling
func safeDivide(a: Int64, b: Int64): Result<Int64, String> {
    if (b == 0) { return Err("division by zero") }
    return Ok(a / b)
}

func unwrapOr(r: Result<Int64, String>, fallback: Int64): Int64 {
    match (r) {
        case Ok(v) => v
        case Err(_) => fallback
    }
}

func maybe(b: Bool): Option<Int64> {
    if (b) { return Some(7) }
    return None
}

// Test assertions
main(): Int64 {
    let fib = fibonacci(10)
    @Assert(fib, 55)

    let result = unwrapOr(safeDivide(10, 2), 0)
    @Assert(result, 5)

    let opt = maybe(false) ?? 42
    @Assert(opt, 42)

    let dog = Dog(1)
    @Assert(dog.speak(), 1)

    let square = area(Shape.Square(4))
    @Assert(square, 16)

    let pair = Pair<Int64, Int64>(1, 2)
    @Assert(pair.first + pair.second, 3)

    return fib + result
}
```

> This example is verified to compile with `target/release/cjwasm` and run under wasmtime (`main` returns 60, all `@Assert`s pass).

More examples are available under [`tests/examples/`](tests/examples/).

## Test Coverage

### System Tests (40/40 Passing)

| Category | Examples | Status |
|------|------|------|
| **Basics** | hello.cj, println.cj, literals.cj, advanced.cj | ✅ |
| **Control flow** | control_flow.cj, loop_control.cj, for_in_and_guards.cj | ✅ |
| **Functions & methods** | functions.cj, methods.cj | ✅ |
| **OOP** | class.cj, inheritance.cj, interface.cj, phase5_interface.cj | ✅ |
| **Data structures & patterns** | enum.cj, patterns.cj | ✅ |
| **Generics** | generic.cj, generic_advanced.cj | ✅ |
| **Error handling** | error_handling.cj, phase6_error_module.cj | ✅ |
| **Collections** | p3_collections.cj, p4_collections.cj, p3_option_tuple.cj | ✅ |
| **Operators & strings** | operators.cj, strings.cj, str_methods_test.cj | ✅ |
| **Concurrency** | p5_concurrent.cj | ✅ |
| **Stdlib** | std_math.cj, std_features.cj, p5_stdlib.cj, type_methods.cj | ✅ |
| **P2/P6 features** | p2_features.cj, phase2_types.cj, p6_new_features.cj | ✅ |
| **Memory & modules** | memory_management.cj, modules.cj, math.cj | ✅ |
| **Multi-file / project** | multifile/, project/ (cjpm), std/ (L1 stdlib) | ✅ |
| **Std compatibility** | std_test.sh — `third_party/cangjie_test`, L1 modules 100% | ✅ |

> The std compatibility test requires the submodule: `git submodule update --init third_party/cangjie_test`.

### Code Coverage

> The figures below were measured on **2026-11-23, before the compatibility sprint** (64.7k lines / 1,320 tests at the time); they have not been re-measured after the sprint added ~55k lines of semantic checks (requires the `cargo-llvm-cov` toolchain).

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

### Unit and Integration Tests (2,188 Passing)

```bash
cargo test
# 1,543 passed (unit) + 631 passed (integration) + 14 passed (stdlib)
```

| Category | Count | Notes |
|------|--------|------|
| **Semantic analysis** | 642 | class/interface/member/constructor rules + expression typing, operator, overload-resolution and generic-constraint rules — the bulk of the compatibility sprint |
| **Parsing** | 416 | declarations, statements, expressions, patterns (incl. property syntax, modifiers, @C retention) |
| **CHIR lowering** | 161 | AST → CHIR expressions/statements, CHIR-level type inference |
| **Code generation** | 91 | WASM generation, CHIR codegen, macro compilation |
| **Lexing** | 77 | tokens, integer suffixes, hex floats, nested comments, raw identifiers |
| **Pipeline** | 57 | end-to-end compilation, cjpm project parsing |
| **Optimizer/monomorph** | 41 | constant folding, DCE, generic instantiation |
| **Metadata** | 31 | stdlib type metadata |
| **Type checking** | 12 | symbol resolution, inference |
| **AST/memory** | 16 | syntax-tree construction, memory allocator |
| **Integration** | 631 | full source → WASM pipeline |
| **Stdlib** | 14 | std module compile validation |

### Official Suite Compatibility Tests

`third_party/cangjie_test` (git submodule) provides two official suites:

- **Conformance compiler suite** (29,705 files / 17 spec chapters): positive tests (5,132) must compile, negative tests (20,069) must fail compilation — currently **99.1%** (positive **100%** / negative 98.8%)
- **std API suite** (1,441 files / 33 std modules): L1+L2 compile validation — currently **99.3%** (L1 **100%** / L2 99.2%)

See the "Official Test Suite Compatibility" section above for methodology and sprint progress.

### Known Limitations

1. **Remaining negative-test makeup** — 237 failing negatives (1.2% of 20,069) are all negatives; positive tests are 5,132/5,132 with zero failures. By category: **160 "X" official contradictions + 54 "L" type-system-depth + 8 "G" grammar/macro + 4 "R" runtime + 11 "W" multi-file pseudo-negatives**. Of the std suite's 1,441 files, 1,413 pass (99.3%); the 28 non-passing are 10 official negatives correctly rejected + 18 structural skips (deriving / argOpt), not compiler defects. Per-item classification and evidence are in `.compat-run/final-report/classification.md`
2. **Official contradictions (160 "X" items, unfixable)** — 144 are the chapter-13 CPointer conversion group: `a04/gen/cvt.cj.tpl` positives and `cvt_neg.cj.tpl` negatives emit byte-identical conversion statements in the same non-`unsafe` context (132 have byte-for-byte positive counterparts + 12 are `Src==Dst` identity conversions), indistinguishable from a frontend; 16 more are cross-chapter metadata contradictions (unsigned/Bool unary minus, `!Bool`, a mislabeled range positive, setter tail values, interface extensions, …). All frozen
3. **Type-system depth ("L" 54 items)** — needs an overload-resolution engine (candidate set / most-specific matching / named args / defaults / variadic sugar / generic instantiation in one solver), branch least-common-supertype (user types and interface-set intersection), integer match value-range exhaustiveness, `as` type compatibility, lambda parameter-annotation marking with contextual inference, interface default-method this-type, enum bare-name / first-class-constructor ambiguity, etc. — mostly major infrastructure, with low per-rule yield and a high risk of breaking positives
4. **Grammar/macro/runtime/multi-file (G 8 + R 4 + W 11 = 23 items)** — macro declaration tables, annotation-name resolution, undefined-symbol checks under `import` (G); `throw` executing and trapping during global/field/static initialization as `@Mode:run` semantics (R); and pseudo-negatives whose errors come from `custom.py`/directory-layout-driven multi-file package builds that a single-file harness cannot reproduce (W)
5. **WASM garbage collection is not wired up** — the default CHIR backend is a bump-only allocator, hardened per GC route A: `__alloc`'s bump branch calls `memory.grow` on demand when `heap_ptr + size` would cross the current linear-memory size (initial 4 pages = 256 KB, declared max 256 pages = 16 MB), and traps only if grow fails; grow runs only on the paths that would have been out-of-bounds, so default behavior is byte-for-byte unchanged. The heap still never shrinks, and `src/memory.rs`'s free-list / reference counting / mark-sweep remain implemented-but-never-emitted stock code. Because both the 5,132 positive tests and the 1,413 std red lines are "compile-only" verdicts and examples create a fresh instance per run and exit, GC has zero impact on the red lines; for long-lived / very-large-memory modules the mid-term recommendation is to evaluate WasmGC (wasmtime 51 fully supports it, wasm-encoder 0.220 can emit it) rather than hand-rolling a linear-memory GC — route comparison and implementation checklist in `.compat-run/gc-research.md`
6. **Other engineering boundaries** — the `std/` package (97 stdlib files) compiles, but complex nested generics (e.g. `Map<K, Tuple<Array<...>>>`) still produce WASM validation type mismatches; some generic methods still use stubs instead of complete monomorphization; macro expansion is still limited

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
│   ├── sema/              # semantic analysis
│   ├── typeck/            # type checking
│   ├── metadata/          # stdlib type metadata
│   ├── chir/              # CHIR intermediate representation (default codegen path)
│   │   └── optimize.rs    # CHIR optimization passes
│   ├── optimizer/         # AST-level optimizations
│   ├── monomorph/
│   ├── codegen/           # incl. chir_codegen.rs (default) and legacy path
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
  CHIR Lowering         AST → CHIR intermediate representation
    │
    ▼
  CHIR Optimizer        small-function inlining / redundant local elimination (on by default)
    │
    ▼
  CHIR CodeGen (wasm-encoder) CHIR → WASM bytecode (including WASI runtime helpers)
    │
    ▼
  .wasm                 runnable in wasmtime / browsers
```

> The CHIR path is the default; `NO_CHIR=1` falls back to the legacy direct AST → WASM path and `NO_CHIR_OPT=1` disables just the CHIR optimization passes.

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
| **Memory** | heap allocation (bump + free-list + `memory.grow` fallback, up to 16 MB), designed for one run per instance; RC/mark-sweep implemented but not wired | ✅ |
| **Operators** | arithmetic, comparison, logical, bitwise, power `**`, `!in`, interpolation, `as`, `?.`, overloads | ✅ |
| **Collections** | HashMap, HashSet, ArrayList, LinkedList, ArrayStack | ✅ |
| **Concurrency stubs** | spawn, synchronized, AtomicInt64/AtomicBool, Mutex/ReentrantMutex | ✅ |
| **Stdlib** | math, string ops, array helpers, `now()` | ✅ |
| **Formatting** | `Int64.format`, `Float64.format`, `toString()` | ✅ |
| **WASI** | println/print, time, random, sort, exit | ✅ |
| **Testing** | `@Assert`, `@Expect`, single-argument boolean form, automatic type coercion | ✅ |
| **External interfaces** | `extern func` + `@import("module", "name")` | ✅ |
| **C interop** | `@C` structs/classes, foreign functions, `CFunc`/`CType` type rules, `@C` constructor rules (compile-time subset of ch. 13) | ✅ |
| **Raw identifiers** | backtick identifiers (e.g. `` `type` ``) with reserved-word conflict handling | ✅ |
| **unsafe context** | `@C` / `foreign` / `CFunc` calls must be inside `unsafe`, otherwise a compile-time error | ✅ |
| **extend validation** | arity and bound checks on `extend` declarations | ✅ |
| **Constructor parameters** | member-parameter visibility/mutability (`let` / `private`), named-parameter consistency | ✅ |
| **Closure capture** | closures capturing mutable variables cannot escape as first-class values | ✅ |
| **Interpolation diagnostics** | type errors and undeclared calls inside string interpolation | ✅ |
| **Concurrency type constraints** | compile-time type constraints for Future / spawn / ThreadLocal / Atomic | ✅ |
| **Enum semantics** | enums cannot be inherited; duplicate constructor-name detection | ✅ |
| **Literal details** | radix prefixes cannot initialize a `Float` annotation; float exponents allow only a negative sign | ✅ |
| **Generic-constraint solving** | type-alias inherited constraints, explicit type-argument constraints on instance methods, interface-as-type-argument static-function implementation checks | ✅ |
| **Overload resolution** | multi-candidate equally-matching ambiguity (defaults / named args / variadic sugar / generic-parameter normalization) | ✅ |
| **Runtime checks** | `--checked` integer overflow / array bounds / assertion traps | ✅ |
| **AST root-cause variants** | VArray length / float suffix / i64 overflow / generic function as value | ✅ |
| **Exhaustiveness & arity** | selector-less enum-match exhaustiveness, scalar least common supertype, call arity under-supply | ✅ |

For the full grammar, see [`docs/spec.md`](docs/spec.md).

## Benchmarks

Comparison against the native Cangjie compiler (`cjc`):

| Metric | cjwasm | cjc | Ratio |
|------|--------|-----|------|
| Compile speed (small) | ~5.4ms | ~184ms | **~34x faster** |
| Compile speed (medium) | ~6.4ms | ~189ms | **~29x faster** |
| Output size (small) | 8.2 KB | 1034 KB | **~126x smaller** |
| Output size (medium) | 9.2 KB | 1036 KB | **~113x smaller** |

> CJWasm is a lightweight frontend that only emits WASM bytecode. `cjc` is a full native compiler that produces executables bundled with the standard library. The two tools have different goals.
> Measured 2026-11-26 (Apple Silicon / Darwin 27, cjc 1.0.5, macOS 14.4 SDK sysroot); end-to-end process-time basis.

### CHIR Optimization Impact

Measured on `bench_heavy.cj` (100,000 dense small-function calls: identity / double / square / addOne / Counter.get, …):

| Build | wasmtime time | WASM size |
|------|---------------|-----------|
| CHIR + optimization (default) | 13.84 ms | 9116 B |
| CHIR without optimization (`NO_CHIR_OPT=1`) | 19.92 ms | 9132 B |

- **1.44x faster under wasmtime (JIT)** — the JIT optimizes both builds, partially masking the difference
- Interpreter mode (wasm-interp, no JIT) reflects the instruction-level gain more directly, but the local wasm-interp build lacks WASI support, so the WASI-importing `bench_heavy` could not be re-measured

### Runtime Performance (wasmtime vs cjc native)

| Workload | wasmtime | cjc native | Ratio |
|------|----------|------------|------|
| Small (27 lines) | 8.04 ms | 22.43 ms | wasm 2.8x faster |
| Medium (109 lines) | 8.23 ms | 22.21 ms | wasm 2.7x faster |
| Large (124 lines) | 8.37 ms | 22.46 ms | wasm 2.7x faster |
| Heavy (89 lines) | 13.76 ms | 73.95 ms | wasm 5.4x faster |

> Measured 2026-11-26 (Apple Silicon / Darwin 27, cjc 1.0.5, macOS 14.4 SDK sysroot); end-to-end process time: wasmtime figures include JIT compilation, native figures include process startup.

```bash
# Full comparison (compile speed + size + runtime + CHIR optimization)
./scripts/benchmark.sh

# Quick mode
./scripts/benchmark.sh --quick

# Single dimension
./scripts/benchmark.sh --compile    # compile speed only
./scripts/benchmark.sh --runtime    # runtime only
./scripts/benchmark.sh --size       # output size only

# Criterion micro-benchmarks (per-stage timings)
cargo bench
```

Outputs:
- Comparison report: `benches/report.html`
- Criterion report: `target/criterion/report/index.html`

## Scripts

| Script | Purpose | Usage |
|------|------|------|
| `run_test.sh` | test runner with interactive levels | `./scripts/run_test.sh [1-5]` |
| `system_test.sh` | compile + validate + run all example `.cj` files | `./scripts/system_test.sh [--verbose] [file.cj]` |
| `benchmark.sh` | CJWasm vs CJC benchmarking (compile/runtime/size/CHIR opts) | `./scripts/benchmark.sh [--quick\|--compile\|--runtime\|--size\|--criterion]` |
| `coverage.sh` | coverage report generation | `./scripts/coverage.sh [--html]` |

## Development

### Build and Test

```bash
cargo build --release
cargo test                       # 2,188 tests
./scripts/run_test.sh
./scripts/coverage.sh
./scripts/coverage.sh --html

# Official suite compatibility (initialize the submodule first)
git submodule update --init third_party/cangjie_test
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
