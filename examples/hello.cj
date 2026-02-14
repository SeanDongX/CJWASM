// 简单的加法函数
func add(a: Int64, b: Int64): Int64 {
    return a + b
}

// 阶乘函数
func factorial(n: Int64): Int64 {
    if (n <= 1) {
        1
    } else {
        n * factorial(n - 1)
    }
}

// 斐波那契数列
func fib(n: Int64): Int64 {
    if (n <= 1) {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

main(): Int64 {
    return factorial(5)
}
