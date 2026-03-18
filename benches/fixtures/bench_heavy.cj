// 高负载基准：密集调用小函数，放大内联优化效果

class Counter {
    var count: Int64;
    init(count: Int64) { this.count = count }
    func get(self: Counter): Int64 { return self.count }
    func increment(self: Counter): Int64 { return self.count + 1 }
    func decrement(self: Counter): Int64 { return self.count - 1 }
}

class Vector2D {
    var x: Int64;
    var y: Int64;
    init(x: Int64, y: Int64) { this.x = x; this.y = y }
    func dot(self: Vector2D, ox: Int64, oy: Int64): Int64 { return self.x * ox + self.y * oy }
    func lengthSq(self: Vector2D): Int64 { return self.x * self.x + self.y * self.y }
}

func identity(x: Int64): Int64 { return x }
func double(x: Int64): Int64 { return x * 2 }
func square(x: Int64): Int64 { return x * x }
func addOne(x: Int64): Int64 { return x + 1 }
func subOne(x: Int64): Int64 { return x - 1 }
func negate(x: Int64): Int64 { return -x }
func abs2(x: Int64): Int64 { if (x < 0) { return -x } else { return x } }
func clamp(x: Int64, lo: Int64, hi: Int64): Int64 {
    if (x < lo) { return lo }
    if (x > hi) { return hi }
    return x
}

// 密集调用 identity/double/square/addOne 各 N 次
func testInlineCalls(n: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let a = identity(i)
        let b = double(a)
        let c = square(addOne(a))
        let d = subOne(b)
        sum = sum + a + b + c + d
        i = i + 1
    }
    return sum
}

// 密集调用 Counter 方法
func testCounterCalls(n: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let c = Counter(i)
        let g = c.get()
        let inc = c.increment()
        let dec = c.decrement()
        sum = sum + g + inc + dec
        i = i + 1
    }
    return sum
}

// 密集调用 Vector2D 方法
func testVectorCalls(n: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let v = Vector2D(i, i + 1)
        let d = v.dot(2, 3)
        let l = v.lengthSq()
        sum = sum + d + l
        i = i + 1
    }
    return sum
}

// 密集调用 clamp/abs2
func testUtilCalls(n: Int64): Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while (i < n) {
        let a = abs2(i - n / 2)
        let b = clamp(i, 10, n - 10)
        let c = negate(subOne(i))
        sum = sum + a + b + c
        i = i + 1
    }
    return sum
}

main(): Int64 {
    let N: Int64 = 100000
    let a = testInlineCalls(N)
    let b = testCounterCalls(N)
    let c = testVectorCalls(N)
    let d = testUtilCalls(N)
    return (a + b + c + d) % 1000000007
}
