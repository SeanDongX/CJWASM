# cjwasm 标准库复用策略：最大化复用 cangjie_runtime std 实现

## 概述

仓颉标准库 (`third_party/cangjie_runtime/std/libs/std/`) 包含 **485 个 .cj 文件**（约 13.5 万行代码）和大量 native C/C++ 实现。本文档说明如何最大化复用 .cj 部分来实现 WASM 上的标准库。

## 核心原则

### 1. 分层复用策略

```
┌─────────────────────────────────────────────────┐
│   用户代码 (import std.xxx)                      │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  L1: 纯 Cangjie 实现模块（100% 复用）           │
│  - std.io (interfaces, buffering)               │
│  - std.collection (ArrayList, HashMap 逻辑)     │
│  - std.binary, std.console                      │
│  - std.overflow, std.crypto                     │
│  - std.deriving, std.ast                        │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  L2: 轻量 native 依赖（选择性复用+overlay）      │
│  - std.time → vendor + WASI clock 映射          │
│  - std.math → 复用 .cj + WASM intrinsics        │
│  - std.random → overlay WASI random_get         │
│  - std.convert → 部分复用 + 简化实现             │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  L3: 重度 native 依赖（overlay 替代实现）        │
│  - std.env → WASI environ_get                   │
│  - std.runtime → WASI args/proc_exit            │
│  - std.fs → WASI fd_* (待实现)                  │
│  - std.net → 不支持/桩实现                       │
│  - std.posix → 不支持                            │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  Runtime 层: cjwasm 内置 + WASI                  │
│  - __get_time_sec_ns, __alloc_native_clock      │
│  - __arraylist_*, __hashmap_*                   │
│  - memory allocation, GC                        │
└─────────────────────────────────────────────────┘
```

## 模块分类与复用计划

### L1: 100% 纯 Cangjie 实现 - 直接复用

这些模块不依赖 native 代码或仅有轻微依赖，可以直接使用 vendor 实现：

| 模块 | .cj 文件数 | Native 依赖 | 复用策略 |
|------|-----------|------------|---------|
| **std.io** | 11 | 无 | ✅ 完全复用：interfaces (InputStream/OutputStream), buffered streams, string reader/writer |
| **std.binary** | 1 | 无 | ✅ 完全复用：二进制数据处理 |
| **std.console** | 3 | 无 | ✅ 完全复用：控制台输出（基于 std.io） |
| **std.overflow** | 6 | 无 | ✅ 完全复用：溢出检查工具 |
| **std.crypto** | 3 | 无 | ✅ 完全复用：密码学接口定义 |
| **std.deriving** | 40 | 无 | ✅ 完全复用：derive 宏实现 |
| **std.ast** | 21 | 2 native | ⚠️ 大部分复用：AST 节点定义，native 部分可能需要桩实现 |
| **std.argopt** | 1 | 无 | ✅ 完全复用：命令行参数解析 |
| **std.sort** | ? | 无 | ✅ 完全复用：排序算法 |
| **std.ref** | ? | 无 | ✅ 完全复用：引用类型工具 |

**实施方式**：
- 在 cjwasm 的模块解析器中，对这些模块 **优先解析 vendor 目录**
- 自动排除 `native/` 子目录
- 如遇到 `@When[backend == "cjnative"]`，需要在 cjwasm 中：
  - 要么支持该条件编译
  - 要么提供等效的 `@When[backend == "wasm"]` 实现

### L2: 部分 Cangjie + 选择性复用

这些模块有 .cj 实现但依赖少量 native，可以复用大部分 .cj 代码：

#### std.collection (23 .cj, 1 native)

**复用策略**：
- **ArrayList, HashMap, HashSet** 等类型定义和算法逻辑 100% 复用
- 内存分配通过 cjwasm 的 `__arraylist_new`, `__hashmap_new` 等内置函数
- native 部分（array_blocking_queue_ffi.c）可能用于并发，暂不支持

