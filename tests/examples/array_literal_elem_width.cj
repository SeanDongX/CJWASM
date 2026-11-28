// 回归测试：4 字节元素（Int32/Float32/嵌套数组/元组指针）的数组字面量，
// 与 8 字节（Int64）元素混用，验证写入步长恒等于读侧 ArrayGet 步长。
// 同时覆盖：嵌套元组/数组下标经 TupleGet（无数组头 4 字节偏移）读取。
main(): Int64 {
    let i32s: Array<Int32> = [10, 20, 30]
    let f32s = [Float32(1.5), Float32(2.5)]
    let nested = [[1, 2], [3, 4]]
    let tupleArr = [(1, 10), (2, 20)]
    let s = Int64(i32s[1]) + Int64(i32s[2])
    let f = Int64(Float64(f32s[0]) + Float64(f32s[1]))
    let n = nested[1][0] + nested[0][1]
    let t = tupleArr[1][0] + tupleArr[0][1]
    return s + f + n + t
    // 预期输出: 71
}
