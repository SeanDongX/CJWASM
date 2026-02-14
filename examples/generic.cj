// 泛型函数与结构体示例 (cjc 兼容: init + 构造调用语法)
struct Pair<T, U> {
    var first: T;
    var second: U;
    init(first: T, second: U) {
        this.first = first
        this.second = second
    }
}

func identity<T>(value: T): T {
    return value
}

main(): Int64 {
    let p = Pair<Int64, Int64>(1, 2)
    let x = identity<Int64>(42)
    return p.first + x  // 预期输出: 43
}
