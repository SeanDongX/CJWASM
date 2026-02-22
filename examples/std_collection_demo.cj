// 测试 std.collection 从 vendor 完整复用
package main

import std.collection.*

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

    // 测试 HashMap
    let map = HashMap<String, Int64>()
    map.put("one", 1)
    map.put("two", 2)
    map.put("three", 3)

    println("HashMap size: ${map.size}")
    println("map[\"one\"]: ${map.get("one")}")
    println("map[\"two\"]: ${map.get("two")}")

    // 测试 HashSet
    let set = HashSet<String>()
    set.put("apple")
    set.put("banana")
    set.put("apple")  // 重复元素

    println("HashSet size: ${set.size}")  // 应该是 2
    println("HashSet contains \"apple\": ${set.contains("apple")}")
}
