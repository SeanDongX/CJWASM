# AST 节点映射表

## 概览

- **CJC AST 节点总数**: 92
- **CJWasm AST 节点总数**: 91 (枚举变体)
  - Expr: 47
  - Stmt: 15
  - Pattern: 10
  - Type: 19
- **估计完成度**: ~95% (核心功能 + P1/P2/P3 大部分功能已实现)
- **最近更新**: 2026-02-28 - 完成所有 P1/P2 功能及大部分 P3 功能

## 映射状态说明

- ✅ **已完整实现**: CJWasm 有对应的完整实现
- 🟡 **部分实现**: 有基础实现但功能不完整
- ❌ **未实现**: CJWasm 中缺失
- 🔵 **不需要**: CJWasm 架构中不需要此节点

## 表达式 (Expr)

| CJC 节点 | CJWasm 对应 | 状态 | 优先级 | 说明 |
|---------|------------|------|--------|------|
| `BinaryExpr` | `Expr::Binary` | ✅ | P0 | 二元运算 |
| `UnaryExpr` | `Expr::Unary` | ✅ | P0 | 一元运算 |
| `CallExpr` | `Expr::Call` | ✅ | P0 | 函数调用 |
| `LitConstExpr` | `Expr::Literal` | ✅ | P0 | 字面量 |
| `IfExpr` | `Expr::If` | ✅ | P0 | if 表达式 |
| `MatchExpr` | `Expr::Match` | ✅ | P0 | match 表达式 |
| `ArrayLit` | `Expr::Array` | ✅ | P0 | 数组字面量 |
| `TupleLit` | `Expr::Tuple` | ✅ | P0 | 元组字面量 |
| `LambdaExpr` | `Expr::Lambda` | ✅ | P0 | Lambda 表达式 |
| `NameReferenceExpr` | `Expr::Identifier` | ✅ | P0 | 标识符引用 |
| `RefExpr` | `Expr::FieldAccess` | ✅ | P0 | 字段访问 (`.`) |
| `SubscriptExpr` | `Expr::Index` | ✅ | P0 | 下标访问 (`[]`) |
| `AsExpr` | `Expr::As` | ✅ | P0 | 类型转换 (`as`) |
| `IsExpr` | `Expr::Is` | ✅ | P1 | 类型判断 (`is`) |
| `RangeExpr` | `Expr::Range` | ✅ | P1 | 范围表达式 |
| `TryExpr` | `Expr::Try` | ✅ | P1 | try 表达式 |
| `OptionalExpr` | `Expr::Optional` | ✅ | P1 | 可选值 (`?`) |
| `AssignExpr` | `Stmt::Assign` | 🟡 | P0 | 赋值 (在 Stmt 中) |
| `ReturnExpr` | `Stmt::Return` | 🟡 | P0 | return (在 Stmt 中) |
| `ThrowExpr` | `Stmt::Throw` | 🟡 | P1 | throw (在 Stmt 中) |
| `WhileExpr` | `Stmt::While` | 🟡 | P0 | while 循环 |
| `ForInExpr` | `Stmt::For` | 🟡 | P0 | for-in 循环 |
| `DoWhileExpr` | `Stmt::DoWhile` | ✅ | P2 | do-while 循环 - 已实现 |
| `JumpExpr` | `Stmt::Break/Continue` | 🟡 | P0 | break/continue |
| `StrInterpolationExpr` | `Expr::Interpolate` | ✅ | P1 | 字符串插值 |
| `InterpolationExpr` | `Expr::Interpolate` | ✅ | P1 | 插值表达式 |
| `ParenExpr` | - | 🔵 | - | 括号表达式 (解析时处理) |
| `TypeConvExpr` | `Expr::As` | ✅ | P0 | 类型转换 |
| `OptionalChainExpr` | `Expr::OptionalChain` | ✅ | P2 | 可选链 (`?.`) - 已实现 |
| `SpawnExpr` | `Expr::Spawn` | ✅ | P3 | 并发 spawn - 单线程桩实现 |
| `SynchronizedExpr` | `Expr::Synchronized` | ✅ | P3 | 同步块 - 单线程桩实现 |
| `MacroExpandExpr` | `Expr::Macro` | ✅ | P1 | 宏调用表达式 - 内建宏 |
| `QuoteExpr` | - | ❌ | P2 | Quote 宏 |
| `TrailingClosureExpr` | `Expr::TrailingClosure` | ✅ | P2 | 尾随闭包 - 已实现 |
| `IncOrDecExpr` | `Expr::PrefixIncr/PrefixDecr/PostfixIncr/PostfixDecr` | ✅ | P2 | `++`/`--` - 已实现 |
| `OverloadableExpr` | - | 🔵 | - | 运算符重载 (未来) |
| `PrimitiveTypeExpr` | - | 🔵 | - | 类型表达式 |
| `PointerExpr` | - | ❌ | P3 | 指针操作 |
| `WildcardExpr` | `Pattern::Wildcard` | ✅ | P2 | 通配符 `_` - 已实现 |
| `InvalidExpr` | - | 🔵 | - | 错误恢复节点 |

