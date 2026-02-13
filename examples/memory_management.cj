// Phase 8: 内存管理示例
//
// 本示例展示 cjwasm 的三种内存管理机制：
// 1. Free List Allocator (malloc/free) - 替代 bump allocator 的可回收内存分配器
// 2. 引用计数 (RC) - 对象头引用计数字段，赋值/离开作用域时 inc/dec，归零释放
// 3. 垃圾回收 (Mark-Sweep GC) - 遍历堆上所有块，回收引用计数为 0 的对象
//
// 编译命令: cargo run -- examples/memory_management.cj -o memory_management.wasm
//
// 内存布局 (Phase 8):
//   每个堆对象有 8 字节头部:
//   [block_size: i32][refcount: i32][user_data...]
//                                   ^-- __alloc 返回的用户指针
//
// 导出的内存管理 API:
//   __alloc(size: i32) -> i32     -- 分配 size 字节，返回用户指针
//   __free(ptr: i32)              -- 释放对象，将块加入空闲链表
//   __rc_inc(ptr: i32)            -- 递增引用计数
//   __rc_dec(ptr: i32)            -- 递减引用计数，归零时自动释放
//   __gc_collect() -> i32         -- 执行垃圾回收，返回回收的字节数

// ==========================================================
// 示例 1: 结构体分配（自动通过 __alloc 管理）
// ==========================================================
struct Point {
    x: Int64
    y: Int64
}

struct Rect {
    origin: Int64
    width: Int64
    height: Int64
}

func createPoint(x: Int64, y: Int64) -> Point {
    return Point { x: x, y: y }
}

func pointSum(p: Point) -> Int64 {
    return p.x + p.y
}

// ==========================================================
// 示例 2: 类与引用计数
// ==========================================================
class Counter {
    var count: Int64;
    init(start: Int64) {
        this.count = start
    }
    func get(self: Counter) -> Int64 {
        return self.count
    }
    func increment(self: Counter) -> Int64 {
        return self.count + 1
    }
}

// 带 deinit 的类 —— deinit 会在引用计数归零时触发
class Resource {
    var id: Int64;
    init(id: Int64) {
        this.id = id
    }
    deinit {
        // 释放外部资源（示意）
        let cleanup = 0
    }
    func getId(self: Resource) -> Int64 {
        return self.id
    }
}

// ==========================================================
// 示例 3: 枚举与关联值的堆分配
// ==========================================================
enum Shape {
    Circle(Int64)
    Rectangle(Int64)
    Empty
}

func shapeValue(s: Shape) -> Int64 {
    return match s {
        Shape.Circle(r) => r * r * 3,
        Shape.Rectangle(side) => side * side,
        Shape.Empty => 0,
        _ => -1
    }
}

// ==========================================================
// 示例 4: 数组和元组的动态分配
// ==========================================================
func sumArray(arr: Array<Int64>) -> Int64 {
    var sum: Int64 = 0
    var i: Int64 = 0
    while i < 3 {
        sum = sum + arr[i]
        i = i + 1
    }
    return sum
}

// ==========================================================
// 示例 5: 引用计数与变量重新赋值
// ==========================================================
// 当 var 变量被重新赋值时，编译器自动：
// 1. 对旧值调用 __rc_dec（可能触发释放）
// 2. 存储新值
func testReassignment() -> Int64 {
    var p = Point { x: 1, y: 2 }  // refcount = 1
    p = Point { x: 3, y: 4 }      // 旧 Point 的 rc_dec，新 Point refcount = 1
    p = Point { x: 5, y: 6 }      // 同上
    return p.x + p.y               // 返回 11
}

// ==========================================================
// 示例 6: 循环中的内存分配
// ==========================================================
// 每次循环迭代创建新的堆对象。
// 释放的块进入空闲链表，后续迭代可复用。
func loopAllocation() -> Int64 {
    var total: Int64 = 0
    var i: Int64 = 0
    while i < 10 {
        let item = Point { x: i, y: i * 2 }  // 分配（可能从空闲链表复用）
        total = total + item.x + item.y
        i = i + 1
    }
    return total
}

// ==========================================================
// 主函数：运行所有示例
// ==========================================================
func main() -> Int64 {
    // 示例 1: 结构体
    let p = createPoint(10, 20)
    let pSum = pointSum(p)

    // 示例 2: 类
    let c = Counter(100)
    let cVal = c.get()

    let r = Resource(42)
    let rId = r.getId()

    // 示例 3: 枚举
    let circle = Shape.Circle(5)
    let sv = shapeValue(circle)

    // 示例 4: 数组和元组
    let arr = [10, 20, 30]
    let aSum = sumArray(arr)
    let t = (1, 2, 3)

    // 示例 5: 引用计数
    let rcResult = testReassignment()

    // 示例 6: 循环分配
    let loopResult = loopAllocation()

    return pSum + cVal + rId + sv + aSum + rcResult + loopResult
}
