// cjwasm 标准库 overlay：std.math 基础数学函数
// 提供最常用的数学函数，通过 WASM 指令或运行时实现
package std.math

@import("env", "__math_sqrt")
foreign func __math_sqrt(x: Float64): Float64
@import("env", "__math_ceil")
foreign func __math_ceil(x: Float64): Float64
@import("env", "__math_floor")
foreign func __math_floor(x: Float64): Float64
@import("env", "__math_trunc")
foreign func __math_trunc(x: Float64): Float64
@import("env", "__math_round")
foreign func __math_round(x: Float64): Float64
@import("env", "__math_sin")
foreign func __math_sin(x: Float64): Float64
@import("env", "__math_cos")
foreign func __math_cos(x: Float64): Float64
@import("env", "__math_tan")
foreign func __math_tan(x: Float64): Float64
@import("env", "__math_exp")
foreign func __math_exp(x: Float64): Float64
@import("env", "__math_log")
foreign func __math_log(x: Float64): Float64
@import("env", "__math_pow")
foreign func __math_pow(base: Float64, exp: Float64): Float64

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

public func trunc(x: Float64): Float64 {
    return __math_trunc(x)
}

public func round(x: Float64): Float64 {
    return __math_round(x)
}

public func nearest(x: Float64): Float64 {
    return __math_round(x)
}

public func sin(x: Float64): Float64 {
    return __math_sin(x)
}

public func cos(x: Float64): Float64 {
    return __math_cos(x)
}

public func tan(x: Float64): Float64 {
    return __math_tan(x)
}

public func exp(x: Float64): Float64 {
    return __math_exp(x)
}

public func log(x: Float64): Float64 {
    return __math_log(x)
}

public func pow(base: Float64, exponent: Float64): Float64 {
    return __math_pow(base, exponent)
}

// min - Int64 重载
public func min(a: Int64, b: Int64): Int64 {
    if (a < b) {
        return a
    }
    return b
}

// min - Float64 重载
public func min(a: Float64, b: Float64): Float64 {
    if (a < b) {
        return a
    }
    return b
}

// max - Int64 重载
public func max(a: Int64, b: Int64): Int64 {
    if (a > b) {
        return a
    }
    return b
}

// max - Float64 重载
public func max(a: Float64, b: Float64): Float64 {
    if (a > b) {
        return a
    }
    return b
}

// 常量
public const PI: Float64 = 3.141592653589793
public const E: Float64 = 2.718281828459045
