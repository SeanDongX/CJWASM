// 回归测试：Array<Float64>(n, {i => ...}) 初始化 lambda 的返回类型推断。
// 缺陷（batch-48 w48b）：未注解 lambda 的声明返回类型默认取形参类型 Int64，
// 导致 Float64 体（如 `Float64(i) + 0.5`）被截断后落槽，元素值丢失小数部分。
// cjc 语义：构造 lambda 的期望签名是 (Int64) -> Float64，返回类型按元素类型推断。
main(): Int64 {
    let a = Array<Float64>(5, { i => Float64(i) + 0.5 })
    var s: Float64 = 0.0
    var k: Int64 = 0
    while (k < 5) {
        s = s + a[k]
        k = k + 1
    }
    // sum = 0.5 + 1.5 + 2.5 + 3.5 + 4.5 = 12.5；×10 取整避免浮点打印差异
    return Int64(s * 10.0)  // 预期输出: 125
}
