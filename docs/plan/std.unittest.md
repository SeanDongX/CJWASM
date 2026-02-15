# std.unittest 实现方案

> 文档版本: 2026-02-14

## 1. 背景与目标

仓颉标准库提供了 `std.unittest` 包，通过 `@Assert` / `@Expect` 宏实现测试断言。
cjwasm 需要实现兼容的断言机制，用于系统测试的 `.cj` 示例文件中。

## 2. cjc 官方实现分析

**源码位置**: https://gitcode.com/Cangjie/cangjie_runtime.git `release/1.0` 分支  
**路径**: `std/libs/std/unittest/`

### 2.1 架构概览

```
std/libs/std/unittest/           (164 个 .cj 文件)
├── testmacro/
│   ├── assertion_macro.cj       ← @Assert/@Expect 宏定义
│   ├── power_assert.cj          ← @PowerAssert 宏（带表达式树可视化）
│   ├── test_macro.cj            ← @Test 宏
│   ├── lifecycle_macros.cj      ← @BeforeAll/@AfterAll 等
│   └── ...                      ← 30+ 宏文件
├── assert.cj                    ← assertEqual/expectEqual 运行时函数
├── entry_main.cj                ← 测试入口
├── test_case_executor.cj        ← 用例执行器
├── suite_executor.cj            ← 套件执行器
├── parallel.cj                  ← 并行测试
├── fuzz.cj                      ← 模糊测试
├── statistics.cj                ← 统计报告
└── ...
```

### 2.2 @Assert/@Expect 宏展开机制

`@Assert(a, b)` 是一个**编译时宏**（`assertion_macro.cj`），展开流程：

```
@Assert(a, b)
    ↓ 宏展开 (std.ast.*)
assertEqual("a", "b", a, b)
    ↓ 运行时 (assert.cj)
if (expected != actual) {
    let cr = buildCheckResult(leftStr, rightStr, expected, actual, Assert)
    Framework.withCurrentContext { ctx => ctx.checkFailed(cr) }
    throw AssertException("", cr)
}
```

`@Expect(a, b)` 展开为 `expectEqual(...)` — 相同逻辑但**不抛异常**，仅记录失败。

### 2.3 核心依赖链

```
@Assert 宏
  └─ import std.ast.*                    ← 编译时 AST 操作框架
  └─ import std.collection.*             ← Array, HashMap 等
  └─ assertEqual<T>()                    ← 泛型函数
       └─ where T <: Equatable<T>        ← trait 约束 + 动态分派
       └─ Framework.withCurrentContext    ← 全局测试框架上下文
       └─ AssertionCtx                   ← 断言上下文（错误收集树）
       └─ buildCheckResult               ← 构建对比结果
            └─ AssertPrintable<T>        ← diff 美化输出
            └─ PrettyText                ← 富文本格式化
       └─ throw AssertException          ← 异常抛出
```

### 2.4 复用可行性评估

| 维度 | cjc `std.unittest` | cjwasm 现状 | 可复用 |
|------|-------------------|-------------|--------|
| 宏系统 (`std.ast.*`) | 编译时 Token/AST 变换 | 无宏系统 | ❌ |
| 泛型 trait 分派 | `where T <: Equatable<T>` + 动态 dispatch | 泛型仅单态化，无 trait dispatch | ❌ |
| 异常机制 | `throw AssertException` + 框架捕获 | try-catch 有限支持 | ⚠️ 部分 |
| Framework 上下文 | 全局单例，线程安全，嵌套上下文 | 不存在 | ❌ |
| diff 美化输出 | `AssertPrintable`, `PrettyText` | 不存在 | ❌ |
| 并行/fuzz/bench | 线程池，种子生成器 | 不适用于 WASM | ❌ |
| 文件规模 | 164 个 .cj，~15000 行 | — | ❌ 不现实 |

**结论: 无法直接复用 cjc 的 `std.unittest` 实现。**

