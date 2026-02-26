# 参考 cjc 重构 src/ast、src/parser、src/codegen

本文档对比 `third_party/cangjie_compiler` 的 AST/Parse（及 CodeGen）与 cjwasm 当前实现，并给出可落地的重构建议。**不修改 third_party/**，仅指导 cjwasm 自身重构。

---

## 一、cjc AST 结构（third_party/cangjie_compiler）

### 1.1 目录与职责

| 路径 | 职责 |
|------|------|
| `include/cangjie/AST/Node.h` | 基类 Node、ASTKind 枚举、Decl/Expr/Type/Pattern 继承体系 |
| `include/cangjie/AST/ASTKind.inc` | 所有 AST 节点种类（Decl/Expr/Type/Pattern 等）集中定义 |
| `include/cangjie/AST/Types.h` | TypeKind 等类型相关枚举 |
| `src/AST/Node.cpp` | ToString、位置/宏展开等通用逻辑 |
| `src/AST/Create.cpp` | 工厂函数（如 CreateUnitExpr） |
| `src/AST/Clone.cpp` | 深拷贝 |
| `src/AST/Walker.cpp` | 树遍历 |
| `src/AST/Utils.cpp`、Searcher、Query 等 | 查询与工具 |

### 1.2 节点分类（ASTKind.inc）

- **Decl**：MainDecl, FuncDecl, MacroDecl, ClassDecl, InterfaceDecl, ExtendDecl, EnumDecl, StructDecl, TypeAliasDecl, PrimaryCtorDecl, VarDecl, PropDecl, FuncParam, GenericParamDecl, PackageDecl, MacroExpandDecl, InvalidDecl 等。
- **Pattern**：VarPattern, ConstPattern, TuplePattern, EnumPattern, TypePattern, WildcardPattern, InvalidPattern。
- **Type**：RefType, QualifiedType, OptionType, ConstantType, VArrayType, PrimitiveType, ParenType, FuncType, TupleType, ThisType, InvalidType。
- **Expr**：CallExpr, ParenExpr, MemberAccess, RefExpr, OptionalExpr, LitConstExpr, AssignExpr, UnaryExpr, BinaryExpr, SubscriptExpr, RangeExpr, ArrayLit, MatchExpr, Block, IfExpr, TryExpr, WhileExpr, LambdaExpr, ForInExpr, ThrowExpr, SpawnExpr, SynchronizedExpr, MacroExpandExpr 等。
- **辅助**：Generic, GenericConstraint, MatchCase, FuncArg, FuncParamList, ClassBody/StructBody/InterfaceBody, ImportSpec, File, Package。

### 1.3 设计要点

- **统一基类 + ASTKind**：所有节点继承 Node，带 `astKind`，便于类型判断和访问。
- **显式 ExprKind 上下文**：`ParseExpr(ExprKind ek)`，用于区分 if 条件、元组内、数组内等，避免歧义。
- **Decl/Expr/Type/Pattern 分离**：四种顶层抽象清晰，对应不同解析入口。

---

## 二、cjc Parse 结构（third_party/cangjie_compiler）

### 2.1 模块拆分

| 文件 | 职责 |
|------|------|
| `Parser.cpp` / `ParserImpl.h` | 对外 Parser API、ParserImpl 状态（lookahead/lastToken、Lexer）、Peek/Next/Skip/Seeing |
| `ParserImpl.cpp` | ParseTopLevel、ParseDecl/ParseExpr/ParseType/ParsePattern 入口及协调 |
| `ParseDecl.cpp` | 所有声明：class/interface/struct/enum/extend、func/macro、var/prop、主构造、const、finalizer 等；`declHandlerMap` 按 TokenKind 分发 |
| `ParseExpr.cpp` | 表达式主流程：优先级、二元/一元、组合赋值、SeeingExprOperator、ParseLeftParenExpr 等 |
| `ParseAtom.cpp` | 原子表达式：字面量、标识符、括号表达式、数组字面量、if/match/try/for/while/spawn/synchronized 等；`exprHandlerMap` 按 TokenKind 分发 |
| `ParseType.cpp` | ParseBaseType、ParseQualifiedType、ParseTypeWithParen、VArray、函数类型等 |
| `ParsePattern.cpp` | 模式解析（变量、常量、元组、枚举、类型、通配符等） |
| `ParseModifiers.cpp` | 修饰符解析与校验 |
| `ParseImports.cpp` | import/package |
| `ParseAnnotations.cpp` | 注解 |
| `ParseMacro.cpp` | 宏相关 |
| `ParseQuote.cpp` | quote 相关 |
| `ParserUtils.cpp` | SeeingLiteral、SeeingPrimitiveTypeAndLParen 等工具 |
| `ParserDiag.cpp` | 所有解析期诊断（期望 token、非法修饰符等） |

### 2.2 设计要点

- **Token → 处理函数映射**：`exprHandlerMap`、`declHandlerMap` 用 TokenKind 直接派发到对应 ParseXxx，避免巨型 match。
- **ExprKind 约束**：不同上下文（如 if 条件、元组元素、数组元素）用 ExprKind 限制可接受的表达式，减少歧义与特殊分支。
- **SeeingXxx 谓词**：如 `SeeingLiteral()`、`SeeingPrimaryConstructor()`，把“是否可解析为 X”集中成小函数，主流程更清晰。
- **诊断集中**：ParserDiag 统一管理错误信息与修复建议，便于维护和 i18n。

---

## 三、cjwasm 当前结构

### 3.1 src/ast/mod.rs

- **单文件**：Type、Expr、Stmt、Pattern、Literal、AssignTarget、各 Def（Struct/Class/Interface/Enum/Function/Import/Program）等均在同一文件。
- **无显式 AST 种类枚举**：依赖 Rust 的 enum 变体做“隐式 ASTKind”。
- **类型与 cjc 大体对齐**：有 Struct/Class/Interface/Enum/Function/Param/MatchArm 等，足够表达当前语法。

### 3.2 src/parser/mod.rs

- **单文件巨型解析器**：约 7600+ 行，包含：
  - 顶层：package、import、各类 Decl（struct/class/interface/enum/extend/function/main/const）。
  - 类型：parse_type、类型参数、where 子句。
  - 表达式：parse_expr → parse_primary、parse_binary、后缀、lambda、块、括号、元组等。
  - 语句：parse_stmt、parse_stmts。
  - 模式：match/pattern 相关。
- **无 ExprKind 式上下文**：同一 parse_expr 用于所有表达式上下文，歧义通过 peek_next/peek_at(2) 等临时解决。
- **错误信息**：直接 inline 在解析逻辑中（如 "var、let、init、~init 或 func"）。

### 3.3 src/codegen/mod.rs

- **单文件**：约 12000+ 行，从 Program 到 WASM 的翻译、函数/类/方法/表达式/控制流等都在同一模块。

---

## 四、重构建议（在不改 third_party 的前提下）

### 4.1 AST（src/ast）

- **可选：引入轻量 AST 种类枚举**  
  - 若需要“按种类分发”的通用逻辑（遍历、打印、序列化），可增加 `enum AstKind { Decl, Expr, Type, Pattern, ... }` 或 per-node 的 kind，便于后续扩展。  
  - 不必完全照搬 cjc 的 ASTKind.inc，可按 cjwasm 实际节点逐步加。
- **按概念拆文件（推荐）**  
  - `ast/type.rs`：Type。  
  - `ast/expr.rs`：Expr、Literal、MatchArm。  
  - `ast/pattern.rs`：Pattern。  
  - `ast/stmt.rs`：Stmt、AssignTarget。  
  - `ast/decl.rs`：各 *Def、Param、FieldDef、Visibility、Import。  
  - `ast/mod.rs`：Program、re-exports、共用类型。  
  - 这样与 cjc 的 Decl/Expr/Type/Pattern 分层更接近，后续加 Walker/Visitor 也更自然。

### 4.2 Parser（src/parser）

- **按职责拆文件（强烈建议）**  
  - `parser/mod.rs`：Parser 结构体、tokens/pos/pushback、peek/advance/expect/check、错误构造。  
  - `parser/decl.rs`：parse_program 顶层循环、parse_struct、parse_class、parse_interface、parse_enum、parse_function、parse_main、parse_const、extend；可进一步把“类体/结构体体”拆成子模块。  
  - `parser/expr.rs`：parse_expr、parse_primary、parse_binary、后缀、lambda、块、括号与元组。  
  - `parser/type_.rs`：parse_type、类型参数、where。  
  - `parser/stmt.rs`：parse_stmt、parse_stmts。  
  - `parser/pattern.rs`：match 与 pattern 解析。  
  - `parser/diag.rs` 或 `parser/error.rs`：统一 ParseError/ParseErrorAt 与“期望 xxx”等文案，便于后续多语言或统一风格。
- **引入“表达式上下文”**  
  - 定义 `enum ExprContext { General, InTuple, InBlock, InIfCond, InForIter, ... }`，在 parse_expr 或 parse_primary 传入。  
  - 在歧义处（如 `{` 是块还是 lambda、`(` 是元组还是调用）根据 ExprContext 决定行为，减少 ad-hoc 的 peek_at(2) 分支。
- **可选：TokenKind → 解析函数表**  
  - 对“当前 token 决定解析入口”的路径（如顶层 Decl、原子 Expr），可用 `HashMap<Token, fn(&mut Parser) -> Result<...>>` 或 match + 小函数，把大 match 拆成多个小函数，逻辑更接近 cjc 的 exprHandlerMap/declHandlerMap。

### 4.3 CodeGen（src/codegen）

- **按阶段/层次拆文件**  
  - `codegen/mod.rs`：CodeGen 状态、模块/函数表/类型表入口、emit 总控。  
  - `codegen/decl.rs`：从 Program/Struct/Class/Interface/Enum 到 WASM 的声明与 vtable 等。  
  - `codegen/expr.rs`：表达式到 WASM 的翻译（可再拆为 literal、call、method、binary、unary、control_flow 等）。  
  - `codegen/type_.rs`：类型布局、字段偏移、签名到 wasm 类型。  
  - 若存在“线性 IR 或中间表示”，可单独 `codegen/ir.rs` 或 `codegen/basic_block.rs`，再映射到 WASM。
- **与 cjc 的对应**  
  - cjc 有 IRGenerator、EmitFunctionIR、EmitExpressionIR、EmitBasicBlockIR、各类 *Impl（InvokeImpl、ArithmeticOpImpl 等）。  
  - cjwasm 可保持“直接生成 WASM”，但把“如何生成某类节点”按 Decl/Expr/Type 拆到不同子模块，便于维护和测试。

### 4.4 优先级建议

1. **先拆 parser**：单文件 7600+ 行最影响可读与排错；按 decl/expr/type/stmt/pattern 拆分即可带来明显收益。  
2. **再拆 codegen**：按 decl/expr/type 分离，方便后续优化或换后端。  
3. **最后整理 ast**：按 type/expr/pattern/stmt/decl 拆文件并视需要加 AstKind。

---

## 五、可直接借鉴的 cjc 点

- **ParseDecl 入口**：先 ParseAnnotations、ParseModifiers，再用 `declHandlerMap` 按 token 分发；cjwasm 顶层循环可类似“先属性/修饰符，再按 token 分派”。  
- **ParseAtom + exprHandlerMap**：原子表达式按 TokenKind 查表调用对应 ParseXxxExpr，避免一个巨大 match。  
- **ParseType 独立**：ParseBaseType → 标识符/括号/基本类型/This/VArray/…，与 cjwasm 的 parse_type 可一一对照，类型相关都放进 `parser/type_.rs`。  
- **SeeingXxx 谓词**：把“是否可解析为 lambda/主构造/字面量”等抽成 `fn seeing_xxx(&self) -> bool`，主流程更短、歧义处理更集中。  
- **诊断集中**：所有“期望 xxx”“非法 xxx”放到 parser/diag 或 error 模块，便于统一修改和翻译。

---

## 六、小结

| 维度 | cjc | cjwasm 现状 | 建议 |
|------|-----|-------------|------|
| AST | 多文件 + ASTKind.inc + Node 基类 | 单文件，enum 即 kind | 按 type/expr/pattern/stmt/decl 拆文件；可选 AstKind |
| Parser | ParseDecl/Expr/Type/Pattern 分文件 + 表驱动 | 单文件 7600+ 行 | 拆 decl/expr/type/stmt/pattern + 集中 diag；引入 ExprContext；可选 token→fn 表 |
| CodeGen | IRGenerator + Emit* 分文件 | 单文件 12000+ 行 | 拆 mod/decl/expr/type，必要时加 ir 层 |

参考 `third_party/cangjie_compiler/src/AST` 与 `third_party/cangjie_compiler/src/Parse` 的**模块划分与入口设计**（而非照搬 C++ 实现），对重构 `src/ast`、`src/parser`、`src/codegen` 会很有帮助；优先拆 parser，再 codegen，最后整理 ast，可显著提升可维护性和与官方实现的对照能力。
