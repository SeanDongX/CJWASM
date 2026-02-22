// cjwasm 标准库 overlay：std.time 精简实现，使用运行时 __get_time_ns 内置
// 完整 std.time 见 cangjie_runtime，此处仅提供可编译的最小 API

package std.time

// 由 cjwasm 运行时实现，不生成 WASM import
foreign func __get_time_ns(): Int64

/// 当前时间（Unix 纳秒时间戳）
func nowNs(): Int64 {
    return __get_time_ns()
}
