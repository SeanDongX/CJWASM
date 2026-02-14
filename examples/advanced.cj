// 结构体定义 (cjc 兼容: init + 构造调用语法)
struct Point {
    var x: Int64;
    var y: Int64;
    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }
}

struct Rectangle {
    var width: Int64;
    var height: Int64;
    init(width: Int64, height: Int64) {
        this.width = width
        this.height = height
    }
}

// 创建点
func makePoint(x: Int64, y: Int64): Point {
    return Point(x, y)
}

// 计算矩形面积
func area(r: Rectangle): Int64 {
    return r.width * r.height
}

// 数组求和
func sum(arr: Array<Int64>): Int64 {
    var total: Int64 = 0
    var i: Int64 = 0
    while (i < 3) {
        total = total + arr[i]
        i = i + 1
    }
    return total
}

// 测试字符串
func greet(): String {
    return "Hello, Cangjie!"
}

// 主函数
main(): Int64 {
    // 测试数组
    let numbers = [10, 20, 30]
    let total = sum(numbers)

    // 测试结构体
    let rect = Rectangle(5, 10)
    let a = area(rect)

    return total + a  // 预期输出: 110
}
