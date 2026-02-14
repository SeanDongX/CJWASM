// 模块与可见性示例（cjc：无 module 块，包级组织）

// 导入示例（语法展示）
// import std.io
// import std.math.{sin, cos}
// import utils as u
// import helpers.*

// public 可见性 - 可从模块外部访问
public func publicAdd(a: Int64, b: Int64): Int64 {
    return a + b
}

public struct PublicPoint {
    var x: Int64;
    var y: Int64;

    init(x: Int64, y: Int64) {
        this.x = x
        this.y = y
    }
}

// private 可见性 - 仅模块内部可访问
private func privateHelper(x: Int64): Int64 {
    return x * 2
}

private struct PrivateData {
    var value: Int64;

    init(value: Int64) {
        this.value = value
    }
}

// 默认可见性（等同于 internal）
func internalFunc(x: Int64): Int64 {
    return privateHelper(x) + 1
}

// 结构体方法的可见性
public struct Calculator {
    var value: Int64;

    init(value: Int64) {
        this.value = value
    }

    func add(n: Int64): Calculator {
        return Calculator(this.value + n)
    }

    func validate(): Bool {
        return this.value >= 0
    }

    func safeAdd(n: Int64): Calculator {
        let next = Calculator(this.value + n)
        return next
    }
}

// 枚举的可见性
public enum Status {
    | Active
    | Inactive
    | Pending
}

private enum InternalState {
    | Ready
    | Processing
    | Done
}

main(): Int64 {
    let sum = publicAdd(10, 20)
    let p = PublicPoint(5, 10)
    let calc = Calculator(100)
    let calc2 = calc.add(50)
    let status = Status.Active
    let internal_result = internalFunc(5)
    return sum + p.x + calc2.value + internal_result  // 预期输出: 196
}
