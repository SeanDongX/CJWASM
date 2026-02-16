// P5 特性验证: spawn / synchronized / AtomicInt64 / AtomicBool / Mutex

// =============================================================
// spawn: 单线程桩 — 直接同步执行 block
// =============================================================

func testSpawn(): Int64 {
    var result: Int64 = 0

    spawn {
        result = result + 10
    }

    spawn {
        result = result + 20
    }

    return result  // 30
}

// =============================================================
// synchronized: 单线程桩 — 直接执行 block
// =============================================================

func testSynchronized(): Int64 {
    var result: Int64 = 0
    let lock = Mutex()

    synchronized(lock) {
        result = result + 100
    }

    synchronized(lock) {
        result = result + 50
    }

    return result  // 150
}

// =============================================================
// AtomicInt64: load / store / fetchAdd / compareAndSwap
// =============================================================

func testAtomicInt64(): Int64 {
    var result: Int64 = 0

    let counter = AtomicInt64(0)

    // store + load
    counter.store(42)
    result = result + counter.load()  // +42

    // fetchAdd
    let old = counter.fetchAdd(8)
    result = result + old             // +42 (旧值)
    result = result + counter.load()  // +50 (42+8)

    // compareAndSwap (成功)
    let ok = counter.compareAndSwap(50, 100)
    result = result + ok              // +1 (true)
    result = result + counter.load()  // +100

    // compareAndSwap (失败)
    let fail = counter.compareAndSwap(999, 200)
    result = result + fail            // +0 (false)
    result = result + counter.load()  // +100 (未变)

    return result  // 42 + 42 + 50 + 1 + 100 + 0 + 100 = 335
}

// =============================================================
// AtomicBool: load / store / compareAndSwap
// =============================================================

func testAtomicBool(): Int64 {
    var result: Int64 = 0

    let flag = AtomicBool()  // 默认 false (0)
    result = result + flag.load()  // +0

    flag.store(1)  // true
    result = result + flag.load()  // +1

    // compareAndSwap: 1 → 0
    let ok = flag.compareAndSwap(1, 0)
    result = result + ok           // +1 (成功)
    result = result + flag.load()  // +0

    return result  // 0 + 1 + 1 + 0 = 2
}

// =============================================================
// Mutex: lock / unlock / tryLock (均为空操作桩)
// =============================================================

func testMutex(): Int64 {
    var result: Int64 = 0

    let m = Mutex()
    m.lock()
    result = result + 10
    m.unlock()

    let ok = m.tryLock()
    result = result + ok  // +1
    m.unlock()

    let rm = ReentrantMutex()
    rm.lock()
    rm.lock()  // 可重入
    result = result + 5
    rm.unlock()
    rm.unlock()

    return result  // 10 + 1 + 5 = 16
}

// =============================================================
// 主函数
// =============================================================

main(): Int64 {
    let a = testSpawn()          // 30
    let b = testSynchronized()   // 150
    let c = testAtomicInt64()    // 335
    let d = testAtomicBool()     // 2
    let e = testMutex()          // 16

    println(a)
    println(b)
    println(c)
    println(d)
    println(e)

    @Assert(a, 30)
    @Assert(b, 150)
    @Assert(c, 335)
    @Assert(d, 2)
    @Assert(e, 16)

    let total = a + b + c + d + e
    @Assert(total, 533)

    return total
    // 预期输出: 533
}
