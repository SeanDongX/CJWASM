// 方法定义示例

struct Point {
    x: Int64,
    y: Int64
}

struct Rectangle {
    width: Int64,
    height: Int64
}

// 实例方法（使用 func Type.method 语法）
func Point.distance(self: Point) -> Int64 {
    // 使用 this 或 self 引用实例
    return self.x * self.x + self.y * self.y
}

func Point.add(self: Point, other: Point) -> Point {
    return Point { x: self.x + other.x, y: self.y + other.y }
}

// this 关键字示例
func Rectangle.area(this: Rectangle) -> Int64 {
    return this.width * this.height
}

func Rectangle.perimeter(this: Rectangle) -> Int64 {
    return 2 * (this.width + this.height)
}

func Rectangle.scale(this: Rectangle, factor: Int64) -> Rectangle {
    return Rectangle {
        width: this.width * factor,
        height: this.height * factor
    }
}

// 静态方法
func Point.origin() -> Point {
    return Point { x: 0, y: 0 }
}

func Rectangle.square(size: Int64) -> Rectangle {
    return Rectangle { width: size, height: size }
}

// 构造函数语法糖
func createPoint(x: Int64, y: Int64) -> Point {
    // Point(x, y) 转换为 Point { x: x, y: y }
    return Point(x, y)
}

func main() -> Int64 {
    // 使用构造函数语法
    let p1 = Point(3, 4)
    let p2 = Point { x: 1, y: 2 }

    // 调用实例方法
    let dist = p1.distance()           // 9 + 16 = 25
    let p3 = p1.add(p2)                // Point { x: 4, y: 6 }

    // 调用静态方法
    let origin = Point.origin()        // Point { x: 0, y: 0 }
    let sq = Rectangle.square(5)       // 5x5 正方形

    // 矩形方法
    let rect = Rectangle { width: 10, height: 5 }
    let area = rect.area()             // 50
    let peri = rect.perimeter()        // 30
    let scaled = rect.scale(2)         // 20x10

    return dist + area + peri          // 25 + 50 + 30 = 105
}
