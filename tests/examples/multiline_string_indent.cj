// 多行字符串缩进语义回归（batch-52 w52b）
// 与 cjc 1.0.5 对齐：仅移除起始 """ 后紧跟的第一个换行；
// 其余内容——每行前导空格/Tab、空行、结尾 """ 前的缩进空白——全部原样保留。
// cjc 不 strip 公共缩进。

func sumSizes(): Int64 {
    // 首行换行被移除；4 空格前导与末尾 4 空格缩进保留 → "    abc\n    def\n    " = 20
    let a = """
    abc
    def
    """
    // Tab 前导与末尾 Tab 保留 → "\tx\n\t" = 4
    let b = """
	x
	"""
    // 空行与末尾缩进保留 → "\n    " = 5
    let c = """

    """
    // 无缩进、仅保留行尾换行 → "noindent\n" = 9
    let d = """
noindent
"""
    return a.size + b.size + c.size + d.size
}

main(): Int64 {
    return sumSizes()  // 预期输出: 38
}
