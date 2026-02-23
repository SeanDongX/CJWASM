// std API 可编译烟测：通过 import 拉取 overlay（fs/time/math/random），合并后使用无前缀名
package main

import std.fs
import std.time
import std.math
import std.random

main() {
    println("=== std API smoke ===")
    // std.fs（合并后解析为 overlay 的 exists/Path）
    let _ = exists(".")
    let q = Path("a")
    let _ = q.toString()
    // std.time
    let _ = nowNs()
    let (s, ns) = nowSecNs()
    let _ = monotonicNs()
    // std.math
    let _ = sqrt(2.0)
    let _ = ceil(3.2)
    let _ = floor(3.8)
    let _ = trunc(2.7)
    let _ = round(2.5)
    let _ = abs(-1.0)
    let _ = min(1.0, 2.0)
    let _ = max(1.0, 2.0)
    let _ = sin(0.0)
    let _ = cos(0.0)
    let _ = tan(0.0)
    let _ = exp(0.0)
    let _ = log(2.718281828459045)
    let _ = pow(2.0, 3.0)
    let _ = PI
    let _ = E
    // std.random
    let _ = randomInt64()
    let _ = randomFloat64()
    let rng = Random()
    let _ = rng.nextInt64()
    let _ = rng.nextFloat64()
    println("smoke OK")
}
