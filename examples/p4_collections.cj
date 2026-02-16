// P4 特性验证: HashMap补全 / HashSet / Range属性

// =============================================================
// HashMap: put, get, containsKey, remove, size
// =============================================================

func testHashMapFull(): Int64 {
    var result: Int64 = 0

    let map = HashMap<Int64, Int64>()
    map.put(10, 100)
    map.put(20, 200)
    map.put(30, 300)

    // size
    result = result + map.size  // +3

    // get
    result = result + map.get(10)  // +100
    result = result + map.get(20)  // +200

    // containsKey
    if (map.containsKey(30) != 0) {
        result = result + 1  // +1
    }
    if (map.containsKey(99) != 0) {
        result = result + 10000  // 不应执行
    }

    // remove
    let removed = map.remove(10)
    result = result + removed  // +100
    result = result + map.size  // +2

    return result  // 3 + 100 + 200 + 1 + 100 + 2 = 406
}

// =============================================================
// HashSet: add, contains, size
// =============================================================

func testHashSet(): Int64 {
    var result: Int64 = 0

    let set = HashSet<Int64>()
    set.add(1)
    set.add(2)
    set.add(3)
    set.add(2)  // 重复，应被忽略

    // size
    result = result + set.size  // +3

    // contains
    if (set.contains(1) != 0) {
        result = result + 10  // +10
    }
    if (set.contains(99) != 0) {
        result = result + 10000  // 不应执行
    }

    return result  // 3 + 10 = 13
}

// =============================================================
// ArrayList: size 属性方法
// =============================================================

func testArrayListSize(): Int64 {
    let list = ArrayList<Int64>()
    list.append(10)
    list.append(20)
    list.append(30)

    return list.size  // 3
}

// =============================================================
// Range 属性: start, end
// =============================================================

func testRangeProps(): Int64 {
    let r = 5..10
    let s = r.start  // 5
    let e = r.end    // 10
    return s + e     // 15
}

// =============================================================
// 主函数
// =============================================================

main(): Int64 {
    let a = testHashMapFull()    // 406
    let b = testHashSet()        // 13
    let c = testArrayListSize()  // 3
    let d = testRangeProps()     // 15

    println(a)
    println(b)
    println(c)
    println(d)

    @Assert(a, 406)
    @Assert(b, 13)
    @Assert(c, 3)
    @Assert(d, 15)

    let total = a + b + c + d
    @Assert(total, 437)

    return total
    // 预期输出: 437
}
