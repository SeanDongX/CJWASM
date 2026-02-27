// 测试宏系统的集成测试

main(): Int64 {
    let a = 42
    let b = 42

    // 测试 @Assert 宏
    @Assert(a, b)

    let x = 10
    let y = 20

    // 测试 @Expect 宏
    @Expect(x + y, 30)

    return 0
}
