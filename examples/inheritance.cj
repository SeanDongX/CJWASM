// 类与继承示例 (Phase 3)

// === 1. 基础类（无继承，展平为结构体） ===
class Point {
    var x: Int64;
    var y: Int64;

    func sum(self: Point) -> Int64 {
        return self.x + self.y
    }
}

// === 2. 继承类 ===
class Animal {
    var kind: Int64;
    var age: Int64;

    func speak(self: Animal) -> Int64 {
        return 0
    }

    func getAge(self: Animal) -> Int64 {
        return self.age
    }
}

class Dog extends Animal {
    var breed: Int64;

    override func speak(self: Dog) -> Int64 {
        return 1  // "Woof"
    }

    func fetch(self: Dog) -> Int64 {
        return self.breed + 10
    }
}

class Cat extends Animal {
    var indoor: Int64;

    override func speak(self: Cat) -> Int64 {
        return 2  // "Meow"
    }
}

// === 3. 带 init 构造函数的类 ===
class Counter {
    var count: Int64;

    init(start: Int64) {
        this.count = start
    }

    func getValue(self: Counter) -> Int64 {
        return self.count
    }
}

// === 4. 带 deinit 析构函数的类 ===
class Resource {
    var id: Int64;

    init(rid: Int64) {
        this.id = rid
    }

    func getId(self: Resource) -> Int64 {
        return self.id
    }
}

// === 5. 带 prop 属性的类 ===
class Rect {
    var w: Int64;
    var h: Int64;

    prop area: Int64 {
        get() {
            return this.w * this.h
        }
    }
}

// === 测试函数 ===

func testPoint() -> Int64 {
    let p = Point { x: 3, y: 4 }
    return p.sum()  // 7
}

func testInheritedFields() -> Int64 {
    let dog = Dog { kind: 42, age: 5, breed: 100 }
    let age = dog.getAge()
    let breed_val = dog.fetch()
    return age + breed_val  // 5 + 110 = 115
}

func testOverride() -> Int64 {
    let dog = Dog { kind: 1, age: 3, breed: 50 }
    let cat = Cat { kind: 2, age: 4, indoor: 1 }
    return dog.speak() + cat.speak()  // 1 + 2 = 3
}

func testInit() -> Int64 {
    let c = Counter(10)
    return c.getValue()  // 10
}

func testResource() -> Int64 {
    let r = Resource(42)
    return r.getId()  // 42
}

func testProp() -> Int64 {
    let r = Rect { w: 6, h: 7 }
    return r.__get_area()  // 6 * 7 = 42
}

func main() -> Int64 {
    let a = testPoint()            // 7
    let b = testInheritedFields()  // 115
    let c = testOverride()         // 3
    let d = testInit()             // 10
    let e = testResource()         // 42
    let f = testProp()             // 42

    return a + b + c + d + e + f
    // 7 + 115 + 3 + 10 + 42 + 42 = 219
}
