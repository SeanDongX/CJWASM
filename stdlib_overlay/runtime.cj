// cjwasm 标准库 overlay：std.runtime
// WASM 下无线程/GC/堆信息，以下为桩实现，行为与 native 不同
package std.runtime

foreign func __get_args(): Array<String>
foreign func __exit(code: Int64)

/// 命令行参数
func getArgs(): Array<String> {
    return __get_args()
}

/// 以指定退出码退出进程
func exit(code: Int64) {
    __exit(code)
}

/// 桩：WASM 单线程，返回 1
func getThreadCount(): Int64 {
    return 1
}

/// 桩：返回 1
func getProcessorCount(): Int64 {
    return 1
}

/// 桩：返回 0（无堆信息）
func getMaxHeapSize(): Int64 {
    return 0
}

/// 桩：返回 0
func getAllocatedHeapSize(): Int64 {
    return 0
}

/// 桩：返回 0
func getUsedHeapSize(): Int64 {
    return 0
}

/// 桩：无操作
func GC(heavy: Bool) {
}

/// 桩：无操作
func setGCThreshold(value: UInt64) {
}

/// 桩：原样返回（防止优化消除）
func blackBox<T>(input: T): T {
    return input
}
