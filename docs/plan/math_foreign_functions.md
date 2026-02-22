# std.math Foreign 函数映射方案

## 概述

std.math 模块包含大量 foreign 数学函数，这些函数需要映射到 WASM 环境。

## WASM 原生支持的数学指令

WASM 本身支持的数学指令（无需导入）：

| 操作 | f64 指令 | f32 指令 |
|------|---------|---------|
| 绝对值 | f64.abs | f32.abs |
| 取负 | f64.neg | f32.neg |
| 平方根 | f64.sqrt | f32.sqrt |
| 向上取整 | f64.ceil | f32.ceil |
| 向下取整 | f64.floor | f32.floor |
| 截断 | f64.trunc | f32.trunc |
| 四舍五入 | f64.nearest | f32.nearest |
| 最小值 | f64.min | f32.min |
| 最大值 | f64.max | f32.max |
| 符号复制 | f64.copysign | f32.copysign |

## 需要导入的数学函数

从 vendor 中的 math/native.cj 找到的 foreign 函数：

### 基本三角函数
- `CJ_MATH_Tan(x: Float64): Float64` - tan
- `CJ_MATH_Tanf(x: Float32): Float32` - tanf
- (注: sin/cos 可能在其他文件中)

### 反三角函数
- `CJ_MATH_Asin(x: Float64): Float64` - asin
- `CJ_MATH_Asinf(x: Float32): Float32` - asinf
- `CJ_MATH_Acos(x: Float64): Float64` - acos
- `CJ_MATH_Acosf(x: Float32): Float32` - acosf
- `CJ_MATH_Atan(x: Float64): Float64` - atan
- `CJ_MATH_Atanf(x: Float32): Float32` - atanf
- `CJ_MATH_Atan2(y: Float64, x: Float64): Float64` - atan2
- `CJ_MATH_Atan2f(y: Float32, x: Float32): Float32` - atan2f

### 双曲函数
- `CJ_MATH_Sinh(x: Float64): Float64` - sinh
- `CJ_MATH_Sinhf(x: Float32): Float32` - sinhf
- `CJ_MATH_Cosh(x: Float64): Float64` - cosh
- `CJ_MATH_Coshf(x: Float32): Float32` - coshf
- `CJ_MATH_Tanh(x: Float64): Float64` - tanh
- `CJ_MATH_Tanhf(x: Float32): Float32` - tanhf

### 反双曲函数
- `CJ_MATH_Asinh(x: Float64): Float64` - asinh
- `CJ_MATH_Asinhf(x: Float32): Float32` - asinhf
- `CJ_MATH_Acosh(x: Float64): Float64` - acosh
- `CJ_MATH_Acoshf(x: Float32): Float32` - acoshf
- `CJ_MATH_Atanh(x: Float64): Float64` - atanh
- `CJ_MATH_Atanhf(x: Float32): Float32` - atanhf

### 幂函数
- `CJ_CORE_FastPowerDoubleInt64(x: Float64, y: Int64): Float64` - pow

## 实现策略

### 阶段 1：使用 WASI libc 导入

WASI 运行时通常提供 C 标准库（libc），包括 math.h 中的函数。我们可以：

1. 添加 WASM import section，从 "wasi_snapshot_preview1" 或 "env" 模块导入
2. 在 codegen 中识别这些 foreign 函数并映射到导入的函数

示例：
```wasm
(import "env" "sin" (func $sin (param f64) (result f64)))
(import "env" "cos" (func $cos (param f64) (result f64)))
```

### 阶段 2：使用 WASM 原生指令

对于 WASM 原生支持的操作，直接生成指令：
- sqrt -> f64.sqrt
- abs -> f64.abs
- ceil/floor/trunc/nearest -> 相应指令

### 阶段 3：纯 Cangjie 实现（长期）

对于不依赖 WASI libc 的环境，可以考虑纯 Cangjie 实现这些函数（参考 musl/fdlibm）。

## 实施计划

### 短期（本次）

1. **扩展 is_builtin_alias**：添加所有 CJ_MATH_* 函数
2. **添加 import 类型**：在 compile 中定义这些函数的类型
3. **生成 import section**：从 "env" 模块导入这些函数
4. **处理调用**：在 compile_call 中识别并调用这些函数

### 中期

1. 创建测试用例验证数学函数
2. 优化：对于简单操作（sqrt, abs）直接用 WASM 指令

### 长期

1. 实现纯 WASM 版本的数学函数（不依赖 import）
2. 性能优化

## 代码修改清单

### src/codegen/mod.rs

1. 修改 `is_builtin_alias`：
```rust
fn is_builtin_alias(name: &str) -> bool {
    matches!(
        name,
        "__get_time_ns" | "__random_i64" | "__random_f64" | "__get_env" | "__get_args" | "__exit"
            | "CJ_TIME_Now" | "CJ_TIME_MonotonicNow"
            | "CJ_MATH_Tan" | "CJ_MATH_Tanf"
            | "CJ_MATH_Asin" | "CJ_MATH_Asinf"
            | "CJ_MATH_Acos" | "CJ_MATH_Acosf"
            // ... 其他数学函数
    )
}
```

2. 添加类型定义（在 compile 中）：
```rust
// (f64) -> f64 : 单参数数学函数
let ty_f64_f64 = ...;
// (f32) -> f32 : 单参数数学函数
let ty_f32_f32 = ...;
// (f64, f64) -> f64 : 双参数数学函数（如 atan2, pow）
let ty_f64f64_f64 = ...;
```

3. 生成 import section：
```rust
import_section.import(
    "env",
    "sin",
    EntityType::Function(ty_f64_f64)
);
```

4. 处理调用：
```rust
"CJ_MATH_Tan" => {
    // 调用导入的 tan 函数
    func.instruction(&Instruction::Call(self.func_indices["tan"]));
}
```

## 注意事项

1. **WASI 兼容性**：不是所有 WASI 运行时都提供 math 函数，可能需要链接 libc
2. **性能**：导入函数有调用开销，对于简单操作优先使用 WASM 指令
3. **精度**：确保 Float32 和 Float64 版本都正确映射
4. **错误处理**：数学函数可能产生 NaN/Inf，需要正确处理

## 测试用例

创建 `examples/std_math_demo.cj`：
```cangjie
import std.math.*

main() {
    let x = 3.14159 / 4.0  // π/4
    println("sin(π/4) = ${sin(x)}")
    println("cos(π/4) = ${cos(x)}")
    println("tan(π/4) = ${tan(x)}")
    println("sqrt(2) = ${sqrt(2.0)}")
}
```
