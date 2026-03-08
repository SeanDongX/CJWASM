# CJWasm std LLT 测试兼容性修复计划

> 基准：`third_party/cangjie_test/testsuites/LLT/API/std`（1,441 个测试文件）
>
> ~~初始编译兼容率：62% (896/1441)~~
>
> ~~修复后编译兼容率：65% (942/1441)~~
>
> **修复后编译兼容率：66% (955/1441)**
>
> L1 模块（pipeline.rs L1_STD_TOP）：~~177/190 通过 (93%)~~ → ~~185/206 通过~~ → **188/206 通过 (91.3%)**
>
> 运行脚本：`./scripts/std_test.sh`

---

## 一、L1 模块修复前后对比

| 模块 | 修复前 | 第一轮修复 | 第二轮修复 | 变化 | 剩余失败原因 |
|------|--------|-----------|-----------|------|-------------|
| io | 88/90 | **90/90** | **90/90** | ✅ +2 | — |
| overflow | 5/12 | 11/12 | **12/12** | ✅ +7 | — |
| deriving | 16/32 | 16/32 | 16/32 | — | `@Test`/`@TestCase`/`quote()` 宏系统 (F6) |
| argOpt | 0/1 | 0/1 | 0/1 | — | `@UnittestOption` 宏注解 (F6) |
| sort | 7/8 | 7/8 | **8/8** | ✅ +1 | — |
| binary | 0/1 | 0/1 | **1/1** | ✅ +1 | — |
| console | 55/55 | **55/55** | **55/55** | ✅ | — |
| unicode | 6/6 | **6/6** | **6/6** | ✅ | — |

---

## 二、根因分类与修复方案

### F1: UInt64 大数字面量溢出（8 个文件）

**影响**: overflow/\*（7 个）、io/StringWriter_UTF8_Int64.cj（1 个）

**现象**: 词法错误 `未知字符: '18446744073709551614'`

**根因**: `src/lexer/mod.rs` 整数解析使用 `i64::from_str_radix`，超出 `i64::MAX` 的 UInt64 字面量（如 `0x8000_0000_0000_0000`、`18446744073709551614`）直接报错。

**修复方案**:

```
位置: src/lexer/mod.rs — 整数字面量解析逻辑
策略: i64 解析失败时回退 u64 解析，存为 Literal::UInt64(u64)

1. 在 Token 枚举中添加 UInt64 字面量变体（或复用 Int64 以 bit pattern 存储）
2. lexer 中 parse_integer：
   - 先尝试 i64::from_str_radix
   - 失败则尝试 u64::from_str_radix
   - 仍失败才报词法错误
3. AST Literal 枚举添加 UInt64(u64) 变体
4. codegen 中 Literal::UInt64 → i64.const (bit reinterpret)
```

**复杂度**: 中 | **预计影响**: +8 个文件通过

---

### F2: `const init()` 构造函数（8 个文件）

**影响**: deriving/annotated_test.cj、deriving/equatable_test.cj、deriving/primary.cj 等 8 个

**现象**: 语法错误 `意外的 token: Const, 期望: var、let、init、~init 或 func`

**根因**: `src/parser/decl.rs` 的类体解析在遇到 `const` 时不识别 `const init()` 语法。cjc 支持 `const` 修饰的构造函数（编译期常量初始化）。

**修复方案**:

```
位置: src/parser/decl.rs — parse_class_body / parse_struct_body
策略: 识别 const init() 并解析为带 is_const 标记的 InitDef

1. 在类体解析的 token 匹配中，增加 Token::Const 分支：
   - 若后续 token 为 init → 解析为 const init()
   - 否则按 const 字段声明处理
2. AST InitDef 添加 is_const: bool 字段
3. codegen 中忽略 is_const 标记（WASM 不区分 const/非 const init）
```

**复杂度**: 低 | **预计影响**: +8 个文件通过

---

### F3: `###"..."###` 多定界符原始字符串（1 个文件）

**影响**: io/StringReader/iterator.cj

**现象**: 语法错误 `意外的 token: Hash`

