// 模式匹配高级特性示例

struct Point {
    x: Int64,
    y: Int64
}

struct Rectangle {
    topLeft: Point,
    width: Int64,
    height: Int64
}

// if-let 模式
func processOption(opt: Option<Int64>) -> Int64 {
    if let Some(value) = opt {
        return value * 2
    }
    return 0
}

// if-let 与 Result
func processResult(res: Result<Int64, String>) -> Int64 {
    if let Ok(value) = res {
        return value + 10
    }
    return 0
}

// while-let 模式（模拟迭代）
func countDown(start: Int64) -> Int64 {
    var current = Some(start)
    var sum: Int64 = 0

    while let Some(n) = current {
        sum = sum + n
        if n > 0 {
            current = Some(n - 1)
        } else {
            current = None
        }
    }
    return sum
}

// 解构绑定 - 结构体
func deconstructPoint() -> Int64 {
    let p = Point { x: 10, y: 20 }
    let Point { x, y } = p
    return x + y  // 30
}

// 解构绑定在 match 中
func matchPoint(p: Point) -> Int64 {
    match p {
        Point { x: 0, y: 0 } => 0,            // 原点
        Point { x: 0, y } => y,                // y 轴上
        Point { x, y: 0 } => x,                // x 轴上
        Point { x, y } => x + y                // 其他
    }
}

// 嵌套解构
func matchRectangle(r: Rectangle) -> Int64 {
    match r {
        Rectangle { topLeft: Point { x: 0, y: 0 }, width, height } =>
            width * height,
        Rectangle { topLeft, width, height } =>
            topLeft.x + topLeft.y + width + height
    }
}

// 复杂 guard 条件
func classifyPoint(p: Point) -> String {
    match p {
        Point { x, y } if x == y => "diagonal",
        Point { x, y } if x > 0 && y > 0 => "first quadrant",
        Point { x, y } if x < 0 && y > 0 => "second quadrant",
        Point { x, y } if x < 0 && y < 0 => "third quadrant",
        Point { x, y } if x > 0 && y < 0 => "fourth quadrant",
        _ => "on axis"
    }
}

func main() -> Int64 {
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

    return a + b + c + d + e + f    // 10 + 0 + 15 + 30 + 5 + 7 = 67
}
