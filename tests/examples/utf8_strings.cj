// UTF-8 字符串字面量：多字节字节保真回归
// 确保 String 在 wasm 内存中是原始 UTF-8 字节序列，
// size() 返回字节数（与源文件字节一致），无 mojibake 重编码。
//
// 返回值 = 各字面量字节数之和（58）；任何一处编码错误都会改变该值。
// 内容用 == 逐字节核对，不等则额外 +1000 使结果偏离预期。

main(): Int64 {
    // 纯多字节（中文 3 字节/字）
    let s = "中文"                       // 6
    // emoji（U+1F64F U+1F3FF = 8 字节组合 emoji）
    let e = "🙏🏿"                        // 8
    // ASCII + 多字节 + emoji 混合
    let mix = "a中🙏🏿z"                  // 1 + 3 + 8 + 1 = 13
    // 原始字符串（含内嵌引号与多字节）
    let raw = #"say "hi" 中"#             // 12
    // 转义引号 + 多字节
    let esc = "引\"号中"                  // 3 + 1 + 3 + 3 = 10
    // 字符串插值 + 多字节
    let word = "中"
    let interp = "前${word}后"            // 3 + 3 + 3 = 9

    var total = s.size + e.size + mix.size + raw.size + esc.size + interp.size

    if (s != "中文") { total += 1000 }
    if (e != "🙏🏿") { total += 1000 }
    if (mix != "a中🙏🏿z") { total += 1000 }
    if (esc != "引\"号中") { total += 1000 }
    // 插值结果的字节已由 size() 覆盖；w52a：String ==/!= 现按内容（字节）比较，
    // 因此插值构造的堆串可与字面量直接比较（此前指针比较会误判）。
    if (!("前${word}后" == "前中后")) { total += 1000 }
    if (interp != "前中后") { total += 1000 }

    return total  // 预期输出: 58
}
