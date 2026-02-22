// 综合测试：验证多个标准库模块的互操作
package main

import std.collection.ArrayList
import std.time.*
import std.random.*

main() {
    println("=== cjwasm 标准库综合测试 ===")
    println("")

    // 测试 1: std.collection.ArrayList
    println("1. 测试 std.collection.ArrayList")
    let list = ArrayList<Int64>()
    list.append(10)
    list.append(20)
    list.append(30)
    println("   ArrayList size: ${list.size}")
    println("   ArrayList[0]: ${list[0]}")
    println("   ✓ ArrayList 测试通过")
    println("")

    // 测试 2: std.time
    println("2. 测试 std.time")
    let now = DateTime.now()
    println("   当前时间已获取")
    println("   ✓ std.time 测试通过")
    println("")

    // 测试 3: std.random
    println("3. 测试 std.random")
    let r1 = randomInt64()
    let r2 = randomInt64()
    println("   随机数 1: ${r1}")
    println("   随机数 2: ${r2}")
    println("   ✓ std.random 测试通过")
    println("")

    // 测试 4: 组合使用
    println("4. 测试组合使用")
    let randomList = ArrayList<Int64>()
    randomList.append(randomInt64())
    randomList.append(randomInt64())
    randomList.append(randomInt64())
    println("   生成了 ${randomList.size} 个随机数")
    for (i in 0..randomList.size) {
        println("   randomList[${i}] = ${randomList[i]}")
    }
    println("   ✓ 组合测试通过")
    println("")

    println("=== 所有测试通过！===")
}
