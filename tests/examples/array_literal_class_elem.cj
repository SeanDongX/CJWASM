// 回归测试：数组字面量 [N(0), N(1)] 的类元素按 4 字节槽写入。
// 缺陷形态（cjc 1.0.5 对照）：ArrayLiteral 统一按 8 字节槽写入，而 ArrayGet/
// ArraySet 以元素实际宽度（类指针 I32=4 字节）为步长读取，导致 a[1].v 读到 0。
// 修复后写入步长/对齐由元素类型决定，元素值按元素类型归一化。
class LitNode {
    var v: Int64
    init(v: Int64) {
        this.v = v
    }
}

func sumLit(a: Array<LitNode>): Int64 {
    return a[0].v + a[1].v + a[2].v
}

class LitBag {
    var items: Array<LitNode>
    init() {
        this.items = [LitNode(2), LitNode(3)]
    }
}

main(): Int64 {
    let a = [LitNode(1), LitNode(2), LitNode(3)]
    let b: Array<LitNode> = [LitNode(4), LitNode(5), LitNode(6)]
    let s = sumLit([LitNode(7), LitNode(8), LitNode(9)])
    let bag = LitBag()
    return a[2].v * 10 + b[1].v + s + bag.items[1].v
    // 预期输出: 62
}