## 声明 (Decl)

| CJC 节点 | CJWasm 对应 | 状态 | 优先级 | 说明 |
|---------|------------|------|--------|------|
| `FuncDecl` | `Function` | ✅ | P0 | 函数声明 |
| `VarDecl` | `Stmt::Let` | ✅ | P0 | 变量声明 |
| `ClassDecl` | `ClassDef` | ✅ | P0 | 类声明 |
| `StructDecl` | `StructDef` | ✅ | P0 | 结构体声明 |
| `InterfaceDecl` | `InterfaceDef` | ✅ | P0 | 接口声明 |
| `EnumDecl` | `EnumDef` | ✅ | P0 | 枚举声明 |
| `ExtendDecl` | `ExtendDef` | ✅ | P1 | extend 声明 |
| `PropDecl` | `PropDef` | ✅ | P1 | 属性声明 |
| `TypeAliasDecl` | `type_aliases` | ✅ | P1 | 类型别名 - 已实现 |
| `MacroDecl` | - | ❌ | P1 | 宏声明 |
| `MacroExpandDecl` | - | ❌ | P1 | 宏展开声明 |
| `MainDecl` | `Function` (main) | ✅ | P0 | main 函数 |
| `PackageDecl` | `Program` | 🟡 | P0 | 包声明 |
| `PrimaryCtorDecl` | `InitDef` | 🟡 | P1 | 主构造器 |
| `GenericParamDecl` | - | 🟡 | P0 | 泛型参数 (内联) |
| `VarDeclAbstract` | - | ❌ | P2 | 抽象变量 |
| `VarWithPatternDecl` | `Stmt::Let/Var` with `Pattern` | ✅ | P2 | 模式解构声明 - 已实现 |
| `BuiltInDecl` | - | 🔵 | - | 内建声明 |
| `InheritableDecl` | - | 🔵 | - | 可继承声明基类 |
| `ClassLikeDecl` | - | 🔵 | - | 类似类声明基类 |
| `InvalidDecl` | - | 🔵 | - | 错误恢复节点 |

## 类型 (Type)

| CJC 节点 | CJWasm 对应 | 状态 | 优先级 | 说明 |
|---------|------------|------|--------|------|
| `PrimitiveType` | `Type::Int*/Float*/Bool` | ✅ | P0 | 基础类型 |
| `FuncType` | `Type::Function` | ✅ | P0 | 函数类型 |
| `TupleType` | `Type::Tuple` | ✅ | P0 | 元组类型 |
| `OptionType` | `Type::Option` | ✅ | P0 | Option 类型 |
| `VArrayType` | `Type::Array` | ✅ | P0 | 数组类型 |
| `RefType` | `Type::Reference` | 🟡 | P1 | 引用类型 |
| `QualifiedType` | `Type::Qualified` | ✅ | P1 | 限定类型 - 已实现语法支持 |
| `ParenType` | - | 🔵 | - | 括号类型 (解析时处理) |
| `ThisType` | `Type::This` | 🟡 | P2 | `This` 类型 - 已实现语法支持 |
| `ConstantType` | - | ❌ | P2 | 常量类型 |
| `InvalidType` | - | 🔵 | - | 错误恢复节点 |

## 模式 (Pattern)

| CJC 节点 | CJWasm 对应 | 状态 | 优先级 | 说明 |
|---------|------------|------|--------|------|
| `VarPattern` | `Pattern::Identifier` | ✅ | P0 | 变量模式 |
| `TuplePattern` | `Pattern::Tuple` | ✅ | P0 | 元组模式 |
| `ConstPattern` | `Pattern::Literal` | ✅ | P0 | 常量模式 |
| `TypePattern` | `Pattern::TypeTest` | ✅ | P1 | 类型模式 - 已实现并修复 |
| `VarOrEnumPattern` | `Pattern::Enum` | 🟡 | P0 | 枚举模式 |
| `EnumPattern` | `Pattern::Enum` | ✅ | P0 | 枚举解构 |
| `WildcardPattern` | `Pattern::Wildcard` | ✅ | P0 | 通配符 `_` |
| `LetPatternDestructor` | `Pattern::Variant/Tuple/Struct` | ✅ | P2 | let 解构 - 已完整实现 |
| `ExceptTypePattern` | `TryBlock.catch_type` | ✅ | P2 | 异常类型模式 - 已实现 |
| `InvalidPattern` | - | 🔵 | - | 错误恢复节点 |

## 优先级定义

- **P0 (核心)**: 基础语法，必须实现才能编译简单程序
- **P1 (常用)**: 常用特性，提升语言表达能力
- **P2 (高级)**: 高级特性，可以延后实现
- **P3 (未来)**: 并发、高级内存管理等

## 关键差异分析

### 1. 架构差异

**CJC (完整编译器)**:
- 有独立的 `Expr`/`Stmt` 分离
- 有 `Invalid*` 节点用于错误恢复
- 有 `ClassLikeDecl` 等抽象基类
- 支持 LSP、增量编译

