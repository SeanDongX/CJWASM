# examples/std — L1 标准库测试目录

本目录用于验证 **L1 模块**（纯 Cangjie、Vendor 优先）的解析与构建，并包含各命名空间的 **全量 API 测试**。

## L1 模块列表

- `std.io`, `std.binary`, `std.console`
- `std.overflow`, `std.crypto`, `std.deriving`
- `std.ast`, `std.argopt`, `std.sort`, `std.ref`, `std.unicode`

## 全量 API 测试文件

| 命名空间 | 文件 | 覆盖 API（见文件内注释） |
|----------|------|---------------------------|
| std.io | `src/api_std_io.cj` | IOException, InputStream/OutputStream, ByteBuffer, SeekPosition, readString, readToEnd, copy |
| std.binary | `src/api_std_binary.cj` | BigEndianOrder, LittleEndianOrder, SwapEndianOrder |
| std.console | `src/api_std_console.cj` | Console, ConsoleWriter, ConsoleReader |
| std.overflow | `src/api_std_overflow.cj` | WrappingOp, ThrowingOp, SaturatingOp, CheckedOp, CarryingOp, OvershiftException |
| std.crypto | `src/api_std_crypto.cj` | std.crypto + std.crypto.digest + std.crypto.cipher |
| std.deriving | `src/api_std_deriving.cj` | 宏 Derive, DeriveInclude, DeriveExclude, DeriveOrder |
| std.ast | `src/api_std_ast.cj` | Token, TokenKind, Position, TypeNode/Pattern 子类, parseType, parseProgram, parseExpr |
| std.argopt | `src/api_std_argopt.cj` | ArgOpt, parseArguments, ArgumentSpec, ParsedArguments |
| std.sort | `src/api_std_sort.cj` | sort, stableSort, unstableSort, SortExtension |
| std.ref | `src/api_std_ref.cj` | WeakRef, CleanupPolicy |
| std.unicode | `src/api_std_unicode.cj` | UnicodeRuneExtension, UnicodeStringExtension, CasingOption |

入口 `src/main.cj` 导入上述 11 个 L1 模块并调用各 `api_std_*_used()`，用于验证解析与后续编译。

## 构建

在 **仓库根目录** 执行（以便找到 `third_party/cangjie_runtime/std/libs/std`）：

```bash
cjwasm build -p examples/std
```

或进入本目录并设置 vendor 路径后构建：

```bash
cd examples/std
CJWASM_STD_PATH=../../third_party/cangjie_runtime/std/libs/std cjwasm build
```

## 说明

- 解析阶段会从 vendor 拉取对应 L1 包下的全部 `.cj` 文件（L1 解析已实现）。lexer 已支持反引号、块注释预处理。
- 部分 vendor 代码使用 `@Intrinsic`、class-level `where`、复杂 `extend`、**struct 主构造函数**等，若编译报错可预期，需后续 parser 增强。

### 已修复（parser/lexer）

- **struct 主构造函数**：struct 体内 `StructName(var a: T, ...) {}` 已支持，参数作为字段。
- **顶层 let/var/const**：`Program.constants` + `parse_top_level_const`，支持可选类型 `name [: Type] = expr`。
- **十六进制 0X**：lexer 支持 `0[xX]...`。
- **类型中 >>**：parser 在 expect(Gt) 时将 Shr 视为 `> >`（pushback）。
- **enum 内 operator func**：`parsing_operator_func` + 运算符名解析。
- **枚举变体多类型/多表达式**：变体 `V(T1, T2)` 解析为 `Type::Tuple`；`V(e1, e2)` 解析为 `Expr::Tuple`。
- **类型转换多参数**：`T(e1, e2, ...)` 使用 `parse_args()`，并支持 TypeRune。

### 当前编译失败原因（待查）

- **出错文件**：`third_party/cangjie_runtime/std/libs/std/unicode/unicode_extension.cj`
- **错误**：`语法错误: 意外的 token: Comma, 期望: RParen (字节偏移 101004-101005)`
- **位置**：约在 `SPECIAL_UNICODE_MAP` 数组字面量内，行 2571 附近 `([0x1FB7], [0x0391, 0x0342, 0x0345], ...)`。
- **可能原因**：某一处仍按「单参数后即 RParen」解析（如 `Some`/`Ok`/`Err` 或其它构造），遇到逗号报错；需在 parser 中定位该 expect(RParen) 并改为支持多参数或元组。
