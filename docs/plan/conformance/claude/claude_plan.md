# CJWasm2 Conformance Fix Plan

**Created:** 2026-03-20
**Status:** Planning
**Target:** Fix 7,071 tests passing only in CJC

## Executive Summary

CJWasm2 currently has 7,071 conformance test failures where the official CJC compiler passes. Analysis reveals that **98.9% of these are negative tests** - meaning CJWasm2 is accepting invalid code that should be rejected. This plan outlines a phased approach to add the missing validation.

## Problem Breakdown

| Category | Tests | Type | Priority |
|----------|-------|------|----------|
| Class/Interface | 2,699 | 2,683 neg, 16 pos | P0 |
| Types | 1,192 | 1,184 neg, 8 pos | P0 |
| Expressions | 1,099 | 1,096 neg, 3 pos | P1 |
| Packages/Modules | 448 | 441 neg, 7 pos | P1 |
| Names/Scopes | 443 | 432 neg, 11 pos | P1 |
| Lexical | 349 | 345 neg, 4 pos | P2 |
| Functions | 282 | 280 neg, 2 pos | P2 |
| Others | 559 | 532 neg, 27 pos | P2 |

## Phase 1: Quick Wins (Week 1-2)

### 1.1 Integer Literal Range Validation
**Impact:** 26 tests
**Effort:** Low
**Priority:** P0

**Problem:**
```cangjie
// Currently accepted, should reject
0b1010_1010_i8        // 170 > Int8::MAX (127)
0xAAAA_i16            // 43690 > Int16::MAX (32767)
```

**Implementation:**
- **File:** `src/lexer/mod.rs`
- **Location:** Integer literal parsing with type suffix
- **Changes:**
  1. After parsing integer value, check if type suffix present
  2. Validate value fits in target type's range:
     - Int8: -128 to 127
     - Int16: -32768 to 32767
     - Int32: -2147483648 to 2147483647
     - Int64: -9223372036854775808 to 9223372036854775807
  3. Return error if out of range

**Test Cases:**
```bash
cargo run -- third_party/cangjie_test/.../test_bug_0005869_bil_i8.cj
cargo run -- third_party/cangjie_test/.../test_bug_0005869_bil_i16.cj
cargo run -- third_party/cangjie_test/.../test_bug_0005869_oil_i32.cj
cargo run -- third_party/cangjie_test/.../test_bug_0005869_xil_i64.cj
```

**Success Criteria:** All 26 integer range tests fail compilation with appropriate error message.

---

### 1.2 Extension Re-implementation Validation
**Impact:** 57 tests
**Effort:** Low
**Priority:** P0

**Problem:**
```cangjie
// Currently accepted, should reject
extend Int64 <: ToString {}  // Int64 already implements ToString
```

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Location:** Extension declaration lowering
- **Changes:**
  1. When processing `extend Type <: Interface`, check if Type already implements Interface
  2. Check both direct implementations and inherited implementations
  3. Return error if interface already implemented

**Test Cases:**
```bash
cargo run -- third_party/cangjie_test/.../test_bug_0005814.cj
cargo run -- third_party/cangjie_test/.../test_bug_0006121_04.cj
```

**Success Criteria:** All 57 extension re-implementation tests fail compilation.

---

### 1.3 Raw Identifier Validation
**Impact:** 50+ tests
**Effort:** Low
**Priority:** P0

**Problem:**
```cangjie
// Currently accepted, should reject
var ` `: Int32 = 1           // Space in raw identifier
var `123`: Int32 = 1         // Starts with digit
```

**Implementation:**
- **File:** `src/lexer/mod.rs`
- **Location:** Raw identifier parsing (backtick identifiers)
- **Changes:**
  1. After extracting content between backticks, validate:
     - Not empty
     - Not only whitespace
     - First character is valid identifier start (letter or underscore)
     - Remaining characters are valid identifier characters
  2. Return error if validation fails

**Test Cases:**
```bash
cargo run -- third_party/cangjie_test/.../test_a05_05.cj
cargo run -- third_party/cangjie_test/.../test_a05_06.cj
```

