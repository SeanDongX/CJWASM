// 测试 std.math 基础功能
package main

import std.math.*

main() {
    println("=== std.math 基础测试 ===")
    println("")

    // 测试常量
    println("1. 数学常量")
    println("   PI = ${PI}")
    println("   E  = ${E}")
    println("")

    // 测试基础函数
    println("2. 基础函数")

    let x = -5.7
    println("   abs(${x}) = ${abs(x)}")

    let a = 3.2
    let b = 7.8
    println("   min(${a}, ${b}) = ${min(a, b)}")
    println("   max(${a}, ${b}) = ${max(a, b)}")
    println("")

    // 测试组合使用
    println("3. 组合使用")
    let values = [-3.5, 2.1, -7.9, 5.3, -1.2]

    var maxVal = values[0]
    var minVal = values[0]
    var sumAbs = abs(values[0])

    for (i in 1..5) {
        let v = values[i]
        maxVal = max(maxVal, v)
        minVal = min(minVal, v)
        sumAbs = sumAbs + abs(v)
    }

    println("   数组: [-3.5, 2.1, -7.9, 5.3, -1.2]")
    println("   最大值: ${maxVal}")
    println("   最小值: ${minVal}")
    println("   绝对值之和: ${sumAbs}")
    println("")

    println("=== 测试完成 ===")
}
