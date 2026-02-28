# CJC 到 CJWasm 迁移快速参考

## 🎯 核心策略

**不要直接拷贝代码，而是参考实现逻辑**

CJC (C++) 和 CJWasm (Rust) 的架构差异太大，直接翻译不可行。正确的方法是：
1. 阅读 CJC 的实现逻辑
2. 理解语法规范
3. 用 Rust 的方式重新实现

## 📊 当前状态

```
CJC:     22,893 行 (Parse + AST)
CJWasm:  ~12,500 行 (parser + ast)
完成度:   ~55% (核心功能 + 高级特性)
```

**已实现核心功能** ✅:
- 基础类型、函数、类、结构体、接口、枚举
- 控制流、模式匹配、泛型、错误处理
- Lambda、字符串插值

**已实现高级功能** ✅:
- 宏系统 (P1) - @Assert, @Expect
- 类型别名 (P1) - type Name = Type
- 完整模式匹配 (P1) - if-let, while-let
- 可选链 (P2) - obj?.field
- 尾随闭包 (P2) - f(args) { params => body }
- do-while 循环 (P2)
- spawn/synchronized (P3) - 单线程桩实现

**仍需实现** ❌:
- Quote 宏 (P3)
- 指针操作 (P3)
- MapLiteral 完整实现 (codegen 待实现)

## 🔧 实用工具

### 1. 分析工具
```bash
# 查看功能缺口
./scripts/analyze_gaps.sh

# 提取 CJC AST 节点
./scripts/extract_cjc_ast.sh

# 提取 CJC Parser 方法
./scripts/extract_cjc_parser_methods.sh
```

### 2. 查找 CJC 实现
```bash
# 查找某个语法的解析实现
grep -r "ParseMacro" third_party/cangjie_compiler/src/Parse/

# 查找 AST 节点定义
grep -A 20 "class MacroInvocation" third_party/cangjie_compiler/include/cangjie/AST/Node.h

# 查找测试用例
find third_party/cangjie_compiler/unittests -name "*Macro*"
```

### 3. 对比实现
```bash
# CJC 的表达式解析
less third_party/cangjie_compiler/src/Parse/ParseExpr.cpp

# CJWasm 的表达式解析
less src/parser/expr.rs
```

## 📝 典型迁移流程

### 示例：添加宏系统支持

#### 步骤 1: 研究 CJC 实现 (30-60分钟)

```bash
# 1. 查看 Parser 接口
cat third_party/cangjie_compiler/include/cangjie/Parse/Parser.h | grep -A 5 Macro

# 2. 查看实现
less third_party/cangjie_compiler/src/Parse/ParseMacro.cpp

# 3. 查看 AST 节点
grep -A 30 "class MacroInvocation" third_party/cangjie_compiler/include/cangjie/AST/Node.h

# 4. 查看测试
ls third_party/cangjie_compiler/unittests/Macro/srcFiles/
```

**关键发现**:
- 宏调用以 `@` 开头
- 格式: `@MacroName(arg1, arg2)`
- AST 节点: `MacroInvocation { name, args }`
- 内建宏: `@Assert`, `@Deprecated`, `@CallingConv` 等

#### 步骤 2: 设计 Rust 实现 (30分钟)

```rust
// src/ast/mod.rs - 添加 AST 节点
#[derive(Debug, Clone, PartialEq)]
pub struct MacroCall {
    pub name: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

// 添加到 Expr 枚举
pub enum Expr {
    // ... 现有变体
    Macro(Box<MacroCall>),
}
```

```rust
// src/parser/macro.rs - 新建文件
use crate::ast::{Expr, MacroCall};
use crate::lexer::Token;
use crate::parser::{Parser, ParseError};

impl Parser {
    /// 解析宏调用: @MacroName(args)
    pub(crate) fn parse_macro_call(&mut self) -> Result<Expr, ParseError> {
        let start = self.current_pos();

        // 1. 期望 @ 符号
        self.expect(Token::At)?;

        // 2. 解析宏名称
        let name = self.parse_identifier()?;

        // 3. 解析参数列表
        self.expect(Token::LeftParen)?;
        let args = self.parse_comma_separated(|p| p.parse_expr())?;
        self.expect(Token::RightParen)?;

        Ok(Expr::Macro(Box::new(MacroCall {
            name,
            args,
            span: self.span_from(start),
        })))
    }
}
```

