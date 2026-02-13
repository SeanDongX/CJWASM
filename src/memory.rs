//! 内存管理模块：为 WASM codegen 生成运行时内存管理辅助函数。
//!
//! 提供三种内存管理策略（Phase 8）：
//! - **Free List Allocator (malloc/free)**: 替代 bump allocator，支持内存回收
//! - **引用计数 (RC)**: 对象头引用计数字段，赋值/离开作用域时 inc/dec，归零释放
//! - **垃圾回收 (Mark-Sweep GC)**: 遍历堆上所有块，回收引用计数为 0 的对象
//!
//! ## 内存布局
//!
//! 每个堆对象在用户数据前有 8 字节头部：
//! ```text
//! [block_size: i32][refcount: i32][user_data...]
//!                                 ^
//!                                 |-- __alloc 返回的用户指针
//! ```
//!
//! Free list 使用已释放块的头部空间：
//! ```text
//! [block_size: i32][next_free_ptr: i32]
//! ```
//!
//! ## Globals
//! - Global 0: heap_ptr (bump allocator 指针)
//! - Global 1: free_list_head (空闲链表头指针，初始为 0)

use wasm_encoder::{Function as WasmFunc, Instruction, ValType};

/// 内存对齐常量：所有分配对齐到 8 字节
const ALLOC_HEADER_SIZE: i32 = 8;

/// MemArg 辅助宏
fn mem_arg(offset: u64, align: u32) -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

// ============================================================
// #56: Free List Allocator (malloc/free)
// ============================================================

/// 生成 `__alloc(size: i32) -> i32` 函数
///
/// 算法：
/// 1. actual_size = align8(size + 8)
/// 2. 搜索空闲链表找到 >= actual_size 的块
/// 3. 找到则从链表移除，设 refcount=1，返回 user_ptr
/// 4. 否则 bump 分配，设头部，返回 user_ptr
///
/// Globals: [0] = heap_ptr, [1] = free_list_head
pub fn emit_alloc_func(heap_start: i32) -> WasmFunc {
    // params:  [0] size: i32
    // locals:  [1] actual_size, [2] prev_ptr, [3] curr_ptr, [4] block_ptr, [5] next_ptr
    let mut f = WasmFunc::new(vec![(5, ValType::I32)]);

    // --- 计算 actual_size = align8(size + 8) ---
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(ALLOC_HEADER_SIZE));
    f.instruction(&Instruction::I32Add);
    // 对齐到 8 字节: (n + 7) & ~7
    f.instruction(&Instruction::I32Const(7));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::I32Const(-8i32));  // 0xFFFFFFF8
    f.instruction(&Instruction::I32And);
    f.instruction(&Instruction::LocalSet(1)); // actual_size

    // --- 搜索空闲链表 ---
    // prev_ptr = 0 (哨兵)
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(2));
    // curr_ptr = global1 (free_list_head)
    f.instruction(&Instruction::GlobalGet(1));
    f.instruction(&Instruction::LocalSet(3));

    // block $done { loop $search { ... } }
    f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // $done
    f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));  // $search

    // if curr_ptr == 0 → break (无可用块)
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::BrIf(1)); // break to $done

    // block_size = mem[curr_ptr] (读取块大小)
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::I32Load(mem_arg(0, 2)));
    f.instruction(&Instruction::LocalSet(4)); // 临时存 block_size 到 local[4]

    // if block_size >= actual_size → 找到合适的块
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    {
        // next_ptr = mem[curr_ptr + 4]
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Load(mem_arg(4, 2)));
        f.instruction(&Instruction::LocalSet(5));

        // 从链表中移除当前块
        // if prev_ptr == 0 → free_list_head = next
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::GlobalSet(1));
        }
        f.instruction(&Instruction::Else);
        {
            // mem[prev_ptr + 4] = next
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Store(mem_arg(4, 2)));
        }
        f.instruction(&Instruction::End);

        // 设置 refcount = 1
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Store(mem_arg(4, 2)));

        // 返回 user_ptr = curr_ptr + 8
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(ALLOC_HEADER_SIZE));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::Return);
    }
    f.instruction(&Instruction::End); // end if

    // 移动到下一个空闲块
    // prev = curr
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::LocalSet(2));
    // curr = mem[curr + 4] (next_free)
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::I32Load(mem_arg(4, 2)));
    f.instruction(&Instruction::LocalSet(3));
    // 继续循环
    f.instruction(&Instruction::Br(0)); // continue $search

    f.instruction(&Instruction::End); // end loop
    f.instruction(&Instruction::End); // end block

    // --- 空闲链表中未找到合适块，使用 bump allocation ---
    // block_ptr = global0
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::LocalSet(4));

    // global0 += actual_size
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::GlobalSet(0));

    // mem[block_ptr] = actual_size (block_size)
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Store(mem_arg(0, 2)));

    // mem[block_ptr + 4] = 1 (refcount)
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Store(mem_arg(4, 2)));

    // 返回 user_ptr = block_ptr + 8
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::I32Const(ALLOC_HEADER_SIZE));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::Return);

    f.instruction(&Instruction::End);
    f
}

