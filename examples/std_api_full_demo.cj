// std API 全量用例：覆盖 L3 overlay 六模块（fs / time / math / env / random / runtime）
// 通过 import 拉取 overlay，合并后使用无前缀名
package main

import std.fs
import std.time
import std.math
import std.env
import std.random
import std.runtime

main() {
    println("=== std API 全量用例 ===")
    println("")

    // 【1】std.fs：SEEK_*、Path、exists
    println("【1】std.fs")
    println("  SEEK_SET = ${SEEK_SET}, SEEK_CUR = ${SEEK_CUR}, SEEK_END = ${SEEK_END}")
    let p = Path("test.txt")
    println("  Path(\"test.txt\").toString() = ${p.toString()}")
    let ex = exists(".")
    println("  exists(\".\") = ${ex}")
    let ex2 = exists("nonexistent_xyz_123")
    println("  exists(\"nonexistent_xyz_123\") = ${ex2}")
    println("")

    // 【2】std.time：overlay API（nowNs/nowSecNs/monotonic*）
    println("【2】std.time")
    let ns = nowNs()
    println("  nowNs() = ${ns}")
    let (sec, nsec) = nowSecNs()
    println("  nowSecNs() = (${sec}, ${nsec})")
    let (msec, mns) = monotonicSecNs()
    println("  monotonicSecNs() = (${msec}, ${mns})")
    let mns2 = monotonicNs()
    println("  monotonicNs() = ${mns2}")
    println("")

    // 【3】std.math：常量与全部函数
    println("【3】std.math")
    println("  PI = ${PI}, E = ${E}")
    println("  sqrt(2.0) = ${sqrt(2.0)}, ceil/floor = ${ceil(3.2)}/${floor(3.8)}")
    println("  trunc/round = ${trunc(-2.7)}/${round(2.5)}, abs(-4.5) = ${abs(-4.5)}")
    println("  min/max = ${min(1.0, 2.0)}/${max(1.0, 2.0)}")
    println("  sin/cos/tan(0) = ${sin(0.0)}/${cos(0.0)}/${tan(0.0)}")
    println("  exp(0)=${exp(0.0)}, log(E)=${log(E)}, pow(2,3)=${pow(2.0, 3.0)}")
    println("")

    // 【4】std.env：getEnv、getVariable(Option)
    println("【4】std.env")
    let home = getEnv("HOME")
    println("  getEnv(\"HOME\") = \"${home}\"")
    match (getVariable("HOME")) {
        case Some(s) => println("  getVariable(\"HOME\") = Some(\"${s}\")")
        case None => println("  getVariable(\"HOME\") = None")
    }
    println("")

    // 【5】std.random：randomInt64/Float64、Random 类
    println("【5】std.random")
    let r1 = randomInt64()
    let r2 = randomFloat64()
    println("  randomInt64() = ${r1}, randomFloat64() = ${r2}")
    let rng = Random()
    println("  Random().nextInt64() = ${rng.nextInt64()}, nextFloat64() = ${rng.nextFloat64()}")
    println("")

    // 【6】std.runtime：getArgs、线程/堆桩、GC、blackBox
    println("【6】std.runtime")
    let args = getArgs()
    println("  getArgs().size = ${args.size}")
    println("  getThreadCount() = ${getThreadCount()}, getProcessorCount() = ${getProcessorCount()}")
    println("  getMaxHeapSize() = ${getMaxHeapSize()}, getAllocatedHeapSize() = ${getAllocatedHeapSize()}")
    // GC(false)、blackBox(42) 等见 std.runtime overlay，此处省略以免与类型解析歧义
    println("")

    println("=== std API 全量用例执行完成 ===")
}