## 3. cjwasm 轻量实现方案

### 3.1 设计原则

1. **语法兼容** — 保持 `@Assert(a, b)` / `@Expect(a, b)` 的调用形式
2. **语义一致** — Assert 失败立即终止（fail-fast），Expect 失败记录但继续
3. **编译器内建** — 作为编译器直接支持的语句，非宏展开
4. **零运行时依赖** — 直接编译为 WASM 指令，无需引入额外库

### 3.2 语法定义

```cangjie
// 基本形式：比较两个表达式是否相等
@Assert(expr1, expr2)          // 失败 → 打印错误 + 立即终止
@Expect(expr1, expr2)          // 失败 → 打印错误 + 继续执行

// 单参数形式：检查布尔条件
@Assert(boolExpr)              // 失败 → 立即终止
@Expect(boolExpr)              // 失败 → 继续执行
```

### 3.3 AST 表示

```rust
pub enum Stmt {
    // ... 已有变体 ...
    
    /// @Assert(left, right) — 断言 left == right，失败则立即终止
    Assert {
        left: Expr,
        right: Expr,
        line: usize,    // 源码行号，用于错误报告
    },
    /// @Expect(left, right) — 断言 left == right，失败则记录但继续
    Expect {
        left: Expr,
        right: Expr,
        line: usize,
    },
}
```

### 3.4 Parser 变更

在 `parse_stmt()` 中识别 `Token::At`，后接 `Ident("Assert")` 或 `Ident("Expect")`：

```rust
Some(Token::At) => {
    self.advance(); // consume @
    match self.peek() {
        Some(Token::Ident(name)) if name == "Assert" => {
            self.advance();
            let line = self.current_line();
            self.expect(Token::LParen)?;
            let left = self.parse_expr()?;
            // 单参数形式：@Assert(cond) → @Assert(cond, true)
            let right = if self.check(&Token::Comma) {
                self.advance();
                self.parse_expr()?
            } else {
                Expr::Bool(true)
            };
            self.expect(Token::RParen)?;
            Ok(Stmt::Assert { left, right, line })
        }
        // @Expect 同理
    }
}
```

### 3.5 Codegen 策略

#### @Assert(a, b) 编译产物

```wasm
block $assert_ok
    ;; 计算 left 和 right
    <compile left>       ;; → i64 (或其他类型)
    <compile right>      ;; → i64
    i64.eq               ;; 比较
    br_if $assert_ok     ;; 相等则跳过错误处理
    
    ;; 失败路径：打印错误信息
    ;; "ASSERT FAILED: line N\n"
    <emit string constant>
    call $__println_str
    
    unreachable          ;; 立即终止（对应 cjc 的 throw AssertException）
end
```

#### @Expect(a, b) 编译产物

```wasm
block $expect_ok
    <compile left>
    <compile right>
    i64.eq
    br_if $expect_ok     ;; 相等则跳过
    
    ;; 失败路径：打印错误信息
    <emit string constant>
    call $__println_str
    
    ;; 递增全局失败计数器
    global.get $__expect_fail_count
    i32.const 1
    i32.add
    global.set $__expect_fail_count
end
```

程序退出前可检查 `$__expect_fail_count > 0` 来决定退出码。

### 3.6 类型支持

根据表达式推断类型，选择对应的比较指令：

| 类型 | 比较指令 | 值打印 |
|------|---------|--------|
| `Int64` / 整数类型 | `i64.eq` | `__println_i64` |
| `Float64` | `f64.eq` | `__println_str` (via toString) |
| `Bool` | `i64.eq` (0/1) | `__println_bool` |
| `String` | `call __str_eq` | `__println_str` |

### 3.7 错误输出格式

```
ASSERT FAILED: line 42
EXPECT FAILED: line 55
```

未来可扩展为包含实际值和预期值：

```
ASSERT FAILED: line 42, expected 100, got 99
```

