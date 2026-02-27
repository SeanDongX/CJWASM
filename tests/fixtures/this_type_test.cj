// 测试 This 类型 - 用于链式调用

class Builder {
    var value: Int64

    init(v: Int64) {
        this.value = v
    }

    func add(n: Int64): This {
        this.value = this.value + n
        return this
    }

    func multiply(n: Int64): This {
        this.value = this.value * n
        return this
    }

    func getValue(): Int64 {
        return this.value
    }
}

main(): Int64 {
    let builder = Builder(10)
    let result = builder.add(5).multiply(2).getValue()
    return result  // 应该返回 30: (10 + 5) * 2
}
