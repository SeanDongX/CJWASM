// 几何计算工具
package demo

func rectArea(width: Int64, height: Int64): Int64 {
    return width * height
}

func rectPerimeter(width: Int64, height: Int64): Int64 {
    return 2 * (width + height)
}

func triangleArea(base: Int64, height: Int64): Int64 {
    return base * height / 2
}

func circleArea(radius: Int64): Int64 {
    return 3 * radius * radius
}
