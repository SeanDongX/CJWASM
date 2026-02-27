# CJC 语法迁移总结

## 📊 现状分析

### 代码量对比
```
CJC (C++):
  - Parse:  12,531 行
  - AST:    10,362 行
  - 总计:   22,893 行

CJWasm (Rust):
  - Parser:  7,624 行
  - AST:       868 行
  - 总计:    8,492 行

完成度: ~37% (按代码量)
功能完成度: ~60% (核心功能已实现)
```

### AST 节点对比
- CJC: 92 个 AST 节点类型
- CJWasm: ~30 个枚举变体
- 差距: 主要在高级特性（宏、并发、LSP 支持等）

## ✅ 已完成的核心功能

CJWasm 已经实现了编译器的核心功能，可以编译大部分仓颉程序：

1. **类型系统** ✅
   - 基础类型（Int8-Int64, UInt8-UInt64, Float32/64, Bool, String, Rune）
   - 复合类型（Array, Tuple, Option, Result）
   - 泛型类型（泛型函数、类、结构体、枚举）

2. **表达式** ✅
   - 算术/逻辑/比较运算
   - 函数调用、方法调用
   - 字段访问、下标访问
   - Lambda 表达式
   - 字符串插值
   - 类型转换（as）、类型判断（is）

3. **控制流** ✅
   - if/else 表达式
   - while/for-in 循环
   - match 表达式（模式匹配）
   - break/continue
   - return

4. **声明** ✅
   - 函数声明（含泛型、默认参数）
   - 类声明（含继承、abstract/sealed）
   - 结构体声明
   - 接口声明（含默认实现）
   - 枚举声明（含关联值）
   - 属性（prop）声明
   - extend 声明

5. **错误处理** ✅
   - try-catch-finally
   - throws 声明
   - Result<T,E> / Option<T>
   - `?` 运算符
   - 空值合并 `??`

6. **模块系统** ✅
   - import 语句
   - 多文件编译
   - cjpm 工程支持

## ❌ 缺失的重要功能

### 高优先级 (P1) - 建议优先实现

1. **宏系统** 🔴
   - 参考: `third_party/cangjie_compiler/src/Parse/ParseMacro.cpp`
   - 功能: `@Assert`, `@Deprecated`, `@CallingConv` 等注解
   - 影响: 测试断言、编译器指令
   - 工作量: 2-3 天

2. **类型别名** 🟡
   - 参考: `ParseDecl.cpp` 中的 `ParseTypeAlias`
   - 功能: `type MyInt = Int64`
   - 影响: 代码可读性
   - 工作量: 1 天

3. **完整的 if-let 支持** 🟡
   - 参考: `ParsePattern.cpp`
   - 功能: `if let Some(x) = opt { ... }`
   - 影响: Option/Result 的便捷使用
   - 工作量: 1-2 天

### 中优先级 (P2) - 可以延后

4. **可选链** (`?.`)
5. **尾随闭包**
6. **Quote 宏**（元编程）
7. **do-while 循环**

### 低优先级 (P3) - 未来考虑

8. **并发原语** (spawn, synchronized)
9. **指针操作**
10. **LSP 支持**
11. **增量编译**

## 🎯 推荐的迁移策略

### ❌ 不推荐：直接拷贝

**为什么不能直接拷贝？**
1. **语言差异**: C++ vs Rust（内存模型、错误处理、类型系统完全不同）
2. **架构差异**: CJC 是完整编译器（含 Sema/CHIR/LSP），CJWasm 是轻量编译器
3. **代码量差异**: 2万+ 行 vs 8千行，直接翻译会引入不必要的复杂度

### ✅ 推荐：增量式参考移植

**正确的方法：**
1. **理解** CJC 的语法规范和实现逻辑
2. **设计** 符合 CJWasm 架构的 Rust 实现
3. **测试** 用 CJC 的测试用例验证
4. **迭代** 小步快跑，每次只实现一个功能

## 🛠️ 已创建的工具

### 1. 分析脚本
```bash
./scripts/analyze_gaps.sh           # 分析功能缺口
./scripts/extract_cjc_ast.sh        # 提取 CJC AST 节点
./scripts/extract_cjc_parser_methods.sh  # 提取 CJC Parser 方法
```

### 2. 文档
```
docs/
├── CJC_MIGRATION_GUIDE.md   # 完整迁移指南（策略、流程、工具）
├── ast_mapping.md            # AST 节点映射表（92 个节点的对照）
├── QUICK_REFERENCE.md        # 快速参考（常用命令、示例）
├── cjc_ast_nodes_list.txt    # CJC AST 节点列表
├── cjwasm_ast_nodes_list.txt # CJWasm AST 节点列表
├── cjc_parser_methods.txt    # CJC Parser 方法列表
└── cjwasm_parser_methods_list.txt  # CJWasm Parser 方法列表
```

## 📋 立即可执行的行动计划

### 今天（1-2 小时）

1. **运行分析脚本，了解现状**
   ```bash
   cd /Users/sean/workspace/cangjie_oss/CJWasm2
   ./scripts/analyze_gaps.sh
   ```

2. **阅读文档**
   - `docs/QUICK_REFERENCE.md` (15 分钟)
   - `docs/ast_mapping.md` (30 分钟)

