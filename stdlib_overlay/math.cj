// cjwasm 标准库 overlay：std.math 基础数学函数
// 提供最常用的数学函数，通过 WASM 指令或运行时实现
package std.math

// 运行时函数（codegen 注册为 f64.sqrt/ceil/floor 单指令包装；@import 仅满足语法，实际不生成 import）
@import("env", "__math_sqrt")
foreign func __math_sqrt(x: Float64): Float64
@import("env", "__math_ceil")
foreign func __math_ceil(x: Float64): Float64
@import("env", "__math_floor")
foreign func __math_floor(x: Float64): Float64

// WASM f64.sqrt
public func sqrt(x: Float64): Float64 {
    return __math_sqrt(x)
}

// 绝对值（纯 Cangjie 实现，与 WASM f64.abs 等价）
public func abs(x: Float64): Float64 {
    if (x < 0.0) {
        return -x
    }
    return x
}

// WASM f64.ceil
public func ceil(x: Float64): Float64 {
    return __math_ceil(x)
}

// WASM f64.floor
public func floor(x: Float64): Float64 {
    return __math_floor(x)
}

// 使用 WASM f64.min 指令
public func min(a: Float64, b: Float64): Float64 {
    if (a < b) {
        return a
    }
    return b
}

// 使用 WASM f64.max 指令
public func max(a: Float64, b: Float64): Float64 {
    if (a > b) {
        return a
    }
    return b
}

// 常量
public const PI: Float64 = 3.141592653589793
public const E: Float64 = 2.718281828459045