**根因**: lexer 支持 `r"..."` 原始字符串和 `"""..."""` 多行字符串，但不支持 `###"..."###`（带 `#` 定界符的多行原始字符串）。这是 cjc 的原始字符串语法：N 个 `#` + `"` 开头，`"` + N 个 `#` 结尾。

**修复方案**:

```
位置: src/lexer/mod.rs — 字符串字面量解析
策略: 识别 # 开头的定界符字符串

1. 在主 lex 循环中检测连续 '#' 序列：
   - 统计 '#' 数量 N
   - 期望紧跟 '"'
   - 读取到 '"' + N 个 '#' 时结束
2. 生成 Token::String 字面量（内容不做转义处理）
```

**复杂度**: 中 | **预计影响**: +1 个文件通过

---

### F4: `r'a'` 原始字符字面量（1 个文件）

**影响**: argOpt/test_argopt.cj

**现象**: 词法错误 `未知字符: '''`

**根因**: lexer 中 `r` 前缀仅处理字符串 `r"..."`，不处理字符（Rune）`r'...'`。cjc 支持 `r'...'` 作为不转义的 Rune 字面量。

**修复方案**:

```
位置: src/lexer/mod.rs — r 前缀处理逻辑
策略: r 后跟 ' 时解析为原始 Rune 字面量

1. 在 r 前缀分支中增加 '\'' 检测：
   - r"..." → 原始字符串（已有）
   - r'...' → 原始字符（新增）
2. 读取 ' 和 ' 之间的字符/转义序列
3. 生成 Token::Rune 字面量
```

**复杂度**: 低 | **预计影响**: +1 个文件通过

---

### F5: `Range<T>` 泛型 Range 类型参数（1 个文件）

**影响**: sort/test_sort_list.cj

**现象**: 语法错误 `意外的 token: Minus` — 解析 `func remove(range: Range<Int64>)` 时，`Range<Int64>` 被误解

**根因**: CJWasm 的 `Range` 是非泛型固定类型（`ast::Type::Range`），不接受类型参数。cjc 的 Range 是泛型 `Range<T>`。

**修复方案**:

```
位置: src/ast/type_.rs, src/parser/type_.rs
策略: Range 泛型化为 Range(Box<Type>)

1. AST: Type::Range → Type::Range(Box<Type>)（默认 Int64）
2. parser/type_.rs: 解析 Range<T> 类型语法
3. codegen: Range 内存布局不变（start/end/inclusive/step 仍为 i64），
   仅在类型签名中携带类型参数
4. 现有 Range 字面量 a..b 推断为 Range(Int64)
```

**复杂度**: 高 | **预计影响**: +1 个文件通过

---

### F6: `@Annotation` / `quote(...)` 宏系统（8 个文件）

**影响**: deriving/api-2.cj、deriving/api.cj、deriving/diagnostics\*.cj 等

**现象**:
- `意外的 token: At, 期望: @Assert 或 @Expect` — 遇到 `@Annotation`、`@Expect(ty.toTokens().toString(), ...)` 后的 `@TestCase`
- `quote(...)` 宏表达式无法解析

**根因**: CJWasm 不支持 cjc 的宏系统（`macro func`、`quote(...)`、`@Annotation` 自定义注解）。这些是 cjc 元编程特性，依赖完整的 AST 引用和宏展开引擎。

**修复方案**:

```
策略: 不修复（架构限制）

原因:
- @Annotation 自定义注解需要宏展开引擎
- quote(...) 需要 AST 引用系统
- 这些属于 cjc 元编程核心特性，WASM 编译器不需要
- deriving 模块本身依赖宏系统（@Derive），CJWasm 已在 spec.md 中标注为不支持

替代: 在 std_test.sh 中可标记这些为 "已知不支持" 跳过
```

**复杂度**: — | **预计影响**: 0（不修复）

---

### F7: binary 模块依赖（1 个文件）

**影响**: binary/binary_test.cj

**现象**: 编译失败（模块依赖）

**根因**: binary 模块的测试文件 `import std.binary.*` 需要加载 vendor std 中的 binary 包，但编译时可能缺少依赖解析上下文。