**Success Criteria:** All raw identifier validation tests fail compilation.

---

## Phase 2: Type System Validation (Week 3-4)

### 2.1 Type Compatibility Checking
**Impact:** 407 tests
**Effort:** Medium
**Priority:** P0

**Problem:**
```cangjie
// Currently accepted, should reject
var f: Float64 = 1           // Integer literal for Float64 (no implicit conversion)
var x: Int32 = 1.0           // Float literal for Int32
```

**Implementation:**
- **File:** `src/chir/lower_expr.rs`
- **Location:** Variable initialization, assignment type checking
- **Changes:**
  1. Strengthen type inference for literals
  2. Add explicit type compatibility checks
  3. Reject implicit conversions between numeric types
  4. Only allow exact type matches or explicit conversions

**Test Cases:**
```bash
cargo run -- third_party/cangjie_test/.../test_a01_18.cj
```

**Success Criteria:** Type mismatch tests fail compilation with clear error messages.

---

### 2.2 Integer Type Literal Context
**Impact:** 200+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Integer literals should infer type from context
- Currently too permissive in type inference

**Implementation:**
- **File:** `src/chir/lower_expr.rs`
- **Location:** Literal type inference
- **Changes:**
  1. Implement proper context-based type inference for integer literals
  2. Validate inferred type is appropriate for context
  3. Reject ambiguous cases

**Success Criteria:** Integer literal type inference tests pass.

---

## Phase 3: Class/Interface Validation (Week 5-8)

### 3.1 Abstract Class Validation
**Impact:** 706 tests
**Effort:** Medium
**Priority:** P0

**Problem:**
```cangjie
// Currently accepted, should reject
class SomeClass {
  public func inc(x: Int32): Int32  // No body, but class not abstract
}
```

**Implementation:**
- **File:** `src/chir/lower.rs` or new semantic validation pass
- **Location:** Class definition validation
- **Changes:**
  1. Track which functions have bodies vs declarations only
  2. If any function lacks a body, verify class has `abstract` modifier
  3. Return error if non-abstract class has abstract functions

**Test Cases:**
```bash
cargo run -- third_party/cangjie_test/.../test_a07_03.cj
cargo run -- third_party/cangjie_test/.../test_a07_04.cj
```

**Success Criteria:** All abstract class validation tests fail compilation.

---

### 3.2 Class Modifier Validation
**Impact:** 500+ tests
**Effort:** Medium-High
**Priority:** P1

**Problem:**
- Invalid combinations of class modifiers
- Modifier conflicts (e.g., `open sealed`)
- Missing required modifiers

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Location:** Class declaration processing
- **Changes:**
  1. Define valid modifier combinations
  2. Check for conflicting modifiers
  3. Validate modifier requirements (e.g., abstract functions require abstract class)

**Success Criteria:** Class modifier tests fail appropriately.

---

### 3.3 Interface Implementation Validation
**Impact:** 800+ tests
**Effort:** High
**Priority:** P1

**Problem:**
- Missing interface method implementations
- Incorrect method signatures
- Visibility violations

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Location:** Interface implementation checking
- **Changes:**
  1. For each implemented interface, verify all methods are implemented
  2. Check method signatures match interface declarations
  3. Validate visibility rules

**Success Criteria:** Interface implementation tests fail appropriately.

---

### 3.4 Member Visibility Validation
**Impact:** 400+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Invalid visibility modifiers
- Access violations
- Override visibility conflicts

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Location:** Member access validation
- **Changes:**
  1. Track member visibility (public, protected, private, internal)
  2. Validate access based on context
  3. Check override visibility rules

**Success Criteria:** Visibility violation tests fail compilation.

---

## Phase 4: Expression Validation (Week 9-10)

### 4.1 Operator Type Validation
**Impact:** 300+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Invalid operand types for operators
- Type mismatches in binary operations

**Implementation:**
- **File:** `src/chir/lower_expr.rs`
- **Location:** Binary/unary operator processing
- **Changes:**
  1. Validate operand types for each operator
  2. Check type compatibility
  3. Reject invalid combinations

