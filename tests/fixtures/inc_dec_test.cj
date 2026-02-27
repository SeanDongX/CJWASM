// 测试 ++/-- 运算符

main(): Int64 {
    var x: Int64 = 10
    var y: Int64 = 20

    var result: Int64 = 0

    // 测试前缀自增
    result = result + (++x)  // x=11, result=11

    // 测试前缀自减
    result = result + (--y)  // y=19, result=30

    // 测试后缀自增
    result = result + (x++)  // x=12, result=41

    // 测试后缀自减
    result = result + (y--)  // y=18, result=60

    // 最终: x=12, y=18
    result = result + x + y  // result=90

    return result  // 90
}