**修复方案**:

```
策略: 待诊断 — 需确认是 import 解析问题还是 binary 包本身的语法问题

1. 检查 binary_test.cj 的具体错误
2. 若为 import 路径问题 → 修复 resolve_import_to_files
3. 若为 binary 包源码解析问题 → 视具体语法决定
```

**复杂度**: 待定 | **预计影响**: +1 个文件通过

---

## 三、修复状态

| 编号 | 修复项 | 状态 | 改动文件 |
|------|--------|------|---------|
| F1 | UInt64 大数字面量 (u64 回退) | ✅ 已完成 | `src/lexer/mod.rs` |
| F2 | `const init()` 构造函数 | ✅ 已完成 | `src/parser/decl.rs` |
| F3 | `###"..."###` 定界符原始字符串 | ✅ 已完成 | `src/lexer/mod.rs`, `src/parser/expr.rs`, `src/parser/pattern.rs` |
| F4 | `r'a'` + `\u{XXXX}` 字符转义 | ✅ 已完成 | `src/lexer/mod.rs` |
| F6 | 宏系统 (`@Test`/`quote()`) | ❌ 不修复 | 架构限制 |
| F7 | binary 模块 (`f32`/`f64` 后缀字面量) | ✅ 已完成 | `src/lexer/mod.rs`, `src/parser/expr.rs` |
| 额外 | `{ => body }` 无参 lambda 简写 | ✅ 已完成 | `src/parser/expr.rs` |
| 额外 | 跨行 postfix 表达式边界 | ✅ 已完成 | `src/parser/mod.rs`, `src/parser/expr.rs`, `src/pipeline.rs` |
| 额外 | 范围负步长 `..=0 : -1` | ✅ 已完成 | `src/parser/expr.rs` |
| 额外 | 多字符单引号字符串 `'...'` | ✅ 已完成 | `src/lexer/mod.rs`, `src/parser/expr.rs`, `src/parser/pattern.rs` |
| 额外 | `parse_for_iterable` 通用化 | ✅ 已完成 | `src/parser/expr.rs` |

---

## 四、修复效果

| 指标 | 修复前 | 第一轮 | 第二轮 | 变化 |
|------|--------|--------|--------|------|
| 总编译兼容率 | 62% (896/1441) | 65% (942/1441) | **66% (955/1441)** | +59 文件 |
| L1 通过率 | 93% (177/190) | 89.8% (185/206) | **91.3% (188/206)** | +11 文件 |
| L1 通过率 (排除 F6) | — | 97.1% (169/174) | **100% (171/171)** | — |
| io | 88/90 | 90/90 | **90/90 (100%)** | +2 |
| overflow | 5/12 | 11/12 | **12/12 (100%)** | +7 |
| sort | 7/8 | 7/8 | **8/8 (100%)** | +1 |
| binary | 0/1 | 0/1 | **1/1 (100%)** | +1 |
| console | 55/55 | 55/55 | **55/55 (100%)** | — |
| unicode | 6/6 | 6/6 | **6/6 (100%)** | — |

L1 可修复模块（排除 F6 宏系统依赖的 deriving 16 个 + argOpt 1 个文件）通过率 **100%**。

总兼容率提升 +4%，额外文件来自 L2 模块受益（f32/f64 后缀、表达式边界、范围步长、单引号字符串等修复同时覆盖了非 L1 测试文件）。

---

## 五、验证方法

```bash
# 全量测试
./scripts/std_test.sh

# 仅测试 L1 模块
./scripts/std_test.sh io binary console overflow deriving argOpt sort unicode

# 带 WASM 验证
./scripts/std_test.sh --validate io overflow

# 查看失败详情
./scripts/std_test.sh --verbose overflow
```

---

*创建日期: 2026-03-07*
*第一轮修复: 2026-03-07 (F1-F4 + lambda 简写)*
*第二轮修复: 2026-03-07 (F7 + 表达式边界 + 范围步长 + 单引号字符串)*
*关联: scripts/std_test.sh, src/pipeline.rs L1_STD_TOP*
