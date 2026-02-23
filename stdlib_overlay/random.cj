// cjwasm 标准库 overlay：std.random，WASI random_get；Random 类为最小 API（无确定性种子）
package std.random

foreign func __random_i64(): Int64
foreign func __random_f64(): Float64

/// 随机 Int64
func randomInt64(): Int64 {
    return __random_i64()
}

/// 随机 Float64，范围 [0, 1)
func randomFloat64(): Float64 {
    return __random_f64()
}

/// Random 类：nextInt64/nextFloat64 委托 WASI random_get
public class Random {
    init() {}

    public func nextInt64(): Int64 {
        return __random_i64()
    }

    public func nextFloat64(): Float64 {
        return __random_f64()
    }
}
