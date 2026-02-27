// 测试 let 解构 - 嵌套模式匹配

main(): Int64 {
    // 测试 1: Some((x, y)) 嵌套模式
    let opt1: ?(Int64, Int64) = Some((10, 20))

    var result: Int64 = 0

    if (let Some((x, y)) <- opt1) {
        result = result + x + y  // +30
    }

    // 测试 2: None 情况
    let opt2: ?(Int64, Int64) = None

    if (let Some((a, b)) <- opt2) {
        result = result + a + b
    } else {
        result = result + 5  // +5
    }

    return result  // 30 + 5 = 35
}