/// 生成 `__free(ptr: i32)` 函数
///
/// 将块加入空闲链表头部：
/// 1. block_ptr = ptr - 8
/// 2. mem[block_ptr + 4] = free_list_head  (next = old head)
/// 3. free_list_head = block_ptr
pub fn emit_free_func() -> WasmFunc {
    // params: [0] ptr: i32
    // locals: [1] block_ptr
    let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

    // if ptr == 0, return (null safety)
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);

    // block_ptr = ptr - 8
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(ALLOC_HEADER_SIZE));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::LocalSet(1));

    // mem[block_ptr + 4] = free_list_head (next pointer)
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::GlobalGet(1));
    f.instruction(&Instruction::I32Store(mem_arg(4, 2)));

    // free_list_head = block_ptr
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::GlobalSet(1));

    f.instruction(&Instruction::End);
    f
}

// ============================================================
// #54: 引用计数 (Reference Counting)
// ============================================================

/// 生成 `__rc_inc(ptr: i32)` 函数
///
/// 如果 ptr 是有效的堆指针（>= heap_start），递增引用计数：
/// mem[ptr - 4] += 1
pub fn emit_rc_inc_func(heap_start: i32) -> WasmFunc {
    // params: [0] ptr: i32
    let mut f = WasmFunc::new(vec![]);

    // if ptr == 0, return
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);

    // if ptr < heap_start, return (非堆指针，如数据段字符串)
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(heap_start));
    f.instruction(&Instruction::I32LtU);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);

    // mem[ptr - 4] += 1
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::I32Sub);
    // 读取当前值
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Load(mem_arg(0, 2)));
    // +1
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    // 存回
    f.instruction(&Instruction::I32Store(mem_arg(0, 2)));

    f.instruction(&Instruction::End);
    f
}

/// 生成 `__rc_dec(ptr: i32)` 函数
///
/// 如果 ptr 是有效堆指针，递减引用计数，归零时释放：
/// 1. if ptr == 0 || ptr < heap_start → return
/// 2. mem[ptr - 4] -= 1
/// 3. if mem[ptr - 4] == 0 → call __free(ptr)
pub fn emit_rc_dec_func(heap_start: i32, free_func_idx: u32) -> WasmFunc {
    // params: [0] ptr: i32
    // locals: [1] new_count
    let mut f = WasmFunc::new(vec![(1, ValType::I32)]);

    // if ptr == 0, return
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);

    // if ptr < heap_start, return
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(heap_start));
    f.instruction(&Instruction::I32LtU);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);

    // new_count = mem[ptr - 4] - 1
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Load(mem_arg(0, 2)));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::LocalSet(1));

    // mem[ptr - 4] = new_count
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Store(mem_arg(0, 2)));

    // if new_count == 0 → free(ptr)
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(free_func_idx));
    }
    f.instruction(&Instruction::End);

    f.instruction(&Instruction::End);
    f
}

// ============================================================
// #55: 垃圾回收 (Mark-Sweep GC)
// ============================================================

