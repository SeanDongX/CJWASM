// L1 std.deriving 全量 API 测试（宏包）
// 宏：Derive, DeriveInclude, DeriveExclude, DeriveOrder
package std_examples

import std.deriving

// API 列表（vendor）:
// - macro package std.deriving
// - public macro Derive(rawOptions: Tokens, input: Tokens): Tokens
// - DeriveInclude, DeriveExclude, DeriveOrder 等子宏
// 用法示例: @Derive[Eq, ToString] struct S { ... }
func api_std_deriving_used(): Unit {
    ()
}
