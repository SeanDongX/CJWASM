// 枚举与关联值示例

// 简单枚举
enum Color {
    | Red
    | Green
    | Blue
}

// 带关联值的枚举
enum Message {
    | Quit
    | Move(Int64)
    | Write(String)
}

// 使用 Option 类型 (cjc 标准库)
func divide(a: Int64, b: Int64): Option<Int64> {
    if (b == 0) {
        return None
    }
    return Some(a / b)
}

// 枚举方法（转为独立函数以兼容 cjc）
func colorToCode(c: Color): Int64 {
    match (c) {
        case Color.Red => 0xFF0000
        case Color.Green => 0x00FF00
        case Color.Blue => 0x0000FF
    }
}

// 测试枚举 match
func handleColor(c: Color): Int64 {
    match (c) {
        case Color.Red => 1
        case Color.Green => 2
        case Color.Blue => 3
    }
}

// 测试关联值解构
func handleMessage(m: Message): Int64 {
    match (m) {
        case Message.Quit => 0
        case Message.Move(distance) => distance
        case Message.Write(s) => 100
    }
}

main(): Int64 {
    let c = Color.Red
    let code = colorToCode(c)          // 0xFF0000
    let n = handleColor(Color.Green)   // 2

    // Option 使用
    let result1 = divide(10, 2)        // Some(5)
    let result2 = divide(10, 0)        // None

    return n + code  // 预期输出: 16711682
}
