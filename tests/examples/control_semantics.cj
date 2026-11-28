// 回归测试（w54d）：控制流语义
//   S9  match 守卫绑定 + 分支结果类型（字符串结果不得按指针打印）
//   S10 捕获不可变局部变量的 λ（直接调用 f(...)）
//   S14 try/catch/finally 的表达式值 = catch 块值，finally 必执行
// 预期输出: 127
func apply(f: (Int64) -> Int64, v: Int64): Int64 {
    return f(v)
}

main(): Int64 {
    var s = 0

    // S9：守卫绑定（首个分支为字面量，结果类型取字符串）
    let y = match (3) {
        case 1 => "one"
        case n where n > 2 => "big"
        case _ => "other"
    }
    if (y == "big") { s = s + 1 }

    // 守卫不成立时落到通配分支
    let z = match (1) {
        case n where n > 2 => "big"
        case _ => "other"
    }
    if (z == "other") { s = s + 2 }

    // S10：捕获不可变局部变量 base
    let base = 100
    let f = {v: Int64 => v + base}
    if (f(5) == 105) { s = s + 4 }

    // 无捕获 λ 经函数形参调用仍正常
    let g = {v: Int64 => v * 3}
    if (apply(g, 7) == 21) { s = s + 8 }

    // S14：try 表达式值 = catch 值；finally 必执行
    let t = try {
        throw Exception("bad")
    } catch (e: Exception) {
        42
    } finally {
        s = s + 16
    }
    if (t == 42) { s = s + 32 }

    // try 值为字符串时（catch 块结果类型推断，不能按指针打印）
    let ts = try {
        throw Exception("bad")
    } catch (e: Exception) {
        "caught"
    }
    if (ts == "caught") { s = s + 64 }

    return s
}
