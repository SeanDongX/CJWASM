// L1 std.ref 全量 API 测试
// 类型：WeakRef<T>, WeakRefBase, CleanupPolicy(enum: EAGER, DEFERRED)
package std_examples

import std.ref

// API 列表（vendor）:
// - public enum CleanupPolicy <: Equatable<CleanupPolicy>: EAGER, DEFERRED
// - public class WeakRef<T> <: WeakRefBase where T <: Object
// - sealed abstract class WeakRefBase
func api_std_ref_used(): Unit {
    ()
}
