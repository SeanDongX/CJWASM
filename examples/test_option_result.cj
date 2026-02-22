// 测试 Option 和 Result 类型（不使用 Array）
package main

main() {
    println("=== 测试 Option 和 Result ===")
    println("")

    // 测试 Option
    println("1. 测试 Option")
    let some: Option<Int64> = Some(42)
    let none: Option<Int64> = None

    match (some) {
        case Some(v) => println("   some = Some(${v})")
        case None => println("   some = None")
    }

    match (none) {
        case Some(v) => println("   none = Some(${v})")
        case None => println("   none = None ✓")
    }
    println("")

    // 测试 Result
    println("2. 测试 Result")
    let ok: Result<Int64, String> = Ok(100)
    let err: Result<Int64, String> = Err("error")

    match (ok) {
        case Ok(v) => println("   ok = Ok(${v})")
        case Err(e) => println("   ok = Err(${e})")
    }

    match (err) {
        case Ok(v) => println("   err = Ok(${v})")
        case Err(e) => println("   err = Err(${e}) ✓")
    }
    println("")

    // 测试 Range
    println("3. 测试 Range")
    print("   0..3: ")
    for (i in 0..3) {
        print("${i} ")
    }
    println("")
    println("")

    println("=== 所有测试通过！===")
}
