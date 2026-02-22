// cjwasm 标准库 overlay：std.fs 最小实现
// WASI 下仅提供常量与占位 API，完整实现依赖 fd_prestat_get/path_open/fd_seek 等
package std.fs

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

/// 路径是否存在（WASI 占位：当前返回 false，后续可接 path_open 等）
public func exists(path: String): Bool {
    return false
}
