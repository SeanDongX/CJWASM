// 自增/自减变异语义示例（w53a）
// cjc 语义：后缀 `x++`/`x--` 结果类型为 Unit，但会写回 `x = x ± 1`；
// 前缀 `++x`/`--x` 在 cjc 中是解析错误（此处不涉及）。

class Counter {
    var n: Int64 = 0
    init() {}
}

main(): Int64 {
    var passed = 0

    // 1) 语句位置：++ 必须写回变量
    var a = 0
    a++
    a++
    a++
    if (a == 3) { passed = passed + 1 }

    // 2) 语句位置：-- 必须写回变量
    var b = 10
    b--
    b--
    if (b == 8) { passed = passed + 1 }

    // 3) 多种整数类型
    var i32v: Int32 = 5
    var u8v: UInt8 = 5
    i32v++
    u8v++
    if (Int64(i32v) == 6 && Int64(u8v) == 6) { passed = passed + 1 }

    // 4) 数组元素
    var arr = [1, 2, 3]
    arr[1]++
    if (arr[0] == 1 && arr[1] == 3 && arr[2] == 3) { passed = passed + 1 }

    // 5) 结构体字段
    var c = Counter()
    c.n = 7
    c.n++
    c.n++
    if (c.n == 9) { passed = passed + 1 }

    // 6) 表达式位置：`d++` 的结果类型为 Unit，且 d 已写回
    var d = 100
    let u = d++
    if (d == 101) { passed = passed + 1 }

    println("passed=${passed}")
    return passed
}
// 预期输出: 6
