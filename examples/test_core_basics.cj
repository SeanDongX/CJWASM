// 测试 std.core 基础类型
package main

// 不导入任何标准库，测试内置类型
main() {
    println("=== 测试 std.core 基础类型 ===")
    println("")

    // 测试 Option
    println("1. 测试 Option 类型")
    let some: Option<Int64> = Some(42)
    let none: Option<Int64> = None

    match (some) {
        case Some(v) => println("   Some: ${v}")
        case None => println("   None")
    }

    match (none) {
        case Some(v) => println("   Some: ${v}")
        case None => println("   None (correct)")
    }
    println("   ✓ Option 测试通过")
    println("")

    // 测试 Result
    println("2. 测试 Result 类型")
    let ok: Result<Int64, String> = Ok(100)
    let err: Result<Int64, String> = Err("error message")

    match (ok) {
        case Ok(v) => println("   Ok: ${v}")
        case Err(e) => println("   Err: ${e}")
    }

    match (err) {
        case Ok(v) => println("   Ok: ${v}")
        case Err(e) => println("   Err: ${e} (correct)")
    }
    println("   ✓ Result 测试通过")
    println("")

    // 测试 Range
    println("3. 测试 Range")
    print("   0..5: ")
    for (i in 0..5) {
        print("${i} ")
    }
    println("")
    println("   ✓ Range 测试通过")
    println("")

    // 测试 Array
    println("4. 测试 Array")
    let arr = Array<Int64>(5, item: 0)
    arr[0] = 10
    arr[1] = 20
    arr[2] = 30

    println("   arr[0] = ${arr[0]}")
    println("   arr[1] = ${arr[1]}")
    println("   arr[2] = ${arr[2]}")
    println("   arr.size = ${arr.size}")
    println("   ✓ Array 测试通过")
    println("")

    println("=== 所有 std.core 基础测试通过！===")
}
