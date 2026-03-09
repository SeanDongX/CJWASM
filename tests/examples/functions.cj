// 函数高级特性示例（cjc 兼容：无默认参数、无可变参数）

// 幂运算
func power(base: Int64, exp: Int64): Int64 {
    var result: Int64 = 1
    var i: Int64 = 0
    while (i < exp) {
        result = result * base
        i = i + 1
    }
    return result
}

func greet(name: String, greeting: String): String {
    return "${greeting}, ${name}!"
}

// 范围求和
func createRange(start: Int64, end: Int64, step: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = start
    while (i < end) {
        sum = sum + i
        i = i + step
    }
    return sum
}

// 数组求和（替代可变参数）
func sumArray(arr: Array<Int64>): Int64 {
    var total: Int64 = 0
    for (n in arr) {
        total = total + n
    }
    return total
}

// 数组最大值（替代可变参数）
func maxArray(arr: Array<Int64>): Int64 {
    var maxVal: Int64 = arr[0]
    for (v in arr) {
        if (v > maxVal) {
            maxVal = v
        }
    }
    return maxVal
}

// 函数重载（按参数类型区分）
func process(x: Int64): Int64 {
    return x * 2
}

func processFloat(x: Float64): Float64 {
    return x * 3.0
}

func processStr(x: String): String {
    return "processed: ${x}"
}

// 函数重载（按参数数量区分）
func calculate(a: Int64): Int64 {
    return a * a
}

func calculate2(a: Int64, b: Int64): Int64 {
    return a + b
}

func calculate3(a: Int64, b: Int64, c: Int64): Int64 {
    return a + b + c
}

main(): Int64 {
    let a = power(3, 2)                    // 3^2 = 9
    let b = power(2, 4)                    // 2^4 = 16
    let c = createRange(0, 10, 1)          // 0+1+2+...+9 = 45
    let d = createRange(1, 5, 1)           // 1+2+3+4 = 10

    let arr1 = [1, 2, 3, 4, 5]
    let e = sumArray(arr1)                 // 15
    let arr2 = [3, 1, 4, 1, 5, 9]
    let f = maxArray(arr2)                  // 9

    let g = process(5)                      // 10
    let h = calculate(4)                    // 16
    let i = calculate2(3, 5)               // 8
    let j = calculate3(1, 2, 3)            // 6

    @Assert(a, 9)
    @Assert(b, 16)
    @Assert(c, 45)
    @Assert(d, 10)
    @Assert(e, 15)
    @Assert(f, 9)
    @Assert(g, 10)
    @Assert(h, 16)
    @Assert(i, 8)
    @Assert(j, 6)
    @Assert(a + b + e + f + g + h + i + j, 89)

    return a + b + e + f + g + h + i + j  // 预期输出: 89
}
