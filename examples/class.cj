// 类示例（无继承，展平为结构体编译）
class Point {
    private var x: Int64;
    private var y: Int64;

    func area(self: Point) -> Int64 {
        return self.x * self.y
    }
}

func main() -> Int64 {
    let p = Point { x: 5, y: 10 }
    return p.area()
}
