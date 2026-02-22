# std.core 模块支持状态

更新时间：2026-02-22

## 概述

std.core 是仓颉标准库的核心模块，包含 **59 个 .cj 文件**，定义了最基础的类型和接口。

## 测试结果

### ✅ 已验证功能

**Option<T>**:
```cangjie
let some: Option<Int64> = Some(42)
match (some) {
    case Some(v) => println("Value: ${v}")
    case None => println("No value")
}
```
**状态**: ✅ 编译成功

**Result<T, E>**:
```cangjie
let ok: Result<Int64, String> = Ok(100)
match (ok) {
    case Ok(v) => println("Success: ${v}")
    case Err(e) => println("Error: ${e}")
}
```
**状态**: ✅ 编译成功

**Range**:
```cangjie
for (i in 0..10) { println("${i}") }
```
**状态**: ✅ 编译成功

## 文件分类统计

- ✅ **可直接复用**: 41 文件（70%） - 纯 Cangjie，无依赖
- ⚠️ **需要适配**: 10 文件（17%） - 有 foreign/\@When
- 🚧 **需特殊处理**: 8 文件（13%） - SIMD/并发/\@Intrinsic

**总体复用率**: **87%** (可复用 + 可适配)

## 复用策略

### P0: 核心类型 ✅
- Option, Result, Range 已验证

### P1: 接口和异常（下一步）
- Iterator, Collection 接口
- 16 个异常类型文件

### P2: String/Array 扩展
- 需处理 SIMD 优化

### P3: 并发/异步
- 暂不支持（WASM 限制）