#### 步骤 3: 编写测试 (30分钟)

```rust
// src/parser/mod.rs 或 tests/parser_tests.rs
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
fn test_parse_deprecated_macro() {
    let source = r#"@Deprecated("Use newFunc instead")"#;
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr().unwrap();

    match expr {
        Expr::Macro(m) => {
            assert_eq!(m.name, "Deprecated");
            assert_eq!(m.args.len(), 1);
        }
        _ => panic!("Expected macro call"),
    }
}
```

#### 步骤 4: 集成到主解析器 (15分钟)

```rust
// src/parser/expr.rs
impl Parser {
    pub fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        match self.current_token() {
            Token::At => self.parse_macro_call(),
            Token::Identifier(_) => self.parse_identifier_expr(),
            Token::IntLiteral(_) => self.parse_int_literal(),
            // ... 其他分支
            _ => Err(ParseError::UnexpectedToken(self.current_token())),
        }
    }
}
```

#### 步骤 5: 代码生成 (1-2小时)

```rust
// src/codegen/mod.rs
impl CodeGen {
    fn emit_macro_call(&mut self, macro_call: &MacroCall) -> Result<(), CodeGenError> {
        match macro_call.name.as_str() {
            "Assert" => self.emit_assert_macro(macro_call),
            "Expect" => self.emit_expect_macro(macro_call),
            "Deprecated" => Ok(()), // 编译时警告，不生成代码
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

#### 步骤 6: 端到端测试 (30分钟)

```bash
# 创建测试文件
cat > tests/fixtures/macro_test.cj << 'EOF'
main(): Int64 {
    let a = 42
    let b = 42
    @Assert(a, b)
    return 0
}
EOF

# 编译并运行
cargo build
./target/debug/cjwasm tests/fixtures/macro_test.cj -o test.wasm
wasmtime run --invoke main test.wasm
echo $?  # 应该输出 0
```

## 🎓 学习路径

### 第 1 周：建立映射
- [ ] 运行所有分析脚本
- [ ] 阅读 `docs/ast_mapping.md`
- [ ] 选择 3 个 P1 功能

### 第 2-3 周：实现 P1 功能
- [ ] 宏系统
- [ ] 类型别名
- [ ] 完整的 if-let

### 第 4 周：测试和优化
- [ ] 复用 CJC 测试用例
- [ ] 性能对比
- [ ] 文档更新

## 📚 关键文件速查

### CJC 源码结构
```
third_party/cangjie_compiler/
├── include/cangjie/
│   ├── AST/
│   │   ├── Node.h          # AST 节点定义 ⭐
│   │   ├── Types.h         # 类型系统
│   │   └── Walker.h        # AST 遍历
│   └── Parse/
│       └── Parser.h        # Parser 接口 ⭐
├── src/
│   ├── AST/
│   │   ├── Create.cpp      # AST 节点创建
│   │   ├── Clone.cpp       # AST 克隆
│   │   └── PrintNode.cpp   # AST 打印
│   └── Parse/
│       ├── Parser.cpp      # Parser 主逻辑
│       ├── ParseExpr.cpp   # 表达式解析 ⭐
│       ├── ParseDecl.cpp   # 声明解析 ⭐
│       ├── ParseType.cpp   # 类型解析 ⭐
│       ├── ParsePattern.cpp # 模式解析 ⭐
│       ├── ParseMacro.cpp  # 宏解析 ⭐
│       └── ParseImports.cpp # 导入解析
└── unittests/              # 测试用例 ⭐
    ├── Parse/
    ├── Macro/
    └── Sema/