**Success Criteria:** Operator type tests fail appropriately.

---

### 4.2 Literal Validation
**Impact:** 200+ tests
**Effort:** Low-Medium
**Priority:** P1

**Problem:**
- Invalid literal formats
- Out of range literals
- Invalid escape sequences

**Implementation:**
- **File:** `src/lexer/mod.rs`
- **Location:** Literal parsing
- **Changes:**
  1. Strengthen literal format validation
  2. Add escape sequence validation
  3. Check literal value ranges

**Success Criteria:** Literal validation tests fail appropriately.

---

## Phase 5: Scope and Naming (Week 11-12)

### 5.1 Identifier Validation
**Impact:** 200+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Invalid identifier names
- Keyword conflicts
- Reserved name usage

**Implementation:**
- **File:** `src/lexer/mod.rs`, `src/chir/lower.rs`
- **Location:** Identifier parsing and validation
- **Changes:**
  1. Validate identifier format
  2. Check against keywords
  3. Validate reserved names

**Success Criteria:** Identifier validation tests fail appropriately.

---

### 5.2 Redeclaration Checking
**Impact:** 150+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Variable redeclaration in same scope
- Function redeclaration
- Type redeclaration

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Location:** Symbol table management
- **Changes:**
  1. Track declared symbols per scope
  2. Check for duplicates before adding
  3. Return error on redeclaration

**Success Criteria:** Redeclaration tests fail compilation.

---

## Phase 6: Package and Module Validation (Week 13-14)

### 6.1 Import Validation
**Impact:** 200+ tests
**Effort:** Medium
**Priority:** P1

**Problem:**
- Invalid import paths
- Circular imports
- Missing modules

**Implementation:**
- **File:** Module resolution code
- **Changes:**
  1. Validate import paths
  2. Detect circular dependencies
  3. Check module existence

**Success Criteria:** Import validation tests fail appropriately.

---

### 6.2 Module Visibility
**Impact:** 150+ tests
**Effort:** Medium
**Priority:** P2

**Problem:**
- Access to private modules
- Invalid export declarations

**Implementation:**
- **File:** Module system
- **Changes:**
  1. Track module visibility
  2. Validate cross-module access
  3. Check export declarations

**Success Criteria:** Module visibility tests fail appropriately.

---

## Phase 7: Remaining Categories (Week 15-16)

### 7.1 Function Validation
**Impact:** 282 tests
**Effort:** Medium
**Priority:** P2

**Problem:**
- Invalid function signatures
- Return type mismatches
- Parameter validation

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Changes:**
  1. Validate function signatures
  2. Check return types
  3. Validate parameters

---

### 7.2 Exception Handling
**Impact:** 37 tests
**Effort:** Low
**Priority:** P2

**Problem:**
```cangjie
// Currently accepted, should reject
main() {
    throw Exception()  // main() return type validation
}
```

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Changes:**
  1. Validate main() return type with throw
  2. Check exception handling rules

---

### 7.3 Constant Evaluation
**Impact:** 79 tests
**Effort:** Medium
**Priority:** P2

**Problem:**
- const function restrictions
- Invalid compile-time operations

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Changes:**
  1. Validate const function rules
  2. Check compile-time evaluation

---

### 7.4 Overloading
**Impact:** 69 tests
**Effort:** Medium
**Priority:** P2

**Problem:**
- Invalid overload combinations
- Static vs instance conflicts

**Implementation:**
- **File:** `src/chir/lower.rs`
- **Changes:**
  1. Validate overload rules
  2. Check static/instance conflicts

---

### 7.5 Lexical Structure
**Impact:** 349 tests
**Effort:** Low-Medium
**Priority:** P2

**Problem:**
- Semicolon/newline rules
- Comment validation
- Whitespace handling

**Implementation:**
- **File:** `src/lexer/mod.rs`
- **Changes:**
  1. Strengthen lexical rules
  2. Validate statement separators

---

