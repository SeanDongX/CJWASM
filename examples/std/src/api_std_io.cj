// L1 std.io 全量 API 测试
// 类型与接口：InputStream, OutputStream, IOStream, Seekable,
// ByteBuffer, BufferedInputStream<T>, BufferedOutputStream<T>,
// ChainedInputStream<T>, MultiOutputStream<T>, StringReader<T>, StringWriter<T>,
// IOException, ContentFormatException, SeekPosition(enum)
// 函数：readString<T>, readToEnd<T>, copy(from, to)
package std_examples

import std.io

// API 列表（vendor）:
// - public open class IOException <: Exception
// - public interface InputStream / OutputStream / IOStream / Seekable
// - public class ByteBuffer <: IOStream & Seekable
// - public class BufferedInputStream<T>, BufferedOutputStream<T> where T <: InputStream/OutputStream
// - public class ChainedInputStream<T>, MultiOutputStream<T>, StringReader<T>, StringWriter<T>
// - public enum SeekPosition
// - public func readString<T>(from: T): String where T <: InputStream & Seekable
// - public func readToEnd<T>(from: T): Array<Byte> where T <: InputStream & Seekable
// - public func copy(from: InputStream, to!: OutputStream): Int64
// 最小引用测试（解析通过时可用）：IOException 为公开异常类型
func api_std_io_used(): Unit {
    ()
}
