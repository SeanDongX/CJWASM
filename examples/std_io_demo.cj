// 测试 std.io 从 vendor 完整复用
package main

import std.io.*

main() {
    // 测试 StringReader 和 StringWriter
    let writer = StringWriter()
    writer.write("Hello, ".getBytes())
    writer.write("WASM!".getBytes())

    let content = writer.toString()
    println("StringWriter output: ${content}")

    // 测试 StringReader
    let reader = StringReader(content)
    let buffer = Array<Byte>(5, item: 0)
    let n = reader.read(buffer)

    println("Read ${n} bytes from StringReader")
    println("Buffer content: ${String(buffer[0..n])}")

    // 测试 BufferedOutputStream
    let buffered = BufferedOutputStream(writer)
    buffered.write(" More data".getBytes())
    buffered.flush()

    println("After buffered write: ${writer.toString()}")
}
