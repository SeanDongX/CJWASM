# CJC 宏系统研究总结

## 研究日期
2026-02-27

## 1. 宏系统概述

CJC 的宏系统分为两部分：
1. **宏定义** (`macro` 关键字) - 定义可重用的代码模板
2. **宏调用** (`@` 符号) - 调用宏或注解

## 2. 宏调用语法

### 基本语法
```cangjie
@MacroName
@MacroName(arg1, arg2)
@MacroName[attr1, attr2](arg1, arg2)
```

### 内建注解
根据 `Parser.h` 中的定义：
```cpp
const std::unordered_map<std::string, AST::AnnotationKind> NAME_TO_ANNO_KIND = {
    {"CallingConv", AST::AnnotationKind::CALLING_CONV},
    {"C", AST::AnnotationKind::C},
    {"Attribute", AST::AnnotationKind::ATTRIBUTE},
    {"Intrinsic", AST::AnnotationKind::INTRINSIC},
    {"OverflowThrowing", AST::AnnotationKind::NUMERIC_OVERFLOW},
    {"OverflowWrapping", AST::AnnotationKind::NUMERIC_OVERFLOW},
    {"OverflowSaturating", AST::AnnotationKind::NUMERIC_OVERFLOW},
    {"When", AST::AnnotationKind::WHEN},
    {"FastNative", AST::AnnotationKind::FASTNATIVE},
    {"Annotation", AST::AnnotationKind::ANNOTATION},
    {"ConstSafe", AST::AnnotationKind::CONSTSAFE},
    {"Deprecated", AST::AnnotationKind::DEPRECATED},
    {"Frozen", AST::AnnotationKind::FROZEN},
    {"EnsurePreparedToMock", AST::AnnotationKind::ENSURE_PREPARED_TO_MOCK}
};
```

### 内建宏
```cpp
const std::vector<std::string> buildInMacros = {
    "sourcePackage",  // 返回当前包名
    "sourceFile",     // 返回当前文件名
    "sourceLine"      // 返回当前行号
};
```

## 3. MacroInvocation AST 结构

```cpp
struct MacroInvocation {
    // 基本信息
    std::string fullName;                 // 完整名称: p1.Moo
    std::string identifier;               // 宏名称: Moo
    Position identifierPos;               // 标识符位置
    Position atPos;                       // @ 符号位置

    // 括号位置
    Position leftSquarePos;               // '[' 位置 (可选)
    Position rightSquarePos;              // ']' 位置 (可选)
    Position leftParenPos;                // '(' 位置 (可选)
    Position rightParenPos;               // ')' 位置 (可选)

    // 参数
    std::vector<Token> attrs;             // 属性 [attr1, attr2]
    std::vector<Token> args;              // 参数 (arg1, arg2)
    std::vector<OwnedPtr<Node>> nodes;    // 展开后的节点

    // 元数据
    bool hasParenthesis{false};           // 是否有括号
    bool hasAttr{false};                  // 是否有属性
    bool isCustom{false};                 // 是否是自定义注解
    bool isCompileTimeVisible{};          // @! 注解

    // 关联
    OwnedPtr<Decl> decl;                  // 宏定义
    Ptr<Node> parent{nullptr};            // 父节点
    Ptr<Decl> target{nullptr};            // 目标节点
};
```

## 4. 解析流程

### 4.1 宏调用解析 (ParseAnnotations.cpp)

1. **检测 `@` 符号**
2. **解析宏名称** (标识符)
3. **解析属性** (可选): `[attr1, attr2]`
4. **解析参数** (可选): `(arg1, arg2)`

### 4.2 关键函数

```cpp
// ParseAnnotations.cpp
void ParseAttributeAnnotation(Annotation& anno);  // 解析 [attr]
void ParseOverflowAnnotation(Annotation& anno);   // 解析溢出策略
void ParseWhenAnnotation(Annotation& anno);       // 解析 @When 条件
OwnedPtr<FuncArg> ParseAnnotationArgument();      // 解析参数
void ParseAnnotationArguments(Annotation& anno);  // 解析参数列表
```

## 5. 测试用例分析

### 示例 1: 简单宏调用 (func.cj)
```cangjie
@M
func test(): Unit {
    return
}
```

### 示例 2: 带参数的宏调用 (func_arg.cj)
```cangjie
var a = asd(@B1(6))
var b = asd(@B2(6))
```

### 示例 3: 内建宏 (@Expect)
```cangjie
@Expect(arrayqueue3.head(), Option<Int64>.None)
```

## 6. CJWasm 实现策略

### 6.1 简化方案

CJWasm 不需要完整的宏系统（宏定义、展开、编译期计算），只需要支持：

1. **内建注解** - 编译器指令
   - `@Assert(a, b)` - 断言
   - `@Expect(a, b)` - 期望值
   - `@Deprecated` - 废弃警告
   - `@CallingConv` - 调用约定

2. **编译期宏** - 元信息
   - `@sourceFile` - 文件名
   - `@sourceLine` - 行号
   - `@sourcePackage` - 包名

### 6.2 不需要实现的部分

- ❌ 自定义宏定义 (`macro` 关键字)
- ❌ 宏展开和求值
- ❌ `Tokens` 类型
- ❌ 宏参数列表
- ❌ 宏体解析

### 6.3 AST 设计

