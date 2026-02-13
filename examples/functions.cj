// 函数高级特性示例

// 默认参数
func power(base: Int64, exp: Int64 = 2) -> Int64 {
    var result: Int64 = 1
    for i in 0..exp {
        result = result * base
    }
    return result
}

func greet(name: String, greeting: String = "Hello") -> String {
    return "${greeting}, ${name}!"
}

// 多个默认参数
func createRange(start: Int64 = 0, end: Int64 = 10, step: Int64 = 1) -> Int64 {
    var sum: Int64 = 0
    var i = start
    while i < end {
        sum = sum + i
        i = i + step
    }
    return sum
}

// 可变参数
func sum(numbers: Int64...) -> Int64 {
    var total: Int64 = 0
    for n in numbers {
        total = total + n
    }
    return total
}

func max(values: Int64...) -> Int64 {
    var maxVal = values[0]
    for v in values {
        if v > maxVal {
            maxVal = v
        }
    }
    return maxVal
}

// 函数重载（按参数类型区分）
func process(x: Int64) -> Int64 {
    return x * 2
}

func process(x: Float64) -> Float64 {
    return x * 3.0
}

func process(x: String) -> String {
    return "processed: ${x}"
}

// 函数重载（按参数数量区分）
func calculate(a: Int64) -> Int64 {
    return a * a
}

func calculate(a: Int64, b: Int64) -> Int64 {
    return a + b
}

func calculate(a: Int64, b: Int64, c: Int64) -> Int64 {
    return a + b + c
}

// Lambda 表达式（仅语法解析，codegen 尚未实现）
// func applyOperation() -> Int64 {
//     let double = (x: Int64) -> Int64 { x * 2 }
//     let triple = { x: Int64 => x * 3 }
//     return 42
// }

func main() -> Int64 {
    // 默认参数测试
    let a = power(3)           // 3^2 = 9
    let b = power(2, 4)        // 2^4 = 16
    let c = createRange()      // 0+1+2+...+9 = 45
    let d = createRange(1, 5)  // 1+2+3+4 = 10

    // 可变参数测试
    let e = sum(1, 2, 3, 4, 5)     // 15
    let f = max(3, 1, 4, 1, 5, 9)  // 9

    // 函数重载测试
    let g = process(5)             // 10
    let h = calculate(4)           // 16
    let i = calculate(3, 5)        // 8
    let j = calculate(1, 2, 3)     // 6

    return a + b + e + f + g + h + i + j
    // 9 + 16 + 15 + 9 + 10 + 16 + 8 + 6 = 89
}
