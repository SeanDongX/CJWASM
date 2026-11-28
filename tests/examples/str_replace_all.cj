// String.replace 全量替换回归（batch-50 w50a）
// 语义与 cjc / Rust str::replace 一致：
//   - 替换全部非重叠匹配（"aXbXc".replace("X","-") == "a-b-c"）
//   - 空 old 在每个字符边界（含首尾）插入 new
//   - old 非空但无匹配时原样返回
//   - 替换后再扫描从匹配段之后继续（"abab".replace("ab","aab") == "aabaab"）
//
// 注：@Assert 对 String 只做指针比较，故用 size + indexOf 组合判定内容相等。
func eqstr(a: String, b: String): Bool {
    return a.size() == b.size() && a.indexOf(b) == 0
}

main() {
    // 多匹配：首匹配修复的核心回归
    @Assert(eqstr("aXbXc".replace("X", "-"), "a-b-c"), true)
    @Assert(eqstr("hello".replace("l", "LL"), "heLLLLo"), true)
    // 非重叠：每个 old 独立消费
    @Assert(eqstr("aaaa".replace("aa", "b"), "bb"), true)
    @Assert(eqstr("XXXX".replace("XX", "X"), "XX"), true)
    // 替换成空串（删除）
    @Assert(eqstr("aXbXc".replace("X", ""), "abc"), true)
    @Assert(eqstr("ababab".replace("ab", ""), ""), true)
    // 无匹配 → 原样返回
    @Assert(eqstr("abc".replace("abcd", "Z"), "abc"), true)
    // 空 old → 每个字节边界插入
    @Assert(eqstr("abc".replace("", "-"), "-a-b-c-"), true)
    @Assert(eqstr("".replace("", "-"), "-"), true)
    @Assert(eqstr("ab".replace("", "XY"), "XYaXYbXY"), true)
    // 替换串含原串片段，不得死循环 / 不得重扫已替换内容
    @Assert(eqstr("abab".replace("ab", "aab"), "aabaab"), true)
    @Assert(eqstr("aXbXc".replace("Xb", "-"), "a-Xc"), true)
    println("replace_all tests passed")
    println("PASS")
    // 预期输出: PASS
}
