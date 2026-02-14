// 方法定义示例

struct Point {
    var x: Int64;
    var y: Int64;

    init(x: Int64, y: Int64) {
        this.x = x;
        this.y = y;
    }

    func distance(): Int64 {
        return this.x * this.x + this.y * this.y
    }

    func add(other: Point): Point {
        return Point(this.x + other.x, this.y + other.y)
    }
}

func createOrigin(): Point {
    return Point(0, 0)
}

struct Rectangle {
    var width: Int64;
    var height: Int64;

    init(width: Int64, height: Int64) {
        this.width = width;
        this.height = height;
    }

    func area(): Int64 {
        return this.width * this.height
    }

    func perimeter(): Int64 {
        return 2 * (this.width + this.height)
    }

    func scale(factor: Int64): Rectangle {
        return Rectangle(this.width * factor, this.height * factor)
    }
}

func createSquare(size: Int64): Rectangle {
    return Rectangle(size, size)
}

main(): Int64 {
    let p1 = Point(3, 4)
    let p2 = Point(1, 2)

    let dist = p1.distance()
    let p3 = p1.add(p2)

    let origin = createOrigin()
    let sq = createSquare(5)

    let rect = Rectangle(10, 5)
    let area = rect.area()
    let peri = rect.perimeter()
    let scaled = rect.scale(2)

    return dist + area + peri
}
