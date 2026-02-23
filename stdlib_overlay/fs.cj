// cjwasm 标准库 overlay：std.fs 最小实现
// 真实 exists 基于 WASI path_filestat_get(dir_fd=3)，需 preopen 当前目录
package std.fs

foreign func __fs_exists(path: String): Bool

// 与 C SEEK_* 一致，供 fd_seek 使用
public const SEEK_SET: Int32 = 0
public const SEEK_CUR: Int32 = 1
public const SEEK_END: Int32 = 2

/// 路径类型（最小占位，与 vendor Path 兼容）
public struct Path {
    let _rawPath: String

    init(rawPath: String) {
        _rawPath = rawPath
    }

    func toString(): String {
        return _rawPath
    }
}

/// 路径是否存在（WASI path_filestat_get，dir_fd=3 需 preopen）
public func exists(path: String): Bool {
    return __fs_exists(path)
}
