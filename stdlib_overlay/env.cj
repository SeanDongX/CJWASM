// cjwasm 标准库 overlay：std.env，完整 __get_env 基于 WASI environ_get
package std.env

foreign func __get_env(key: String): String

/// 获取环境变量，未找到返回空串
func getEnv(key: String): String {
    return __get_env(key)
}

/// 获取环境变量为 Option，空串视为未设置（与 getEnv 一致）
func getVariable(key: String): Option<String> {
    let s = getEnv(key)
    if (s == "") {
        return None
    }
    return Some(s)
}
