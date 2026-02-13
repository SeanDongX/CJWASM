// Phase 6: 错误处理完善 + 模块系统 示例
// 展示 Error 类, throws 声明, try-catch-finally, 多文件编译

// =============================================================
// #37 Error 类 — 内置 Error 基类 + 自定义错误继承体系
// =============================================================

// 自定义错误类继承自 Error
open class DivideError extends Error {
    var code: Int64;

    init(code: Int64) {
        this.code = code
    }
}

// 另一个自定义错误
class ValidationError extends Error {
    init() {
    }
}

// =============================================================
// #38 throws 声明 — func f() throws ErrorType -> RetType
// =============================================================

// 带 throws 声明的函数
func safeDivide(a: Int64, b: Int64) throws DivideError -> Int64 {
    if b == 0 {
        throw 1001
    }
    return a / b
}

// throws 不指定具体类型，默认为 Error
func validate(value: Int64) throws -> Int64 {
    if value < 0 {
        throw 0
    }
    return value
}

// =============================================================
// #39 finally — try-catch-finally 完整支持
// =============================================================

// finally 块确保清理代码一定执行
func computeWithCleanup(a: Int64, b: Int64) -> Int64 {
    var result: Int64 = 0
    var cleaned: Int64 = 0

    try {
        if b == 0 {
            throw 0
        }
        result = a / b
    } catch(e) {
        result = -1  // 错误时返回 -1
    } finally {
        // 无论是否异常都执行清理
        cleaned = 1
    }

    return result + cleaned
}

// 嵌套 try-catch-finally
func nestedTry(x: Int64) -> Int64 {
    var outer: Int64 = 0
    var inner: Int64 = 0

    try {
        try {
            if x == 0 {
                throw 0
            }
            inner = x * 10
        } catch(e1) {
            inner = -1
        } finally {
            inner = inner + 1  // 内层 finally
        }
        outer = inner + 100
    } catch(e2) {
        outer = -100
    } finally {
        outer = outer + 1  // 外层 finally
    }

    return outer
}

// try-catch 无 finally（兼容旧语法）
func tryCatchOnly(x: Int64) -> Int64 {
    try {
        if x < 0 {
            throw 0
        }
        return x * 2
    } catch(e) {
        return 0
    }
}

// =============================================================
// 综合测试
// =============================================================

// 使用 Result 处理错误
func divide(a: Int64, b: Int64) -> Result<Int64, String> {
    if b == 0 {
        return Err("Cannot divide by zero")
    }
    return Ok(a / b)
}

func handleResult(r: Result<Int64, String>) -> Int64 {
    match r {
        Ok(value) => value,
        Err(_) => 0
    }
}

func main() -> Int64 {
    // 测试 try-catch-finally
    let t1 = computeWithCleanup(10, 2)     // 5 + 1 = 6
    let t2 = computeWithCleanup(10, 0)     // -1 + 1 = 0

    // 测试 try-catch 无 finally
    let t5 = tryCatchOnly(7)   // 14
    let t6 = tryCatchOnly(-1)  // 0

    // 测试 Result
    let r1 = divide(20, 4)
    let v1 = handleResult(r1)  // 5

    return t1 + t2 + t5 + t6 + v1   // 6 + 0 + 14 + 0 + 5 = 25
}
