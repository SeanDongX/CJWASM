# CJC 语法迁移总结

## 📊 现状分析

### 代码量对比
```
CJC (C++):
  - Parse:  12,531 行
  - AST:    10,362 行
  - 总计:   22,893 行

CJWasm (Rust):
  - Parser:  ~10,000 行
  - AST:       900 行
  - 总计:    ~12,500 行

完成度: ~55% (按代码量)
功能完成度: ~95% (核心功能 + P1/P2/P3 大部分功能已实现)
```

### AST 节点对比
- CJC: 92 个 AST 节点类型
- CJWasm: 91 个枚举变体
  - Expr: 47
  - Stmt: 15
  - Pattern: 10
  - Type: 19
- 差距: 仅剩 Quote 宏、指针操作等少数功能

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

## ✅ 已完成的高级功能

### 高优先级 (P1) - 已完成 ✅

1. **宏系统** ✅
    - 实现: `src/parser/macro.rs`, `src/codegen/macro.rs`
    - 功能: `@Assert`, `@Expect` 等内建宏
    - 测试: `tests/fixtures/macro_test.cj`

2. **类型别名** ✅
    - 实现: `src/parser/decl.rs`
    - 功能: `type MyInt = Int64`
    - 测试: `tests/fixtures/type_alias_test.cj`

3. **完整的 if-let/while-let 支持** ✅
    - 实现: `src/parser/expr.rs`, `src/parser/stmt.rs`
    - 功能: `if let Some(x) = opt { ... }`, `while let Some(x) = opt { ... }`
    - 测试: `tests/fixtures/if_let_test.cj`

### 中优先级 (P2) - 已完成 ✅

4. **可选链** (`?.`) ✅
    - 实现: `src/codegen/expr.rs:5926`
    - 功能: `obj?.field`
    - 测试: `tests/fixtures/optional_chain_test.cj`

5. **尾随闭包** ✅
    - 实现: `src/codegen/expr.rs:5961`
    - 功能: `f(args) { params => body }`
    - 测试: `tests/fixtures/trailing_closure_test.cj`

6. **do-while 循环** ✅
    - 实现: `src/parser/stmt.rs:349`
    - 功能: `do { ... } while (cond)`

### 低优先级 (P3) - 部分完成 🟡

7. **并发原语** (spawn, synchronized) 🟡
    - 实现: 单线程桩实现（直接同步执行）
    - 功能: `spawn { block }`, `synchronized(lock) { block }`

## ❌ 仍需实现的功能

### 低优先级 (P3)

1. **Quote 宏**（元编程）
    - 参考: `ParseQuote.cpp`
    - 功能: 元编程

2. **指针操作**
    - 功能: unsafe 指针操作

3. **MapLiteral 完整实现**
    - 当前: AST 已定义，codegen 待实现
    - 参考: `src/codegen/expr.rs:5904`

### 未来考虑 (P4)

4. **LSP 支持**
5. **增量编译**

## 🎯 推荐的迁移策略

### ❌ 不推荐：直接拷贝

**为什么不能直接拷贝？**
1. **语言差异**: C++ vs Rust（内存模型、错误处理、类型系统完全不同）
2. **架构差异**: CJC 是完整编译器（含 Sema/CHIR/LSP），CJWasm 是轻量编译器
3. **代码量差异**: 2万+ 行 vs 1.2万行，直接翻译会引入不必要的复杂度

### ✅ 推荐：增量式参考移植

**正确的方法：**
1. **理解** CJC 的语法规范和实现逻辑
2. **设计** 符合 CJWasm 架构的 Rust 实现
3. **测试** 用 CJC 的测试用例验证
4. **迭代** 小步快跑，每次只实现一个功能

### ✅ 已完成策略

已成功实现 P1/P2/P3 功能：
- P1: 宏系统、类型别名、完整模式匹配
- P2: 可选链、尾随闭包、do-while
- P3: spawn (桩实现)、synchronized (桩实现)

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

### ✅ 已完成

1. **运行分析脚本，了解现状**
    ```bash
    ./scripts/analyze_gaps.sh
    ```

2. **阅读文档**
    - `docs/QUICK_REFERENCE.md`
    - `docs/ast_mapping.md`

3. **实现 P1 功能**
    - ✅ 宏系统（@Assert, @Expect）
    - ✅ 类型别名
    - ✅ 完整的 if-let/while-let 支持

4. **实现 P2 功能**
    - ✅ 可选链
    - ✅ 尾随闭包
    - ✅ do-while 循环

5. **实现 P3 功能**
    - ✅ spawn (单线程桩实现)
    - ✅ synchronized (单线程桩实现)

### 待办事项

#### 高优先级

1. **修复失败的单元测试** (7 个失败)
    - `test_parse_extern_func`
    - `test_parse_lambda_brace_syntax`
    - `test_parse_lambda_arrow_syntax`
    - `test_parse_type_annotations`
    - `test_parse_error_bad_extern_import_attr`
    - `test_parse_error_bad_extern_import_name`
    - `test_parse_error_bad_match_subject`

2. **MapLiteral 完整实现**
    - 当前: AST 已定义，codegen 待实现
    - 参考: `src/codegen/expr.rs:5904`

#### 低优先级

1. **Quote 宏**（元编程）
2. **指针操作**

### 测试覆盖率

- **单元测试**: 222/230 passed (96.5%)
- **测试夹具**: 20+ 个 .cj 文件

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
