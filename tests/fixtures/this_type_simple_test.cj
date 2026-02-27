// 测试 This 类型 - 简化版本

class Builder {
    var value: Int64

    init(v: Int64) {
        this.value = v
    }

    func add(n: Int64): This {
        this.value = this.value + n
        return this
    }

    func getValue(): Int64 {
        return this.value
    }
}

main(): Int64 {
    let builder = Builder(10)
    let b2 = builder.add(5)
    return b2.getValue()  // 应该返回 15
}
