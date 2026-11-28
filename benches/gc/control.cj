// GC 基准 5/5: control —— 无堆分配的纯计算循环
// 校准两编译器在"完全相同的算术负载"下的执行速度差，排除 cjwasm 因
// 代码生成较慢而污染 GC 指标。两边 checksum 必须逐位一致。

func mix(a: Int64, b: Int64): Int64 {
    return (a * 31 + b) % 1000000007
}

main(): Unit {
    // run.py 会按此行改写 N
    let n: Int64 = 5000000
    var acc: Int64 = 1
    var i: Int64 = 0
    while (i < n) {
        acc = mix(acc, i)
        i = i + 1
    }
    println("BENCH control n=${n} checksum=${acc}")
}
