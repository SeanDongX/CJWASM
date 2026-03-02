# CJWasm2 项目指南

## 项目概述

CJWasm2 是一个将仓颉（Cangjie）语言编译到 WebAssembly 的编译器，使用 Rust 实现。

### 核心组件

- **词法分析器** (`src/lexer/mod.rs`): 将源代码转换为 token 流
- **语法解析器** (`src/parser/`): 将 token 流转换为 AST
  - `mod.rs`: 解析器主结构和工具函数
  - `expr.rs`: 表达式解析
  - `stmt.rs`: 语句解析
  - `decl.rs`: 声明解析（函数、类、接口等）
  - `pattern.rs`: 模式匹配解析
  - `error.rs`: 错误处理
- **AST 定义** (`src/ast/mod.rs`): 抽象语法树节点定义
- **代码生成** (`src/codegen/`): 将 AST 转换为 WebAssembly
  - `mod.rs`: 主代码生成逻辑
  - `expr.rs`: 表达式代码生成

## 常见问题修复模式

### 1. 添加新的 Token 支持

**问题**: 遇到 "意外的 token: XXX" 错误

**解决步骤**:
1. 在 `src/lexer/mod.rs` 中添加 token 定义
2. 在相应的解析器文件中添加处理逻辑
3. 如果需要 AST 支持，在 `src/ast/mod.rs` 中添加节点
4. 在 `src/codegen/` 中添加代码生成逻辑

**示例**: 添加 `as` 类型转换支持
```rust
// 在 parse_match_subject 的后缀循环中添加
Some(Token::As) => {
    self.advance();
    let target_ty = self.parse_type()?;
    expr = Expr::Cast {
        expr: Box::new(expr),
        target_ty,
    };
}
```

### 2. 扩展表达式解析

**问题**: 某些表达式语法不被识别

**位置**: `src/parser/expr.rs`

**关键函数**:
- `parse_expr()`: 表达式解析入口
- `parse_primary()`: 基础表达式解析
- `parse_postfix_from_expr()`: 后缀表达式解析

**示例**: 支持类型关键字在表达式中使用
```rust
// 在 parse_primary 中添加类型关键字处理
Some(tok) if matches!(tok, Token::TypeFloat16 | ...) => {
    let name = match tok {
        Token::TypeFloat16 => "Float16",
        // ...
    };
    // 处理 Float16.method() 或 Float16(value)
}
```

### 3. 修复语句解析

**问题**: 语句语法不正确

**位置**: `src/parser/stmt.rs`

**关键函数**:
- `parse_stmt()`: 语句解析入口
- `expr_to_assign_target()`: 将表达式转换为赋值目标

**示例**: 支持无初始化变量声明
```rust
// 在 parse_stmt 的 Var 分支中
let value = if self.check(&Token::Assign) {
    self.advance();
    self.parse_expr()?
} else {
    // 无初始化值，使用类型默认值
    if let Some(ref t) = ty {
        self.default_value_for_type(t)
    } else {
        return self.bail(/* 错误 */);
    }
};
```

### 4. 扩展 AST 节点

**问题**: 需要表示新的语法结构

**位置**: `src/ast/mod.rs`

**步骤**:
1. 在相应的 enum 中添加新变体
2. 更新所有 match 语句以处理新变体
3. 在 codegen 中添加对应的代码生成

**示例**: 添加复杂数组索引赋值
```rust
pub enum AssignTarget {
    // ... 现有变体
    /// 复杂表达式索引 expr[i]（如 obj.method()[i]）
    ExprIndex { expr: Box<Expr>, index: Box<Expr> },
}
```

### 5. 处理范围操作符

**问题**: 开放式范围语法不支持

**解决**: 在解析时检查边界情况
```rust
// 支持 arr[..end], arr[start..], arr[..]
if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
    self.advance();
    let end = if self.check(&Token::RBracket) {
        Expr::Integer(i64::MAX) // 表示到末尾
    } else {
        self.parse_expr()?
    };
    // ...
}
```

## 调试技巧

### 1. 定位解析错误

错误信息格式: `语法错误: 意外的 token: XXX, 期望: YYY (字节偏移 N-M)`

