// Phase 7.4-7.8 综合测试
// 测试 format、集合类型、WASI扩展、字符串操作、排序

main() {
    // ===== Phase 7.4: 格式化 =====
    let n: Int64 = 255
    // 十六进制
    let hex = n.format("x")
    println(hex) // expect: ff
    // 二进制
    let bin = n.format("b")
    println(bin) // expect: 11111111
    // 八进制
    let oct = n.format("o")
    println(oct) // expect: 377
    // 十进制(默认)
    let dec = n.format("d")
    println(dec) // expect: 255
    // 带宽度
    let padded = n.format("08x")
    println(padded) // expect: 000000ff

    // Float64 格式化
    let pi: Float64 = 3.14159
    let f2 = pi.format("2f")
    println(f2) // expect: 3.14

    // ===== Phase 7.7: 时间/随机 =====
    let ts = now()
    println(ts) // 纳秒时间戳（非零）

    let r1 = randomInt64()
    println(r1) // 随机整数

    let r2 = randomFloat64()
    // 随机浮点数在 [0, 1) 范围 — 转字符串打印
    println(r2.toString())

    // ===== Phase 7.7: exit (不在这里测试，会终止程序) =====

    // ===== Phase 7.8: 字符串操作 =====
    let s = "hello world"
    let hasWorld = s.contains("world")
    let hasXyz = s.contains("xyz")
    let idx = s.indexOf("world")
    println(s.contains("world"))  // expect: 1 (true)
    println(s.contains("xyz"))    // expect: 0 (false)
    println(s.indexOf("world"))   // expect: 6

    let replaced = s.replace("world", "cangjie")
    println(replaced) // expect: hello cangjie

    // 整数类型断言（字符串比较暂不支持，通过 println 验证）
    @Assert(hasWorld, 1)
    @Assert(hasXyz, 0)
    @Assert(idx, 6)

    // ===== Phase 7.8: sort =====
    let arr = [5, 3, 8, 1, 9, 2, 7, 4, 6]
    sort(arr)
    @Assert(arr[0], 1)
    @Assert(arr[8], 9)
    // 打印排序结果
    let i: Int64 = 0
    while (i < 9) {
        print(arr[i])
        if (i < 8) { print(" ") }
        i = i + 1
    }
    println("") // expect: 1 2 3 4 5 6 7 8 9

    println("PASS")
    // 预期输出: PASS
}
