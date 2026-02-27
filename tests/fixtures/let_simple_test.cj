// 测试简单的 Some(x) 模式

main(): Int64 {
    let opt1: ?Int64 = Some(42)

    var result: Int64 = 0

    if (let Some(x) <- opt1) {
        result = result + x  // +42
    }

    return result  // 42
}
