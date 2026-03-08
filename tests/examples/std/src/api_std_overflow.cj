// L1 std.overflow 全量 API 测试
// 接口：WrappingOp<T>, WrappingPow, ThrowingOp<T>, ThrowingPow,
// SaturatingOp<T>, SaturatingPow, CheckedOp<T>, CheckedPow, CarryingOp<T>, CarryingPow
// 异常：OvershiftException, UndershiftException
// 方法由 extend 为整数类型实现（含 @Intrinsic 的 wrappingAdd 等）
package std_examples

import std.overflow

// API 列表（vendor）:
// - public interface WrappingOp<T>: wrappingAdd/Sub/Mul/Div/Mod/Inc/Dec/Neg, wrappingShl/Shr
// - public interface WrappingPow: wrappingPow(y: UInt64): Int64
// - public interface ThrowingOp<T>, ThrowingPow / SaturatingOp<T>, SaturatingPow / CheckedOp<T>, CheckedPow / CarryingOp<T>, CarryingPow
// - public class OvershiftException, UndershiftException <: Exception
func api_std_overflow_used(): Unit {
    ()
}
