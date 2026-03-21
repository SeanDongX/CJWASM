# CJWasm2 项目指南

## 验证方法
每次改动代码时，运行单元测试，系统测试，确保没有失败测试用例

## 测试命令
```bash
cargo test                          # 单元测试
./scripts/system_test.sh            # 系统测试（编译+验证+运行）
cargo run -- file.cj                # 编译单个文件
cargo run -- build -p path/to/pkg   # 编译包
```



## 核心组件
- **词法分析器** (`src/lexer/mod.rs`): token 流生成
- **语法解析器** (`src/parser/`): AST 构建 (expr.rs, stmt.rs, decl.rs, pattern.rs)
- **AST 定义** (`src/ast/mod.rs`): 语法树节点
- **代码生成** (`src/codegen/`): WebAssembly 输出

## 重要规则
**禁止修改 `third_party/` 下的任何文件**


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

## 参考
- 官方编译器: `third_party/cangjie_compiler/`

