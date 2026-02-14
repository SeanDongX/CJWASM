// Phase 4: 泛型系统完善 - 综合示例

// ============================
// 1. 泛型函数（已有功能验证）
// ============================
func identity<T>(value: T): T {
    return value
}

// ============================
// 2. 泛型结构体（已有功能验证）
// ============================
struct Pair<T, U> {
    var first: T;
    var second: U;
    init(first: T, second: U) {
        this.first = first
        this.second = second
    }
}

struct Triple<T, U, V> {
    var a: T;
    var b: U;
    var c: V;
    init(a: T, b: U, c: V) {
        this.a = a
        this.b = b
        this.c = c
    }
}

// ============================
// 3. 泛型函数 + 类型约束 (#21)
// cjc: where T <: Comparable (cjwasm uses where T: Comparable)
// ============================
func maximum<T>(a: T, b: T): T where T <: Comparable<T> {
    return a
}

func clamp<T>(value: T, low: T, high: T): T where T <: Comparable<T> {
    return value
}

// ============================
// 4. 泛型结构体 + 约束 (#22)
// ============================
struct SortedPair<T> where T <: Comparable<T> {
    var first: T;
    var second: T;
    init(first: T, second: T) {
        this.first = first
        this.second = second
    }
}

// ============================
// 5. 多重约束 (#26)
// cjc: where T <: Comparable & Hashable
// ============================
func process<T>(value: T): T where T <: Comparable<T> & Hashable {
    return value
}

func transform_value<T>(x: T): T where T <: Comparable<T> & Equatable<T> & ToString {
    return x
}

// ============================
// 6. 泛型枚举 (#24)
// ============================
enum MyOption<T> {
    | MySome(T)
    | MyNone
}

enum MyResult<T, E> {
    | MyOk(T)
    | MyErr(E)
}

enum Tree<T> {
    | Leaf(T)
    | Empty
}

// ============================
// 7. where 子句 (#27)
// ============================
struct Container<T> where T <: Comparable<T> {
    var value: T;
    init(value: T) {
        this.value = value
    }
}

func swap_pair<T, U>(a: T, b: U): T where T <: Comparable<T>, U <: Hashable {
    return a
}

struct KeyValue<K, V> where K <: Hashable, V <: ToString {
    var key: K;
    var value: V;
    init(key: K, value: V) {
        this.key = key
        this.value = value
    }
}

// ============================
// 8. 泛型类 (#23)
// ============================
class Box<T> {
    var content: T;

    init(content: T) {
        this.content = content
    }

    func getContent(self: Box<T>): T {
        return self.content
    }
}

class Stack<T> {
    var top: T;
    var size: Int64;

    init(top: T, size: Int64) {
        this.top = top
        this.size = size
    }

    func peek(self: Stack<T>): T {
        return self.top
    }

    func getSize(self: Stack<T>): Int64 {
        return self.size
    }
}

// ============================
// 9. 泛型特化 (#28)
// ============================
// 泛型版本
func add<T>(a: T, b: T): T {
    return a  // 通用版本，返回第一个参数
}

// 特化版本：Int64 的 add 直接用非泛型函数覆盖
// 注意：特化通过命名约定实现，单态化时优先使用已存在的具体函数

// ============================
// 测试函数
// ============================

func testBasicGeneric(): Int64 {
    let x = identity<Int64>(42)
    let y = identity<Int32>(10)
    return x  // 42
}

func testGenericStruct(): Int64 {
    let p = Pair<Int64, Int64>(10, 20)
    let t = Triple<Int64, Int64, Int64>(1, 2, 3)
    return p.first + p.second + t.a + t.b + t.c  // 36
}

func testConstrainedFunc(): Int64 {
    let a = maximum<Int64>(100, 200)
    let b = clamp<Int64>(50, 10, 100)
    return a + b  // 150
}

func testConstrainedStruct(): Int64 {
    let sp = SortedPair<Int64>(5, 10)
    return sp.first + sp.second  // 15
}

func testMultiConstraint(): Int64 {
    let a = process<Int64>(55)
    let b = transform_value<Int64>(33)
    return a + b  // 88
}

func testWhereClause(): Int64 {
    let c = Container<Int64>(77)
    let result = swap_pair<Int64, Int64>(10, 20)
    let kv = KeyValue<Int64, Int64>(1, 2)
    return c.value + result + kv.key + kv.value  // 90
}

func testGenericClass(): Int64 {
    let b = Box<Int64>(42)
    let s = Stack<Int64>(99, 1)
    return b.getContent(b) + s.peek(s) + s.getSize(s)
    // 42 + 99 + 1 = 142
}

func testSpecialization(): Int64 {
    let i64_result = add<Int64>(10, 20)
    let i32_result = add<Int32>(5, 15)
    return i64_result  // 10 (通用版本)
}

func testMixedTypes(): Int64 {
    let p1 = Pair<Int64, Int32>(100, 50)
    let p2 = Pair<Bool, Int64>(true, 200)
    return p1.first + p2.second  // 300
}

main(): Int64 {
    let a = testBasicGeneric()       // 42
    let b = testGenericStruct()      // 36
    let c = testConstrainedFunc()    // 150
    let d = testConstrainedStruct()  // 15
    let e = testMultiConstraint()    // 88
    let f = testWhereClause()        // 90
    let g = testGenericClass()       // 142
    let h = testSpecialization()     // 10
    let i = testMixedTypes()         // 300

    return a + b + c + d + e + f + g + h + i
    // 42 + 36 + 150 + 15 + 88 + 90 + 142 + 10 + 300 = 873
}