```

### CJWasm 源码结构
```
src/
├── ast/
│   ├── mod.rs              # AST 节点定义 ⭐
│   └── type_.rs            # 类型系统
├── parser/
│   ├── mod.rs              # Parser 主逻辑 ⭐
│   ├── expr.rs             # 表达式解析 ⭐
│   ├── decl.rs             # 声明解析 ⭐
│   ├── type_.rs            # 类型解析 ⭐
│   ├── pattern.rs          # 模式解析 ⭐
│   ├── stmt.rs             # 语句解析
│   └── error.rs            # 错误处理
├── codegen/
│   └── mod.rs              # WASM 代码生成
└── lexer/
    └── mod.rs              # 词法分析
```

## 🔍 常用搜索命令

```bash
# 查找 CJC 中某个语法的实现
grep -r "ParseTypeAlias" third_party/cangjie_compiler/src/

# 查找 AST 节点定义
grep -A 20 "class TypeAliasDecl" third_party/cangjie_compiler/include/cangjie/AST/Node.h

# 查找测试用例
find third_party/cangjie_compiler/unittests -name "*.cj" | xargs grep -l "type.*="

# 统计某个功能的代码量
wc -l third_party/cangjie_compiler/src/Parse/ParseMacro.cpp

# 查看 CJWasm 现有实现
grep -r "parse_" src/parser/ | wc -l
```

## ⚠️ 常见陷阱

### 1. 不要过度翻译
❌ **错误**: 逐行翻译 C++ 代码到 Rust
```rust
// 不要这样做
let node = Box::new(Node::new());  // 模仿 C++ 的 OwnedPtr
```

✅ **正确**: 用 Rust 的惯用方式
```rust
// 应该这样做
let node = Expr::Binary { ... };  // 直接用枚举
```

### 2. 不要复制所有功能
❌ **错误**: 实现 CJC 的所有功能（LSP、增量编译等）
✅ **正确**: 只实现核心编译功能

### 3. 不要忽略测试
❌ **错误**: 写完代码就认为完成了
✅ **正确**: 每个功能都要有测试用例

### 4. 不要混淆解析和语义分析
❌ **错误**: 在 Parser 里做类型检查
✅ **正确**: Parser 只做语法分析，类型检查放到后续阶段

## 🚀 快速开始

```bash
# 1. 分析当前状态
./scripts/analyze_gaps.sh

# 2. 选择一个功能（例如：宏系统）
less third_party/cangjie_compiler/src/Parse/ParseMacro.cpp

# 3. 创建新模块
touch src/parser/macro.rs

# 4. 编写测试
cat > tests/macro_test.rs << 'EOF'
#[test]
fn test_parse_macro() {
    // ...
}
EOF

# 5. 实现功能
# 编辑 src/parser/macro.rs

# 6. 运行测试
cargo test macro

# 7. 集成测试
./scripts/system_test.sh
```

## 📖 推荐阅读顺序

1. **第一天**: 阅读本文档 + `docs/ast_mapping.md`
2. **第二天**: 运行分析脚本，理解差距
3. **第三天**: 选择一个简单功能（如类型别名），完整走一遍流程
4. **第四天**: 选择一个复杂功能（如宏系统），深入实现
5. **第五天**: 编写测试，对比 CJC 和 CJWasm 的行为

## 💡 最佳实践

1. **小步迭代**: 每次只实现一个小功能
2. **测试驱动**: 先写测试，再写实现
3. **参考为主**: 理解 CJC 的逻辑，不要直接翻译
4. **保持简洁**: CJWasm 的优势是简单，不要过度设计
5. **文档同步**: 每完成一个功能，更新 `docs/ast_mapping.md`

## 🎯 本周目标建议

- [ ] 运行 `./scripts/analyze_gaps.sh`
- [ ] 阅读 CJC 的 `ParseMacro.cpp`
- [ ] 实现基础的宏调用解析
- [ ] 添加 `@Assert` 和 `@Expect` 支持
- [ ] 编写 10 个测试用例
- [ ] 更新文档

---

**记住**: 目标不是 100% 复制 CJC，而是实现一个高效、简洁的仓颉到 WASM 编译器。参考 CJC 的语法规范，但用 Rust 的方式实现。
