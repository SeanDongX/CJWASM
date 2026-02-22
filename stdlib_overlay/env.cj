// cjwasm 标准库 overlay：std.env 精简实现
package std.env

foreign func __get_env(key: String): String

/// 获取环境变量，未找到返回空串
func getEnv(key: String): String {
    return __get_env(key)
}
