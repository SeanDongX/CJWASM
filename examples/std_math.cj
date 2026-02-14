// Phase 7.1 + 7.3: std.core I/O 与 std.math 数学函数测试
//
// 测试: print/println/eprint/eprintln + sqrt/floor/ceil/trunc/sin/cos/exp/log/pow + 常数

func main(): Int64 {
    // ── 1. print (无换行) 与 println (有换行) ──
    print("Hello ")
    println("World")      // 输出: Hello World

    // ── 2. eprint / eprintln (输出到 stderr) ──
    eprint("[stderr] ")
    eprintln("error message")

    // ── 3. println 空行 ──
    println()

    // ── 4. WASM 原生数学指令 ──
    // sqrt(16.0) = 4.0
    let sq = sqrt(16.0)
    let sq_i = nearest(sq) as Int64  // 4

    // floor(3.7) = 3.0, ceil(3.2) = 4.0, trunc(-2.9) = -2.0
    let fl = floor(3.7) as Int64   // 3
    let ce = ceil(3.2) as Int64    // 4
    let tr = trunc(-2.9) as Int64  // -2
    let ne = nearest(2.5) as Int64 // 2 (banker's rounding)

    println("sqrt(16) = ${sq_i}")
    println("floor(3.7) = ${fl}")
    println("ceil(3.2) = ${ce}")
    println("trunc(-2.9) = ${tr}")
    println("nearest(2.5) = ${ne}")

    // ── 5. 三角函数 (泰勒级数) ──
    // sin(0) ≈ 0, cos(0) ≈ 1
    let sin0 = nearest(sin(0.0)) as Int64  // 0
    let cos0 = nearest(cos(0.0)) as Int64  // 1

    // sin(PI/2) ≈ 1
    let sin_hp = nearest(sin(PI / 2.0)) as Int64  // 1

    println("sin(0) = ${sin0}")
    println("cos(0) = ${cos0}")
    println("sin(PI/2) = ${sin_hp}")

    // ── 6. exp / log / pow ──
    // exp(0) = 1, exp(1) ≈ 2.718
    let exp0 = nearest(exp(0.0)) as Int64  // 1
    let exp1 = nearest(exp(1.0)) as Int64  // 3 (nearest rounds 2.718... to 3)

    // log(E) ≈ 1.0
    let logE = nearest(log(E)) as Int64    // 1

    // pow(2, 10) = 1024
    let p = nearest(pow(2.0, 10.0)) as Int64  // 1024

    println("exp(0) = ${exp0}")
    println("exp(1) = ${exp1}")
    println("log(E) = ${logE}")
    println("pow(2, 10) = ${p}")

    // ── 7. 常数 ──
    let pi_i = nearest(PI * 100.0) as Int64   // 314
    let e_i = nearest(E * 100.0) as Int64      // 272
    println("PI*100 = ${pi_i}")
    println("E*100 = ${e_i}")

    // ── 8. 汇总 ──
    // sq_i(4) + fl(3) + ce(4) + tr(-2) + ne(2) + sin0(0) + cos0(1) + sin_hp(1)
    // + exp0(1) + exp1(3) + logE(1) + p(1024) + pi_i(314) + e_i(272) = 1628
    let total = sq_i + fl + ce + tr + ne + sin0 + cos0 + sin_hp + exp0 + exp1 + logE + p + pi_i + e_i
    println("total = ${total}")

    return total  // 预期输出: 1628
}
