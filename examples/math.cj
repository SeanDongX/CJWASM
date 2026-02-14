// 数学函数与运算示例（cjc：无 ** 和 abs，用自定义函数替代）

func pow(base: Int64, exp: Int64): Int64 {
    if (exp <= 0) {
        return 1
    }
    var result: Int64 = 1
    var b: Int64 = base
    var e: Int64 = exp
    while (e > 0) {
        if (e % 2 == 1) {
            result = result * b
        }
        b = b * b
        e = e / 2
    }
    return result
}

func abs(x: Int64): Int64 {
    if (x < 0) {
        return -x
    } else {
        return x
    }
}

// 内置数学函数
func mathFunctions(): Int64 {
    let a = abs(-42)
    let b = min(10, 20)
    let c = max(10, 20)
    return a + b + c
}

// 幂运算
func powerOperations(): Int64 {
    let a = pow(2, 10)
    let b = pow(3, 4)
    let c = pow(5, 3)
    let d = pow(2, pow(2, 3))
    return a + b + c + d
}

// 浮点数运算
func floatOperations(): Float64 {
    let a: Float64 = 10.5
    let b: Float64 = 3.2
    let sum = a + b
    let diff = a - b
    let prod = a * b
    let quot = a / b
    return sum + diff + prod + quot
}

// 混合类型运算（需要显式转换）
func mixedOperations(): Int64 {
    let i: Int64 = 42
    let f: Float64 = 3.5
    let result = (i as Float64) + f
    return result as Int64
}

// 取模运算
func moduloOperations(): Int64 {
    let a = 17 % 5
    let b = 100 % 7
    let c = (-10) % 3
    return a + b
}

// 整数除法
func divisionOperations(): Int64 {
    let a = 17 / 5
    let b = 100 / 7
    let c = 1000 / 3
    return a + b + c
}

// 复杂数学表达式
func complexExpression(x: Int64, y: Int64): Int64 {
    let result = x + y * 2 - x / 2 + y % 3
    return result
}

// 数学公式示例：计算二次方程判别式
func discriminant(a: Int64, b: Int64, c: Int64): Int64 {
    return pow(b, 2) - 4 * a * c
}

// 几何计算：曼哈顿距离
func manhattanDistance(x1: Int64, y1: Int64, x2: Int64, y2: Int64): Int64 {
    return abs(x2 - x1) + abs(y2 - y1)
}

// 统计计算：简单平均值（整数版）
func average(a: Int64, b: Int64, c: Int64): Int64 {
    return (a + b + c) / 3
}

main(): Int64 {
    let a = mathFunctions()
    let b = powerOperations()
    let c = moduloOperations()
    let d = divisionOperations()
    let e = complexExpression(10, 5)
    let f = discriminant(1, 5, 6)
    let g = manhattanDistance(0, 0, 3, 4)
    let h = average(10, 20, 30)
    return a + b + c + d + e + f + g + h
}
