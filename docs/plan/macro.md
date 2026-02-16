# cjwasm 宏系统实现计划

> 文档版本: 2026-02-16
> 路线: WASM 宏执行（推荐方案）

## 1. 背景与目标

cjc release/1.0 的宏系统（`macro func` / `quote(...)` / `@MacroName[...]`）是 cjwasm 与 cjc
差异中最大的缺失特性之一。cjwasm 当前仅通过编译器内建方式支持 `@Assert` / `@Expect` 两个
"伪宏"，无法支持用户自定义宏。

### 1.1 目标

- 支持仓颉语言的 `macro func` 声明和 `quote(...)` 表达式
- 宏函数在编译期执行，输出的 AST 片段替换宏调用处
- 利用 cjwasm 自身的 WASM 目标特性，形成"编译宏→WASM→编译期执行"的自举闭环
- 兼容 cjc 的 `@Assert` / `@Expect` / `@Test` 等标准宏语法

### 1.2 非目标（首期不实现）

- `@When` 条件编译（依赖完整注解系统，独立规划）
- `@PowerAssert` 表达式树可视化（高级特性，后续扩展）
- 宏的增量编译缓存

## 2. 架构设计

### 2.1 核心思路：WASM 宏执行

```
                        ┌─────────────────────────────┐
                        │     cjwasm 编译管线          │
                        │                             │
  .cj 源码 ──→ Lexer ──→ Parser ──→ AST             │
                        │   │                         │
                        │   │ 发现 macro func 声明     │
                        │   ▼                         │
                        │ ┌────────────────────┐      │
                        │ │ Phase 1: 宏编译     │      │
                        │ │ macro func → AST    │      │
                        │ │ AST → WASM 字节码   │      │
                        │ │ (复用 cjwasm codegen)│      │
                        │ └────────┬───────────┘      │
                        │          │ macro.wasm        │
                        │          ▼                   │
                        │ ┌────────────────────┐      │
                        │ │ Phase 2: 宏展开     │      │
                        │ │ 遍历 AST 找宏调用    │      │
                        │ │ 序列化参数 → JSON     │      │
                        │ │ wasmtime 执行宏 WASM  │      │
                        │ │ 反序列化结果 → AST    │      │
                        │ │ 替换宏调用处          │      │
                        │ └────────┬───────────┘      │
                        │          ▼                   │
                        │  展开后 AST ──→ Optimizer     │
                        │               ──→ Monomorph   │
                        │               ──→ CodeGen     │
                        │               ──→ .wasm      │
                        └─────────────────────────────┘
```

### 2.2 与 cjc 宏系统对比

| 维度 | cjc | cjwasm (本方案) |
|------|-----|----------------|
| 宏编译目标 | LLVM IR → 原生动态库 | cjwasm codegen → WASM 模块 |
| 宏执行方式 | 链接动态库，直接调用 | wasmtime 沙箱执行 |
| AST 交换格式 | 内存中 C++ AST 对象 | JSON 序列化的 AST 节点 |
| 安全性 | 宏可执行任意原生代码 | WASM 沙箱隔离，宏无法访问宿主 |
| 启动开销 | 低（动态链接） | 中（WASM 实例化 ~1ms） |

### 2.3 AST 交换协议

宏函数通过标准化的 JSON 格式接收和返回 AST 节点：

```json
{
  "kind": "Call",
  "name": "println",
  "args": [
    {
      "kind": "StringInterp",
      "parts": [
        { "kind": "Literal", "value": "Assert failed: " },
        { "kind": "Expr", "expr": { "kind": "Var", "name": "a" } }
      ]
    }
  ]
}
```

cjwasm 的 Rust AST（`src/ast/mod.rs` 中的 `Expr`/`Stmt`/`Type`）需要实现
`Serialize` / `Deserialize`（通过 serde），作为宏通信的桥梁。

## 3. 分阶段实施计划

### Phase M1: AST 序列化基础（1 周）

**目标**：为 cjwasm 的 AST 添加 JSON 序列化能力

**任务**：

