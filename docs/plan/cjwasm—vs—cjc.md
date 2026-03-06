# CJWasm2 vs cjc-based codegen：方案对比分析

## 背景

问题：能否利用官方 cjc 编译器作为前端（或后端），替代 CJWasm2 现有的自研方案来生成 WASM？

---

## cjc 的实际架构

```
Cangjie 源码
    → 前端（词法 / 语法 / 语义分析）
    → CHIR（Cangjie High-level IR，以 FlatBuffers 二进制序列化）
    → CodeGen → LLVM IR
    → llc → native 目标（ARM / x86 / HarmonyOS）
```

关键事实：**cjc 的 CodeGen 完全基于 LLVM，源码中没有任何 WASM 后端**。

```
third_party/cangjie_compiler/src/CodeGen/
  CGModule.cpp       ← #include "llvm/IR/..."
  EmitFunctionIR.cpp ← 发射 LLVM IR 指令
  EmitExpressionIR.cpp
  ...
```

---

## 三种可能的 cjc-based 路径

### 方案 A：cjc → LLVM IR → `llc -march=wasm32`

**流程：** 让 cjc 输出 LLVM bitcode，再用 LLVM 的 WASM 后端编译。

**根本障碍：Cangjie Runtime**

`third_party/cangjie_runtime/runtime/src/` 包含约 153 个 C++ 源文件，实现了：

- 垃圾回收（GC）
- 协程 / 线程调度
- 异常处理
- OS 系统调用封装

这些组件依赖 POSIX / 鸿蒙 OS 接口，**无法编译到裸 WASM**（WASM 沙箱禁止直接系统调用，WASI 也仅覆盖文件 I/O 等基础接口）。即使强行编译，产出体积极大且无法正常运行。

---

### 方案 B：读取 cjc 的 CHIR 序列化输出，自写 WASM codegen

**流程：** 调用 cjc 生成 `.chir` 二进制文件，在 Rust 中反序列化，再从 CHIR 生成 WASM。

**障碍：**

CHIR 以 **FlatBuffers 二进制**格式序列化（`flatbuffers/PackageFormat_generated.h`），需要：

1. 在 Rust 中完整实现 CHIR FlatBuffers schema 的反序列化
2. 处理 CHIR 中完整的类型系统（泛型、闭包、trait、枚举、class 等）
3. 将 CHIR 节点映射到 WASM 指令

等价于把"写 Cangjie 解析器"替换成"逆向 FlatBuffers schema + 处理完整语言语义"，工作量不减反增。此外，CHIR schema 未公开稳定 API 承诺，随编译器版本变化可能随时失效。

---

### 方案 C：以 cjc 为子进程，解析文本输出

**流程：** 调用 `cjc --dump-chir` 等调试选项，解析文本格式的 CHIR。

**障碍：**

- cjc 没有面向外部工具的稳定 AST/IR dump 格式
- 文本格式仅用于调试，随版本变化，极度脆弱
- 需要 cjc 二进制存在于运行环境中，引入强依赖

---

## CJWasm2 现有方案的优势

```
Cangjie 源码 → 自研 Lexer/Parser → AST → 单态化 → WASM codegen
```

| 维度 | CJWasm2 自研 | cjc-based |
|------|-------------|-----------|
| Runtime 依赖 | 无（仅 WASI fd_write 等） | 需要移植 153 个 C++ 文件 |
| 工具链依赖 | 无 | 需要 cjc + LLVM 或 flatbuffers |
| 语言子集控制 | 自由裁剪 | 被 cjc 完整语义绑定 |
| 调试定位 | AST / codegen 自控 | 跨越 C++ 边界，难以调试 |
| 构建复杂度 | `cargo build` | CMake + LLVM + cjc |
| 版本稳定性 | 自控 | 依赖 cjc 内部 API |

---

## 结论

cjc 方案的核心矛盾是：**cjc 为带完整 runtime 的 native 平台设计，WASM 是受限沙箱环境**。

绕过 runtime 的唯一方法是自己做 WASM codegen——而这正是 CJWasm2 已经在做的事。cjc 能贡献的仅有前端（词法 / 语法 / 语义），引入的代价却是 C++ 构建依赖、FlatBuffers 解析、以及对 cjc 内部 API 的强版本耦合。

**推荐维持现有自研路径。**

如需改善前端质量（解析覆盖率、错误恢复），更合理的方向是参考 `third_party/cangjie_compiler/src/Parse/` 完善 CJWasm2 自己的解析器，而非引入 cjc 整体。
