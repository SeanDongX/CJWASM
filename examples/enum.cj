// 枚举与关联值示例

// 简单枚举
enum Color {
    Red,
    Green,
    Blue
}

// 带关联值的枚举
enum Message {
    Quit,
    Move(Int64),
    Write(String)
}

// 使用 Option 类型
func divide(a: Int64, b: Int64) -> Option<Int64> {
    if b == 0 {
        return None
    }
    return Some(a / b)
}

// 使用 Result 类型
func safeDivide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 {
        return Err("Division by zero")
    }
    return Ok(a / b)
}

// 枚举方法
func Color.toCode(self: Color) -> Int64 {
    match self {
        Color.Red => 0xFF0000,
        Color.Green => 0x00FF00,
        Color.Blue => 0x0000FF
    }
}

// 测试枚举 match
func handleColor(c: Color) -> Int64 {
    match c {
        Color.Red => 1,
        Color.Green => 2,
        Color.Blue => 3
    }
}

// 测试关联值解构
func handleMessage(m: Message) -> Int64 {
    match m {
        Message.Quit => 0,
        Message.Move(distance) => distance,
        Message.Write(s) => 100
    }
}

func main() -> Int64 {
    let c = Color.Red
    let code = c.toCode()              // 0xFF0000
    let n = handleColor(Color.Green)   // 2

    // Option 使用
    let result1 = divide(10, 2)        // Some(5)
    let result2 = divide(10, 0)        // None

    // Result 使用
    let res = safeDivide(20, 4)        // Ok(5)

    return n + code
}
