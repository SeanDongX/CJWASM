// 泛型函数与结构体示例
struct Pair<T, U> {
    first: T,
    second: U
}

func identity<T>(value: T) -> T {
    return value
}

func main() -> Int64 {
    let p = Pair<Int64, Int64> { first: 1, second: 2 }
    let x = identity<Int64>(42)
    return p.first + x
}
