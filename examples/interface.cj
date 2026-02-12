// 接口示例（解析层支持，implements 暂不参与 codegen）
interface Drawable {
    func draw();
    func area() -> Int64;
}

struct Rect {
    width: Int64,
    height: Int64
}

func Rect.area(self: Rect) -> Int64 {
    return self.width * self.height
}

func main() -> Int64 {
    let r = Rect { width: 5, height: 10 }
    return r.area()
}
