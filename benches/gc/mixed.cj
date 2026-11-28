// GC 基准 4/5: mixed —— 分配 + 保留 + 丢弃混合（保留率 ~10%）
// 每轮分配一个节点，i % 10 == 0 的节点写入预分配的保留数组，其余即弃。
// 模拟真实负载的综合分配/存活比，测吞吐 + 峰值内存。
// 预期：cjc GC 只需为约 10% 的活对象付内存；cjwasm 的内存 ≈ N × 对象大小。
// 注：保留集用数组（下标遍历），避开 Option 链遍历的实现差异。

class MNode {
    var value: Int64
    init(value: Int64) {
        this.value = value
    }
}

main(): Unit {
    // run.py 会按此行改写 N
    let n: Int64 = 400000
    let cap: Int64 = n / 10 + 1
    // 预分配保留槽位（占位节点也计入保留集）
    let retained = Array<MNode>(cap, {i => MNode(-1)})
    var slot: Int64 = 0
    var checksum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let node = MNode(i)
        checksum = checksum + node.value
        if (i % 10 == 0) {
            retained[slot] = node
            slot = slot + 1
        }
        i = i + 1
        if (i % 100000 == 0) {
            println("MILE mixed i=${i} checksum=${checksum}")
        }
    }
    // 遍历保留集，避免被当作死代码
    var acc: Int64 = 0
    var k: Int64 = 0
    while (k < cap) {
        acc = acc + retained[k].value % 1000
        k = k + 1
    }
    println("BENCH mixed n=${n} retained=${cap} checksum=${checksum} acc=${acc}")
}
