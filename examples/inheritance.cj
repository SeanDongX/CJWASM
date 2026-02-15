// 类与继承示例 (Phase 3)

// === 1. 基础类（无继承，展平为结构体） ===
class Point {
    var x: Int64;
    var y: Int64;

    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }

    func sum(): Int64 {
        return this.x + this.y
    }
}

// === 2. 继承类 ===
open class Animal {
    var kind: Int64;
    var age: Int64;

    init(kind: Int64, age: Int64) {
        this.kind = kind
        this.age = age
    }

    func speak(): Int64 {
        return 0
    }

    func getAge(): Int64 {
        return this.age
    }
}

class Dog <: Animal {
    var breed: Int64;

    init(kind: Int64, age: Int64, breed: Int64) {
        super(kind, age)
        this.breed = breed
    }

    override func speak(): Int64 {
        return 1  // "Woof"
    }

    func fetch(): Int64 {
        return this.breed + 10
    }
}

class Cat <: Animal {
    var indoor: Int64;

    init(kind: Int64, age: Int64, indoor: Int64) {
        super(kind, age)
        this.indoor = indoor
    }

    override func speak(): Int64 {
        return 2  // "Meow"
    }
}

// === 3. 带 init 构造函数的类 ===
class Counter {
    var count: Int64;

    init(start: Int64) {
        this.count = start
    }

    func getValue(): Int64 {
        return this.count
    }
}

// === 4. 带 ~init 析构函数的类 ===
class Resource {
    var id: Int64;

    init(rid: Int64) {
        this.id = rid
    }

    func getId(): Int64 {
        return this.id
    }
}

// === 5. 带 prop 属性的类 ===
class Rect {
    var w: Int64;
    var h: Int64;

    init(w: Int64, h: Int64) {
        this.w = w
        this.h = h
    }

    prop area: Int64 {
        get() {
            return this.w * this.h
        }
    }
}

// === 测试函数 ===

func testPoint(): Int64 {
    let p = Point(3, 4)
    return p.sum()  // 7
}

func testInheritedFields(): Int64 {
    let dog = Dog(42, 5, 100)
    let age = dog.getAge()
    let breed_val = dog.fetch()
    return age + breed_val  // 5 + 110 = 115
}

func testOverride(): Int64 {
    let dog = Dog(1, 3, 50)
    let cat = Cat(2, 4, 1)
    return dog.speak() + cat.speak()  // 1 + 2 = 3
}

func testInit(): Int64 {
    let c = Counter(10)
    return c.getValue()  // 10
}

func testResource(): Int64 {
    let r = Resource(42)
    return r.getId()  // 42
}

func testProp(): Int64 {
    let r = Rect(6, 7)
    return r.area  // 6 * 7 = 42
}

main(): Int64 {
    let a = testPoint()            // 7
    let b = testInheritedFields()  // 115
    let c = testOverride()         // 3
    let d = testInit()             // 10
    let e = testResource()         // 42
    let f = testProp()             // 42

    @Assert(a + b + c + d + e + f, 219)

    return a + b + c + d + e + f
    // 预期输出: 219
}
