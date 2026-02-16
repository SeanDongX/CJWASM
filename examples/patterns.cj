// 模式匹配高级特性示例

struct Point {
    var x: Int64;
    var y: Int64
}

struct Rectangle {
    var topLeft: Point;
    var width: Int64;
    var height: Int64
}

// if-let 模式 (cjc: if (let x <- expr))
func processOption(opt: Option<Int64>): Int64 {
    if (let Some(value) <- opt) {
        return value * 2
    }
    return 0
}

// if-let 与 Result
func processResult(res: Result<Int64, String>): Int64 {
    if (let Ok(value) <- res) {
        return value + 10
    }
    return 0
}

// while-let 模式 (cjc: while (let x <- expr))
func countDown(start: Int64): Int64 {
    var current = Some(start)
    var sum: Int64 = 0

    while (let Some(n) <- current) {
        sum = sum + n
        if (n > 0) {
            current = Some(n - 1)
        } else {
            current = None
        }
    }
    return sum
}

// 解构绑定 - 结构体
func deconstructPoint(): Int64 {
    let p = Point { x: 10, y: 20 }
    let Point { x, y } = p
    return x + y  // 30
}

// 解构绑定在 match 中
func matchPoint(p: Point): Int64 {
    match (p) {
        case Point { x: 0, y: 0 } => 0,            // 原点
        case Point { x: 0, y } => y,                // y 轴上
        case Point { x, y: 0 } => x,                // x 轴上
        case Point { x, y } => x + y                // 其他
    }
}

// 嵌套解构
func matchRectangle(r: Rectangle): Int64 {
    match (r) {
        case Rectangle { topLeft: Point { x: 0, y: 0 }, width, height } =>
            width * height,
        case Rectangle { topLeft, width, height } =>
            topLeft.x + topLeft.y + width + height
    }
}

// 复杂 guard 条件 (cjc: case pattern where condition =>)
func classifyPoint(p: Point): String {
    match (p) {
        case Point { x, y } where x == y => "diagonal",
        case Point { x, y } where x > 0 && y > 0 => "first quadrant",
        case Point { x, y } where x < 0 && y > 0 => "second quadrant",
        case Point { x, y } where x < 0 && y < 0 => "third quadrant",
        case Point { x, y } where x > 0 && y < 0 => "fourth quadrant",
        case _ => "on axis"
    }
}

main(): Int64 {
    // if-let 测试
    let opt1 = Some(5)
    let opt2: Option<Int64> = None
    let a = processOption(opt1)     // 10
    let b = processOption(opt2)     // 0

    // while-let 测试
    let c = countDown(5)            // 5+4+3+2+1+0 = 15

    // 解构测试
    let d = deconstructPoint()      // 30

    // match 解构测试
    let p1 = Point { x: 0, y: 5 }
    let p2 = Point { x: 3, y: 4 }
    let e = matchPoint(p1)          // 5 (y轴上)
    let f = matchPoint(p2)          // 7

    // a=10(processOption Some 正确返回), b=0(processOption None),
    // c=5(countDown), d=30(deconstructPoint), e=0(matchPoint), f=0(matchPoint)
    @Assert(a + b + c + d + e + f, 45)

    return a + b + c + d + e + f    // 预期输出: 45
}
