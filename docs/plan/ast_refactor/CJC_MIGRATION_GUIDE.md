# CJC 语法 Spec 迁移指南

## 目标
从 cjc (C++) 参考语法规范，增量式移植到 cjwasm (Rust)

## 策略：按功能模块增量迁移

### 阶段 1: 建立映射表（1-2天）

#### 1.1 创建 AST 节点对照表
```bash
# 提取 cjc AST 节点定义
grep -r "class.*Decl\|class.*Expr\|class.*Type" third_party/cangjie_compiler/include/cangjie/AST/Node.h > docs/cjc_ast_nodes.txt

# 对比 cjwasm 现有实现
grep -r "pub enum\|pub struct" src/ast/ > docs/cjwasm_ast_nodes.txt
```

**输出文件:** `docs/ast_mapping.md`
- 列出 cjc 的所有 AST 节点
- 标记 cjwasm 已实现 ✅ / 未实现 ❌ / 部分实现 🟡
- 优先级排序（P0=核心语法，P1=常用特性，P2=高级特性）

#### 1.2 创建语法规则对照表
```bash
# 提取 cjc Parser 方法签名
grep -E "OwnedPtr<.*>.*Parse" third_party/cangjie_compiler/include/cangjie/Parse/Parser.h
```

**输出文件:** `docs/parser_mapping.md`
- ParseDecl 系列（类/函数/变量声明）
- ParseExpr 系列（表达式解析）
- ParseType 系列（类型解析）
- ParsePattern 系列（模式匹配）

### 阶段 2: 按优先级迁移（持续）

#### 2.1 高优先级（P0）- 核心语法
**已完成 ✅:**
- 基础类型（Int/Float/Bool/String）
- 函数声明/调用
- 控制流（if/while/for）
- 类/结构体/接口
- 泛型基础

**待补充（参考 cjc）:**
1. **宏系统** (`ParseMacro.cpp`)
   - 参考: `third_party/cangjie_compiler/src/Parse/ParseMacro.cpp`
   - 实现: `src/parser/macro.rs`（新建）
   - 关键点: `@` 注解解析、宏展开

2. **条件编译** (`ConditionalCompilation/`)
   - 参考: `third_party/cangjie_compiler/src/ConditionalCompilation/`
   - 实现: `src/parser/conditional.rs`（新建）
   - 关键点: `#if`/`#else`/`#endif` 预处理

3. **完整的模式匹配** (`ParsePattern.cpp`)
   - 参考: `third_party/cangjie_compiler/src/Parse/ParsePattern.cpp`
   - 增强: `src/parser/pattern.rs`（已有，需扩展）
   - 关键点: 嵌套解构、guard 条件、通配符

#### 2.2 中优先级（P1）- 常用特性
1. **属性系统** (`ParseAnnotations.cpp`)
   - 参考: `third_party/cangjie_compiler/src/Parse/ParseAnnotations.cpp`
   - 实现: `src/parser/annotation.rs`（新建）

2. **导入系统增强** (`ParseImports.cpp`)
   - 参考: `third_party/cangjie_compiler/src/Parse/ParseImports.cpp`
   - 增强: `src/parser/mod.rs` 中的 `parse_import`

3. **Quote 宏** (`ParseQuote.cpp`)
   - 参考: `third_party/cangjie_compiler/src/Parse/ParseQuote.cpp`
   - 实现: `src/parser/quote.rs`（新建）

#### 2.3 低优先级（P2）- 高级特性
- LSP 支持（`AST/Query.cpp`）
- 增量编译（`IncrementalCompilation/`）
- AST 缓存（`AST/Cache.cpp`）

### 阶段 3: 具体迁移流程（每个功能）

#### 步骤 1: 理解 cjc 实现
```bash
# 以宏系统为例
cd third_party/cangjie_compiler/src/Parse

# 1. 查看头文件接口
cat ../../include/cangjie/Parse/Parser.h | grep -A 5 "ParseMacro"

# 2. 查看实现细节
cat ParseMacro.cpp | less

# 3. 查看 AST 节点定义
cat ../../include/cangjie/AST/Node.h | grep -A 20 "MacroInvocation"

# 4. 查看测试用例
find ../../unittests -name "*Macro*" -type f
```

#### 步骤 2: 设计 Rust 等价实现
```rust
// 示例: 宏调用解析
// cjc: OwnedPtr<MacroInvocation> Parser::ParseMacroInvocation()
// cjwasm:
impl Parser {
    fn parse_macro_invocation(&mut self) -> Result<MacroInvocation, ParseError> {
        // 1. 解析 @ 符号
        self.expect(Token::At)?;

        // 2. 解析宏名称
        let name = self.parse_identifier()?;

        // 3. 解析参数列表
        let args = self.parse_macro_args()?;

        Ok(MacroInvocation { name, args })
    }
}
```

#### 步骤 3: 编写测试用例
```rust
#[test]
fn test_parse_macro_invocation() {
    let source = "@Assert(a, b)";
    let mut parser = Parser::new(source);
    let macro_call = parser.parse_macro_invocation().unwrap();
    assert_eq!(macro_call.name, "Assert");
    assert_eq!(macro_call.args.len(), 2);
}
```

#### 步骤 4: 集成到主解析器
```rust
// src/parser/mod.rs
match self.current_token() {
    Token::At => self.parse_macro_invocation()?,
    // ... 其他分支
}
```

### 阶段 4: 验证工具

