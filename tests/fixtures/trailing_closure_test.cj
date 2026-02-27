// 测试尾随闭包支持

func map(arr: Array<Int64>, f: (Int64) -> Int64): Array<Int64> {
    let result = Array<Int64>()
    for (i in 0..arr.size()) {
        result.append(f(arr[i]))
    }
    return result
}

func filter(arr: Array<Int64>, pred: (Int64) -> Bool): Array<Int64> {
    let result = Array<Int64>()
    for (i in 0..arr.size()) {
        if (pred(arr[i])) {
            result.append(arr[i])
        }
    }
    return result
}

main(): Int64 {
    let arr = Array<Int64>()
    arr.append(1)
    arr.append(2)
    arr.append(3)
    arr.append(4)
    arr.append(5)

    // 测试 1: 尾随闭包 - map
    let doubled = map(arr) { x => x * 2 }

    // 测试 2: 尾随闭包 - filter
    let evens = filter(arr) { x => x % 2 == 0 }

    var result: Int64 = 0

    // 计算 doubled 的和: 2+4+6+8+10 = 30
    for (i in 0..doubled.size()) {
        result = result + doubled[i]
    }

    // 计算 evens 的和: 2+4 = 6
    for (i in 0..evens.size()) {
        result = result + evens[i]
    }

    return result  // 30 + 6 = 36
}
