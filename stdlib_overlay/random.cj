// cjwasm 标准库 overlay：std.random 精简实现
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
