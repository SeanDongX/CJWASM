// 测试类型别名 - 简单版本

// 简单类型别名
type MyInt = Int64
type MyFloat = Float64

main(): Int64 {
    // 使用类型别名
    let x: MyInt = 42
    let y: MyInt = 10

    let f: MyFloat = 3.14

    let result: MyInt = x + y

    return result  // 52
}
