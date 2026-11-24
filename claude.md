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
./scripts/system_test.sh            # 系统测试（编译+验证+运行）
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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **CJWasm** (4330 symbols, 19121 relationships, 381 execution flows).

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "master"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "master" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/CJWasm/context` | Codebase overview, check index freshness |
| `gitnexus://repo/CJWasm/clusters` | All functional areas |
| `gitnexus://repo/CJWasm/processes` | All execution flows |
| `gitnexus://repo/CJWasm/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
