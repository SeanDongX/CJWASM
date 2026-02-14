// 字符串特性示例

// 字符串插值
func greet(name: String): String {
    return "Hello, ${name}!"
}

// 表达式插值
func formatResult(a: Int64, b: Int64): String {
    return "The sum of ${a} and ${b} is ${a + b}"
}

// 多行字符串
func multilineString(): String {
    return """
    This is a
    multi-line
    string literal
    """
}

// 原始字符串（不处理转义）
func rawString(): String {
    return "C:\\path\\to\\file.txt"
}

// 转义字符
func escapedString(): String {
    return "Line1\nLine2\tTabbed\\"
}

// 字符串拼接
func concat(a: String, b: String): String {
    return "${a} ${b}"
}

// 嵌套插值
func nestedInterpolation(x: Int64): String {
    let inner = "value is ${x}"
    return "The ${inner}"
}

main(): Int64 {
    let greeting = greet("World")
    let result = formatResult(3, 5)
    let multi = multilineString()
    let raw = rawString()
    let escaped = escapedString()

    // 返回一个固定值表示成功
    return 42
}
