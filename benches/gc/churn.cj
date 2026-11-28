// GC 基准 1/5: churn —— 高频分配即弃（allocation churn）
// 每轮 new 一个 2 字段对象，读一次即丢。测分配吞吐与内存水位。
// 预期：cjc 的 tracing GC 让 RSS 保持平稳；cjwasm 的 bump 分配器只涨不降，
//       线性内存随分配次数增长，直到 256 页（16MB）上限后 trap。
// 所有类方法用顶层函数替代，保证 cjc 1.0.5 与 cjwasm 都能接受。

class Cell {
    var a: Int64
    var b: Int64
    init(a: Int64, b: Int64) {
        this.a = a
        this.b = b
    }
}

main(): Unit {
    // run.py 会按此行改写 N 做 stress（触发 16MB trap）测试
    let n: Int64 = 400000
    var checksum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let c = Cell(i, i * 3)
        checksum = checksum + c.a + c.b
        i = i + 1
        if (i % 100000 == 0) {
            println("MILE churn i=${i} checksum=${checksum}")
        }
    }
    println("BENCH churn n=${n} checksum=${checksum}")
}
