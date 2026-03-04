# CJWasm2 项目指南

## 核心组件
- **词法分析器** (`src/lexer/mod.rs`): token 流生成
- **语法解析器** (`src/parser/`): AST 构建 (expr.rs, stmt.rs, decl.rs, pattern.rs)
- **AST 定义** (`src/ast/mod.rs`): 语法树节点
- **代码生成** (`src/codegen/`): WebAssembly 输出

## 重要规则
**禁止修改 `third_party/` 下的任何文件**

## 常见修复流程

### Token 错误
1. `src/lexer/mod.rs` 添加 token
2. 对应解析器添加处理逻辑
3. `src/ast/mod.rs` 添加节点（如需）
4. `src/codegen/` 添加代码生成

### 表达式解析 (`src/parser/expr.rs`)
- `parse_expr()`: 入口
- `parse_primary()`: 基础表达式
- `parse_postfix_from_expr()`: 后缀表达式

### 语句解析 (`src/parser/stmt.rs`)
- `parse_stmt()`: 入口
- `expr_to_assign_target()`: 赋值目标转换

### AST 扩展 (`src/ast/mod.rs`)
1. enum 添加新变体
2. 更新所有 match 语句
3. codegen 添加对应生成逻辑

## 调试方法

### 定位错误
```bash
# 字节偏移定位
awk 'BEGIN {pos=0} {line_start=pos; pos+=length($0)+1; if (pos > N && line_start <= N) {print NR": "$0}}' file.cj
```

### 二分查找
```bash
head -N file.cj > /tmp/test.cj
echo "}" >> /tmp/test.cj
cargo run -- /tmp/test.cj
```

### 参考官方实现
```bash
grep -rn "关键字" third_party/cangjie_compiler/src/Parse/
```

## 测试命令
```bash
cargo test                          # 单元测试
./scripts/run_examples.sh           # 运行示例
cargo run -- file.cj                # 编译单个文件
cargo run -- build -p path/to/pkg   # 编译包
```

## 常见错误
- "期望: Assign" → 检查是否支持无初始化声明
- "期望: LParen" → 检查泛型参数处理
- "意外的 token: Lt" → 检查泛型解析上下文
- "简单数组访问" → 扩展 `AssignTarget` enum

## 代码规范
- `parse_xxx()`: 解析函数
- `expect()`: 期望 token
- `check()`: 检查 token
- `advance()`: 消费 token
- 使用 `Box` 包装递归结构
- 使用 `bail()` 返回错误

## 已知限制
- 泛型单态化不完全
- 宏展开有限
- 类型推断不完整
- 错误恢复较弱

## 参考
- 官方文档: https://cangjie-lang.cn/
- 官方编译器: `third_party/cangjie_compiler/`
