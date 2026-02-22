// cjwasm 标准库 overlay：std.runtime 精简实现
package std.runtime

foreign func __get_args(): Array<String>
foreign func __exit(code: Int64)

/// 命令行参数
func getArgs(): Array<String> {
    return __get_args()
}

/// 以指定退出码退出进程
func exit(code: Int64) {
    __exit(code)
}
