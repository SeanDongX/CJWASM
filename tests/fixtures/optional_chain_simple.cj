// 测试可选链的简化语义（返回值或默认值，而非 Option）

struct Point {
    var x: Int64
    var y: Int64
}

main(): Int64 {
    // 测试 1: 直接访问 Option 值
    let opt1: ?Int64 = Some(42)
    let opt2: ?Int64 = None

    var result: Int64 = 0

    // 直接解包测试
    if (let Some(v) <- opt1) {
        result = result + v  // +42
    }

    if (let Some(v) <- opt2) {
        result = result + v
    } else {
        result = result + 10  // +10
    }

    return result  // 42 + 10 = 52
}
