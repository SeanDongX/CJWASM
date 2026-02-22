// 测试 std.io 入口与 import 能解析并编译（泛型类 StringWriter/StringReader 需类型参数，此处仅验证流水线）
package main

import std.io.*

main() {
    println("std.io import and entry compile OK")
    // 泛型类需提供类型参数，例如: StringWriter<ConsoleWriter>(stream)、StringReader<ByteArrayInputStream>(input)
}
