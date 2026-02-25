// L1 std.sort 全量 API 测试
// 函数：sort, stableSort, unstableSort（Array/List/ArrayList 重载，支持 comparator/lessThan/key）
// 接口：SortByExtension<T>, SortExtension
package std_examples

import std.sort

// API 列表（vendor）:
// - public func sort<T>(data: Array<T>, stable?, descending?, by?, lessThan?, key?): Unit where T <: Comparable<T>
// - public func sort<T>(data: List<T>, ...): Unit; sort<T>(data: ArrayList<T>, ...): Unit
// - public func stableSort<T>(data: Array<T>, ...): Unit; unstableSort<T>(data: Array<T>, ...): Unit
// - public interface SortByExtension<T>, SortExtension
func api_std_sort_used(): Unit {
    ()
}