3. **选择第一个功能**
   - 建议: 宏系统（最重要）
   - 阅读: `third_party/cangjie_compiler/src/Parse/ParseMacro.cpp`

### 本周（3-5 天）

**目标: 实现宏系统基础支持**

#### Day 1: 研究 CJC 实现
```bash
# 查看宏解析实现
less third_party/cangjie_compiler/src/Parse/ParseMacro.cpp

# 查看 AST 节点定义
grep -A 30 "MacroInvocation" third_party/cangjie_compiler/include/cangjie/AST/Node.h

# 查看测试用例
ls third_party/cangjie_compiler/unittests/Macro/srcFiles/
cat third_party/cangjie_compiler/unittests/Macro/srcFiles/func.cj
```

#### Day 2-3: 实现基础功能
1. 添加 AST 节点 (`src/ast/mod.rs`)
2. 实现 Parser (`src/parser/macro.rs`)
3. 编写单元测试
4. 集成到主解析器

#### Day 4: 代码生成
1. 实现 `@Assert` 宏的 WASM 生成
2. 实现 `@Expect` 宏的 WASM 生成
3. 端到端测试

#### Day 5: 测试和文档
1. 复用 CJC 测试用例
2. 运行 `cargo test`
3. 更新 `docs/ast_mapping.md`

### 本月（4 周）

**目标: 完成所有 P1 功能**

- Week 1: 宏系统 ✅
- Week 2: 类型别名
- Week 3: 完整的 if-let 支持
- Week 4: 测试、优化、文档

## 📖 关键文件速查

### 最重要的 CJC 文件（必读）

1. **AST 节点定义**
   ```
   third_party/cangjie_compiler/include/cangjie/AST/Node.h
   ```
   - 包含所有 AST 节点的定义
   - 理解仓颉的语法结构

2. **Parser 接口**
   ```
   third_party/cangjie_compiler/include/cangjie/Parse/Parser.h
   ```
   - Parser 的公共接口
   - 了解解析入口点

3. **表达式解析**
   ```
   third_party/cangjie_compiler/src/Parse/ParseExpr.cpp
   ```
   - 表达式解析的完整实现
   - 运算符优先级、结合性

4. **声明解析**
   ```
   third_party/cangjie_compiler/src/Parse/ParseDecl.cpp
   ```
   - 函数、类、结构体等声明的解析

5. **宏解析**
   ```
   third_party/cangjie_compiler/src/Parse/ParseMacro.cpp
   ```
   - 宏系统的实现（P1 功能）

### CJWasm 对应文件

1. `src/ast/mod.rs` - AST 节点定义
2. `src/parser/mod.rs` - Parser 主逻辑
3. `src/parser/expr.rs` - 表达式解析
4. `src/parser/decl.rs` - 声明解析
5. `src/codegen/mod.rs` - WASM 代码生成

## 💡 关键建议

### DO ✅

1. **小步迭代**: 每次只实现一个功能，立即测试
2. **测试驱动**: 先写测试，再写实现
3. **参考为主**: 理解 CJC 的逻辑，用 Rust 的方式实现
4. **保持简洁**: CJWasm 的优势是简单，不要过度设计
5. **复用测试**: 使用 CJC 的测试用例验证正确性

### DON'T ❌

1. **不要逐行翻译**: C++ 和 Rust 的编程范式不同
2. **不要复制所有功能**: 只实现核心编译功能
3. **不要忽略测试**: 每个功能都要有测试
4. **不要混淆职责**: Parser 只做语法分析，不做类型检查
5. **不要过度优化**: 先实现功能，再考虑性能

## 🎓 学习曲线

```
Week 1: 理解架构差异，建立映射关系
Week 2: 实现第一个功能（宏系统）
Week 3: 加速实现（类型别名、if-let）
Week 4: 测试和优化
```

**预期成果**:
- 1 个月内完成所有 P1 功能
- 覆盖 80% 的 CJC 测试用例
- 保持编译速度优势（CJWasm 比 CJC 快 10-100 倍）

## 🚀 下一步

**立即执行**:
```bash
# 1. 查看当前状态
./scripts/analyze_gaps.sh

# 2. 阅读快速参考
cat docs/QUICK_REFERENCE.md

# 3. 开始第一个功能
less third_party/cangjie_compiler/src/Parse/ParseMacro.cpp
```

**本周目标**:
- [ ] 理解 CJC 宏系统的实现
- [ ] 实现基础的宏调用解析
- [ ] 添加 `@Assert` 和 `@Expect` 支持
- [ ] 编写 10 个测试用例

**本月目标**:
- [ ] 完成宏系统
- [ ] 完成类型别名
- [ ] 完成 if-let 支持
- [ ] 覆盖 80% 的 CJC 测试用例

---

## 📞 需要帮助？

如果在迁移过程中遇到问题：

1. **查看文档**: `docs/QUICK_REFERENCE.md` 有常见问题的解答
2. **查看映射表**: `docs/ast_mapping.md` 有详细的节点对照
3. **查看测试**: CJC 的 `unittests/` 目录有大量示例
4. **运行分析**: `./scripts/analyze_gaps.sh` 可以随时查看进度

**记住**: 目标不是 100% 复制 CJC，而是实现一个高效、简洁的仓颉到 WASM 编译器！
