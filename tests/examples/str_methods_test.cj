// Minimal test for toArray, indexOf, replace
main(): Int64 {
    let chars = "abc".toArray()
    println(chars[0])
    println(chars[1])
    println(chars[2])

    let s = "hello world"
    let wIdx = s.indexOf("world")
    let xIdx = s.indexOf("xyz")
    println(wIdx)
    println(xIdx)

    let replaced = s.replace("world", "cangjie")
    println(replaced)
    println("PASS")
    return 0
    // 预期输出: 0
}
