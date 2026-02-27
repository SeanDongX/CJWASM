// 综合测试：低频高级特性

// 1. 异常类型模式测试
func testExceptionTypePattern(): Int64 {
    var result: Int64 = 0

    try {
        result = result + 10
    } catch (e: Int64) {
        result = result + 100
    }

    return result  // 10
}

// 2. This 类型测试（语法支持）
// 注意：This 类型需要运行时上下文替换，当前作为语法支持
class SimpleBuilder {
    var value: Int64

    init(v: Int64) {
        this.value = v
    }

    func getValue(): Int64 {
        return this.value
    }
}

// 3. 限定类型测试（语法支持）
// 注意：限定类型用于模块化，当前作为语法支持

main(): Int64 {
    var total: Int64 = 0

    // 测试异常类型模式
    total = total + testExceptionTypePattern()  // +10 = 10

    // 测试简单类
    let builder = SimpleBuilder(25)
    total = total + builder.getValue()  // +25 = 35

    return total
}
