// Phase 7.2: 内建类型方法测试
// 测试 Int64, Float64, Bool, String 的实例/静态方法

main() {
    // ===== #48: Int64.toString() =====
    let n: Int64 = 42
    let s1 = n.toString()
    println(s1) // expect: 42

    let neg: Int64 = -123
    println(neg.toString()) // expect: -123

    // ===== #49: Float64.toString() =====
    let f: Float64 = 3.14
    let s2 = f.toString()
    println(s2) // expect: 3.140000 (or similar float repr)

    // ===== #50: Bool.toString() =====
    let b1: Bool = true
    let b2: Bool = false
    println(b1.toString()) // expect: true
    println(b2.toString()) // expect: false

    // ===== #51: Int64.toFloat64() / Float64.toInt64() =====
    let i: Int64 = 7
    let fv = i.toFloat64()
    // fv 应该是 Float64 类型; 7.0 或 7.000000 均可接受
    let fv_as_int = fv as Int64
    println(fv_as_int) // expect: 7 (验证 toFloat64 值正确)

    let pi: Float64 = 3.14
    let iv = pi.toInt64()
    println(iv) // expect: 3

    // ===== #52: String.size / String.isEmpty() =====
    let hello = "Hello"
    let empty = ""
    println(hello.size) // expect: 5
    println(empty.size) // expect: 0
    println(hello.isEmpty()) // expect: false
    println(empty.isEmpty()) // expect: true

    // ===== #53: String.toInt64() / String.toFloat64() =====
    let numStr = "12345"
    let val = numStr.toInt64()
    println(val) // expect: 12345

    let negStr = "-99"
    println(negStr.toInt64()) // expect: -99

    let floatStr = "2.718"
    let fval = floatStr.toFloat64()
    println(fval) // expect: 2.718000

    // ===== #54: Int64.abs() / Int64.compareTo() =====
    let x: Int64 = -42
    println(x.abs()) // expect: 42

    let a: Int64 = 10
    let b: Int64 = 20
    let c: Int64 = 10
    println(a.compareTo(b))  // expect: -1 (a < b)
    println(b.compareTo(a))  // expect: 1  (b > a)
    println(a.compareTo(c))  // expect: 0  (a == c)

    // 整数类型断言（字符串比较暂不支持，通过 println 验证）
    @Assert(fv_as_int, 7)
    @Assert(iv, 3)
    @Assert(val, 12345)
    @Assert(x.abs(), 42)
    @Assert(a.compareTo(b), -1)
    @Assert(b.compareTo(a), 1)
    @Assert(a.compareTo(c), 0)

    println("PASS")
    // 预期输出: PASS
}
