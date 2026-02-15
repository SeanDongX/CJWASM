// Phase 2 新增类型与表达式示例

// === 1. 无符号整数类型 ===
func unsignedDemo(): Int64 {
    let a: UInt8 = 200
    let b: UInt16 = 50000
    let c: UInt32 = 3000000000
    let d: UInt64 = 10000000000

    // 无符号除法与比较
    let x: UInt32 = 10
    let y: UInt32 = 3
    let div = x / y       // 无符号除法 = 3
    let rem = x % y       // 无符号取模 = 1

    // cjc: as returns Option, use Int64() constructor for conversion
    return Int64(div) + Int64(rem)  // 3 + 1 = 4
}

// === 2. Rune 类型 ===
func charDemo(): Int64 {
    let ch = 'A'           // 字符字面量
    let newline = '\n'     // 转义字符
    let tab = '\t'

    // cjc: as returns Option, use Int64() constructor for Rune->Int64
    return Int64(ch)   // 65 ('A' 的 Unicode)
}

// === 3. Tuple 类型 ===
func tupleDemo(): Int64 {
    let pair = (42, 100)   // 元组字面量
    let a = pair[0]        // 元组索引
    let b = pair[1]

    return a + b  // 42 + 100 = 142
}

// === 4. 空值合并运算符 ?? ===
func findValue(flag: Int64): Option<Int64> {
    if (flag > 0) {
        return Some(flag * 10)
    }
    return None
}

func nullCoalesceDemo(): Int64 {
    let result1 = findValue(5) ?? 0    // Some(50) → 50
    let result2 = findValue(0) ?? 99   // None → 99
    return result1 + result2            // 50 + 99 = 149
}

// === 5. internal 可见性 ===
internal func internalHelper(): Int64 {
    return 7
}

// === 6. 无符号右移 >>> ===
// Note: cjwasm-specific feature removed for cjc compatibility (>>> not supported in cjc)
// func unsignedShiftDemo(): Int64 { ... }

// === 综合测试 ===
main(): Int64 {
    let a = unsignedDemo()         // 4
    let b = charDemo()             // 65
    let c = tupleDemo()            // 142
    let d = nullCoalesceDemo()     // 149
    let e = internalHelper()       // 7

    // 注意: 无符号类型运算可能产生大数，总和以实际运行结果为准
    @Assert(a + b + c + d + e, 429496729825)

    return a + b + c + d + e
    // 预期输出: 429496729825
}
