// println 示例 - 演示标准输出支持和字符串插值
// 通过 WASI fd_write 实现的 println 函数
// 字符串插值语法: "Hello, ${expr}!"

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

// 测试字符串插值
func testInterpolation(): Int64 {
    let name = "Cangjie"
    let version: Int64 = 1
    println("Hello, ${name}!")
    println("Version: ${version}")

    let a: Int64 = 10
    let b: Int64 = 20
    println("${a} + ${b} = ${a + b}")
    println("${a} * ${b} = ${a * b}")
    println("${a} > ${b} is ${a > b}")

    let flag = true
    println("flag = ${flag}")
    return 6
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
    let r5 = testInterpolation()
    let r6 = testPrintEmpty()

    @Assert(r1, 1)
    @Assert(r2, 2)
    @Assert(r3, 3)
    @Assert(r4, 4)
    @Assert(r5, 6)
    @Assert(r6, 5)
    @Assert(r1 + r2 + r3 + r4 + r5 + r6, 21)

    // 预期 stdout 输出:
    // 42
    // 0
    // -1
    // 9223372036854775807
    // Hello, Cangjie!
    // (空行)
    // true
    // false
    // 30
    // 200
    // false
    // Hello, Cangjie!
    // Version: 1
    // 10 + 20 = 30
    // 10 * 20 = 200
    // 10 > 20 is false
    // flag = true
    // (空行)
    // 预期输出: 21
    return r1 + r2 + r3 + r4 + r5 + r6
}
