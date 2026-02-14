// 接口示例（extend 实现接口，方法在 struct 内部）
interface Drawable {
    func draw(): Int64;
    func area(): Int64;
}

struct Rect {
    var width: Int64;
    var height: Int64;

    init(width: Int64, height: Int64) {
        this.width = width
        this.height = height
    }

    func draw(): Int64 {
        return 0
    }

    func area(): Int64 {
        return this.width * this.height
    }
}

main(): Int64 {
    let r = Rect(5, 10)
    return r.area()
}
