// 测试 for 循环
func sumRange() -> Int64 {
    var total: Int64 = 0
    for i in 0..10 {
        total = total + i
    }
    return total
}

// 测试包含边界的 for 循环
func sumRangeInclusive() -> Int64 {
    var total: Int64 = 0
    for i in 1..=5 {
        total = total + i
    }
    return total
}

// 测试 match 表达式 - 字面量匹配
func matchNumber(n: Int64) -> Int64 {
    match n {
        0 => 100,
        1 => 200,
        _ => 999
    }
}

// 测试 match 表达式 - 多模式匹配
func matchMultiple(n: Int64) -> Int64 {
    match n {
        1 | 2 | 3 => 10,
        4 | 5 => 20,
        _ => 0
    }
}

// 测试 match 表达式 - 范围匹配
func matchRange(n: Int64) -> Int64 {
    match n {
        0..10 => 1,
        10..100 => 2,
        _ => 3
    }
}

// 综合测试
func main() -> Int64 {
    let a = sumRange()           // 0+1+2+...+9 = 45
    let b = sumRangeInclusive()  // 1+2+3+4+5 = 15
    let c = matchNumber(1)       // 200
    let d = matchMultiple(2)     // 10
    let e = matchRange(50)       // 2
    return a + b + c + d + e     // 45+15+200+10+2 = 272
}
