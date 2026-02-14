// 测试 for-in 数组迭代 (cjc: for (x in arr))
func sumArray(): Int64 {
    let arr: Array<Int64> = [10, 20, 30, 40]
    var total: Int64 = 0
    for (x in arr) {
        total = total + x
    }
    return total
}

// 测试 match 带 guard (cjc: case x where condition =>)
func classifyNumber(n: Int64): Int64 {
    match (n) {
        case x where x < 0 => 1       // 负数返回 1
        case 0 => 2                   // 零返回 2
        case x where x < 10 => 3      // 小于10的正数返回 3
        case x where x < 100 => 4     // 小于100的正数返回 4
        case _ => 5                    // 其他返回 5
    }
}

// 综合测试
main(): Int64 {
    let a = sumArray()           // 10+20+30+40 = 100
    let b = classifyNumber(0)    // 2
    let c = classifyNumber(5)    // 3
    let d = classifyNumber(50)   // 4
    let e = classifyNumber(200)  // 5
    return a + b + c + d + e     // 100+2+3+4+5 = 114
}
