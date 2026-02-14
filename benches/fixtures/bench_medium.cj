// 中等规模基准：结构体 + 枚举 + 控制流

struct Point {
    var x: Int64 = 0;
    var y: Int64 = 0;
    init(x: Int64, y: Int64) {
        this.x = x;
        this.y = y;
    }
}

struct Rect {
    var origin: Point = Point(0, 0);
    var width: Int64 = 0;
    var height: Int64 = 0;
    init(origin: Point, width: Int64, height: Int64) {
        this.origin = origin;
        this.width = width;
        this.height = height;
    }
}

enum Shape {
    | Circle(Int64)
    | Rectangle(Int64)
    | Triangle(Int64)
}

func distance(p1: Point, p2: Point): Int64 {
    let dx = p1.x - p2.x
    let dy = p1.y - p2.y
    return dx * dx + dy * dy
}

func rectArea(r: Rect): Int64 {
    return r.width * r.height
}

func shapeValue(s: Shape): Int64 {
    match (s) {
        case Shape.Circle(r) => r * r * 3,
        case Shape.Rectangle(a) => a * a,
        case Shape.Triangle(b) => b * b / 2,
        case _ => 0
    }
}

func sumRange(start: Int64, end: Int64): Int64 {
    var sum: Int64 = 0
    var i = start
    while (i < end) {
        sum = sum + i
        i = i + 1
    }
    return sum
}

func fibonacci(n: Int64): Int64 {
    if (n <= 1) {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}

func isPrime(n: Int64): Bool {
    if (n < 2) {
        return false
    }
    var i: Int64 = 2
    while (i * i <= n) {
        if (n % i == 0) {
            return false
        }
        i = i + 1
    }
    return true
}

func countPrimes(limit: Int64): Int64 {
    var count: Int64 = 0
    var i: Int64 = 2
    while (i < limit) {
        if (isPrime(i)) {
            count = count + 1
        }
        i = i + 1
    }
    return count
}

main(): Int64 {
    let p1 = Point(1, 2)
    let p2 = Point(4, 6)
    let d = distance(p1, p2)

    let r = Rect(Point(0, 0), 10, 20)
    let area = rectArea(r)

    let s1 = shapeValue(Shape.Circle(5))
    let s2 = shapeValue(Shape.Rectangle(4))
    let s3 = shapeValue(Shape.Triangle(6))

    let sum = sumRange(1, 100)
    let fib = fibonacci(10)
    let primes = countPrimes(50)

    return d + area + s1 + s2 + s3 + sum + fib + primes
}
