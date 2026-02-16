// P3 特性验证: Option/Tuple/Lambda

// =============================================================
// Option / Some / None + if let + ??
// =============================================================

func testOptionBasic(): Int64 {
    var result: Int64 = 0

    let a: ?Int64 = Some(42)
    let b: ?Int64 = None

    // ?? 空合并运算符
    let v1 = a ?? 0   // 42
    let v2 = b ?? 99  // 99
    result = result + v1 + v2  // 42 + 99 = 141

    return result  // 141
}

func testIfLet(): Int64 {
    var result: Int64 = 0

    let opt1: ?Int64 = Some(10)
    let opt2: ?Int64 = None

    if (let Some(x) <- opt1) {
        result = result + x  // +10
    }

    if (let Some(y) <- opt2) {
        result = result + 1000  // 不应执行
    } else {
        result = result + 5  // +5
    }

    return result  // 15
}

func findValue(arr: Array<Int64>, target: Int64): ?Int64 {
    let n = arr.size
    for (i in 0..n) {
        if (arr[i] == target) {
            return Some(i)
        }
    }
    return None
}

func testOptionFunction(): Int64 {
    let arr = [10, 20, 30, 40, 50]

    let found = findValue(arr, 30)     // Some(2)
    let notFound = findValue(arr, 99)  // None

    let idx = found ?? -1       // 2
    let idx2 = notFound ?? -1   // -1

    return idx + 100 + idx2  // 2 + 100 + (-1) = 101
}

// =============================================================
// 元组 (使用 let 推断类型)
// =============================================================

func makePair(a: Int64, b: Int64): (Int64, Int64) {
    return (a, b)
}

func testTupleBasic(): Int64 {
    let t = (10, 20, 30)
    let a = t.0  // 10
    let b = t.1  // 20
    let c = t.2  // 30
    return a + b + c  // 60
}

func testTupleReturn(): Int64 {
    let pair = makePair(7, 8)
    return pair.0 + pair.1  // 15
}

// =============================================================
// Lambda + 函数类型参数
// =============================================================

func apply(f: (Int64) -> Int64, x: Int64): Int64 {
    return f(x)
}

func testLambdaBasic(): Int64 {
    let double = { x: Int64 => x * 2 }
    let square = { x: Int64 => x * x }

    let a = apply(double, 5)   // 10
    let b = apply(square, 4)   // 16
    return a + b  // 26
}

func applyBinary(f: (Int64, Int64) -> Int64, a: Int64, b: Int64): Int64 {
    return f(a, b)
}

func testLambdaMultiParam(): Int64 {
    let add = { a: Int64, b: Int64 => a + b }
    let mul = { a: Int64, b: Int64 => a * b }

    let r1 = applyBinary(add, 3, 4)  // 7
    let r2 = applyBinary(mul, 3, 4)  // 12
    return r1 + r2  // 19
}

// =============================================================
// type alias
// =============================================================

type Transformer = (Int64) -> Int64

func applyTransform(f: Transformer, x: Int64): Int64 {
    return f(x)
}

func testTypeAlias(): Int64 {
    let triple = { x: Int64 => x * 3 }
    return applyTransform(triple, 10)  // 30
}

// =============================================================
// 主函数
// =============================================================

main(): Int64 {
    let a = testOptionBasic()       // 141
    let b = testIfLet()             // 15
    let c = testOptionFunction()    // 101
    let d = testTupleBasic()        // 60
    let e = testTupleReturn()       // 15
    let f = testLambdaBasic()       // 26
    let g = testLambdaMultiParam()  // 19
    let h = testTypeAlias()         // 30

    println(a)
    println(b)
    println(c)
    println(d)
    println(e)
    println(f)
    println(g)
    println(h)

    @Assert(a, 141)
    @Assert(b, 15)
    @Assert(c, 101)
    @Assert(d, 60)
    @Assert(e, 15)
    @Assert(f, 26)
    @Assert(g, 19)
    @Assert(h, 30)

    let total = a + b + c + d + e + f + g + h
    @Assert(total, 407)

    return total
    // 预期输出: 407
}