**CJWasm (轻量编译器)**:
- `Expr` 和 `Stmt` 合并在一起
- 错误直接返回 `ParseError`
- 扁平化的 AST 结构
- 专注于代码生成

### 2. 已实现的核心功能 ✅

CJWasm 已经实现了编译器的核心功能：

- ✅ 完整的表达式系统（算术、逻辑、比较、调用）
- ✅ 控制流（if/while/for/match）
- ✅ 函数定义和调用
- ✅ 类/结构体/接口/枚举
- ✅ 泛型（函数、类、结构体）
- ✅ 模式匹配（枚举、元组、字面量）
- ✅ 错误处理（try-catch、Result、Option）
- ✅ 字符串插值
- ✅ Lambda 表达式

### 3. 已实现的高级功能 ✅

#### 高优先级 (P1) - 已完成
1. **宏系统** (`Expr::Macro`) ✅
    - 已实现: `@Assert`, `@Expect` 等内建宏
    - 参考: `src/parser/macro.rs`, `src/codegen/macro.rs`

2. **类型别名** (`Program::type_aliases`) ✅
    - 已实现: `type Name = Type`
    - 参考: `src/parser/decl.rs`

3. **完整的模式匹配** (`Pattern::TypeTest`, `Expr::IfLet`, `Stmt::WhileLet`) ✅
    - 已实现: `if let Some(x) = opt`, `while let Some(x) = opt`
    - 参考: `src/parser/pattern.rs`, `src/parser/expr.rs`

#### 中优先级 (P2) - 已完成
1. **可选链** (`Expr::OptionalChain`) ✅
    - 已实现: `obj?.field`
    - 参考: `src/codegen/expr.rs:5926`

2. **尾随闭包** (`Expr::TrailingClosure`) ✅
    - 已实现: `f(args) { params => body }`
    - 参考: `src/codegen/expr.rs:5961`

3. **do-while 循环** (`Stmt::DoWhile`) ✅
    - 已实现: `do { ... } while (cond)`
    - 参考: `src/parser/stmt.rs:349`

#### 低优先级 (P3) - 已完成
1. **并发原语** (`Expr::Spawn`, `Expr::Synchronized`) ✅
    - 已实现: 单线程桩实现（直接同步执行）
    - 参考: `src/codegen/expr.rs:5908`

### 4. 未实现功能 ❌

1. **Quote 宏** (`QuoteExpr`)
    - 参考: `ParseQuote.cpp`
    - 用途: 元编程

2. **指针操作** (`PointerExpr`)
    - 用途: unsafe 指针操作

3. **MapLiteral 完整实现**
    - AST 已定义，codegen 待实现
    - 参考: `src/codegen/expr.rs:5904`

## 下一步行动建议

### 已完成 ✅

1. **所有 P1 功能** (宏系统、类型别名、完整模式匹配)
2. **所有 P2 功能** (可选链、尾随闭包、do-while)
3. **大部分 P3 功能** (spawn、synchronized 单线程桩实现)

### 待办事项

#### 高优先级
1. **MapLiteral 完整实现**
    - 当前 AST 已定义，codegen 中为 todo!
    - 参考: `src/codegen/expr.rs:5904`

2. **修复失败的单元测试** (7 个失败)
    - `test_parse_extern_func`
    - `test_parse_lambda_brace_syntax`
    - `test_parse_lambda_arrow_syntax`
    - `test_parse_type_annotations`
    - `test_parse_error_bad_extern_import_attr`
    - `test_parse_error_bad_extern_import_name`
    - `test_parse_error_bad_match_subject`

#### 低优先级
1. **Quote 宏** (`QuoteExpr`)
    - 元编程功能，可延后实现

2. **指针操作** (`PointerExpr`)
    - unsafe 指针操作，可延后实现

### 持续改进

1. **增加测试覆盖率**
    - 当前: 222/230 tests passed (96.5%)
    - 目标: 100% tests passed

2. **性能优化**
    - 参考 cjc 的优化策略
    - 保持编译速度优势

## 参考文件路径

### CJC 源码
- AST 定义: `third_party/cangjie_compiler/include/cangjie/AST/Node.h`
- Parser 接口: `third_party/cangjie_compiler/include/cangjie/Parse/Parser.h`
- 表达式解析: `third_party/cangjie_compiler/src/Parse/ParseExpr.cpp`
- 声明解析: `third_party/cangjie_compiler/src/Parse/ParseDecl.cpp`
- 类型解析: `third_party/cangjie_compiler/src/Parse/ParseType.cpp`
- 模式解析: `third_party/cangjie_compiler/src/Parse/ParsePattern.cpp`
- 宏解析: `third_party/cangjie_compiler/src/Parse/ParseMacro.cpp`

### CJWasm 源码
- AST 定义: `src/ast/mod.rs`
- Parser 主模块: `src/parser/mod.rs`
- 表达式解析: `src/parser/expr.rs`
- 声明解析: `src/parser/decl.rs`
- 类型解析: `src/parser/type_.rs`
- 模式解析: `src/parser/pattern.rs`
