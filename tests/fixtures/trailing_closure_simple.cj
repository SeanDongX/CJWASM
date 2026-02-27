// 测试尾随闭包支持（简化版）

func apply(x: Int64, f: (Int64) -> Int64): Int64 {
    return f(x)
}

func applyTwice(x: Int64, f: (Int64) -> Int64): Int64 {
    return f(f(x))
}

main(): Int64 {
    var result: Int64 = 0

    // 测试 1: 基础尾随闭包
    let doubled = apply(5) { x => x * 2 }
    result = result + doubled  // +10

    // 测试 2: 多次应用
    let quadrupled = applyTwice(3) { x => x * 2 }
    result = result + quadrupled  // +12

    // 测试 3: 无参数闭包
    let constant = apply(100) { => 42 }
    result = result + constant  // +42

    return result  // 10 + 12 + 42 = 64
}