## 4. 系统测试改造

### 4.1 改造策略

在现有 `.cj` 示例文件中**追加** `@Assert` 断言，同时**保留原有 `return` 返回值**，
确保两种验证机制并存：

```cangjie
main(): Int64 {
    let result = compute()
    @Assert(result, 42)         // 新增：编译器级断言
    return result               // 保留：system_test.sh 返回值验证
}
```

### 4.2 收益

- **双重保障** — 返回值检查 + 断言检查同时生效
- **更细粒度** — 每个中间结果都可以单独验证，而非只检查最终返回值
- **向前兼容** — 与仓颉标准测试框架 API 保持一致

## 5. 与 cjc `std.unittest` 的对比

| 特性 | cjc 官方 | cjwasm 实现 |
|------|---------|-------------|
| `@Assert(a, b)` | ✅ 宏展开 → assertEqual | ✅ 编译器内建语句 |
| `@Expect(a, b)` | ✅ 宏展开 → expectEqual | ✅ 编译器内建语句 |
| `@Assert(bool)` | ✅ 单参数形式 | ✅ 转为与 `true` 比较 |
| `@PowerAssert` | ✅ 表达式树可视化 | ❌ 不实现 |
| `@Test` | ✅ 测试注册 | ❌ 不需要（单文件执行） |
| 值 diff 输出 | ✅ AssertPrintable | ⚠️ 仅行号 |
| 并行测试 | ✅ 线程池 | ❌ WASM 单线程 |
| 自定义断言 | ✅ @CustomAssertion | ❌ 不实现 |
| Delta 比较 | ✅ 浮点容差 | ❌ 不实现 |

## 6. 实现步骤

1. **AST** — 添加 `Stmt::Assert` 和 `Stmt::Expect` 变体 ✅ 已完成
2. **Parser** — 在 `parse_stmt()` 中解析 `@Assert(...)` / `@Expect(...)` 语法 ✅ 已完成
3. **Codegen** — 在 `compile_stmt()` 中生成比较 + 错误输出 + 终止/计数逻辑 ✅ 已完成
4. **测试改造** — 在所有 system test `.cj` 文件中添加 `@Assert` 断言 ✅ 已完成
5. **验证** — 运行 `system_test.sh` 确认全部通过 ✅ 已完成

## 7. 实现细节记录

### 7.1 类型协调

`compile_assert_expect` 推断左右表达式的 WASM 类型后，自动协调类型不匹配：
- `i32` vs `i64` → 将 i32 扩展为 i64 后用 `i64.eq`
- `f32` vs `f64` → 将 f32 提升为 f64 后用 `f64.eq`
- 同类型 → 直接使用对应的 eq 指令

典型场景: `@Assert(s.contains("world"), 1)` — contains 返回 Bool(i32)，1 是 Integer(i64)

### 7.2 失败路径实现

- **@Assert**: 打印 `ASSERT FAILED: offset N` 到 stderr → 调用 `proc_exit(1)` → `unreachable`
- **@Expect**: 打印 `EXPECT FAILED: offset N` 到 stderr → 继续执行

错误消息在编译时嵌入为 WASM 内存中的常量字符串，通过逐字节 `i32.store8` 写入。

### 7.3 已知限制

- **字符串比较不支持**: `@Assert(str1, str2)` 比较的是指针地址，不是内容。需要实现 `__str_eq` 运行时函数。
- **错误信息使用字节偏移**: 显示的是源码字节偏移而非行号，后续可通过嵌入源码映射改进。
- **@Expect 无全局计数器**: 当前 @Expect 失败仅打印，不影响退出码。后续可添加全局变量追踪。

### 7.4 测试覆盖

28 个系统测试文件中已添加 `@Assert` 断言，覆盖：
- 整数运算结果验证
- 函数返回值验证
- 总和一致性检查
- 布尔条件检查（如 `contains` 返回值）
