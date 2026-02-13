// 错误处理示例

// 使用 Result 类型
func divide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 {
        return Err("Cannot divide by zero")
    }
    return Ok(a / b)
}

// ? 操作符进行错误传播
func compute(x: Int64, y: Int64) -> Result<Int64, String> {
    let result = divide(x, y)?  // 如果是 Err，自动返回
    return Ok(result * 2)
}

// 链式错误传播
func chainedCompute(a: Int64, b: Int64, c: Int64) -> Result<Int64, String> {
    let step1 = divide(a, b)?
    let step2 = divide(step1, c)?
    return Ok(step2 + 10)
}

// try-catch 表达式
func safeDivide(a: Int64, b: Int64) -> Int64 {
    try {
        if b == 0 {
            throw "Division error"
        }
        return a / b
    } catch(e) {
        return 0  // 错误时返回默认值
    }
}

// Option 类型与 ? 操作符
func findValue(arr: Array<Int64>, index: Int64) -> Option<Int64> {
    if index < 0 {
        return None
    }
    return Some(arr[0])  // 简化示例
}

func doubleValue(arr: Array<Int64>, index: Int64) -> Option<Int64> {
    let value = findValue(arr, index)?  // None 则早期返回
    return Some(value * 2)
}

// 解构 Result
func handleResult(r: Result<Int64, String>) -> Int64 {
    match r {
        Ok(value) => value,
        Err(_) => 0
    }
}

// 解构 Option
func handleOption(opt: Option<Int64>) -> Int64 {
    match opt {
        Some(v) => v,
        None => 0
    }
}

func main() -> Int64 {
    // Result 测试
    let r1 = divide(10, 2)      // Ok(5)
    let r2 = divide(10, 0)      // Err("...")

    let v1 = handleResult(r1)   // 5
    let v2 = handleResult(r2)   // 0

    // ? 操作符测试
    let c1 = compute(20, 4)     // Ok(10)
    let c2 = compute(20, 0)     // Err(...)

    // try-catch 测试
    let s1 = safeDivide(10, 2)  // 5
    let s2 = safeDivide(10, 0)  // 0

    // Option 测试
    let arr = [100, 200, 300]
    let opt = doubleValue(arr, 0)
    let optVal = handleOption(opt)  // 200

    return v1 + s1 + s2 + optVal    // 5 + 5 + 0 + 200 = 210
}