- [ ] 为 `src/ast/mod.rs` 中所有类型添加 `#[derive(Serialize, Deserialize)]`
  - `Type`, `Expr`, `Stmt`, `BinOp`, `UnaryOp`, `Pattern`, `MatchArm`
  - `FuncDef`, `StructDef`, `EnumDef`, `ClassDef`, `InitDef`, `FieldDef`
  - `Param`, `AssignTarget`, `Visibility`
- [ ] 添加 `serde` + `serde_json` 依赖到 `Cargo.toml`
- [ ] 编写 AST 序列化/反序列化单元测试
- [ ] 验证往返一致性：`AST → JSON → AST` 无损

**验收**：`cargo test` 通过，AST 序列化测试全部通过

### Phase M2: 宏声明解析（1 周）

**目标**：Parser 支持 `macro func` 语法

**语法**：

```cangjie
// 宏声明
public macro func myMacro(args: Tokens): Tokens {
    // 宏体
    return quote(
        println("expanded!")
    )
}

// 宏调用
@myMacro[some_arg]
```

**任务**：

- [ ] 在 `src/ast/mod.rs` 中添加新的 AST 节点：
  ```rust
  // 宏函数声明
  pub struct MacroDef {
      pub name: String,
      pub params: Vec<Param>,
      pub body: Vec<Stmt>,
  }

  // quote 表达式 — 编译时构造 AST 片段
  // quote(expr) → 将 expr 转为 AST 节点值
  Expr::Quote(Box<Expr>),

  // 宏调用（@ 符号触发）
  Expr::MacroCall {
      name: String,
      args: Vec<Expr>,    // token 参数
  },
  ```
- [ ] 在 `src/parser/mod.rs` 中：
  - 解析 `macro func name(params): Tokens { body }` → `MacroDef`
  - 解析 `quote(expr)` → `Expr::Quote`
  - 扩展 `@` 符号解析：除 `@Assert`/`@Expect` 外，支持 `@UserMacro[...]`
- [ ] 单元测试：验证宏声明和调用的 AST 正确性

**验收**：含宏声明的 `.cj` 文件能正确解析为 AST

### Phase M3: 宏编译到 WASM（2 周）

**目标**：将 `macro func` 编译为独立 WASM 模块

**任务**：

- [ ] 在 `src/codegen/mod.rs` 中实现宏编译流程：
  1. 从 AST 中提取所有 `MacroDef`
  2. 将每个宏函数转换为普通 `FuncDef`（签名变为 `(i32, i32) -> (i32, i32)`，接收 JSON 指针+长度，返回 JSON 指针+长度）
  3. 注入 `std.ast` 桩函数（见 Phase M4）
  4. 调用现有 codegen 管线编译为 `.wasm`
- [ ] 宏 WASM 模块的导出接口约定：
  ```
  导出函数: macro_<name>(json_ptr: i32, json_len: i32) -> (result_ptr: i32, result_len: i32)
  导入函数: __alloc(size: i32) -> i32     // 内存分配
  导入内存: memory (1 page min)
  ```
- [ ] 实现 `quote(expr)` 编译：
  - 将 `quote` 内的表达式转为 AST JSON 字符串常量
  - 编译为写入 JSON 到线性内存的指令序列
- [ ] 单元测试：简单宏函数能编译为合法 WASM

**验收**：`macro func identity(args: Tokens): Tokens { return args }` 编译为合法 WASM 模块

### Phase M4: std.ast 桩库（2 周）

**目标**：为宏提供 AST 操作 API（仓颉侧接口）

**范围**：实现 cjc `std.ast` 的核心子集，足以编写常见宏

```cangjie
// std.ast 核心 API（cjwasm 子集）
package std.ast

// Token 流类型
public class Tokens {
    public func toList(): ArrayList<Token>
    public func fromExprs(exprs: ArrayList<Expr>): Tokens
}

// AST 节点类型
public open class ASTNode { }
public class Expr <: ASTNode {
    public static func intLiteral(v: Int64): Expr
    public static func stringLiteral(v: String): Expr
    public static func call(name: String, args: ArrayList<Expr>): Expr
    public static func binary(op: String, left: Expr, right: Expr): Expr
    public static func varRef(name: String): Expr
}
public class Stmt <: ASTNode {
    public static func exprStmt(e: Expr): Stmt
    public static func letDecl(name: String, value: Expr): Stmt
    public static func returnStmt(value: Expr): Stmt
}
```