**当前状态**：
- `stdlib_overlay/collection/ArrayList.cj` 提供最小桩实现
- **改进方案**：移除 overlay，直接使用 vendor 的完整实现

#### std.time (6+ .cj, 1 native)

**已完成** ✅
- 完全复用 vendor 目录下所有 .cj 文件（constants, date_time, mono_time, timezone, format）
- `CJ_TIME_Now()`, `CJ_TIME_MonotonicNow()` foreign 函数映射到 cjwasm 内置：
  ```rust
  "__get_time_sec_ns"     // WASI clock_time_get(REALTIME)
  "__get_monotonic_sec_ns" // WASI clock_time_get(MONOTONIC)
  "__alloc_native_clock"   // 分配 (sec: Int64, nanosec: Int64) 结构
  ```
- overlay/time.cj 作为回退实现保留

#### std.math (12 .cj, 1 native)

**复用策略**：
- 数学接口、枚举、扩展方法全部复用
- BigInt, Decimal 的 .cj 实现复用
- native 部分（native.c）实现三角函数等，需要：
  - **选项 A**：映射到 WASM `f64.sqrt`, `f64.sin` 等指令
  - **选项 B**：通过 WASI 调用 libc math（如果 WASI 运行时支持）
  - **选项 C**：纯 Cangjie 实现（参考 musl/fdlibm）

**实施优先级**：P1（数学运算是基础功能）

#### std.convert (3 .cj, 2 native)

**复用策略**：
- 类型转换接口和部分实现复用
- native SIMD 优化部分用 WASM SIMD 替代（如果支持）
- 当前 overlay/convert.cj 提供桩实现

**实施**：逐步将 vendor 实现替换 overlay

#### std.core (59 .cj, 13 native)

**最复杂的模块**，包含 String, Array, Option, Result 等核心类型：

| 文件 | Native 依赖 | 复用策略 |
|------|-----------|---------|
| string.cj | vectorCompare32, SIMD | WASM SIMD / 标量实现 |
| libc.cj | malloc/free | 映射到 cjwasm 内存管理 |
| thread.cj | pthread | 不支持/桩实现 |
| future.cj | async runtime | 不支持/桩实现 |
| endian.cj | byteswap | WASM 指令 |
| atexit.cj | libc atexit | WASI reactor 模型 |

**实施**：
- 核心类型（Option, Result, Iterator）100% 复用
- String 需要处理 SIMD 优化（可降级为标量）
- 线程/异步相关暂不支持

### L3: 重度 Native 依赖 - Overlay 实现

这些模块的 vendor 实现严重依赖操作系统 API，需要 WASI overlay：

| 模块 | Vendor .cj | Native | Overlay 策略 |
|------|-----------|--------|-------------|
| **std.env** | 5 | 1 native | ✅ 已完成：WASI environ_get |
| **std.runtime** | ? | ? | ✅ 已完成：WASI args/proc_exit |
| **std.random** | 1 | 1 native | ✅ 已完成：WASI random_get |
| **std.fs** | 10 | 3 native | ⏸️ 待实现：WASI fd_read/write/seek |
| **std.net** | 36 | 4 native | ❌ 不支持：WASI preview1 无 socket |
| **std.posix** | 4 | 4 native | ❌ 不支持：POSIX 专有 |
| **std.process** | 7 | 6 native | ❌ 不支持：进程管理 |
| **std.database** | 19 | 0 | ❌ 不支持：需要 native driver |
| **std.unittest** | ? | ? | ⚠️ 部分支持：基础断言可用 |

## 技术实施方案

### 1. 模块解析优先级

修改 `src/parser/mod.rs` 中的模块解析逻辑：

