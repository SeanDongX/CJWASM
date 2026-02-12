// 模块与可见性示例

module examples.demo

// 导入示例（语法展示）
// import std.io
// import std.math.{sin, cos}
// import utils as u
// import helpers.*

// public 可见性 - 可从模块外部访问
public func publicAdd(a: Int64, b: Int64) -> Int64 {
    return a + b
}

public struct PublicPoint {
    x: Int64,
    y: Int64
}

// private 可见性 - 仅模块内部可访问
private func privateHelper(x: Int64) -> Int64 {
    return x * 2
}

private struct PrivateData {
    value: Int64
}

// 默认可见性（等同于 internal）
func internalFunc(x: Int64) -> Int64 {
    // 可以调用私有函数
    return privateHelper(x) + 1
}

// 结构体方法的可见性
public struct Calculator {
    value: Int64
}

// 公开方法
public func Calculator.add(self: Calculator, n: Int64) -> Calculator {
    return Calculator { value: self.value + n }
}

// 私有辅助方法
private func Calculator.validate(self: Calculator) -> Bool {
    return self.value >= 0
}

// 公开方法可以使用私有方法
public func Calculator.safeAdd(self: Calculator, n: Int64) -> Calculator {
    let next = Calculator { value: self.value + n }
    // 这里可以调用 validate
    return next
}

// 枚举的可见性
public enum Status {
    Active,
    Inactive,
    Pending
}

private enum InternalState {
    Ready,
    Processing,
    Done
}

// extern 函数导入（与外部 WASM 模块互操作）
// @import("env", "log")
// extern func hostLog(ptr: Int32, len: Int32)

// @import("env", "get_time")
// extern func getTime() -> Int64

func main() -> Int64 {
    // 使用公开 API
    let sum = publicAdd(10, 20)

    let p = PublicPoint { x: 5, y: 10 }

    let calc = Calculator { value: 100 }
    let calc2 = calc.add(50)

    let status = Status.Active

    // 使用内部函数
    let internal_result = internalFunc(5)

    return sum + p.x + calc2.value + internal_result
    // 30 + 5 + 150 + 11 = 196
}
