// w54c 回归：Option 谓词/map、整数类型 Max/Min、round/pow、float inf/nan 打印。
// cjc 1.0.5 可编译；cjwasm(CHIR) 输出须与 cjc 字节一致。
import std.math.*

main(): Int64 {
    // ── S2 Option.isSome / isNone / map / getOrThrow ──
    let a: Option<Int64> = Some(42)
    let b: Option<Int64> = None
    println(a.isSome())
    println(a.isNone())
    println(b.isSome())
    println(b.isNone())
    println(a ?? 0)
    println(b ?? -1)
    println(a.getOrThrow())
    let m: Int64 = (a.map({x => x * 2})) ?? 0
    let n: Int64 = (b.map({x => x * 2})) ?? -1
    println(m)
    println(n)

    // ── S7 整数类型 Max/Min ──
    println(Int64.Max)
    println(Int64.Min)
    println(Int32.Max)
    println(Int32.Min)
    println(Int16.Max)
    println(Int16.Min)
    println(Int8.Max)
    println(Int8.Min)
    println(UInt8.Max)
    println(UInt16.Max)
    println(UInt32.Max)
    println(UInt64.Max)

    // ── S8 round（银行家舍入）/ pow（整数指数精确）──
    println(round(0.5))
    println(round(1.5))
    println(round(2.5))
    println(round(3.5))
    println(round(-2.5))
    println(pow(2.0, 10.0))
    println(pow(2.0, 62.0))

    // ── S13 float 非有限值打印 ──
    println(1e300 * 1e300)
    println(-1e300 * 1e300)
    println(0.0 / 0.0)
    return 0  // 预期输出: 0
}