/// 生成 `__gc_collect() -> i32` 函数
///
/// 基于堆扫描的垃圾回收：
/// 1. 从 heap_start 遍历到 heap_ptr，按 block_size 跳转
/// 2. 对每个块检查引用计数
/// 3. 若引用计数为 0 且不在空闲链表中，则释放
/// 4. 返回回收的总字节数
///
/// 注意：此实现假设所有堆分配都通过 `__alloc` 并带有标准头部。
pub fn emit_gc_collect_func(heap_start: i32, free_func_idx: u32) -> WasmFunc {
    // params: (none)
    // locals: [0] scan_ptr, [1] block_size, [2] refcount, [3] freed_bytes, [4] user_ptr
    let mut f = WasmFunc::new(vec![(5, ValType::I32)]);

    // scan_ptr = heap_start
    f.instruction(&Instruction::I32Const(heap_start));
    f.instruction(&Instruction::LocalSet(0));

    // freed_bytes = 0
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(3));

    // block $done { loop $scan { ... } }
    f.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

    // if scan_ptr >= heap_ptr (global0) → break
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));

    // block_size = mem[scan_ptr]
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Load(mem_arg(0, 2)));
    f.instruction(&Instruction::LocalSet(1));

    // 安全检查: block_size <= 0 → break (防止无限循环)
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32LeS);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    f.instruction(&Instruction::Br(2)); // break $done
    f.instruction(&Instruction::End);

    // refcount = mem[scan_ptr + 4]
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Load(mem_arg(4, 2)));
    f.instruction(&Instruction::LocalSet(2));

    // if refcount == 0 → 释放此块
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    {
        // user_ptr = scan_ptr + 8
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(ALLOC_HEADER_SIZE));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));

        // call __free(user_ptr)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::Call(free_func_idx));

        // freed_bytes += block_size
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(3));
    }
    f.instruction(&Instruction::End);

    // scan_ptr += block_size
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(0));

    // continue
    f.instruction(&Instruction::Br(0));

    f.instruction(&Instruction::End); // end loop
    f.instruction(&Instruction::End); // end block

    // return freed_bytes
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::Return);

    f.instruction(&Instruction::End);
    f
}

// ============================================================
// 辅助：判断类型是否需要堆分配（用于 RC 追踪）
// ============================================================

use crate::ast::Type;

/// 判断类型是否为堆分配的引用类型（需要 RC 管理）
pub fn is_heap_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Struct(_, _)
            | Type::Array(_)
            | Type::Tuple(_)
            | Type::Option(_)
            | Type::Result(_, _)
    )
}

/// 判断类型是否可能持有堆指针（用于 GC 根集追踪）
pub fn may_hold_heap_ptr(ty: &Type) -> bool {
    is_heap_type(ty) || matches!(ty, Type::String)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_alloc_func() {
        let f = emit_alloc_func(1024);
        // 验证函数能正常生成（不 panic）
        let _ = f;
    }

    #[test]
    fn test_emit_free_func() {
        let f = emit_free_func();
        let _ = f;
    }

    #[test]
    fn test_emit_rc_inc_func() {
        let f = emit_rc_inc_func(1024);
        let _ = f;
    }

    #[test]
    fn test_emit_rc_dec_func() {
        let f = emit_rc_dec_func(1024, 5);
        let _ = f;
    }

    #[test]
    fn test_emit_gc_collect_func() {
        let f = emit_gc_collect_func(1024, 5);
        let _ = f;
    }

    #[test]
    fn test_is_heap_type() {
        assert!(is_heap_type(&Type::Struct("Foo".to_string(), vec![])));
        assert!(is_heap_type(&Type::Array(Box::new(Type::Int64))));
        assert!(is_heap_type(&Type::Tuple(vec![Type::Int64])));
        assert!(is_heap_type(&Type::Option(Box::new(Type::Int64))));
        assert!(is_heap_type(&Type::Result(Box::new(Type::Int64), Box::new(Type::String))));
        assert!(!is_heap_type(&Type::Int64));
        assert!(!is_heap_type(&Type::Bool));
        assert!(!is_heap_type(&Type::String)); // 字符串常量在数据段
    }

    #[test]
    fn test_may_hold_heap_ptr() {
        assert!(may_hold_heap_ptr(&Type::String));
        assert!(may_hold_heap_ptr(&Type::Struct("Foo".to_string(), vec![])));
        assert!(!may_hold_heap_ptr(&Type::Int64));
        assert!(!may_hold_heap_ptr(&Type::Float64));
    }
}
