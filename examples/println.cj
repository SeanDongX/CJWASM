// println 示例 - 演示标准输出支持
// 通过 WASI fd_write 实现的 println 函数

// 测试 println 输出整数
func testPrintInt(): Int64 {
    println(42)
    println(0)
    println(-1)
    println(9223372036854775807)  // Int64 最大值
    return 1
}

// 测试 println 输出字符串
func testPrintStr(): Int64 {
    println("Hello, Cangjie!")
    println("仓颉语言")
    println("")  // 空字符串
    return 2
}

// 测试 println 输出布尔值
func testPrintBool(): Int64 {
    println(true)
    println(false)
    return 3
}

// 测试 println 输出表达式结果
func testPrintExpr(): Int64 {
    let x: Int64 = 10
    let y: Int64 = 20
    println(x + y)      // 30
    println(x * y)      // 200
    println(x > y)      // false
    return 4
}

// 测试 println() 输出空行
func testPrintEmpty(): Int64 {
    println()
    return 5
}

main(): Int64 {
    let r1 = testPrintInt()
    let r2 = testPrintStr()
    let r3 = testPrintBool()
    let r4 = testPrintExpr()
    let r5 = testPrintEmpty()
    // 预期输出:
    // 42
    // 0
    // -1
    // 9223372036854775807
    // Hello, Cangjie!
    // 仓颉语言
    // (空行，空字符串的 println)
    // true
    // false
    // 30
    // 200
    // false
    // (空行)
    // 返回值: 15
    return r1 + r2 + r3 + r4 + r5
}
