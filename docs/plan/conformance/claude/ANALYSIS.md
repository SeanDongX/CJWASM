# CJWasm2 Conformance Analysis: CJC-Only Passing Tests

**Date:** 2026-03-20
**Test Run:** target/conformance/20260320_205738/

## Executive Summary

**Total tests passing only in CJC: 7,071**
- **Negative tests (CJWasm too permissive): 6,993 (98.9%)**
- **Positive tests (CJWasm too strict): 78 (1.1%)**

### Key Finding
The overwhelming majority (98.9%) of failures are **negative tests** where CJWasm2 should reject invalid code but instead accepts it. This indicates missing semantic validation and error checking throughout the compiler pipeline.

## Top Priority Areas

### 1. Class and Interface (2,699 tests)
- 2,683 negative tests (missing validation)
- 16 positive tests (too strict)
- **Issues:**
  - Abstract functions in non-abstract classes
  - Invalid class modifiers
  - Interface implementation errors
  - Member visibility violations

**Example:** `test_a07_03.cj` - Non-abstract class declaring abstract function
```cangjie
class SomeClass {
  public func inc(x: Int32): Int32  // Missing body, should error
}
```

### 2. Types (1,192 tests)
- 1,184 negative tests
- 8 positive tests
- **Issues:**
  - Integer literal range validation (e.g., `0b1010_1010_i8` exceeds Int8 range)
  - Type compatibility checks
  - Invalid type conversions

**Example:** `test_a01_18.cj` - Integer assigned to Float64
```cangjie
var f: Float64 = 1  // Should require explicit conversion
```

### 3. Expressions (1,099 tests)
- 1,096 negative tests
- 3 positive tests
- **Issues:**
  - Literal type inference
  - Invalid operations
  - Type mismatches

### 4. Packages and Module Management (448 tests)
- 441 negative tests
- 7 positive tests
- **Issues:**
  - Import validation
  - Module visibility
  - Package structure

### 5. Names, Scopes, Variables (443 tests)
- 432 negative tests
- 11 positive tests
- **Issues:**
  - Identifier validation
  - Scope resolution
  - Variable redeclaration

## Specific Validation Gaps

### Lexical Structure (349 tests)
- **Raw identifier validation:** Invalid symbols in backtick identifiers
- **Semicolon/newline rules:** Missing statement separators
- **Literal suffixes:** Invalid type suffixes

**Example:** `test_a05_05.cj` - Raw identifier with space
```cangjie
var ` `: Int32 = 1  // Should reject
```

### Integer Literal Range Checking (14+ tests)
Binary, octal, and hex literals that appear to fit in the type but are actually out of range when interpreted as signed integers.

**Example:** `test_bug_0005869_bil_i16.cj`
```cangjie
main():Int16 {
  0b1010_1010_1010_1010_i16  // 0xAAAA = 43690, exceeds Int16 max (32767)
}
```

### Extension Validation (101 tests)
- Re-extending standard types with interfaces they already implement
- Invalid extension declarations

**Example:** `test_bug_0005814.cj`
```cangjie
extend Int64 <: ToString {}  // Int64 already implements ToString
```

### Exception Handling (37 tests)
- Invalid throw statements
- Return type validation with exceptions

**Example:** `test_bug_0005819.cj`
```cangjie
main() {
    throw Exception()  // Should validate main return type
}
```

### Constant Evaluation (79 tests)
- const function restrictions
- Compile-time evaluation errors

### Overloading (69 tests)
- Invalid overload combinations
- Static vs instance conflicts

## Positive Test Failures (78 tests)

These are valid programs that CJWasm2 incorrectly rejects:

### Categories:
- **Lexical (4):** Valid semicolon/newline usage, literal formats
- **Types (8):** Valid type declarations
- **Names/Scopes (11):** Valid identifier usage
- **Class/Interface (16):** Valid class structures
- **Properties (9):** Valid property declarations
- **Exceptions (7):** Valid exception handling

## Recommended Fix Strategy

### Phase 1: High-Impact Validation (Priority)
1. **Integer literal range checking** (14 tests)
   - Add validation in lexer/parser for typed integer literals
   - Check against min/max values for Int8/16/32/64

2. **Abstract class validation** (100+ tests)
   - Reject abstract functions in non-abstract classes
   - Validate abstract modifier usage

3. **Extension validation** (101 tests)
   - Check for duplicate interface implementations
   - Validate extension targets

### Phase 2: Type System Validation
4. **Type compatibility** (1,000+ tests)
   - Strengthen type checking in expressions
   - Add implicit conversion validation
   - Literal type inference rules

### Phase 3: Semantic Validation
5. **Class/Interface semantics** (2,000+ tests)
   - Member visibility rules
   - Override/redefine validation
   - Interface implementation checking

6. **Scope and naming** (400+ tests)
   - Identifier validation
   - Redeclaration checking
   - Namespace rules

### Phase 4: Edge Cases
7. **Lexical validation** (300+ tests)
   - Raw identifier rules
   - Statement separator rules
   - Literal format validation

## Testing Approach

For each fix:
1. Identify the validation rule from spec
2. Add validation in appropriate compiler phase
3. Run specific test category: `./scripts/system_test.sh <category>`
4. Verify negative tests now fail compilation
5. Ensure positive tests still pass

## Files to Modify

Based on validation type:
- **Lexer:** `src/lexer/mod.rs` - Literal validation, identifier rules
- **Parser:** `src/parser/*.rs` - Syntax validation
- **AST/Semantic:** `src/ast/mod.rs`, `src/chir/lower.rs` - Type checking, semantic rules
- **Type System:** Type inference and compatibility checking

## Conclusion

CJWasm2 is currently too permissive, accepting many invalid programs. The fix priority should focus on:
1. Integer literal range validation (quick win)
2. Class/interface semantic validation (highest impact)
3. Type system strengthening (medium-term)
4. Comprehensive semantic validation (long-term)

Most issues are in semantic validation rather than parsing, suggesting the compiler pipeline needs stronger validation passes after AST construction.
