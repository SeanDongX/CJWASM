// 测试 for 循环
func sumRange(): Int64 {
    var total: Int64 = 0
    for (i in 0..10) {
        total = total + i
    }
    return total
}

// 测试包含边界的 for 循环
func sumRangeInclusive(): Int64 {
    var total: Int64 = 0
    for (i in 1..6) {
        total = total + i
    }
    return total
}

// 测试 match 表达式 - 字面量匹配
func matchNumber(n: Int64): Int64 {
    match (n) {
        case 0 => 100
        case 1 => 200
        case _ => 999
    }
}

// 测试 match 表达式 - 多模式匹配
func matchMultiple(n: Int64): Int64 {
    match (n) {
        case 1 | 2 | 3 => 10
        case 4 | 5 => 20
        case _ => 0
    }
}

// 测试 match 表达式 - 范围匹配 (cjc 不支持范围模式，用 if-else 替代)
func matchRange(n: Int64): Int64 {
    if (n >= 0 && n < 10) {
        return 1
    } else {
        if (n >= 10 && n < 100) {
            return 2
        } else {
            return 3
        }
    }
}

// 综合测试
main(): Int64 {
    let a = sumRange()           // 0+1+2+...+9 = 45
    let b = sumRangeInclusive()  // 1+2+3+4+5 = 15
    let c = matchNumber(1)       // 200
    let d = matchMultiple(2)     // 10
    let e = matchRange(50)       // 2
    @Assert(a, 45)
    @Assert(b, 15)
    @Assert(c, 200)
    @Assert(d, 10)
    @Assert(e, 2)
    @Assert(a + b + c + d + e, 272)
    return a + b + c + d + e     // 45+15+200+10+2 = 272
    // 预期输出: 272
}
