// 多文件编译示例 - 主文件
// 编译命令: cjwasm examples/module_main.cj examples/module_lib.cj -o module_test.wasm
module examples.main

import examples.lib

func main() -> Int64 {
    let a = add(10, 20)       // 30
    let b = multiply(3, 4)    // 12
    let c = square(5)         // 25
    return a + b + c          // 67
}
