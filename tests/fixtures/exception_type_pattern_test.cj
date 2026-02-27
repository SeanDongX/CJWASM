// 测试异常类型模式

main(): Int64 {
    var result: Int64 = 0

    // 测试 catch 带类型注解
    try {
        result = result + 10
    } catch (e: Int64) {
        result = result + 100
    }

    // 测试 catch 不带类型注解
    try {
        result = result + 20
    } catch (e) {
        result = result + 200
    }

    // 测试 catch 通配符带类型
    try {
        result = result + 5
    } catch (_: Int64) {
        result = result + 500
    }

    return result  // 应该返回 35 (10 + 20 + 5)
}
