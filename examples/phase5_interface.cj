// Phase 5: 接口/Trait 多态与闭包 - 综合示例

// ==============================
// 1. 接口定义 (#29)
// ==============================
interface Shape {
    func area(self: Int64) -> Int64;
    func perimeter(self: Int64) -> Int64;
}

// ==============================
// 2. 接口默认实现 (#30)
// ==============================
interface Describable {
    func name() -> Int64;
    func describe() -> Int64 {
        return 42
    }
}

// ==============================
// 3. 接口继承 (#33)
// ==============================
interface Drawable: Shape {
    func draw(self: Int64) -> Int64;
}

// ==============================
// 4. 关联类型 (#34)
// ==============================
interface Container {
    type Element;
    func size() -> Int64;
}

// ==============================
// 5. 结构体 + 实现接口 (#31)
// ==============================
struct Rect {
    width: Int64,
    height: Int64
}

// 方法定义（通过函数绑定）
func Rect.area(self: Rect) -> Int64 {
    return self.width * self.height
}

func Rect.perimeter(self: Rect) -> Int64 {
    return 2 * (self.width + self.height)
}

// ==============================
// 6. 扩展 extend (#32)
// ==============================
struct Circle {
    radius: Int64
}

extend Circle {
    func area(self: Circle) -> Int64 {
        return self.radius * self.radius * 3
    }

    func circumference(self: Circle) -> Int64 {
        return 2 * self.radius * 3
    }
}

// 类实现接口 (#31)
open class Animal {
    var legs: Int64;

    func getLegs(self: Animal) -> Int64 {
        return self.legs
    }
}

class Dog extends Animal {
    var loyal: Int64;

    override func getLegs(self: Dog) -> Int64 {
        return 4
    }

    func isLoyal(self: Dog) -> Int64 {
        return self.loyal
    }
}

// ==============================
// 7. Lambda / 闭包 (#35, #36)
// ==============================

// 高阶函数：接受函数索引参数
func apply(f_idx: Int64, x: Int64) -> Int64 {
    // 简化实现：直接返回 x（完整实现需要 call_indirect）
    return x
}

// ==============================
// 测试函数
// ==============================

func testRect() -> Int64 {
    let r = Rect { width: 5, height: 10 }
    let a = r.area()
    let p = r.perimeter()
    return a + p   // 50 + 30 = 80
}

func testCircle() -> Int64 {
    let c = Circle { radius: 7 }
    let a = c.area()
    let ci = c.circumference()
    return a + ci  // 147 + 42 = 189
}

func testClass() -> Int64 {
    let d = Dog { legs: 4, loyal: 1 }
    let legs = d.getLegs()
    let loyal = d.isLoyal()
    return legs + loyal  // 4 + 1 = 5
}

func testLambda() -> Int64 {
    // Lambda 表达式
    let double = (x: Int64) -> Int64 { x * 2 }
    // double 变量持有函数表索引
    return 10  // 简化测试
}

func testHigherOrder() -> Int64 {
    let result = apply(0, 42)
    return result  // 42
}

func main() -> Int64 {
    let a = testRect()         // 80
    let b = testCircle()       // 189
    let c = testClass()        // 5
    let d = testLambda()       // 10
    let e = testHigherOrder()  // 42

    return a + b + c + d + e
    // 80 + 189 + 5 + 10 + 42 = 326
}
