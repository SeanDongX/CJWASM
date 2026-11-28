// 回归测试：Array<T>(n, repeat: v) 命名实参初始化。
// 缺陷（batch-49 w49a）：parser 把 `repeat:` 放进 named_args，而 lower_expr 的
// Array 构造分支此前要求 args.len()==2 且完全忽略 named_args，导致固定初值从未
// 写入（数组长度/内容全为 0 / 垃圾）。cjc 语义：Array<T>(n, repeat: v) = n 个 v。
// 同文件顺带回归位置形态 Array<Int64>(n, v)（cjwasm 支持的值填充扩展）。
main(): Int64 {
    // Float64 repeat: 3 个 2.5，和 = 7.5；×10 取整避免浮点打印差异
    let a = Array<Float64>(3, repeat: 2.5)
    var s: Float64 = 0.0
    var i: Int64 = 0
    while (i < 3) {
        s = s + a[i]
        i = i + 1
    }

    // Int64 repeat: 4 个 7，和 = 28，长度 4
    let b = Array<Int64>(4, repeat: 7)
    var bs: Int64 = 0
    i = 0
    while (i < 4) {
        bs = bs + b[i]
        i = i + 1
    }

    // 位置形态回归：Array<Int64>(5, 7) — 5 个 7，和 = 35，长度 5
    let c = Array<Int64>(5, 7)
    var cs: Int64 = 0
    i = 0
    while (i < 5) {
        cs = cs + c[i]
        i = i + 1
    }

    // Bool repeat：3 个 true
    let d = Array<Bool>(3, repeat: true)
    var ok: Int64 = 0
    if (d[0] && d[1] && d[2]) { ok = 1 }

    // 零长度 repeat
    let e = Array<Int64>(0, repeat: 9)

    // 75 + 28 + 4 + 35 + 5 + 1 + 0 = 148
    return Int64(s * 10.0) + bs + b.size + cs + c.size + ok + e.size  // 预期输出: 148
}
