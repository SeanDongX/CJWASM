// GC 路线 A 冒烟测试：单次分配超过初始内存容量（4 页 = 256KB）。
//
// 默认 CHIR 后端内存声明为 min=4 页 / max=256 页，`__alloc` 的 bump 分支
// 在 `heap_ptr + size` 越过当前线性内存时会按需 `memory.grow`。
// 下面构造 100000 个 Int64 的数组（约 800KB），必然触发一次 grow；
// 若 grow 兜底缺失，这里会 out-of-bounds trap。
//
// 预期输出: 14
main(): Int64 {
    let arr = Array<Int64>(100000, 7)
    arr[0] = 3
    arr[99999] = 4
    return arr[0] + arr[1] + arr[99999]
}
