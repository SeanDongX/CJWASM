// 测试尾随闭包支持（显式类型）

func apply(x: Int64, f: (Int64) -> Int64): Int64 {
    return f(x)
}

main(): Int64 {
    var result: Int64 = 0

    // 测试 1: 显式类型的尾随闭包
    let doubled = apply(5) { x: Int64 => x * 2 }
    result = result + doubled  // +10

    // 测试 2: 另一个显式类型的闭包
    let tripled = apply(7) { x: Int64 => x * 3 }
    result = result + tripled  // +21

    return result  // 10 + 21 = 31
}
