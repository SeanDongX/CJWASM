// P6: 新特性全量测试
// 覆盖: do-while, const, 主构造函数, inout 参数, try-with-resources,
//       可选链(?.), 尾随闭包, !in 运算符

// =============================================================
// 1. do-while 循环
// =============================================================

func test_do_while_basic(): Int64 {
    var sum = 0
    var i = 1
    do {
        sum = sum + i
        i = i + 1
    } while (i <= 5)
    return sum  // 1+2+3+4+5 = 15
}

func test_do_while_once(): Int64 {
    // do-while 至少执行一次，即使条件一开始就为 false
    var count = 0
    do {
        count = count + 1
    } while (false)
    return count  // 1
}

func test_do_while_nested(): Int64 {
    var result = 0
    var i = 1
    do {
        var j = 1
        do {
            result = result + i * j
            j = j + 1
        } while (j <= 3)
        i = i + 1
    } while (i <= 2)
    // i=1: j=1(1) + j=2(2) + j=3(3) = 6
    // i=2: j=1(2) + j=2(4) + j=3(6) = 12
    return result  // 18
}

// =============================================================
// 2. const 声明（编译期常量）
// =============================================================

func test_const_basic(): Int64 {
    const MAX_VALUE: Int64 = 100
    const OFFSET = 42
    return MAX_VALUE + OFFSET  // 142
}

func test_const_in_expr(): Int64 {
    const BASE = 10
    const MULTIPLIER = 5
    const RESULT = BASE * MULTIPLIER
    return RESULT  // 50
}

func test_const_with_var(): Int64 {
    const LIMIT = 100
    var sum = 0
    var i = 1
    while (i <= 10) {
        sum = sum + i
        i = i + 1
    }
    // sum = 55, LIMIT = 100
    return LIMIT - sum  // 45
}

// =============================================================
// 3. 主构造函数
// =============================================================

class Point(var x: Int64, var y: Int64) {
    func sum(): Int64 {
        return this.x + this.y
    }

    func distSq(): Int64 {
        return this.x * this.x + this.y * this.y
    }
}

func test_primary_ctor(): Int64 {
    let p = Point(3, 4)
    return p.sum()  // 7
}

func test_primary_ctor_fields(): Int64 {
    let p = Point(5, 12)
    return p.distSq()  // 25 + 144 = 169
}

class Pair(var first: Int64, var second: Int64) {
    func swap(): Pair {
        return Pair(this.second, this.first)
    }
}

func test_primary_ctor_method(): Int64 {
    let p = Pair(10, 20)
    let q = p.swap()
    return q.first + q.second * 10  // 20 + 10*10 = 120
}

// =============================================================
// 4. inout 参数
// =============================================================

func increment(inout x: Int64): Int64 {
    return x + 1
}

func test_inout_basic(): Int64 {
    var val = 10
    let result = increment(val)
    return result  // 11
}

func addBoth(inout a: Int64, inout b: Int64): Int64 {
    return a + b
}

func test_inout_multi(): Int64 {
    var x = 30
    var y = 12
    return addBoth(x, y)  // 42
}

// =============================================================
// 5. try-with-resources
// =============================================================

func test_try_with_resources(): Int64 {
    var result = 0
    try (let res = 42) {
        result = res
    }
    return result  // 42
}

func test_try_with_resources_catch(): Int64 {
    var result = 0
    try (let val = 100) {
        result = val
    } catch (e: Exception) {
        result = 0
    }
    return result  // 100
}

func test_try_with_resources_multi(): Int64 {
    var result = 0
    try (let a = 10, let b = 20) {
        result = a + b
    }
    return result  // 30
}

// =============================================================
// 6. 可选链 (?.)
// =============================================================

class Box(var value: Int64) {
}

func test_optional_chain(): Int64 {
    let b = Box(99)
    let v = b?.value
    return v  // 99
}

// =============================================================
// 7. 尾随闭包 (trailing closure)
// =============================================================

func apply(x: Int64, f: (Int64) -> Int64): Int64 {
    return f(x)
}

func test_trailing_closure_basic(): Int64 {
    // apply(10) { x => x * 3 } 等价于 apply(10, { x: Int64 => x * 3 })
    let result = apply(10) { x: Int64 => x * 3 }
    return result  // 30
}

func combine(a: Int64, b: Int64, f: (Int64, Int64) -> Int64): Int64 {
    return f(a, b)
}

func test_trailing_closure_multi_param(): Int64 {
    let result = combine(7, 8) { x: Int64, y: Int64 => x * y }
    return result  // 56
}

func transform(x: Int64, f: (Int64) -> Int64): Int64 {
    return f(x)
}

func test_trailing_closure_chain(): Int64 {
    // 连续调用
    let r1 = transform(5) { x: Int64 => x + 10 }    // 15
    let r2 = transform(r1) { x: Int64 => x * 2 }     // 30
    return r2  // 30
}

// =============================================================
// 8. !in 运算符
// =============================================================

func test_not_in_string(): Int64 {
    let s = "hello world"
    var result = 0
    if ("xyz" !in s) {
        result = result + 1  // "xyz" 不在 s 中 → +1
    }
    if ("hello" !in s) {
        result = result + 100  // "hello" 在 s 中 → 不应执行
    }
    return result  // 1
}

func test_not_in_hashset(): Int64 {
    let set = HashSet<Int64>()
    set.add(10)
    set.add(20)
    set.add(30)

    var result = 0
    if (99 !in set) {
        result = result + 1  // 99 不在集合中 → +1
    }
    if (10 !in set) {
        result = result + 100  // 10 在集合中 → 不应执行
    }
    return result  // 1
}

// =============================================================
// 综合测试
// =============================================================

main() {
    // --- 1. do-while ---
    let dw1 = test_do_while_basic()
    @Assert(dw1, 15)

    let dw2 = test_do_while_once()
    @Assert(dw2, 1)

    let dw3 = test_do_while_nested()
    @Assert(dw3, 18)

    // --- 2. const ---
    let c1 = test_const_basic()
    @Assert(c1, 142)

    let c2 = test_const_in_expr()
    @Assert(c2, 50)

    let c3 = test_const_with_var()
    @Assert(c3, 45)

    // --- 3. 主构造函数 ---
    let pc1 = test_primary_ctor()
    @Assert(pc1, 7)

    let pc2 = test_primary_ctor_fields()
    @Assert(pc2, 169)

    let pc3 = test_primary_ctor_method()
    @Assert(pc3, 120)

    // --- 4. inout ---
    let io1 = test_inout_basic()
    @Assert(io1, 11)

    let io2 = test_inout_multi()
    @Assert(io2, 42)

    // --- 5. try-with-resources ---
    let tw1 = test_try_with_resources()
    @Assert(tw1, 42)

    let tw2 = test_try_with_resources_catch()
    @Assert(tw2, 100)

    let tw3 = test_try_with_resources_multi()
    @Assert(tw3, 30)

    // --- 6. 可选链 ---
    let oc1 = test_optional_chain()
    @Assert(oc1, 99)

    // --- 7. 尾随闭包 ---
    let tc1 = test_trailing_closure_basic()
    @Assert(tc1, 30)

    let tc2 = test_trailing_closure_multi_param()
    @Assert(tc2, 56)

    let tc3 = test_trailing_closure_chain()
    @Assert(tc3, 30)

    // --- 8. !in ---
    let ni1 = test_not_in_string()
    @Assert(ni1, 1)

    let ni2 = test_not_in_hashset()
    @Assert(ni2, 1)
    
    println("PASS")
    // 预期输出: PASS
}
