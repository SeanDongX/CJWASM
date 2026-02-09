# 测试覆盖率

本项目使用 [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) 统计测试覆盖率。

## 环境准备

1. 安装 cargo-llvm-cov：
   ```bash
   cargo install cargo-llvm-cov
   ```

2. 安装 LLVM 工具（二选一）：
   - `rustup component add llvm-tools-preview`（若可用）
   - 或设置环境变量指向已安装的 `llvm-cov` / `llvm-profdata`（例如在 `scripts/coverage.sh` 中已做查找）

## 运行覆盖率

在项目根目录执行：

```bash
# 仅终端报告
cargo llvm-cov --all-features

# 生成 HTML 报告（输出到 target/llvm-cov/html/index.html）
cargo llvm-cov --all-features --html
```

若未安装 `llvm-tools-preview`，可先设置工具路径再执行，或使用脚本：

```bash
./scripts/coverage.sh
./scripts/coverage.sh --html
```

## 当前覆盖率概览（最近一次统计）

| 模块        | 行覆盖率 | 说明 |
|-------------|----------|------|
| lexer       | ~95%     | 词法分析，用例较全 |
| codegen     | ~83%     | 代码生成，部分分支未覆盖 |
| parser      | ~73%     | 语法分析，复杂/错误路径可补测 |
| ast         | ~63%     | AST 定义，部分由 codegen/parser 间接覆盖 |
| main.rs     | 0%       | 仅二进制入口，不参与库测试，可忽略 |

**总体行覆盖率约 77%。**

未覆盖部分主要集中在：解析错误分支、部分 match 分支、未使用的常量/类型等。可通过查看 `target/llvm-cov/html/index.html` 定位具体未覆盖行。
