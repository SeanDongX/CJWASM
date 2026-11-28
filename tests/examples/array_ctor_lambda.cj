// 回归测试：Array<类>(n, {i => ...}) 下标 lambda 逐元素初始化。
// cjc 语义：lambda 参数 i 是元素下标 0..n-1；类元素为 4 字节指针，
// 写入步长必须与读侧 ArrayGet 一致（否则下标错位，见 gc-bench BASELINE 附录缺陷 1）。
class CtorNode {
    var v: Int64
    init(v: Int64) {
        this.v = v
    }
}

main(): Int64 {
    let a = Array<CtorNode>(5, {i => CtorNode(i)})
    var s: Int64 = 0
    var k: Int64 = 0
    while (k < 5) {
        s = s + a[k].v
        k = k + 1
    }
    return s  // 预期输出: 10
}
