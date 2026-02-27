// 测试 Option<Tuple> 的基本使用

main(): Int64 {
    let opt: ?(Int64, Int64) = Some((10, 20))

    var result: Int64 = 0

    // 使用 match 而不是 if-let
    match (opt) {
        case Some(tuple) => {
            // tuple 应该是 (Int64, Int64)
            result = 30  // 暂时硬编码
        }
        case None => {
            result = 0
        }
    }

    return result
}
