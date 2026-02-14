// 数学工具模块
package demo

func add(a: Int64, b: Int64): Int64 {
    return a + b
}

func subtract(a: Int64, b: Int64): Int64 {
    return a - b
}

func multiply(a: Int64, b: Int64): Int64 {
    return a * b
}

func divide(a: Int64, b: Int64): Int64 {
    if (b == 0) {
        return 0
    }
    return a / b
}

func sumRange(from: Int64, to: Int64): Int64 {
    var sum: Int64 = 0
    var i = from
    while (i <= to) {
        sum = sum + i
        i = i + 1
    }
    return sum
}