## Implementation Strategy

### Development Workflow

For each fix:

1. **Understand the Issue**
   - Read 3-5 sample test files
   - Understand the validation rule from spec
   - Identify common patterns

2. **Implement Validation**
   - Locate appropriate code location
   - Add validation check
   - Return appropriate error message

3. **Test**
   ```bash
   # Single test
   cargo run -- path/to/test.cj

   # Verify it now fails compilation
   # Check error message is clear
   ```

4. **Verify Category**
   ```bash
   # Run all tests in category
   ./scripts/system_test.sh tests/category

   # Check negative tests now fail
   # Ensure positive tests still pass
   ```

5. **Commit**
   ```bash
   git add -A
   git commit -m "fix(validation): add [validation type] checking

   - Reject [invalid pattern]
   - Fixes [N] conformance tests
   - Test: [sample test file]"
   ```

### Testing Strategy

- **Unit Tests:** Add for each validation rule
- **Integration Tests:** Use conformance test suite
- **Regression Tests:** Ensure existing passing tests still pass

### Progress Tracking

Create tracking file: `docs/plan/conformance/progress.md`

```markdown
## Phase 1 Progress

- [x] 1.1 Integer Literal Range (26/26 tests fixed)
- [ ] 1.2 Extension Re-implementation (0/57 tests fixed)
- [ ] 1.3 Raw Identifier Validation (0/50 tests fixed)

Total: 26/133 tests fixed (19.5%)
```

## Success Metrics

### Phase 1 (Week 1-2)
- Target: 133 tests fixed
- Success: All quick wins implemented

### Phase 2 (Week 3-4)
- Target: 607 tests fixed (cumulative)
- Success: Type system validation complete

### Phase 3 (Week 5-8)
- Target: 2,313 tests fixed (cumulative)
- Success: Class/interface validation complete

### Phase 4 (Week 9-10)
- Target: 2,813 tests fixed (cumulative)
- Success: Expression validation complete

### Phase 5 (Week 11-12)
- Target: 3,263 tests fixed (cumulative)
- Success: Scope/naming validation complete

### Phase 6 (Week 13-14)
- Target: 3,711 tests fixed (cumulative)
- Success: Package/module validation complete

### Phase 7 (Week 15-16)
- Target: 7,071 tests fixed (100%)
- Success: All conformance gaps closed

## Risk Mitigation

### Risk: Breaking Existing Tests
- **Mitigation:** Run full test suite after each change
- **Fallback:** Revert if regression > 10 tests

### Risk: Validation Too Strict
- **Mitigation:** Compare with official CJC behavior
- **Fallback:** Adjust validation rules based on spec

### Risk: Performance Impact
- **Mitigation:** Profile validation overhead
- **Fallback:** Optimize hot paths if needed

## Resources

- **Conformance Test Suite:** `third_party/cangjie_test/`
- **Analysis Documents:** `docs/plan/conformance/`
- **Official Compiler:** `third_party/cangjie_compiler/` (reference)
- **Language Spec:** https://cangjie-lang.cn/

## Timeline

| Phase | Duration | Tests Fixed | Completion |
|-------|----------|-------------|------------|
| Phase 1 | Week 1-2 | 133 | 1.9% |
| Phase 2 | Week 3-4 | 474 | 8.6% |
| Phase 3 | Week 5-8 | 1,706 | 33.0% |
| Phase 4 | Week 9-10 | 500 | 40.1% |
| Phase 5 | Week 11-12 | 450 | 46.4% |
| Phase 6 | Week 13-14 | 448 | 52.8% |
| Phase 7 | Week 15-16 | 3,360 | 100.0% |

**Total Duration:** 16 weeks
**Total Tests:** 7,071

## Next Steps

1. Review this plan with team
2. Set up progress tracking
3. Begin Phase 1.1: Integer Literal Range Validation
4. Update progress.md after each fix
5. Weekly review of progress and adjustments

---

**Document Version:** 1.0
**Last Updated:** 2026-03-20
**Owner:** CJWasm2 Team
