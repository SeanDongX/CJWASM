// P2 核心语法扩展 综合示例
// 覆盖 P2.4 ~ P2.10 所有已实现特性

// =============================================================
// P2.4: static 类成员
// =============================================================

class Counter {
    var count: Int64;

    init(n: Int64) {
        this.count = n
    }

    static func create(): Counter {
        return Counter(0)
    }

    func inc(): Counter {
        this.count = this.count + 1
        return this
    }

    func getCount(): Int64 {
        return this.count
    }
}

func testStatic(): Int64 {
    // 静态方法调用 + 链式方法调用
    let c = Counter.create()
    let c2 = c.inc().inc().inc()
    return c2.getCount()  // 3
}

// =============================================================
// P2.5: 抽象方法（无 body 的 func 声明）
// =============================================================

open class Shape {
    func area(): Int64;   // 抽象方法（无 body，以 ; 结尾）
}

class Circle <: Shape {
    var radius: Int64;
    init(r: Int64) { this.radius = r }
    func area(): Int64 {
        return this.radius * this.radius * 3
    }
}

class Rect <: Shape {
    var w: Int64;
    var h: Int64;
    init(w: Int64, h: Int64) { this.w = w; this.h = h }
    func area(): Int64 {
        return this.w * this.h
    }
}

func testAbstract(): Int64 {
    let c = Circle(5)
    let r = Rect(3, 4)
    return c.area() + r.area()  // 75 + 12 = 87
}

// =============================================================
// P2.6: for-in 带步长
// =============================================================

func testForStep(): Int64 {
    // 偶数求和: 0 + 2 + 4 + 6 + 8 + 10 = 30
    var sum1: Int64 = 0
    for (i in 0..=10 : 2) {
        sum1 = sum1 + i
    }

    // 步长 3: 1 + 4 + 7 = 12
    var sum2: Int64 = 0
    for (i in 1..10 : 3) {
        sum2 = sum2 + i
    }

    return sum1 + sum2  // 30 + 12 = 42
}

// =============================================================
// P2.7: 动态 Array 构造
// =============================================================

func testDynArray(): Int64 {
    // 常量初始化: 5 个元素全为 7
    let arr1 = Array<Int64>(5, 7)
    var sum1: Int64 = 0
    for (i in 0..5) {
        sum1 = sum1 + arr1[i]
    }
    // sum1 = 35

    // Lambda 初始化: [0, 1, 4, 9, 16]
    let arr2 = Array<Int64>(5, { i: Int64 => i * i })
    var sum2: Int64 = 0
    for (i in 0..5) {
        sum2 = sum2 + arr2[i]
    }
    // sum2 = 0 + 1 + 4 + 9 + 16 = 30

    // 零长度数组
    let empty = Array<Int64>(0, 0)

    return sum1 + sum2 + empty.size  // 35 + 30 + 0 = 65
}

// =============================================================
// P2.8: Array 实例方法 (clone, isEmpty, slice)
// =============================================================

func testArrayMethods(): Int64 {
    let arr = [10, 20, 30, 40, 50]
    let empty = Array<Int64>(0, 0)
    var result: Int64 = 0

    // isEmpty
    if (arr.isEmpty()) {
        result = result + 1000  // 不应执行
    }
    if (empty.isEmpty()) {
        result = result + 1  // +1
    }

    // clone
    let cloned = arr.clone()
    result = result + cloned[0]   // +10
    result = result + cloned[4]   // +50
    result = result + cloned.size // +5

    // slice(start, end)
    let sliced = arr.slice(1, 4)  // [20, 30, 40]
    result = result + sliced.size   // +3
    result = result + sliced[0]     // +20
    result = result + sliced[1]     // +30
    result = result + sliced[2]     // +40

    return result  // 1 + 10 + 50 + 5 + 3 + 20 + 30 + 40 = 159
}

// =============================================================
// P2.9: 命名参数 (name!: Type = default)
// =============================================================

func createValue(m: Int64, n: Int64, offset!: Int64 = 0, scale!: Int64 = 1): Int64 {
    return (m + n + offset) * scale
}

func testNamedParams(): Int64 {
    // 仅位置参数（使用默认值）
    let r1 = createValue(3, 4)             // (3 + 4 + 0) * 1 = 7

    // 指定一个命名参数
    let r2 = createValue(3, 4, offset!: 10) // (3 + 4 + 10) * 1 = 17

    // 指定两个命名参数
    let r3 = createValue(3, 4, offset!: 10, scale!: 2) // (3 + 4 + 10) * 2 = 34

    // 仅指定 scale（offset 使用默认值）
    let r4 = createValue(3, 4, scale!: 3)  // (3 + 4 + 0) * 3 = 21

    return r1 + r2 + r3 + r4  // 7 + 17 + 34 + 21 = 79
}

// =============================================================
// P2.10: String 方法
// =============================================================

func testStringMethods(): Int64 {
    var result: Int64 = 0

    // trim — 验证 trim 后长度正确且内容匹配
    let padded = "  hello  "
    let trimmed = padded.trim()
    if (trimmed.size == 5) {
        if (trimmed.startsWith("hello")) {
            result = result + 1  // +1
        }
    }

    // startsWith / endsWith
    let greeting = "hello world"
    if (greeting.startsWith("hello")) {
        result = result + 2  // +2
    }
    if (greeting.endsWith("world")) {
        result = result + 4  // +4
    }
    if (!greeting.startsWith("xyz")) {
        result = result + 8  // +8
    }

    // contains
    if (greeting.contains("lo wo")) {
        result = result + 16  // +16
    }
    if (!greeting.contains("xyz")) {
        result = result + 32  // +32
    }

    // isEmpty / isBlank
    let blank = "   "
    let emptyStr = ""
    if (blank.isBlank()) {
        result = result + 64  // +64
    }
    if (!greeting.isBlank()) {
        result = result + 128  // +128
    }
    if (emptyStr.isEmpty()) {
        result = result + 256  // +256
    }

    return result  // 1+2+4+8+16+32+64+128+256 = 511
}

// =============================================================
// 主函数：汇总所有测试
// =============================================================

main(): Int64 {
    let a = testStatic()         // 3
    let b = testAbstract()       // 87
    let c = testForStep()        // 42
    let d = testDynArray()       // 65
    let e = testArrayMethods()   // 159
    let f = testNamedParams()    // 79
    let g = testStringMethods()  // 511

    println(a)
    println(b)
    println(c)
    println(d)
    println(e)
    println(f)
    println(g)

    @Assert(a, 3)
    @Assert(b, 87)
    @Assert(c, 42)
    @Assert(d, 65)
    @Assert(e, 159)
    @Assert(f, 79)
    @Assert(g, 511)

    let total = a + b + c + d + e + f + g
    @Assert(total, 946)

    println(total)
    return total
    // 预期输出: 946
}
