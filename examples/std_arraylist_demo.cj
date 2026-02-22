// 测试 std.collection.ArrayList 从 vendor 复用
package main

import std.collection.ArrayList

main() {
    // 测试 ArrayList
    let list = ArrayList<Int64>()
    list.append(1)
    list.append(2)
    list.append(3)

    println("ArrayList size: ${list.size}")
    println("ArrayList[0]: ${list[0]}")
    println("ArrayList[1]: ${list[1]}")
    println("ArrayList[2]: ${list[2]}")

    // 测试迭代
    print("ArrayList elements: ")
    for (item in list) {
        print("${item} ")
    }
    println("")

    // 测试其他操作
    list.remove(1)
    println("After remove index 1, size: ${list.size}")
    println("New ArrayList[1]: ${list[1]}")
}
