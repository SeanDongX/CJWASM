// Phase 4: 泛型系统完善 - 综合示例

// ============================
// 1. 泛型函数（已有功能验证）
// ============================
func identity<T>(value: T) -> T {
    return value
}

// ============================
// 2. 泛型结构体（已有功能验证）
// ============================
struct Pair<T, U> {
    first: T,
    second: U
}

struct Triple<T, U, V> {
    a: T,
    b: U,
    c: V
}

// ============================
// 3. 泛型函数 + 类型约束 (#21)
// ============================
func maximum<T: Comparable>(a: T, b: T) -> T {
    return a
}

func clamp<T: Comparable>(value: T, low: T, high: T) -> T {
    return value
}

// ============================
// 4. 泛型结构体 + 约束 (#22)
// ============================
struct SortedPair<T: Comparable> {
    first: T,
    second: T
}

// ============================
// 5. 多重约束 (#26)
// ============================
func process<T: Comparable & Hashable>(value: T) -> T {
    return value
}

func transform_value<T: Comparable & Equatable & ToString>(x: T) -> T {
    return x
}

// ============================
// 6. 泛型枚举 (#24)
// ============================
enum MyOption<T> {
    MySome(T),
    MyNone
}

enum MyResult<T, E> {
    MyOk(T),
    MyErr(E)
}

enum Tree<T> {
    Leaf(T),
    Empty
}

// ============================
// 7. where 子句 (#27)
// ============================
struct Container<T> where T: Comparable {
    value: T
}

func swap_pair<T, U>(a: T, b: U) -> T where T: Comparable, U: Hashable {
    return a
}

struct KeyValue<K, V> where K: Hashable, V: ToString {
    key: K,
    value: V
}

// ============================
// 8. 泛型类 (#23)
// ============================
class Box<T> {
    var content: T;

    func getContent(self: Box<T>) -> T {
        return self.content
    }
}

class Stack<T> {
    var top: T;
    var size: Int64;

    func peek(self: Stack<T>) -> T {
        return self.top
    }

    func getSize(self: Stack<T>) -> Int64 {
        return self.size
    }
}

// ============================
// 9. 泛型特化 (#28)
// ============================
// 泛型版本
func add<T>(a: T, b: T) -> T {
    return a  // 通用版本，返回第一个参数
}

// 特化版本：Int64 的 add 直接用非泛型函数覆盖
// 注意：特化通过命名约定实现，单态化时优先使用已存在的具体函数

// ============================
// 测试函数
// ============================

func testBasicGeneric() -> Int64 {
    let x = identity<Int64>(42)
    let y = identity<Int32>(10)
    return x  // 42
}

func testGenericStruct() -> Int64 {
    let p = Pair<Int64, Int64> { first: 10, second: 20 }
    let t = Triple<Int64, Int64, Int64> { a: 1, b: 2, c: 3 }
    return p.first + p.second + t.a + t.b + t.c  // 36
}

func testConstrainedFunc() -> Int64 {
    let a = maximum<Int64>(100, 200)
    let b = clamp<Int64>(50, 10, 100)
    return a + b  // 150
}

func testConstrainedStruct() -> Int64 {
    let sp = SortedPair<Int64> { first: 5, second: 10 }
    return sp.first + sp.second  // 15
}

func testMultiConstraint() -> Int64 {
    let a = process<Int64>(55)
    let b = transform_value<Int64>(33)
    return a + b  // 88
}

func testWhereClause() -> Int64 {
    let c = Container<Int64> { value: 77 }
    let result = swap_pair<Int64, Int64>(10, 20)
    let kv = KeyValue<Int64, Int64> { key: 1, value: 2 }
    return c.value + result + kv.key + kv.value  // 90
}

func testGenericClass() -> Int64 {
    let b = Box<Int64> { content: 42 }
    let s = Stack<Int64> { top: 99, size: 1 }
    return b.getContent() + s.peek() + s.getSize()
    // 42 + 99 + 1 = 142
}

func testSpecialization() -> Int64 {
    let i64_result = add<Int64>(10, 20)
    let i32_result = add<Int32>(5, 15)
    return i64_result  // 10 (通用版本)
}

func testMixedTypes() -> Int64 {
    let p1 = Pair<Int64, Int32> { first: 100, second: 50 }
    let p2 = Pair<Bool, Int64> { first: true, second: 200 }
    return p1.first + p2.second  // 300
}

func main() -> Int64 {
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
