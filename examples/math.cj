// 数学函数与运算示例

// 内置数学函数
func mathFunctions() -> Int64 {
    let a = abs(-42)           // 42
    let b = min(10, 20)        // 10
    let c = max(10, 20)        // 20

    return a + b + c           // 72
}

// 幂运算
func powerOperations() -> Int64 {
    let a = 2 ** 10            // 1024
    let b = 3 ** 4             // 81
    let c = 5 ** 3             // 125

    // 幂运算是右结合的
    let d = 2 ** 2 ** 3        // 2 ** (2 ** 3) = 2 ** 8 = 256

    return a + b + c + d       // 1024 + 81 + 125 + 256 = 1486
}

// 浮点数运算
func floatOperations() -> Float64 {
    let a: Float64 = 10.5
    let b: Float64 = 3.2

    let sum = a + b            // 13.7
    let diff = a - b           // 7.3
    let prod = a * b           // 33.6
    let quot = a / b           // 3.28125

    return sum + diff + prod + quot
}

// 混合类型运算（需要显式转换）
func mixedOperations() -> Int64 {
    let i: Int64 = 42
    let f: Float64 = 3.5

    // Int64 转 Float64 进行运算
    let result = (i as Float64) + f  // 45.5

    // 结果转回 Int64
    return result as Int64     // 45
}

// 取模运算
func moduloOperations() -> Int64 {
    let a = 17 % 5             // 2
    let b = 100 % 7            // 2
    let c = (-10) % 3          // 负数取模

    return a + b               // 4
}

// 整数除法
func divisionOperations() -> Int64 {
    let a = 17 / 5             // 3 (整数除法)
    let b = 100 / 7            // 14
    let c = 1000 / 3           // 333

    return a + b + c           // 350
}

// 复杂数学表达式
func complexExpression(x: Int64, y: Int64) -> Int64 {
    // 体现运算符优先级
    let result = x + y * 2 - x / 2 + y % 3
    return result
}

// 数学公式示例：计算二次方程判别式
func discriminant(a: Int64, b: Int64, c: Int64) -> Int64 {
    // b^2 - 4ac
    return b ** 2 - 4 * a * c
}

// 几何计算：曼哈顿距离
func manhattanDistance(x1: Int64, y1: Int64, x2: Int64, y2: Int64) -> Int64 {
    return abs(x2 - x1) + abs(y2 - y1)
}

// 统计计算：简单平均值（整数版）
func average(a: Int64, b: Int64, c: Int64) -> Int64 {
    return (a + b + c) / 3
}

func main() -> Int64 {
    let a = mathFunctions()        // 72
    let b = powerOperations()      // 1486
    let c = moduloOperations()     // 4
    let d = divisionOperations()   // 350
    let e = complexExpression(10, 5)  // 10 + 10 - 5 + 2 = 17
    let f = discriminant(1, 5, 6)  // 25 - 24 = 1
    let g = manhattanDistance(0, 0, 3, 4)  // 7
    let h = average(10, 20, 30)    // 20

    return a + b + c + d + e + f + g + h
    // 72 + 1486 + 4 + 350 + 17 + 1 + 7 + 20 = 1957
}
