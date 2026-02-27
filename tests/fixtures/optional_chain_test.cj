// 测试可选链 (?.) 支持

struct Point {
    var x: Int64
    var y: Int64
}

main(): Int64 {
    let p1: ?Point = Some(Point(10, 20))
    let p2: ?Point = None

    var result: Int64 = 0

    // 测试 1: Some 值的可选链 - 直接返回值或 0
    let x1 = p1?.x
    result = result + x1  // +10

    // 测试 2: None 值的可选链 - 返回 0
    let x2 = p2?.x
    if (x2 == 0) {
        result = result + 5  // +5
    }

    // 测试 3: 访问 y 字段
    let y1 = p1?.y
    result = result + y1  // +20

    return result  // 10 + 5 + 20 = 35
}