**定位方法**:
```bash
# 使用 awk 找到具体行号和位置
awk 'BEGIN {pos=0} {
    line_start=pos;
    pos+=length($0)+1;
    if (pos > N && line_start <= N) {
        print NR": "$0;
        print "Byte offset in line:", N-line_start
    }
}' file.cj
```

### 2. 二分查找问题代码

当整个文件编译失败时:
```bash
# 测试前 N 行
head -N file.cj > /tmp/test.cj
echo "}" >> /tmp/test.cj  # 补充缺失的括号
echo "func main(): Int64 { return 0 }" >> /tmp/test.cj
cargo run -- /tmp/test.cj
```

### 3. 创建最小复现用例

从失败的代码中提取最小示例:
```cangjie
// 只保留导致错误的核心语法
class MyClass {
    private var field: Type = value
}
```

### 4. 参考官方编译器

查看 `third_party/cangjie_compiler/src/Parse/` 中的实现:
```bash
# 搜索相关语法处理
grep -rn "关键字" third_party/cangjie_compiler/src/Parse/
```

## 测试流程

### 运行单元测试
```bash
cargo test
```

### 运行示例
```bash
./scripts/run_examples.sh
```

### 编译单个文件
```bash
cargo run -- path/to/file.cj
```

### 编译包
```bash
cargo run -- build -p path/to/package
```

## 常见错误模式

### 1. "期望: Assign"
- **原因**: 变量声明后期望 `=` 但遇到其他 token
- **检查**: 是否支持无初始化声明？

### 2. "期望: LParen"
- **原因**: 函数调用或构造函数期望 `(` 但遇到其他 token
- **检查**: 是否正确处理了泛型参数 `<T>`？

### 3. "意外的 token: Lt"
- **原因**: `<` 被误认为是其他语法结构
- **检查**: 是否在正确的上下文中解析泛型？

### 4. "简单数组访问"
- **原因**: 赋值目标不支持复杂表达式
- **解决**: 扩展 `AssignTarget` enum

## 代码风格

### 解析器函数命名
- `parse_xxx()`: 解析特定语法结构
- `parse_xxx_list()`: 解析列表
- `expect()`: 期望特定 token，否则报错
- `check()`: 检查当前 token 类型
- `advance()`: 消费当前 token 并前进

### 错误处理
```rust
// 使用 bail 返回错误
return self.bail(ParseError::UnexpectedToken(
    tok,
    "期望的内容".to_string(),
));

// 使用 expect 期望特定 token
self.expect(Token::RParen)?;
```

### AST 构造
```rust
// 使用 Box 包装递归结构
Expr::Binary {
    op: BinOp::Add,
    left: Box::new(left_expr),
    right: Box::new(right_expr),
}
```

## 性能优化建议

1. **避免不必要的克隆**: 使用引用或 `std::mem::replace`
2. **提前返回**: 在循环中尽早检测结束条件
3. **缓存常用结果**: 如类型推断结果

## 贡献指南

### 添加新功能

1. 在 `src/lexer/mod.rs` 添加必要的 token
2. 在 `src/parser/` 添加解析逻辑
3. 在 `src/ast/mod.rs` 添加 AST 节点（如需要）
4. 在 `src/codegen/` 添加代码生成
5. 添加测试用例
6. 更新文档

### 修复 Bug

1. 创建最小复现用例
2. 定位问题代码
3. 参考官方编译器实现
4. 实现修复
5. 验证测试通过
6. 提交 PR

## 已知限制

1. **泛型单态化**: 目前不完全支持泛型的单态化
2. **宏系统**: 宏展开功能有限
3. **类型推断**: 某些复杂场景的类型推断不完整
4. **错误恢复**: 解析错误后的恢复机制较弱

## 参考资源

- 仓颉语言官方文档: https://cangjie-lang.cn/
- 官方编译器源码: `third_party/cangjie_compiler/`
- WebAssembly 规范: https://webassembly.github.io/spec/

## 最近修复的问题

1. ✅ As token 在 match 表达式中的支持
2. ✅ TypeFloat16 类型关键字在表达式中的使用
3. ✅ 开放式范围操作符 `arr[..end]`, `arr[start..]`
4. ✅ 复杂数组索引赋值 `obj.method()[i] = value`
5. ✅ 无初始化变量声明 `var x: Int64`
6. ✅ 泛型 None 支持 `None<T>`

## 联系方式

如有问题或建议，请提交 Issue 或 PR。