**任务**：

- [ ] 在 cjwasm 中实现 `std.ast` 的内建映射：
  - 这些类型在宏 WASM 模块中以 JSON 操作实现
  - `Tokens` → JSON 数组
  - `Expr` / `Stmt` → JSON 对象（与 Phase M1 格式一致）
- [ ] 编译器识别 `import std.ast.*` 并注入桩函数
- [ ] 实现 `quote(...)` 到 JSON 构建代码的编译
- [ ] 单元测试：`std.ast` API 可在宏中使用

**验收**：宏函数中可以使用 `Expr.call(...)` 等 API 构造 AST

### Phase M5: 宏展开引擎（2 周）

**目标**：编译期执行宏 WASM 模块，展开宏调用

**任务**：

- [ ] 添加 `wasmtime` 依赖到 `Cargo.toml`（已通过用户 wasmtime CLI 验证可用）
- [ ] 实现宏展开管线 `src/macro_expand/mod.rs`：
  ```rust
  pub struct MacroExpander {
      macro_modules: HashMap<String, Vec<u8>>,  // name → WASM bytes
  }

  impl MacroExpander {
      /// 编译所有 macro func 声明为 WASM 模块
      pub fn compile_macros(macros: &[MacroDef], codegen: &CodeGen) -> Self;

      /// 遍历 AST，展开所有宏调用
      pub fn expand(&self, stmts: &mut Vec<Stmt>) -> Result<(), MacroError>;

      /// 执行单个宏：序列化参数 → wasmtime 执行 → 反序列化结果
      fn invoke_macro(&self, name: &str, args: &[Expr]) -> Result<Vec<Stmt>, MacroError>;
  }
  ```
- [ ] 集成到主编译管线（`src/main.rs`）：
  ```
  parse → extract_macros → compile_macros → expand_all → optimize → monomorph → codegen
  ```
- [ ] 实现错误处理：
  - 宏编译失败：报告宏函数中的编译错误
  - 宏执行失败：wasmtime trap → 报告宏运行时错误
  - 宏输出无效：JSON 反序列化失败 → 报告格式错误
- [ ] 集成测试：端到端宏展开

**验收**：包含宏定义和宏调用的 `.cj` 文件能正确编译运行

### Phase M6: 内建宏迁移与标准宏（1 周）

**目标**：将现有 `@Assert` / `@Expect` 迁移为标准宏实现，新增 `@Test`

**任务**：

- [ ] 用新宏系统重实现 `@Assert(a, b)`：
  ```cangjie
  public macro func Assert(args: Tokens): Tokens {
      let exprs = args.toList()
      let left = exprs[0]
      let right = exprs[1]
      return quote(
          if ($(left) != $(right)) {
              println("Assert failed at line ${__LINE__}")
              abort()
          }
      )
  }
  ```
- [ ] 保留编译器内建 `@Assert`/`@Expect` 作为回退（无宏系统时使用）
- [ ] 实现 `@Test` 宏（标记测试函数，收集到测试入口）
- [ ] 编写示例文件 `examples/macro_basic.cj`

**验收**：现有测试全部通过（宏版 @Assert 与内建版行为一致），`@Test` 可用

## 4. 技术细节

### 4.1 编译管线变更

当前管线：
```
source → Lexer → Parser → AST → Optimizer → Monomorphizer → CodeGen → .wasm
```

新管线：
```
source → Lexer → Parser → AST
                           │
                    ┌──────┴──────┐
                    │ 提取 MacroDef │
                    │ 编译为 WASM   │
                    └──────┬──────┘
                           │
                    ┌──────┴──────┐
                    │ 展开宏调用    │
                    │ wasmtime 执行 │
                    │ AST 替换      │
                    └──────┬──────┘
                           │
                    展开后 AST → Optimizer → Monomorphizer → CodeGen → .wasm
```

