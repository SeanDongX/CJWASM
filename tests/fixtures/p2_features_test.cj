// 测试 P2 功能：模式解构、++/--, 通配符、类型模式

struct Point {
    x: Int64,
    y: Int64
}

main(): Int64 {
    var result: Int64 = 0

    // 1. 测试元组解构
    let (a, b) = (10, 20)
    result = result + a + b  // result = 30

    // 2. 测试结构体解构
    let p = Point { x: 5, y: 15 }
    let Point { x, y } = p
    result = result + x + y  // result = 50

    // 3. 测试前缀自增/自减
    var n: Int64 = 10
    result = result + (++n)  // n=11, result=61
    result = result + (--n)  // n=10, result=71

    // 4. 测试后缀自增/自减
    result = result + (n++)  // n=11, result=81
    result = result + (n--)  // n=10, result=92

    // 5. 测试通配符表达式
    let _ = n + 100  // 丢弃值

    // 6. 测试类型模式
    let value: Int64 = 8
    let type_result = match (value) {
        case v: Int64 => v
        case _ => 0
    }
    result = result + type_result  // result = 100

    return result
}
