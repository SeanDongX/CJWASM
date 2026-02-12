// 运算符示例

// 位运算
func bitOperations(a: Int64, b: Int64) -> Int64 {
    let and = a & b      // 按位与
    let or = a | b       // 按位或
    let xor = a ^ b      // 按位异或
    let not = ~a         // 按位取反
    let shl = a << 2     // 左移
    let shr = b >> 1     // 右移
    return and + or + xor + shl + shr
}

// 幂运算
func powerTest() -> Int64 {
    let a = 2 ** 10      // 1024
    let b = 3 ** 4       // 81
    let c = 2 ** 3 ** 2  // 右结合: 2 ** (3 ** 2) = 2 ** 9 = 512
    return a + b + c
}

// 类型转换
func typeConversion() -> Int64 {
    let i: Int64 = 42
    let f: Float64 = 3.14

    // Int64 -> Float64
    let toFloat = i as Float64

    // Float64 -> Int64
    let toInt = f as Int64      // 截断为 3

    // Int64 -> Int32
    let small = i as Int32

    // Bool 转换
    let b = true
    let boolToInt = b as Int64  // 1

    return toInt + boolToInt
}

// 逻辑运算（短路求值）
func logicShortCircuit(x: Int64) -> Bool {
    // 如果 x > 0 为 false，不会计算右边
    let a = x > 0 && x < 100

    // 如果 x <= 0 为 true，不会计算右边
    let b = x <= 0 || x > 50

    // 逻辑非
    let c = !a

    return a && b || c
}

// 复合赋值
func compoundAssignment() -> Int64 {
    var x: Int64 = 10
    x += 5      // 15
    x -= 3      // 12
    x *= 2      // 24
    x /= 4      // 6
    x %= 4      // 2
    return x
}

// 一元负号
func negation(x: Int64) -> Int64 {
    let neg = -x
    let doubleNeg = -(-x)
    return neg + doubleNeg
}

func main() -> Int64 {
    let a = bitOperations(12, 10)    // 各种位运算结果
    let b = powerTest()              // 1024 + 81 + 512 = 1617
    let c = typeConversion()         // 3 + 1 = 4
    let d = compoundAssignment()     // 2
    let e = negation(5)              // -5 + 5 = 0
    return b + c + d
}