```rust
// src/ast/mod.rs

/// 宏调用
#[derive(Debug, Clone, PartialEq)]
pub struct MacroCall {
    pub name: String,           // 宏名称
    pub args: Vec<Expr>,        // 参数列表
    pub span: Span,             // 源码位置
}

// 添加到 Expr 枚举
pub enum Expr {
    // ... 现有变体
    Macro(Box<MacroCall>),
}
```

### 6.4 Parser 设计

```rust
// src/parser/macro.rs

impl Parser {
    /// 解析宏调用: @MacroName(args)
    pub(crate) fn parse_macro_call(&mut self) -> Result<Expr, ParseError> {
        let start = self.current_pos();

        // 1. 期望 @ 符号
        self.expect(Token::At)?;

        // 2. 解析宏名称
        let name = self.parse_identifier()?;

        // 3. 解析参数列表 (可选)
        let args = if self.check(Token::LeftParen) {
            self.expect(Token::LeftParen)?;
            let args = self.parse_comma_separated(|p| p.parse_expr())?;
            self.expect(Token::RightParen)?;
            args
        } else {
            Vec::new()
        };

        Ok(Expr::Macro(Box::new(MacroCall {
            name,
            args,
            span: self.span_from(start),
        })))
    }
}
```

### 6.5 CodeGen 设计

```rust
// src/codegen/mod.rs

impl CodeGen {
    fn emit_macro_call(&mut self, macro_call: &MacroCall) -> Result<(), CodeGenError> {
        match macro_call.name.as_str() {
            "Assert" => self.emit_assert_macro(macro_call),
            "Expect" => self.emit_expect_macro(macro_call),
            "Deprecated" => {
                // 编译时警告
                eprintln!("Warning: Using deprecated feature");
                Ok(())
            }
            "sourceFile" => {
                // 返回文件名字符串
                self.emit_string_literal(&self.current_file)
            }
            "sourceLine" => {
                // 返回行号
                self.func.instruction(&Instruction::I64Const(macro_call.span.line as i64));
                Ok(())
            }
            _ => Err(CodeGenError::UnknownMacro(macro_call.name.clone())),
        }
    }

    fn emit_assert_macro(&mut self, macro_call: &MacroCall) -> Result<(), CodeGenError> {
        if macro_call.args.len() != 2 {
            return Err(CodeGenError::InvalidMacroArgs);
        }

        // 生成: if (arg1 != arg2) { panic }
        self.emit_expr(&macro_call.args[0])?;
        self.emit_expr(&macro_call.args[1])?;

        // 比较
        self.func.instruction(&Instruction::I64Ne);

        // if 不相等则 panic
        self.func.instruction(&Instruction::If(BlockType::Empty));
        self.emit_panic("Assertion failed")?;
        self.func.instruction(&Instruction::End);

        Ok(())
    }
}
```

## 7. 实现优先级

### P0 (已完成 ✅)
- ✅ `@Assert(a, b)` - 测试断言
- ✅ `@Expect(a, b)` - 期望值检查

### P1 (已完成 ✅)
- ✅ `@Deprecated` - 废弃警告
- ✅ `@sourceFile` - 文件名
- ✅ `@sourceLine` - 行号
- ✅ `@sourcePackage` - 包名

### P2 (可延后)
- ❌ `@CallingConv` - 调用约定
- ❌ `@When[condition]` - 条件编译
- ❌ 自定义宏定义

## 8. 测试计划

### 单元测试
```rust
#[test]
fn test_parse_macro_call() {
    let source = "@Assert(a, b)";
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().unwrap();

    match expr {
        Expr::Macro(m) => {
            assert_eq!(m.name, "Assert");
            assert_eq!(m.args.len(), 2);
        }
        _ => panic!("Expected macro call"),
    }
}

#[test]
fn test_parse_macro_no_args() {
    let source = "@Deprecated";
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().unwrap();

    match expr {
        Expr::Macro(m) => {
            assert_eq!(m.name, "Deprecated");
            assert_eq!(m.args.len(), 0);
        }
        _ => panic!("Expected macro call"),
    }
}
```

### 集成测试
```cangjie
// tests/fixtures/macro_test.cj
main(): Int64 {
    let a = 42
    let b = 42
    @Assert(a, b)

    let c = 10
    let d = 20
    @Expect(c + d, 30)

    return 0
}
```

## 9. 下一步行动

1. ✅ 完成研究
2. ✅ 设计 AST 结构
3. ✅ 实现 Parser
4. ✅ 编写测试
5. ✅ 实现 CodeGen
6. ✅ 集成测试通过

## 10. 关键发现

1. **CJC 的宏系统很复杂** - 包含宏定义、展开、求值
2. **CJWasm 只需要简化版本** - 内建注解 + 编译期宏
3. **语法很简单** - `@Name` 或 `@Name(args)`
4. **实现成本低** - 约 200-300 行代码
5. **测试用例丰富** - 可以复用 CJC 的测试

## 11. 参考文件

- `third_party/cangjie_compiler/src/Parse/ParseMacro.cpp` - 宏解析
- `third_party/cangjie_compiler/src/Parse/ParseAnnotations.cpp` - 注解解析
- `third_party/cangjie_compiler/include/cangjie/AST/Node.h` - AST 定义
- `third_party/cangjie_compiler/unittests/Macro/srcFiles/` - 测试用例
