// 测试类型别名

// 简单类型别名
type MyInt = Int64
type MyString = String

// 函数类型别名
type Transformer = (Int64) -> Int64
type BinaryOp = (Int64, Int64) -> Int64

func double(x: Int64): Int64 {
    return x * 2
}

func add(a: Int64, b: Int64): Int64 {
    return a + b
}

main(): Int64 {
    // 使用类型别名
    let x: MyInt = 42
    let s: MyString = "hello"

    // 函数类型别名
    let f: Transformer = double
    let result1 = f(10)  // 20

    let g: BinaryOp = add
    let result2 = g(5, 3)  // 8

    return result1 + result2  // 28
}