```rust
fn resolve_std_module(module_path: &[String]) -> Option<PathBuf> {
    let module_name = module_path[1]; // 跳过 "std"

    // 特殊处理列表（按优先级）
    match module_name.as_str() {
        // L1: 优先 vendor
        "io" | "binary" | "console" | "overflow" | "crypto" |
        "deriving" | "argopt" | "sort" | "ref" => {
            find_in_vendor_std(module_path)
                .or_else(|| find_in_overlay(module_path))
        },

        // L2: vendor + fallback overlay
        "collection" | "math" | "convert" | "time" => {
            find_in_vendor_std(module_path)
                .or_else(|| find_in_overlay(module_path))
        },

        // L3: 仅 overlay
        "env" | "runtime" | "random" | "fs" => {
            find_in_overlay(module_path)
        },

        // 不支持
        "net" | "posix" | "process" | "database" => {
            emit_error("Module not supported on WASM");
            None
        },

        _ => find_in_vendor_std(module_path)
    }
}

fn find_in_vendor_std(path: &[String]) -> Option<PathBuf> {
    let vendor_root = "third_party/cangjie_runtime/std/libs/std";
    let module_dir = vendor_root.join(path[1..].join("/"));

    if module_dir.is_dir() {
        // 收集目录下所有 .cj，排除 native/
        Some(collect_cj_files_excluding_native(&module_dir))
    } else {
        // 单文件模块
        let file = format!("{}.cj", module_dir.display());
        if file.exists() { Some(file) } else { None }
    }
}
```

### 2. Foreign 函数映射表

在 `src/codegen/mod.rs` 中维护 foreign → builtin 映射：

```rust
const FOREIGN_TO_BUILTIN: &[(&str, &str)] = &[
    // std.time
    ("CJ_TIME_Now", "__get_time_sec_ns"),
    ("CJ_TIME_MonotonicNow", "__get_monotonic_sec_ns"),

    // std.random
    ("__random_i64", "__random_i64"),
    ("__random_f64", "__random_f64"),

    // std.env
    ("__get_env", "__get_env"),

    // std.runtime
    ("__get_args", "__get_args"),
    ("__exit", "__exit"),

    // std.core (libc)
    ("malloc", "__wasm_alloc"),
    ("free", "__wasm_free"),
    ("memcpy", "__wasm_memcpy"),

    // std.math (待实现)
    ("sin", "f64.sin"),  // WASM 指令或 import
    ("cos", "f64.cos"),
    ("sqrt", "f64.sqrt"),
];
```

### 3. @When 条件编译支持

增强 parser 处理 `@When[backend == "cjnative"]`：

```rust
fn parse_when_attribute(attr: &Attribute) -> Option<Condition> {
    // 解析 @When[backend == "xxx"]
    // 当 backend != "wasm" 时跳过该项
    // 或要求 vendor 代码提供 @When[backend == "wasm"] 版本
}
```

**短期方案**：跳过 `@When[backend == "cjnative"]` 块
**长期方案**：向上游贡献 `@When[backend == "wasm"]` 实现

### 4. WASI Builtin 实现清单

| 功能 | WASI API | cjwasm Builtin | 状态 |
|------|----------|---------------|------|
| 时钟（实时） | clock_time_get(REALTIME) | `__get_time_sec_ns` | ✅ 完成 |
| 时钟（单调） | clock_time_get(MONOTONIC) | `__get_monotonic_sec_ns` | ✅ 完成 |
| 随机数 | random_get | `__random_i64/f64` | ✅ 完成 |
| 环境变量 | environ_get | `__get_env` | ✅ 完成 |
| 命令行参数 | args_get | `__get_args` | ✅ 完成 |
| 退出 | proc_exit | `__exit` | ✅ 完成 |
| 文件读写 | fd_read/fd_write | `__fd_read/write` | ⏸️ 待实现 |
| 文件定位 | fd_seek | `__fd_seek` | ⏸️ 待实现 |
| 文件信息 | fd_filestat_get | `__fd_stat` | ⏸️ 待实现 |

## 测试与验证

### 分阶段测试

**Phase 1: L1 模块验证**
```bash
# 测试纯 Cangjie 模块
cjwasm examples/std_io_demo.cj -o /tmp/io.wasm
cjwasm examples/std_collection_demo.cj -o /tmp/collection.wasm
```

