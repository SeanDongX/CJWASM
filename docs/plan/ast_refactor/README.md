# AST Refactor 计划文档索引

本目录包含从 CJC 迁移语法 spec 到 CJWasm 的完整计划和分析文档。

## 📚 文档列表

### 核心文档

1. **MIGRATION_SUMMARY.md** - 迁移总结
   - 当前状态分析（代码量、完成度）
   - 已完成和缺失的功能清单
   - 立即可执行的行动计划
   - 关键文件速查表

2. **QUICK_REFERENCE.md** - 快速参考指南
   - 典型迁移流程示例（宏系统）
   - 常用搜索命令
   - 最佳实践和常见陷阱
   - 本周目标建议

3. **CJC_MIGRATION_GUIDE.md** - 完整迁移指南
   - 增量式迁移策略
   - 按功能模块的迁移计划
   - 实用工具脚本说明
   - 验证工具和测试方法

4. **ast_mapping.md** - AST 节点映射表
   - 92 个 CJC AST 节点的详细对照
   - 实现状态标记（✅/🟡/❌）
   - 优先级排序（P0/P1/P2/P3）
   - 关键差异分析

5. **ARCHITECTURE_COMPARISON.md** - 架构对比分析
   - CJC vs CJWasm 架构详细对比
   - 设计目标差异分析
   - 中间表示 (IR) 的差异
   - 性能对比和建议

### 数据文件

6. **cjc_ast_nodes_list.txt** - CJC AST 节点列表
   - 从 CJC 源码提取的 92 个 AST 节点

7. **cjwasm_ast_nodes_list.txt** - CJWasm AST 节点列表
   - 当前 CJWasm 实现的 AST 节点

8. **cjc_parser_methods.txt** - CJC Parser 方法列表
   - CJC Parser 的公共接口方法

9. **cjwasm_parser_methods_list.txt** - CJWasm Parser 方法列表
   - 当前 CJWasm 实现的 Parser 方法

## 🚀 快速开始

### 第一次阅读建议顺序

1. **MIGRATION_SUMMARY.md** (10 分钟)
   - 了解整体状况和目标

2. **QUICK_REFERENCE.md** (15 分钟)
   - 学习具体的迁移流程

3. **ast_mapping.md** (30 分钟)
   - 查看详细的节点对照表

4. **ARCHITECTURE_COMPARISON.md** (20 分钟)
   - 理解架构差异的根本原因

5. **CJC_MIGRATION_GUIDE.md** (按需查阅)
   - 详细的工具和流程说明

### 立即可执行的命令

```bash
# 1. 分析功能缺口
./scripts/analyze_gaps.sh

# 2. 查看 AST 节点对照
cat docs/plan/ast_refactor/ast_mapping.md

# 3. 开始第一个功能（宏系统）
less third_party/cangjie_compiler/src/Parse/ParseMacro.cpp
```

## 📊 关键数据

### 代码量对比
```
CJC:     ~70,000 行 (完整编译器)
CJWasm:  ~12,500 行 (核心功能 + 高级特性)
完成度:  ~95% (核心功能 + P1/P2/P3 大部分功能已实现)
```

### AST 节点对比
```
CJC:     92 个节点类型
CJWasm:  91 个枚举变体
  - Expr: 47
  - Stmt: 15
  - Pattern: 10
  - Type: 19
差距:    仅剩 Quote 宏、指针操作等少数功能
```

### 优先级功能

**P0 (核心)** - 已完成 ✅
- 基础类型、函数、类、结构体、接口、枚举
- 控制流、模式匹配、泛型、错误处理

**P1 (常用)** - 已完成 ✅
- 宏系统（@Assert, @Expect）
- 类型别名
- 完整的 if-let/while-let 支持

**P2 (高级)** - 已完成 ✅
- 可选链、尾随闭包、do-while

**P3 (低频)** - 部分完成 🟡
- spawn/synchronized (单线程桩实现)
- ❌ Quote 宏、指针操作

## 🎯 本周目标

- [x] 运行 `./scripts/analyze_gaps.sh`
- [x] 阅读 CJC 的 `ParseMacro.cpp`
- [x] 实现基础的宏调用解析
- [x] 添加 `@Assert` 和 `@Expect` 支持
- [x] 编写 20+ 个测试用例

## 📈 测试覆盖率

- **单元测试**: 222/230 passed (96.5%)
- **测试夹具**: 20+ 个 .cj 文件
- **失败测试**: 7 个 (待修复)
  - test_parse_extern_func
  - test_parse_lambda_brace_syntax
  - test_parse_lambda_arrow_syntax
  - test_parse_type_annotations
  - test_parse_error_bad_extern_import_attr
  - test_parse_error_bad_extern_import_name
  - test_parse_error_bad_match_subject

## 💡 核心建议

1. **不要直接拷贝** - 参考逻辑，用 Rust 的方式实现
2. **小步迭代** - 每次只实现一个功能
3. **测试驱动** - 先写测试，再写实现
4. **保持简洁** - CJWasm 的优势是简单快速
5. **复用测试** - 使用 CJC 的测试用例验证

## 📞 相关脚本

```bash
# 分析工具（在项目根目录运行）
../scripts/analyze_gaps.sh              # 分析功能缺口
../scripts/extract_cjc_ast.sh           # 提取 CJC AST 节点
../scripts/extract_cjc_parser_methods.sh # 提取 CJC Parser 方法
```

## 🔗 相关目录

- `third_party/cangjie_compiler/` - CJC 源码
- `src/parser/` - CJWasm Parser 实现
- `src/ast/` - CJWasm AST 定义
- `scripts/` - 分析和提取工具

---

**最后更新**: 2026-02-27
**文档版本**: 1.0
