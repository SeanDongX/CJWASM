// 类示例（无继承，展平为结构体编译）
class Point {
    private var x: Int64;
    private var y: Int64;

    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }

    func area(self: Point): Int64 {
        return self.x * self.y
    }
}

main(): Int64 {
    let p = Point(5, 10)
    return p.area(p)  // 预期输出: 50
}