**Phase 2: L2 模块验证**
```bash
# 已完成
cjwasm examples/std_time.cj -o /tmp/time.wasm
cjwasm examples/std_random_demo.cj -o /tmp/random.wasm

# 待实现
cjwasm examples/std_math_demo.cj -o /tmp/math.wasm
```

**Phase 3: L3 模块验证**
```bash
# 文件系统
cjwasm examples/std_fs_demo.cj -o /tmp/fs.wasm
wasmtime --dir=. /tmp/fs.wasm
```

### 兼容性测试

复用 cangjie_runtime 的单元测试（如果有）：
```bash
# 运行 vendor 测试套件
cjwasm third_party/cangjie_runtime/std/test/collection_test.cj
```

## 依赖关系图

```
std.io ← std.console
  ↑
std.collection ← std.sort
  ↑
std.core (String, Array, Option, Result)
  ↑
std.math ← std.crypto
  ↑
std.convert ← std.binary
  ↑
std.time ← std.runtime
  ↑
std.env
```

**关键路径**：`std.core` → `std.collection` → `std.io` 必须先完成。

## 下一步行动计划

### 短期（本周）

1. **移除冗余 overlay**
   - 删除 `stdlib_overlay/collection/ArrayList.cj`
   - 让 `std.collection` 直接使用 vendor 实现

2. **验证 L1 模块**
   - 创建 `examples/std_io_demo.cj` 测试文件读写
   - 创建 `examples/std_collection_demo.cj` 测试 ArrayList/HashMap

3. **实现 std.math 基础支持**
   - 映射 foreign 数学函数到 WASM f64 指令

### 中期（本月）

4. **实现 std.fs overlay**
   - 基于 WASI fd_* API
   - 支持基本文件操作（open, read, write, close）

5. **处理 std.core 复杂依赖**
   - 解决 String SIMD 优化（降级为标量或使用 WASM SIMD）
   - 处理 thread/future 相关代码（桩实现）

### 长期

6. **向上游贡献**
   - 为 cangjie_runtime 添加 `@When[backend == "wasm"]` 实现
   - 提交 WASM 后端相关 patch

7. **性能优化**
   - 对比 vendor 原生实现与 WASM 版本性能
   - 识别瓶颈并优化（如 String 操作、集合操作）

## 风险与挑战

### 技术风险

1. **@When 条件编译不兼容**
   - vendor 代码假设 backend == "cjnative"
   - **缓解**：先跳过 @When 块，逐步添加 WASM 版本

2. **Intrinsic 函数依赖**
   - `@Intrinsic` 标记的函数可能依赖编译器特殊处理
   - **缓解**：在 cjwasm 中识别并实现对应 intrinsic

3. **SIMD 优化代码**
   - String, Array 等使用 x86/ARM SIMD
   - **缓解**：降级为标量或使用 WASM SIMD（需运行时支持）

### 维护风险

1. **上游更新同步**
   - cangjie_runtime 更新时需要重新测试兼容性
   - **缓解**：设置 CI 定期拉取上游并运行测试

2. **API 差异**
   - WASI 限制导致部分 API 无法完全实现
   - **缓解**：明确文档说明不支持的功能

## 成功指标

1. **代码复用率** ≥ 70%（按行数计算）
2. **L1 模块覆盖** 100%（纯 Cangjie 模块全部可用）
3. **L2 模块覆盖** ≥ 80%（核心功能可用）
4. **测试通过率** ≥ 90%（vendor 单元测试在 WASM 环境通过）
5. **性能损失** ≤ 2x（相比 native 编译版本）

## 总结

通过分层复用策略，cjwasm 可以：
- **直接复用** 60% 的标准库代码（L1 纯 Cangjie 模块）
- **选择性复用** 30% 的代码（L2 轻量 native 依赖，通过映射到 WASM/WASI）
- **重新实现** 10% 的代码（L3 重度平台依赖，提供 overlay）

这样可以最大化利用官方标准库的成熟实现，减少维护负担，并确保与上游的兼容性。
