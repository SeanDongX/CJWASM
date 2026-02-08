// 结构体定义
struct Point {
    x: Int64,
    y: Int64
}

struct Rectangle {
    width: Int64,
    height: Int64
}

// 创建点
func makePoint(x: Int64, y: Int64) -> Point {
    return Point { x: x, y: y }
}

// 计算矩形面积
func area(r: Rectangle) -> Int64 {
    return r.width * r.height
}

// 数组求和
func sum(arr: Array<Int64>) -> Int64 {
    var total: Int64 = 0
    var i: Int64 = 0
    while i < 3 {
        total = total + arr[i]
        i = i + 1
    }
    return total
}

// 测试字符串
func greet() -> String {
    return "Hello, Cangjie!"
}

// 主函数
func main() -> Int64 {
    // 测试数组
    let numbers = [10, 20, 30]
    let total = sum(numbers)

    // 测试结构体
    let rect = Rectangle { width: 5, height: 10 }
    let a = area(rect)

    return total + a
}