#### 4.1 语法对比工具
```bash
#!/bin/bash
# scripts/compare_syntax.sh
# 用 cjc 和 cjwasm 分别解析同一个 .cj 文件，对比 AST

CJ_FILE=$1

# cjc 解析（需要实现 AST dump）
cjc --dump-ast $CJ_FILE > /tmp/cjc_ast.txt

# cjwasm 解析
cjwasm --dump-ast $CJ_FILE > /tmp/cjwasm_ast.txt

# 对比
diff -u /tmp/cjc_ast.txt /tmp/cjwasm_ast.txt
```

#### 4.2 测试用例复用
```bash
# 复用 cjc 的测试文件
cp -r third_party/cangjie_compiler/unittests/Parse/ParseCangjieFiles/*.cj tests/fixtures/cjc_compat/

# 批量测试
for f in tests/fixtures/cjc_compat/*.cj; do
    echo "Testing $f"
    cargo test -- $(basename $f .cj)
done
```

## 实用工具脚本

### 1. AST 节点提取器
```bash
#!/bin/bash
# scripts/extract_cjc_ast.sh
# 从 cjc 头文件提取所有 AST 节点定义

grep -E "^\s*(class|struct|enum)\s+\w+(Decl|Expr|Type|Pattern|Stmt)" \
    third_party/cangjie_compiler/include/cangjie/AST/Node.h \
    | sed 's/class //g; s/struct //g; s/ :.*//g' \
    | sort -u > docs/cjc_ast_nodes_list.txt
```

### 2. Parser 方法提取器
```bash
#!/bin/bash
# scripts/extract_cjc_parser_methods.sh
# 提取所有 Parse* 方法签名

grep -E "OwnedPtr<.*>\s+Parse\w+" \
    third_party/cangjie_compiler/include/cangjie/Parse/Parser.h \
    | sed 's/^\s*//g' > docs/cjc_parser_methods.txt
```

### 3. 差异分析器
```bash
#!/bin/bash
# scripts/analyze_gaps.sh
# 分析 cjwasm 相对于 cjc 的功能缺口

echo "=== Missing AST Nodes ==="
comm -23 <(sort docs/cjc_ast_nodes_list.txt) \
         <(grep -r "pub enum\|pub struct" src/ast/ | awk '{print $3}' | sort)

echo "=== Missing Parser Methods ==="
comm -23 <(grep "Parse" docs/cjc_parser_methods.txt | awk '{print $2}' | sort) \
         <(grep "fn parse_" src/parser/*.rs | awk -F'fn ' '{print $2}' | awk '{print $1}' | sort)
```

## 推荐工作流

### 每日迁移流程
1. **选择目标功能**（从 P0 列表）
2. **阅读 cjc 源码**（30-60分钟）
   - 头文件接口
   - 实现细节
   - 测试用例
3. **设计 Rust 实现**（30分钟）
   - 画出数据结构
   - 写伪代码
4. **编写代码**（2-3小时）
   - AST 节点定义
   - Parser 方法
   - 单元测试
5. **集成测试**（30分钟）
   - 运行 `cargo test`
   - 运行 `./scripts/system_test.sh`
6. **文档更新**（15分钟）
   - 更新 `docs/ast_mapping.md`
   - 标记已完成功能

### 每周回顾
- 统计完成度（已实现 / 总功能数）
- 识别阻塞问题
- 调整优先级

## 关键文件映射

| cjc 文件 | cjwasm 文件 | 说明 |
|---------|------------|------|
| `Parse/ParseDecl.cpp` | `parser/decl.rs` | 声明解析 |
| `Parse/ParseExpr.cpp` | `parser/expr.rs` | 表达式解析 |
| `Parse/ParseType.cpp` | `parser/type_.rs` | 类型解析 |
| `Parse/ParsePattern.cpp` | `parser/pattern.rs` | 模式解析 |
| `Parse/ParseMacro.cpp` | `parser/macro.rs` | 宏解析（待创建）|
| `Parse/ParseAnnotations.cpp` | `parser/annotation.rs` | 注解解析（待创建）|
| `Parse/ParseImports.cpp` | `parser/mod.rs` | 导入解析 |
| `AST/Node.h` | `ast/mod.rs` | AST 节点定义 |
| `AST/Types.h` | `ast/type_.rs` | 类型系统 |

## 注意事项

### 不要直接翻译
- cjc 有很多 Sema 阶段的逻辑混在 Parser 里
- cjwasm 应该保持 Parser 纯粹（只做语法分析）
- 类型检查、语义分析放到后续阶段

### 保持简洁
- cjc 支持 LSP、增量编译等高级特性
- cjwasm 初期只需要核心编译功能
- 避免过度设计

### 测试驱动
- 每个新功能都要有测试用例
- 优先使用 cjc 的测试文件作为参考
- 保持 `cargo test` 全部通过

## 下一步行动

1. **立即执行**（今天）:
   ```bash
   # 创建映射文档
   mkdir -p docs
   ./scripts/extract_cjc_ast.sh
   ./scripts/extract_cjc_parser_methods.sh
   ./scripts/analyze_gaps.sh
   ```

2. **本周目标**:
   - 完成 AST/Parser 映射表
   - 选择 3-5 个 P0 功能开始迁移
   - 建立自动化测试流程

3. **本月目标**:
   - 完成所有 P0 功能
   - 覆盖 80% 的 cjc 测试用例
   - 性能对比报告
