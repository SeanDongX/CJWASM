// P3 特性验证: ArrayList/HashMap/extend/方法重载/嵌套数组

// =============================================================
// ArrayList
// =============================================================

func testArrayList(): Int64 {
    var result: Int64 = 0

    let list = ArrayList<Int64>()
    list.append(10)
    list.append(20)
    list.append(30)

    // get
    result = result + list.get(0)  // +10
    result = result + list.get(1)  // +20
    result = result + list.get(2)  // +30

    // set
    list.set(1, 99)
    result = result + list.get(1)  // +99

    // remove
    let removed = list.remove(0)
    result = result + removed  // +10

    return result  // 10 + 20 + 30 + 99 + 10 = 169
}

// =============================================================
// HashMap
// =============================================================

func testHashMap(): Int64 {
    var result: Int64 = 0

    let map = HashMap<Int64, Int64>()
    map.put(1, 100)
    map.put(2, 200)
    map.put(3, 300)

    // containsKey
    if (map.containsKey(1) != 0) {
        result = result + 1  // +1
    }
    if (map.containsKey(99) != 0) {
        result = result + 1000  // 不应执行
    }

    return result  // 1
}

// =============================================================
// extend (为自定义类型添加方法)
// =============================================================

struct Vec2 {
    var x: Int64;
    var y: Int64;
}

extend Vec2 {
    func length(): Int64 {
        return this.x * this.x + this.y * this.y
    }

    func add(other: Vec2): Vec2 {
        return Vec2 { x: this.x + other.x, y: this.y + other.y }
    }
}

func testExtend(): Int64 {
    let v1 = Vec2 { x: 3, y: 4 }
    let v2 = Vec2 { x: 1, y: 2 }

    let len = v1.length()  // 9 + 16 = 25

    let v3 = v1.add(v2)
    let sum = v3.x + v3.y  // 4 + 6 = 10

    return len + sum  // 25 + 10 = 35
}

// =============================================================
// 方法重载 (同名不同参数)
// =============================================================

func compute(a: Int64): Int64 {
    return a * a
}

func compute(a: Int64, b: Int64): Int64 {
    return a + b
}

func compute(a: Int64, b: Int64, c: Int64): Int64 {
    return a + b + c
}

func testOverload(): Int64 {
    let r1 = compute(5)        // 25
    let r2 = compute(3, 4)     // 7
    let r3 = compute(1, 2, 3)  // 6
    return r1 + r2 + r3  // 38
}

// =============================================================
// 主函数
// =============================================================

main(): Int64 {
    let a = testArrayList()   // 169
    let b = testHashMap()     // 1
    let c = testExtend()      // 35
    let d = testOverload()    // 38

    println(a)
    println(b)
    println(c)
    println(d)

    @Assert(a, 169)
    @Assert(b, 1)
    @Assert(c, 35)
    @Assert(d, 38)

    let total = a + b + c + d
    @Assert(total, 243)

    return total
    // 预期输出: 243
}