### 4.2 依赖变更

```toml
# Cargo.toml 新增依赖
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
wasmtime = "19"     # 宏编译期执行引擎
```

### 4.3 文件结构变更

```
src/
├── ast/mod.rs              # +Serialize/Deserialize derive
├── parser/mod.rs           # +MacroDef/Quote/MacroCall 解析
├── codegen/mod.rs          # +宏函数编译为独立 WASM
├── macro_expand/           # 新模块
│   ├── mod.rs              # MacroExpander 主逻辑
│   ├── ast_json.rs         # AST ↔ JSON 序列化
│   └── runtime.rs          # wasmtime 宏执行运行时
├── monomorph/mod.rs        # +MacroDef/Quote 遍历
├── optimizer/mod.rs        # +MacroDef/Quote 遍历
└── main.rs                 # +宏编译与展开阶段
```

### 4.4 quote 展开语法

`quote(...)` 内部支持 `$(expr)` 插值，将仓颉表达式的值拼接到生成的 AST 中：

```cangjie
macro func double(args: Tokens): Tokens {
    let x = args.toList()[0]
    return quote(
        $(x) + $(x)    // $() 引用外部变量，拼接到输出 AST
    )
}

// @double[5] → 展开为 5 + 5
```

编译策略：
- `quote(expr)` 中非 `$()` 部分：编译为 AST JSON 字面量
- `$(var)` 部分：编译为运行时拼接 JSON 节点的代码

### 4.5 安全模型

宏在 WASM 沙箱中执行，与宿主编译器完全隔离：

| 能力 | 状态 |
|------|------|
| 读写文件 | ❌ 沙箱隔离 |
| 网络访问 | ❌ 沙箱隔离 |
| 访问编译器状态 | ❌ 仅通过 JSON 通信 |
| 无限循环 | ⚠️ 通过 wasmtime fuel/timeout 限制 |
| 内存耗尽 | ⚠️ 通过 wasmtime memory limit 限制 |
| 返回恶意 AST | ⚠️ 反序列化时校验合法性 |

## 5. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| wasmtime 依赖增加编译时间 | 中 | 宏特性作为 Cargo feature gate，默认关闭 |
| JSON 序列化性能瓶颈 | 低 | 小型 AST 序列化 <1ms；大规模可改用 MessagePack |
| quote 内嵌套宏展开 | 高 | 首期不支持嵌套宏，Phase M6 后可迭代 |
| std.ast API 与 cjc 不完全兼容 | 中 | 明确标注为 cjwasm 子集，逐步对齐 |
| 宏 WASM 模块编译自身依赖宏 | 高 | 首期禁止宏相互引用，后续支持拓扑排序多轮编译 |

## 6. 里程碑时间线

| 阶段 | 内容 | 预估工时 | 依赖 |
|------|------|:--------:|------|
| **M1** | AST 序列化 | 1 周 | 无 |
| **M2** | 宏声明解析 | 1 周 | M1 |
| **M3** | 宏编译到 WASM | 2 周 | M2 |
| **M4** | std.ast 桩库 | 2 周 | M3 |
| **M5** | 宏展开引擎 | 2 周 | M3 + M4 |
| **M6** | 内建宏迁移 + @Test | 1 周 | M5 |
| **总计** | | **9 周** | |

## 7. 示例：最终效果

```cangjie
import std.ast.*

// 定义一个日志宏，自动附加文件名和行号
public macro func Log(args: Tokens): Tokens {
    let msg = args.toList()[0]
    return quote(
        println("[LOG] " + $(msg))
    )
}

// 使用宏
main() {
    @Log["Hello from macro!"]
    // 展开为: println("[LOG] " + "Hello from macro!")

    @Assert(1 + 1, 2)
    // 展开为: if (1 + 1 != 2) { println("Assert failed..."); abort() }

    println("PASS")
}
```

---

*文档版本: 1.0.0*
*创建日期: 2026-02-16*
*状态: 规划中*
