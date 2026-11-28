// GC 基准 3/5: cycles —— 循环引用图反复构建 + 丢弃
// 每轮建两个节点并互相引用（p.peer = q; q.peer = p），读完立即丢弃。
// 预期：cjc tracing GC 能回收不可达环，RSS 平稳；cjwasm 无 GC，
//       每轮 2 个对象线性泄漏，直至 16MB 上限 trap —— GC 缺失的直观证据。

class CycleNode {
    var value: Int64
    var peer: Option<CycleNode>
    init(value: Int64) {
        this.value = value
        this.peer = None
    }
}

main(): Unit {
    // run.py 会按此行改写 N
    let n: Int64 = 100000
    var checksum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let p = CycleNode(i)
        let q = CycleNode(i + 1)
        p.peer = Some(q)
        q.peer = Some(p)
        // 穿过环读一次，保证引用关系没有被优化掉
        match (p.peer) {
            case Some(x) => checksum = checksum + x.value
            case None => checksum = checksum + 0
        }
        i = i + 1
        if (i % 50000 == 0) {
            println("MILE cycles i=${i} checksum=${checksum}")
        }
    }
    println("BENCH cycles n=${n} checksum=${checksum}")
}
