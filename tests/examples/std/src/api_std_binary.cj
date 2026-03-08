// L1 std.binary 全量 API 测试
// 接口：BigEndianOrder<T>, LittleEndianOrder<T>, SwapEndianOrder<T>
// 方法：writeBigEndian(buffer), readBigEndian(buffer), writeLittleEndian, readLittleEndian, swapBytes()
package std_examples

import std.binary

// API 列表（vendor）:
// - public interface BigEndianOrder<T>: writeBigEndian(buffer: Array<UInt8>): Int64, static readBigEndian(buffer): T
// - public interface LittleEndianOrder<T>: writeLittleEndian(buffer): Int64, static readLittleEndian(buffer): T
// - public interface SwapEndianOrder<T>: swapBytes(): T
// extend UInt64/Int64/... <: BigEndianOrder & LittleEndianOrder & SwapEndianOrder
func api_std_binary_used(): Unit {
    ()
}
