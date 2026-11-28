// String.indexOf 返回 Option<Int64> 回归（batch-51 w51a）
// 与 cjc 对齐：命中 Some(UTF-8 字节偏移)，未命中 None；
// 支持 indexOf(str) 与 indexOf(str, fromIndex) 两参重载（fromIndex 为字节偏移，负数按 0 处理）。
main() {
    let s = "hello world hello"
    // 命中 / 未命中
    println(s.indexOf("world"))       // Some(6)
    println(s.indexOf("zzz"))         // None
    // 空 needle：在 [0, size] 内返回 Some(fromIndex)
    println(s.indexOf(""))            // Some(0)
    println(s.indexOf("", 5))         // Some(5)
    // 两参重载：fromIndex 起点
    println(s.indexOf("hello"))       // Some(0)
    println(s.indexOf("hello", 1))    // Some(12)
    println(s.indexOf("hello", 12))   // Some(12)
    println(s.indexOf("hello", 13))   // None
    println(s.indexOf("hello", -5))   // Some(0)（负数按 0）
    // 多字节：以 size 差值做编码无关校验，保证 stdout 与 cjc 一致
    // （cjwasm 字符串字面量为双倍编码，绝对字节偏移与 cjc 不同，见 followups/w51b）
    let m = "1に三"
    println((m.indexOf("に") ?? -1) == 1)
    println((m.indexOf("三") ?? -1) == (m.size - "三".size))
    println((m.indexOf("z") ?? -1) == -1)
    println("PASS")
    // 预期输出: PASS
}
