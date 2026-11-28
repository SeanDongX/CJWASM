// w54a: Array 内建方法（contains/indexOf/reverse/slice/concat/fill）
// + std.collection 容器 shim（ArrayList.add/index、HashMap 下标写/size）
//
// cjc 1.0.5 语义要点：
//   - indexOf 返回 Option<Int64>（命中 Some(第一个匹配下标)，未命中 None）
//   - 元素相等：Int/Float 按值，String 按内容
//   - reverse/fill 原地修改，返回 Unit
//   - slice(start, length) 第二参数是长度（不是结束下标）
//   - ArrayList 的 add 与下标读写走集合运行时

import std.collection.*

main(): Int64 {
    // contains / indexOf
    let a = [10, 20, 30, 20]
    println(a.contains(20))
    println(a.contains(99))
    match (a.indexOf(20)) {
        case Some(i) => println("first=${i}")
        case None => println("first none")
    }
    match (a.indexOf(999)) {
        case Some(i) => println("hit=${i}")
        case None => println("miss")
    }
    // String 元素按内容比较
    let s = ["aa", "bb", "cc"]
    println(s.contains("bb"))
    println(s.contains("zz"))
    match (s.indexOf("cc")) {
        case Some(i) => println("si=${i}")
        case None => println("si none")
    }
    // 空数组边界
    let e: Array<Int64> = []
    println(e.contains(1))
    match (e.indexOf(1)) {
        case Some(i) => println("ehit")
        case None => println("emiss")
    }
    // reverse（原地）
    let r = [5, 3, 1, 4, 2]
    r.reverse()
    for (x in r) {
        print("${x},")
    }
    println("")
    // slice(start, length)
    let sl = [0, 1, 2, 3, 4].slice(1, 3)
    println("${sl.size}:${sl[0]},${sl[1]},${sl[2]}")
    let sz = [0, 1, 2, 3, 4].slice(2, 0)
    println("zero=${sz.size}")
    let se = e.slice(0, 0)
    println("empty=${se.size}")
    // concat
    let c = [1, 2].concat([3, 4])
    println("${c.size}:${c[0]}:${c[3]}")
    let ce = [1, 2].concat(e)
    println("ce=${ce.size}:${ce[0]}:${ce[1]}")
    let ec = e.concat([7, 8])
    println("ec=${ec.size}:${ec[0]}:${ec[1]}")
    // fill（原地）
    let f = Array<Int64>(3, repeat: 1)
    f.fill(5)
    println("${f[0]},${f[1]},${f[2]}")
    // ArrayList: add / size / 下标读写
    let list = ArrayList<Int64>()
    list.add(10)
    list.add(20)
    list.add(30)
    println(list.size)
    println(list[0])
    println(list[2])
    list[1] = 99
    println(list[1])
    // HashMap: 下标写 + size
    let h = HashMap<String, Int64>()
    h["x"] = 5
    h["y"] = 6
    println(h.size)
    println(h["x"])
    return 0
}
// 预期输出: 0
