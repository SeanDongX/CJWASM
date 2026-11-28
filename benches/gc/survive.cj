// GC 基准 2/5: survive —— 保留集持续增长（节点数组，全部可达）
// 一次性构造 n 个节点对象并全部保留在 Array 中，再按下标遍历。
// 测峰值内存 / 存活对象成本。
// 预期：两边峰值都随 N 线性增长（这些对象本来就必须活着），用于校准
//       "每存活对象字节成本"；cjwasm 可从线性内存页数精确读出。
// 注：刻意不用 Option 链遍历——保留集用数组表达更直接，也避开 match
//     作用域内重绑定循环变量的实现差异，保证两编译器输出逐位一致。

class LNode {
    var value: Int64
    init(value: Int64) {
        this.value = value
    }
}

main(): Unit {
    // run.py 会按此行改写 N
    let n: Int64 = 300000
    // 占位初始化后逐槽写入新对象：避开两编译器在 "lambda 使用下标 i" 上的
    // 实现差异，保证 checksum 逐位一致。
    let nodes = Array<LNode>(n, {idx => LNode(0)})
    var i: Int64 = 0
    while (i < n) {
        nodes[i] = LNode(i)
        i = i + 1
        if (i % 100000 == 0) {
            println("MILE survive i=${i}")
        }
    }
    var checksum: Int64 = 0
    i = 0
    while (i < n) {
        checksum = checksum + nodes[i].value
        i = i + 1
    }
    println("BENCH survive n=${n} count=${n} checksum=${checksum}")
}
