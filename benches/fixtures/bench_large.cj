// 大规模基准：类 + 泛型 + 错误处理 + 复杂控制流

// ── 泛型结构体 ──
struct Pair<T, U> {
    first: T,
    second: U
}

struct Triple<T, U, V> {
    a: T,
    b: U,
    c: V
}

// ── 枚举 ──
enum Color {
    Red,
    Green,
    Blue
}

enum Message {
    Quit,
    Move(Int64),
    Write(Int64)
}

// ── 类 ──
class Vector2D {
    var x: Int64;
    var y: Int64;

    func length(self: Vector2D) -> Int64 {
        return self.x * self.x + self.y * self.y
    }

    func dot(self: Vector2D, ox: Int64, oy: Int64) -> Int64 {
        return self.x * ox + self.y * oy
    }
}

class Counter {
    var count: Int64;

    func increment(self: Counter) -> Int64 {
        return self.count + 1
    }

    func decrement(self: Counter) -> Int64 {
        return self.count - 1
    }

    func get(self: Counter) -> Int64 {
        return self.count
    }
}

class Matrix {
    var a11: Int64;
    var a12: Int64;
    var a21: Int64;
    var a22: Int64;

    func determinant(self: Matrix) -> Int64 {
        return self.a11 * self.a22 - self.a12 * self.a21
    }

    func trace(self: Matrix) -> Int64 {
        return self.a11 + self.a22
    }
}

// ── 泛型函数 ──
func identity<T>(value: T) -> T {
    return value
}

func maximum<T: Comparable>(a: T, b: T) -> T {
    return a
}

// ── 普通函数 ──
func fibonacci(n: Int64) -> Int64 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}

func gcd(a: Int64, b: Int64) -> Int64 {
    if b == 0 {
        return a
    }
    return gcd(b, a % b)
}

func power(base: Int64, exp: Int64) -> Int64 {
    if exp == 0 {
        return 1
    }
    return base * power(base, exp - 1)
}

func sumOfSquares(n: Int64) -> Int64 {
    var sum: Int64 = 0
    var i: Int64 = 1
    while i <= n {
        sum = sum + i * i
        i = i + 1
    }
    return sum
}

func isPrime(n: Int64) -> Bool {
    if n < 2 {
        return false
    }
    var i: Int64 = 2
    while i * i <= n {
        if n % i == 0 {
            return false
        }
        i = i + 1
    }
    return true
}

func countPrimes(limit: Int64) -> Int64 {
    var count: Int64 = 0
    var i: Int64 = 2
    while i < limit {
        if isPrime(i) {
            count = count + 1
        }
        i = i + 1
    }
    return count
}

func collatz(n: Int64) -> Int64 {
    var steps: Int64 = 0
    var current = n
    while current != 1 {
        if current % 2 == 0 {
            current = current / 2
        } else {
            current = current * 3 + 1
        }
        steps = steps + 1
    }
    return steps
}

// ── 错误处理 ──
func safeDivide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 {
        return Err("Division by zero")
    }
    return Ok(a / b)
}

func handleResult(r: Result<Int64, String>) -> Int64 {
    match r {
        Ok(val) => val,
        Err(e) => -1
    }
}

func handleOption(opt: Option<Int64>) -> Int64 {
    match opt {
        Some(val) => val,
        None => 0
    }
}

// ── try-catch ──
func tryCatchExample(a: Int64, b: Int64) -> Int64 {
    try {
        if b == 0 {
            throw 0
        }
        return a / b
    } catch(e) {
        return -1
    }
}

// ── 颜色处理 ──
func colorValue(c: Color) -> Int64 {
    match c {
        Color.Red => 1,
        Color.Green => 2,
        Color.Blue => 3
    }
}

func messageValue(m: Message) -> Int64 {
    match m {
        Message.Quit => 0,
        Message.Move(distance) => distance,
        Message.Write(code) => code
    }
}

// ── 测试子函数 ──
func testStructs() -> Int64 {
    let p1 = Pair<Int64, Int64> { first: 10, second: 20 }
    let p2 = Pair<Int64, Bool> { first: 42, second: true }
    let t = Triple<Int64, Int64, Int64> { a: 1, b: 2, c: 3 }
    return p1.first + p1.second + p2.first + t.a + t.b + t.c
}

func testClasses() -> Int64 {
    let v1 = Vector2D { x: 3, y: 4 }
    let len = v1.length()
    let d = v1.dot(1, 2)

    let c = Counter { count: 10 }
    let inc = c.increment()
    let dec = c.decrement()

    let m = Matrix { a11: 1, a12: 2, a21: 3, a22: 4 }
    let det = m.determinant()
    let tr = m.trace()

    return len + d + inc + dec + det + tr
}

func testGenerics() -> Int64 {
    let a = identity<Int64>(42)
    let c = maximum<Int64>(100, 200)
    return a + c
}

func testAlgorithms() -> Int64 {
    let fib = fibonacci(10)
    let g = gcd(48, 18)
    let pw = power(2, 10)
    let sq = sumOfSquares(10)
    let primes = countPrimes(100)
    let col = collatz(27)
    return fib + g + pw + sq + primes + col
}

func testErrorHandling() -> Int64 {
    let r1 = safeDivide(100, 5)
    let r2 = safeDivide(100, 0)
    let v1 = handleResult(r1)
    let v2 = handleResult(r2)

    let o1 = handleOption(Some(42))
    let o2 = handleOption(Some(0))

    let tc1 = tryCatchExample(10, 2)
    let tc2 = tryCatchExample(10, 0)

    return v1 + v2 + o1 + o2 + tc1 + tc2
}

func testEnums() -> Int64 {
    let c1 = colorValue(Color.Red)
    let c2 = colorValue(Color.Green)
    let c3 = colorValue(Color.Blue)
    let m1 = messageValue(Message.Quit)
    let m2 = messageValue(Message.Move(100))
    let m3 = messageValue(Message.Write(42))
    return c1 + c2 + c3 + m1 + m2 + m3
}

func testLoops() -> Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while i < 50 {
        if isPrime(i) {
            sum = sum + i
        }
        i = i + 1
    }
    return sum
}

func testNested() -> Int64 {
    var result: Int64 = 0
    var i: Int64 = 1
    while i <= 10 {
        var j: Int64 = 1
        while j <= 10 {
            if i * j % 7 == 0 {
                result = result + i * j
            }
            j = j + 1
        }
        i = i + 1
    }
    return result
}

// ── 主函数 ──
func main() -> Int64 {
    let a = testStructs()
    let b = testClasses()
    let c = testGenerics()
    let d = testAlgorithms()
    let e = testErrorHandling()
    let f = testEnums()
    let g = testLoops()
    let h = testNested()

    return a + b + c + d + e + f + g + h
}
