// CJWasm cjpm 工程示例 - 入口文件
//
// 编译: cjwasm build
// 运行: wasmtime run --invoke main target/wasm/demo.wasm

package demo

func main(): Int64 {
    // 基本运算
    let a = add(10, 20)           // 30
    let b = multiply(3, 7)        // 21

    // 几何计算
    let area = rectArea(5, 8)     // 40
    let perim = rectPerimeter(5, 8) // 26

    // 数组工具
    let sum = sumRange(1, 5)      // 15 (1+2+3+4+5)

    // 字符串
    println("Demo 项目运行结果:")
    println("  add(10, 20) = ${a}")
    println("  multiply(3, 7) = ${b}")
    println("  rectArea(5, 8) = ${area}")
    println("  rectPerimeter(5, 8) = ${perim}")
    println("  sumRange(1, 5) = ${sum}")

    let total = a + b + area + perim + sum
    println("  total = ${total}")

    return total  // 预期输出: 132
}
