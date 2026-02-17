# cjwasm 宏系统第二阶段：支持 CJson 编译的完整计划

> 文档版本: 2026-02-16
> 前置: [宏系统第一阶段 (M1-M6)](macro.md) 已完成
> 目标项目: [CJson](https://gitcode.com/Cangjie-TPC/CJson.git) cjc_1.1.0 分支

## 现状分析

当前 cjwasm 宏系统（M1-M6）已实现基础框架：`macro func` 声明、`quote(...)` 表达式、WASM 沙箱执行、内建宏。但 CJson 使用的是 **cjc 生产级宏系统**，差距体现在以下 4 个维度：

### 差距总览

```
                    ┌─────────────────────────────────────┐
                    │         CJson 编译差距                │
                    ├─────────┬──────────┬────────┬───────┤
                    │ 宏包体系 │ std.ast  │ 语言   │ 标准库 │
                    │         │  API     │ 特性   │       │
                    ├─────────┼──────────┼────────┼───────┤
                    │ macro   │ ClassDcl │ intern │ stdx. │
                    │ package │ StructDcl│ import │ json  │
                    │         │          │        │       │
                    │ $ 拼接   │ VarDecl  │ static │ std.  │
                    │         │ FuncDecl │ let/var│ time  │
                    │         │          │        │       │
                    │ 宏间通信 │ Visitor  │ 泛型   │ std.  │
                    │         │ 遍历     │ extend │convert│
                    │         │          │        │       │
                    │ Tokens  │ parseDecl│ 多类型 │       │
                    │ 拼接    │ 解析     │ catch  │       │
                    └─────────┴──────────┴────────┴───────┘
```

---

## Phase C1: 语言特性补全 (2 周)

补全 CJson 依赖但 cjwasm 缺失的基础语言特性。

### C1.1 `internal import` 支持

- **文件**: `src/ast/mod.rs` -- 为 `Import` 添加 `visibility: Visibility` 字段
- **文件**: `src/parser/mod.rs` -- 修改 import 解析，在 `Token::Import` 前检查 `Token::Internal`/`Token::Public` 等可见性修饰符

### C1.2 `static let` / `static var` 字段

- **文件**: `src/ast/mod.rs` -- 为 `FieldDef` 添加 `is_static: bool`
- **文件**: `src/parser/mod.rs` -- 在类体解析中，`static` 后允许 `let`/`var`
- **文件**: `src/codegen/mod.rs` -- 静态字段存储在全局区域（WASM global 或 data segment）

### C1.3 泛型 `extend`

- **文件**: `src/ast/mod.rs` -- 为 `ExtendDef` 添加 `type_params: Vec<String>` 和 `constraints: Vec<TypeConstraint>`
- **文件**: `src/parser/mod.rs` -- 修改 `parse_extend` 支持 `extend<T> Foo<T> <: Bar where T <: Baz`

### C1.4 多类型 catch

- 支持 `catch(e: TypeA | TypeB)` 语法，parser 中在 catch 类型处解析 `|` 分隔的多个类型

---

## Phase C2: macro package 与宏声明增强 (2 周)

### C2.1 `macro package` 声明

- **文件**: `src/parser/mod.rs` -- 在 `parse_program` 中，如果遇到 `Token::Macro` 后跟 `Token::Package`，解析为宏包声明
- **文件**: `src/ast/mod.rs` -- `Program` 添加 `is_macro_package: bool` 字段
- 宏包中的所有函数默认视为宏函数上下文

### C2.2 顶层 `macro` 函数（无 `func` 关键字）

CJson 使用的语法是：

```cangjie
public macro JsonSerializable(input_Tk: Tokens): Tokens { ... }
```

注意这里没有 `func`，是 `macro Name(...)` 而非 `macro func Name(...)`。

- **文件**: `src/parser/mod.rs` -- `parse_macro_def` 应同时支持 `macro func Name` 和 `macro Name`（无 `func`）

### C2.3 `Tokens` 参数和返回类型

当前宏参数类型为 `String`，CJson 使用 `Tokens`：

```cangjie
public macro JsonSerializable(input_Tk: Tokens): Tokens
```

- **文件**: `src/ast/mod.rs` -- `Type` 枚举中添加 `Tokens` 类型
- 在宏上下文中，`Tokens` 表示 token 流的编译期类型

---

## Phase C3: `$` 拼接与 quote 增强 (3 周)

这是实现 CJson 宏的核心难点。CJson 大量使用：

```cangjie
quote($classModifier $classKeyWord $classIdent $superTypeExpr { $classBody $fromJsonFunc $toJsonFunc })
quote(map.add($key, this.$(varInfo.identifier).toJsonValue()))
```

### C3.1 `$variable` 基本拼接

- **文件**: `src/lexer/mod.rs` -- 在 quote 上下文中，`$` 开头的标识符词法化为 `Token::Splice(name)`
- **文件**: `src/parser/mod.rs` -- quote 体内解析 `$name` 为 `Expr::Splice { expr: Expr::Var("name") }`
- **文件**: `src/ast/mod.rs` -- 添加 `Expr::Splice { expr: Box<Expr> }` 节点

### C3.2 `$(expr)` 表达式拼接

CJson 使用 `$(composeMap(var_Tk_List))` 和 `$(varInfo.identifier)` 形式：

- `$(func_call(...))` -- 调用宏上下文中的函数，结果拼接为 Tokens
- `$(obj.field)` -- 访问对象属性，结果拼接为 Tokens
- **文件**: `src/parser/mod.rs` -- quote 体内解析 `$(...)` 括号中为任意表达式

### C3.3 quote 中的类型表达式

CJson 使用 `quote(<: $superTypes & IJsonSerializable<$classIdent>)` 这种类型级 quote：

- 需要 quote 不仅能包含语句，还能包含类型表达式和声明片段
- **文件**: `src/ast/mod.rs` -- `Expr::Quote` 扩展为支持 `QuoteContent::Stmts(Vec<Stmt>)` | `QuoteContent::TypeExpr(...)` | `QuoteContent::Decls(...)`

### C3.4 Tokens 运算

CJson 使用 `fieldMapToken = fieldMapToken + addFieldToMap(var_Tk)` 进行 Tokens 拼接：

- 宏上下文中 `Tokens` 类型支持 `+` 运算符
- 需要在宏解释器/运行时中实现 Tokens 的连接操作

---

## Phase C4: std.ast API 实现 (4 周)

这是工作量最大的阶段。CJson 使用了以下 std.ast 类型和方法：

### C4.1 核心类型定义

在宏编译期运行时中实现以下类型（作为 Rust 侧内建实现或 WASM 桩库）：

| 类型 | 关键属性/方法 | CJson 使用场景 |
|------|-------------|---------------|
| `Tokens` | `toString()`, `+`运算, `toList()` | 所有宏文件 |
| `Token` | `.value`, `Token(kind, val)` 构造, `TokenKind` | ClassJsonSerilizer, Extension |
| `TokenKind` | `IDENTIFIER`, `STRING_LITERAL` 等常量 | ClassJsonSerilizer |
| `ClassDecl` | `.modifiers`, `.keyword`, `.identifier`, `.body.decls`, `.superTypes` | TokenVerifier, JsonSerializable |
| `StructDecl` | 同 ClassDecl | TokenVerifier |
| `VarDecl` | `.identifier`, `.expr`, `.typeName`, `.toTokens()` | ClassVarDeclVisitor, ClassProcessor |
| `FuncDecl` | 用于 Visitor 遍历 | ClassVarDeclVisitor |
| `Node` | `.traverse(Visitor)` | JsonSerializable |
| `Visitor` | `visit(VarDecl)`, `visit(FuncDecl)`, `breakTraverse()` | ClassVarDeclVisitor |
| `Modifier` | 修饰符列表 | ClassAndStructInfo |
| `Decl` | 声明基类 | ClassAndStructInfo |
| `TypeNode` | `.toTokens()` | Extension, VarInfo |
| `RefType`, `PrimitiveType`, `PrefixType` | 类型节点子类 | Extension |
| `Expr` (std.ast) | AST 表达式节点 | GlobalConfig, DefaultValueExprStore |
| `parseDecl()` | 从 Tokens 解析为声明 | TokenVerifier |

- **新文件**: `src/macro_expand/std_ast.rs` -- std.ast 类型的 Rust 实现
- **新文件**: `src/macro_expand/std_ast_visitor.rs` -- Visitor 模式实现

### C4.2 AST Visitor 模式

```rust
// src/macro_expand/std_ast_visitor.rs
pub trait AstVisitor {
    fn visit_var_decl(&mut self, decl: &VarDecl) -> bool { true }
    fn visit_func_decl(&mut self, decl: &FuncDecl) -> bool { true }
    fn break_traverse(&mut self);
}

pub fn traverse_node(node: &AstNode, visitor: &mut dyn AstVisitor) { ... }
```

### C4.3 宏间通信

CJson 使用 `getChildMessages(macroName)` 实现宏间数据传递（`@JsonCust` 传递配置到 `@JsonSerializable`）：

- **文件**: `src/macro_expand/mod.rs` -- `MacroExpander` 添加 `messages: HashMap<String, Vec<MacroMessage>>` 用于存储子宏消息
- 宏执行时可调用 `setMessage(key, value)` 和 `getChildMessages(macroName)` API

### C4.4 parseDecl / TokenVerifier

`parseDecl(tokens)` 将 token 流重新解析为 AST 声明节点：

- 复用现有 parser，但从 Tokens 对象（而非源文件）输入
- 返回 `ClassDecl` 或 `StructDecl` 供宏代码操作

---

## Phase C5: stdx.encoding.json 标准库 (2 周)

CJson 生成的代码（`toJson()`/`fromJson()`）依赖以下 JSON 类型：

### C5.1 JSON 值类型层次

```cangjie
interface JsonValue {
    func toString(): String
}
class JsonObject <: JsonValue { ... }
class JsonArray <: JsonValue { ... }
class JsonString <: JsonValue { ... }
class JsonInt <: JsonValue { ... }
class JsonFloat <: JsonValue { ... }
class JsonBool <: JsonValue { ... }
class JsonNull <: JsonValue { ... }
```

- 在 cjwasm 中作为内建标准库实现
- **新文件**: `stdlib/stdx/encoding/json.cj` 或在 codegen 中内建注册

### C5.2 IJsonSerializable 接口

```cangjie
interface IJsonSerializable<T> {
    func toJsonValue(): JsonObject
    func toJson(): String
    static func fromJson(jsonStr: String): T
}
```

### C5.3 `@OverflowWrapping` 注解

CJson 的 `IJsonSerializable.cj` 使用了 5 处 `@OverflowWrapping`：

- 作为编译器 hint，在 WASM 中可简化为 no-op（溢出行为已由 WASM 语义保证）

---

## Phase C6: 集成测试与端到端验证 (2 周)

### C6.1 分步编译测试

- 逐个编译 CJson 的 17 个 `macro package` 源文件
- 编译 `IJsonSerializable.cj` 和 example 文件
- 修复集成过程中发现的问题

### C6.2 端到端测试

- 编写简化版 CJson 用例（单文件版）
- 验证 `@JsonSerializable` 宏能正确生成 `toJson()`/`fromJson()` 方法
- 运行 CJson 的 `example/demo/main.cj`

### C6.3 回归测试

- 确保现有 38 个 system test 全部通过
- 确保现有 241+ unit test 全部通过

---

## 依赖关系

```
  C1 (语言特性) ──→ C2 (macro package) ──→ C3 ($ 拼接) ──→ C4 (std.ast) ──→ C5 (JSON 库) ──→ C6 (集成测试)
       │                                                                         ↑
       └─────────────────────────────────────────────────────────────────────────┘
```

## 工时估算

| 阶段 | 内容 | 预估工时 | 依赖 |
|------|------|:--------:|------|
| **C1** | 语言特性补全（internal import, static field, 泛型 extend, 多类型 catch） | 2 周 | 无 |
| **C2** | macro package + 宏声明增强 + Tokens 类型 | 2 周 | C1 |
| **C3** | $ 拼接、$(expr) 表达式拼接、quote 增强 | 3 周 | C2 |
| **C4** | std.ast API（ClassDecl/VarDecl/Visitor/parseDecl/宏间通信） | 4 周 | C3 |
| **C5** | stdx.encoding.json 标准库 + @OverflowWrapping | 2 周 | C1 + C4 |
| **C6** | 集成测试与端到端验证 | 2 周 | C5 |
| **总计** | | **15 周** | |

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| std.ast API 表面积巨大 | 高 | CJson 仅使用子集，优先实现 ~15 个类型和 ~30 个方法，非完整 cjc std.ast |
| 宏解释器能力不足 | 高 | CJson 宏逻辑复杂（循环、条件、方法调用），需增强 WASM 运行时或实现更完整的仓颉子集解释器 |
| Tokens 拼接语义 | 中 | cjc 的 Tokens 是有序 token 流可自由拼接，cjwasm 当前以 AST 节点为粒度，需桥接 token 流与 AST 两种表示 |
| quote 表达力不足 | 高 | CJson 的 quote 可包含完整类声明（含修饰符、关键字、body），远超当前 cjwasm quote 仅支持语句列表的能力 |
| 宏内可执行完整仓颉代码 | 高 | CJson 宏体中使用类继承、方法调用、异常处理等完整 OOP，需要在宏运行时支持这些语义 |

---

*文档版本: 1.0.0*
*创建日期: 2026-02-16*
*前置文档: [宏系统第一阶段](macro.md)*
*状态: 待实施*
