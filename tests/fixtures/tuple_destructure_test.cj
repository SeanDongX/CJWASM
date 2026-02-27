// 测试元组解构（不在 Option 中）

main(): Int64 {
    let tuple: (Int64, Int64) = (10, 20)

    // 直接解构元组
    let (x, y) = tuple

    return x + y  // 30
}
