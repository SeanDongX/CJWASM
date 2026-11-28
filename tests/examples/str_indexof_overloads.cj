// String API 余项回归（batch-52 w52b）：indexOf(UInt8) / lastIndexOf(str|UInt8)[, fromIndex]
// 语义（cjc 1.0.5 实测）：
//   indexOf(x[,from])      自 max(from,0) 起正向找首个匹配（空 needle → Some(from)）
//   lastIndexOf(x[,from])  在 suffix this[max(from,0)..] 内找**最右**出现；空 needle → Some(size)
// 二者均返回 Option<Int64>；from>size → None；负数 from 按 0。
// cjc 1.0.5 无 Rune 重载（`s.indexOf(r'a')` 是编译错误），故只测 UInt8/String。
// stdout 与 cjc 逐字节一致。

func v(o: Option<Int64>): Int64 {
    return o ?? -1
}

main(): Int64 {
    let s = "abcabc"   // len 6；'a'@0,3  'b'@1,4  'c'@2,5  "abc"@0,3  "bc"@1,4
    var ok = 0
    // ── indexOf(UInt8[, fromIndex]) ──
    if (v(s.indexOf(b'a')) == 0) { ok = ok + 1 }
    if (v(s.indexOf(b'c')) == 2) { ok = ok + 1 }
    if (v(s.indexOf(b'c', 3)) == 5) { ok = ok + 1 }
    if (v(s.indexOf(b'a', 1)) == 3) { ok = ok + 1 }
    if (v(s.indexOf(b'z')) == -1) { ok = ok + 1 }        // None
    if (v(s.indexOf(b'z', 6)) == -1) { ok = ok + 1 }     // None
    // ── indexOf(String, fromIndex) ──
    if (v(s.indexOf("bc", 2)) == 4) { ok = ok + 1 }
    if (v(s.indexOf("abc", 5)) == -1) { ok = ok + 1 }    // None
    // ── lastIndexOf(String[, fromIndex]) ──
    if (v(s.lastIndexOf("abc")) == 3) { ok = ok + 1 }
    if (v(s.lastIndexOf("abc", 3)) == 3) { ok = ok + 1 }
    if (v(s.lastIndexOf("abc", 4)) == -1) { ok = ok + 1 }  // suffix 无匹配 → None
    if (v(s.lastIndexOf("abc", -5)) == 3) { ok = ok + 1 }  // 负数按 0
    if (v(s.lastIndexOf("")) == 6) { ok = ok + 1 }         // 空 needle → size
    if (v(s.lastIndexOf("", 6)) == 6) { ok = ok + 1 }
    if (v(s.lastIndexOf("", 7)) == -1) { ok = ok + 1 }
    // ── lastIndexOf(UInt8[, fromIndex]) ──
    if (v(s.lastIndexOf(b'a')) == 3) { ok = ok + 1 }
    if (v(s.lastIndexOf(b'a', 1)) == 3) { ok = ok + 1 }
    if (v(s.lastIndexOf(b'a', 4)) == -1) { ok = ok + 1 }   // suffix "bc" 无 'a'
    if (v(s.lastIndexOf(b'c', 3)) == 5) { ok = ok + 1 }
    if (v(s.lastIndexOf(b'z')) == -1) { ok = ok + 1 }
    println("PASS")
    return ok  // 预期输出: 20
}
