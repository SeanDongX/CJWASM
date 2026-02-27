// 测试 if-let 支持

main(): Int64 {
    let opt1: ?Int64 = Some(42)
    let opt2: ?Int64 = None

    var result: Int64 = 0

    // 基础 if-let
    if (let Some(x) <- opt1) {
        result = result + x  // +42
    }

    // if-let with else
    if (let Some(y) <- opt2) {
        result = result + y
    } else {
        result = result + 10  // +10
    }

    // 嵌套 if-let
    let opt3: ?Int64 = Some(5)
    if (let Some(a) <- opt3) {
        if (let Some(b) <- opt1) {
            result = result + a + b  // +5 +42
        }
    }

    return result  // 42 + 10 + 5 + 42 = 99
}
