// CJWasm 宏系统示例
// 演示 macro func 定义、quote 表达式和宏调用

// ========== 1. 用户定义宏 ==========

// 日志宏：在消息前添加 [LOG] 前缀
public macro func MyLog(msg: String): String {
    return quote(
        println("[LOG] " + msg)
    )
}

// 双倍打印宏：打印两次
public macro func PrintTwice(msg: String): String {
    return quote(
        println(msg)
        println(msg)
    )
}

// ========== 2. 用户宏测试 ==========

func test_user_macro_bracket(): Int64 {
    @MyLog["hello macro"]
    println("PASS test_user_macro_bracket")
    return 0
}

func test_user_macro_paren(): Int64 {
    @MyLog("paren syntax")
    println("PASS test_user_macro_paren")
    return 0
}

func test_print_twice(): Int64 {
    @PrintTwice["repeated"]
    println("PASS test_print_twice")
    return 0
}

// ========== 3. 内建宏测试 ==========

func test_builtin_log(): Int64 {
    @Log["builtin log test"]
    println("PASS test_builtin_log")
    return 0
}

// ========== 4. 内建 @Assert/@Expect ==========

func test_assert_expect(): Int64 {
    @Assert(1 + 1, 2)
    @Assert(true)
    @Expect(3 * 4, 12)
    @Expect(10 - 3, 7)
    println("PASS test_assert_expect")
    return 0
}

// ========== 主函数 ==========

main() {
    test_user_macro_bracket()
    test_user_macro_paren()
    test_print_twice()
    test_builtin_log()
    test_assert_expect()
    println("PASS")
}
// 预期输出: PASS
