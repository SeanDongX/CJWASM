// cjwasm 标准库 overlay：std.time 精简实现，使用运行时 __get_time_ns / __get_time_sec_ns / __get_monotonic_sec_ns
// 完整 std.time 见 cangjie_runtime，此处仅提供可编译的最小 API

package std.time

foreign func __get_time_ns(): Int64
foreign func __get_time_sec_ns(): (Int64, Int64)
foreign func __get_monotonic_sec_ns(): (Int64, Int64)

/// 当前时间（Unix 纳秒时间戳）
func nowNs(): Int64 {
    return __get_time_ns()
}

/// 当前时间：秒与纳秒 (realtime)
func nowSecNs(): (Int64, Int64) {
    return __get_time_sec_ns()
}

/// 单调时钟：秒与纳秒
func monotonicSecNs(): (Int64, Int64) {
    return __get_monotonic_sec_ns()
}

/// 单调时钟纳秒（由秒与纳秒换算）
func monotonicNs(): Int64 {
    let (sec, ns) = __get_monotonic_sec_ns()
    return sec * 1000000000 + ns
}
