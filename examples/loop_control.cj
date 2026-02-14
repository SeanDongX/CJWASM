// 循环控制语句示例

// loop 无限循环与 break
func loopWithBreak(): Int64 {
    var i: Int64 = 0
    var sum: Int64 = 0

    while (true) {
        sum = sum + i
        i = i + 1
        if (i >= 10) {
            break
        }
    }
    return sum  // 0+1+2+...+9 = 45
}

// continue 跳过迭代
func sumEvenNumbers(): Int64 {
    var sum: Int64 = 0
    for (i in 0..20) {
        if (i % 2 != 0) {
            continue  // 跳过奇数
        }
        sum = sum + i
    }
    return sum  // 0+2+4+6+8+10+12+14+16+18 = 90
}

// while 循环中的 break
func findFirst(): Int64 {
    let arr = [5, 12, 8, 20, 3, 15]
    var result: Int64 = 0
    var i: Int64 = 0

    while (i < 6) {
        if (arr[i] > 10) {
            result = arr[i]
            break  // 找到第一个大于10的数
        }
        i = i + 1
    }
    return result  // 12
}

// 组合 break 和 continue
func filterAndSum(): Int64 {
    var sum: Int64 = 0
    var count: Int64 = 0

    for (i in 0..100) {
        // 跳过能被3整除的数
        if (i % 3 == 0) {
            continue
        }

        sum = sum + i
        count = count + 1

        // 只累加前10个
        if (count >= 10) {
            break
        }
    }
    return sum  // 1+2+4+5+7+8+10+11+13+14 = 75
}

// 复杂循环逻辑
func searchMatrix(): Int64 {
    // 模拟在"矩阵"中搜索
    // 找到第一个满足条件的值
    var target: Int64 = 0
    var found = false

    for (i in 0..5) {
        if (found) {
            break
        }
        for (j in 0..5) {
            let value = i * 5 + j
            if (value == 17) {
                target = value
                found = true
                break  // 只退出内层循环
            }
        }
    }
    return target  // 17
}

// loop 实现 do-while 语义
func doWhileStyle(): Int64 {
    var n: Int64 = 0

    while (true) {
        n = n + 1
        // 至少执行一次，然后检查条件
        if (n >= 5) {
            break
        }
    }
    return n  // 5
}

main(): Int64 {
    let a = loopWithBreak()      // 45
    let b = sumEvenNumbers()     // 90
    let c = findFirst()          // 12
    let d = filterAndSum()       // 75
    let e = searchMatrix()       // 17
    let f = doWhileStyle()       // 5

    return a + b + c + d + e + f
    // 45 + 90 + 12 + 75 + 17 + 5 = 244
    // 预期输出: 244
}
