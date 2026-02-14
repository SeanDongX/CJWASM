// 数值字面量示例

// 整数字面量
func integerLiterals() : Int64 {
    let dec = 42              // 十进制
    let hex = 0xFF            // 十六进制 = 255
    let oct = 0o77            // 八进制 = 63
    let bin = 0b1010          // 二进制 = 10
    let sep = 1_000_000       // 数字分隔符 = 1000000

    return dec + hex + oct + bin
    // 42 + 255 + 63 + 10 = 370
}

// 浮点数字面量
func floatLiterals() : Float64 {
    let f64 = 3.14159         // Float64 (默认)
    let sci = 1.5e10          // 科学计数法 = 15000000000
    let neg = 2.5e-3          // 负指数 = 0.0025
    let int_sci = 1e6         // 整数科学计数法 = 1000000

    return f64 + neg
}

// Float32 类型 (cjc: as 返回 Option，使用 Float32(x) 构造函数)
func float32Literals() : Float32 {
    let f1: Float32 = Float32(3.14)     // Float32
    let f2 = Float32(2.718)             // Float32
    let f3 = Float32(1.0e5)             // 科学计数法 Float32

    return f1 + f2 + f3
}

// 不同进制运算
func hexOperations() : Int64 {
    let a = 0xFF       // 255
    let b = 0x0F       // 15

    let and = a & b    // 0x0F = 15
    let or = a | b     // 0xFF = 255
    let xor = a ^ b    // 0xF0 = 240

    return and + or + xor  // 15 + 255 + 240 = 510
}

// 位移运算与二进制
func binaryOperations() : Int64 {
    let a = 0b0001     // 1
    let b = 0b1000     // 8

    let shl = a << 3   // 0b1000 = 8
    let shr = b >> 2   // 0b0010 = 2

    return shl + shr   // 8 + 2 = 10
}

// 类型标注
func typeAnnotations() : Int64 {
    let a: Int32 = 42
    let b: Int64 = 100
    let c: Float32 = Float32(3.14)
    let d: Float64 = 2.718
    let e: Bool = true

    // 类型转换 (cjc: as 返回 Option，使用 Int64(x) 和 if 代替)
    let sum = Int64(a) + b + (if (e) { 1 } else { 0 })
    return sum  // 42 + 100 + 1 = 143
}

main(): Int64 {
    let a = integerLiterals()    // 370
    let b = hexOperations()      // 510
    let c = binaryOperations()   // 10
    let d = typeAnnotations()    // 143

    return a + b + c + d
    // 370 + 510 + 10 + 143 = 1033
    // 预期输出: 1033
}
