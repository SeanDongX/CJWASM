// L1 标准库示例与全量 API 测试入口
// 运行方式（在仓库根目录）：cjwasm build -p examples/std
package std_examples

import std.io
import std.binary
import std.console
import std.overflow
import std.crypto
import std.deriving
import std.ast
import std.argopt
import std.sort
import std.ref
import std.unicode

main(): Int64 {
    api_std_io_used()
    api_std_binary_used()
    api_std_console_used()
    api_std_overflow_used()
    api_std_crypto_used()
    api_std_deriving_used()
    api_std_ast_used()
    api_std_argopt_used()
    api_std_sort_used()
    api_std_ref_used()
    api_std_unicode_used()
    return 0
}
