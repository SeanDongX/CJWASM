// 小规模基准：简单函数
func add(a: Int64, b: Int64): Int64 {
    return a + b
}

func factorial(n: Int64): Int64 {
    if (n <= 1) {
        1
    } else {
        n * factorial(n - 1)
    }
}

func fib(n: Int64): Int64 {
    if (n <= 1) {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

main(): Int64 {
    let a = add(10, 20)
    let b = factorial(10)
    let c = fib(10)
    return a + b + c
}
