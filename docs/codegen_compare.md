# CJWasm vs CJC Codegen 对比

## 主要区别

### 1. 目标后端
- **CJC**: 使用 LLVM 作为后端，生成机器码
- **CJWasm**: 使用 `wasm-encoder` 直接生成 WebAssembly 字节码

### 2. 架构设计
- **CJC**: 采用分层设计
  - `CGContext` - 编译上下文，管理全局状态
  - `CGModule` - 负责整个模块的代码生成
  - `CGFunction` - 单个函数生成
  - 使用 `CHIR` (Cangjie High-level IR) 作为中间表示
  - 通过 `IRBuilder` 构建 LLVM IR

- **CJWasm**: 简化设计
  - `CodeGen` 结构体集中管理所有状态
  - 直接从 AST 编译到 WASM，没有中间 IR
  - `expr.rs` 处理表达式，`decl.rs` 处理声明

### 3. 名字 mangling
- **CJC**: 复杂的 mangling 方案 (见 `Mangle/ASTMangler.cpp`)
  - 使用 `$` 分隔符: `Box$Int64`
  - 支持多种类型后缀

- **CJWasm**: 简化版 mangling
  - 格式: `TypeName$TypeArg1$TypeArg2`

### 4. 方法调用解析
- **CJC**:
  - 通过 `GetMethodIdxInAutoEnvObject` 获取方法索引
  - 使用 vtable 进行虚方法调用
  - 方法查找在语义分析阶段完成

- **CJWasm**:
  - 在 codegen 阶段动态解析方法名
  - 需要 `mangle_name` 配合查找
  - 目前 monomorphization 的方法添加到了 `program.functions`，但解析时可能找不到

### 5. CJWasm 当前问题

1. **方法解析时机**: monomorphized 方法被添加到 `program.functions`，但 codegen 解析方法调用时使用的是简单名称（如 `Box.init`），而不是 mangled 名称（如 `Box$Int64.init`）

2. **ConstructorCall 处理**: 已在 `expr.rs` 添加 mangled name 处理，但有语法错误

3. **缺少 CHIR 层**: CJC 有完整的类型推断和语义分析，CJWasm 的 monomorphization 实现可能在类型解析上不够完整

### 6. CJC 关键文件参考

- `src/CodeGen/CGContext.h` - 编译上下文定义
- `src/CodeGen/CGModule.h` - 模块生成器
- `src/CodeGen/EmitExpressionIR.cpp` - 表达式 IR 生成
- `src/CodeGen/Base/InvokeImpl.cpp` - 方法调用实现
- `src/Mangle/ASTMangler.cpp` - AST 名字 mangling 实现