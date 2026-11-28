//! gc1a 工具链端到端 spike —— 证明 `wasm-encoder 0.220 → wasmtime 51 → Node V8`
//! 三段路都能运行 WasmGC 模块。
//!
//! 本测试**不依赖 CJWasm 编译器本体**，直接用 `wasm-encoder` 发射一个手写 GC 模块：
//!
//!   * 一个 rec group，含：
//!       - type 1: `struct Node { value: i64 (mut), next: (ref null 1) (mut) }`
//!         （自引用 ref 字段，验证 rec group 内 Concrete(1) 前向/自引用编码）
//!       - type 2: `array (mut (ref null 1))`
//!         （类引用元素数组，对应 bench `Array<LNode>` 的表示）
//!   * type 0: `() -> i64`（main 的签名）
//!   * `main`：循环 1,000,000 次，每轮 `struct.new_default 1` 覆盖同一 local
//!     （制造垃圾）+ `array.new_default 2` 制造垃圾数组；循环后构造一个
//!     `value=42` 的 Node，`struct.get 1 0` 读出并返回。
//!
//! 验证手段：
//!   1. 写出 `.wasm` 到 `$CLAUDE_JOB_DIR`（缺省回退 `target/`）；
//!   2. 若 `PATH` 中有 `wasmtime`：`wasmtime run --invoke main <file>` 断言输出 42；
//!      若没有则 `eprintln!` 跳过（不 fail，保持 CI 可移植）；
//!   3. 尽力把该 `.wasm` 复制到 `.compat-run/gc-bench/work/gc_spike.wasm`
//!      （供 gc1b 的 Node 堆观测通道验证；复制失败只告警不 fail）。
//!
//! Node/V8 侧的堆统计验证由本 agent 在交付说明中记录实测（测试只保证 wasmtime 路径）。

use std::path::{Path, PathBuf};
use std::process::Command;

use wasm_encoder::{
    ArrayType, BlockType, CodeSection, CompositeInnerType, CompositeType, ExportKind, ExportSection,
    FieldType, Function, FunctionSection, HeapType, Instruction, Module, RefType, StorageType,
    StructType, SubType, TypeSection, ValType,
};

/// 循环次数：1,000,000 次 struct.new_default + array.new_default。
const ITERS: i32 = 1_000_000;
/// 最终节点的 value；main 应返回它。
const FINAL_VALUE: i64 = 42;

/// 构造 GC spike 模块的字节。
///
/// 类型索引布局：
///   0: `() -> i64`
///   1: `struct Node { value: i64, next: ref null 1 }`   （rec group 第 0 个）
///   2: `array (mut (ref null 1))`                        （rec group 第 1 个）
pub fn build_gc_module() -> Vec<u8> {
    // Node 的 nullable 自引用类型：(ref null 1)
    let node_ref = RefType {
        nullable: true,
        heap_type: HeapType::Concrete(1),
    };
    // 数组类型本身：(ref null 2)
    let array_ref = RefType {
        nullable: true,
        heap_type: HeapType::Concrete(2),
    };

    // ── 类型段 ───────────────────────────────────────────────────────────
    let mut types = TypeSection::new();
    // type 0: () -> i64
    types.ty().function(Vec::<ValType>::new(), vec![ValType::I64]);
    // rec group: type 1 (Node) + type 2 (array of Node ref)
    types.ty().rec(vec![
        SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::Ref(node_ref)),
                            mutable: true,
                        },
                    ]
                    .into(),
                }),
                shared: false,
            },
        },
        SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Array(ArrayType(FieldType {
                    element_type: StorageType::Val(ValType::Ref(node_ref)),
                    mutable: true,
                })),
                shared: false,
            },
        },
    ]);

    // ── 函数段 ───────────────────────────────────────────────────────────
    let mut functions = FunctionSection::new();
    functions.function(0); // main: () -> i64

    // ── 导出段：export "main" ────────────────────────────────────────────
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 0);

    // ── 代码段 ───────────────────────────────────────────────────────────
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![
        (1, ValType::Ref(node_ref)),  // local 0: 当前节点（每轮覆盖 → 垃圾）
        (1, ValType::I32),            // local 1: 循环计数器 i
        (1, ValType::Ref(array_ref)), // local 2: 当前数组（每轮覆盖 → 垃圾）
    ]);

    // i = 0
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(1));

    // block $exit { loop $loop { ... } }
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    // if i >= ITERS -> break（Br 1 = 跳出 block $exit）
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Const(ITERS));
    f.instruction(&Instruction::I32GeS);
    f.instruction(&Instruction::BrIf(1));
    // 垃圾 1：struct.new_default 1 → local 0
    f.instruction(&Instruction::StructNewDefault(1));
    f.instruction(&Instruction::LocalSet(0));
    // 垃圾 2：array.new_default 2 (len=4) → local 2
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::ArrayNewDefault(2));
    f.instruction(&Instruction::LocalSet(2));
    // i = i + 1
    f.instruction(&Instruction::LocalGet(1));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(1));
    // continue（Br 0 = loop）
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End); // loop
    f.instruction(&Instruction::End); // block

    // 最终节点：Node { value: 42, next: null } → local 0
    f.instruction(&Instruction::I64Const(FINAL_VALUE));
    f.instruction(&Instruction::RefNull(HeapType::Concrete(1)));
    f.instruction(&Instruction::StructNew(1));
    f.instruction(&Instruction::LocalSet(0));
    // 读 value 字段（field 0）→ i64 返回值
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::StructGet {
        struct_type_index: 1,
        field_index: 0,
    });
    // 函数体结束
    f.instruction(&Instruction::End);
    code.function(&f);

    // ── 组装模块（GC 模块无需线性内存/WASI）─────────────────────────────
    let mut module = Module::new();
    module.section(&types);
    module.section(&functions);
    module.section(&exports);
    module.section(&code);
    module.finish()
}

/// 选择一个可写目录：优先 `$CLAUDE_JOB_DIR`，回退 `<manifest>/target`。
fn scratch_dir() -> PathBuf {
    if let Ok(job) = std::env::var("CLAUDE_JOB_DIR") {
        if !job.is_empty() {
            let p = PathBuf::from(job);
            if std::fs::create_dir_all(&p).is_ok() {
                return p;
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = manifest.join("target");
    let _ = std::fs::create_dir_all(&target);
    target
}

/// 在 PATH 中查找可执行文件（不依赖 `which`）。
fn which(program: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[test]
fn gc_spike_wasmtime_runs_gc_module() {
    let bytes = build_gc_module();
    assert!(!bytes.is_empty(), "GC 模块不应为空");

    // 写盘
    let dir = scratch_dir();
    let wasm_path = dir.join("gc_spike.wasm");
    std::fs::write(&wasm_path, &bytes).expect("写出 spike .wasm 失败");
    eprintln!(
        "[gc_spike] 已写出 GC 模块: {} ({} bytes)",
        wasm_path.display(),
        bytes.len()
    );

    // 尽力复制到 bench work 目录（供 gc1b 的 Node 观测通道使用）
    let bench_copy =
        Path::new("/Users/sean/workspace/CJWasm/.compat-run/gc-bench/work/gc_spike.wasm");
    if bench_copy.parent().map(|p| p.is_dir()).unwrap_or(false) {
        match std::fs::copy(&wasm_path, bench_copy) {
            Ok(_) => eprintln!("[gc_spike] 已复制到 {}", bench_copy.display()),
            Err(e) => eprintln!("[gc_spike] 复制到 bench work 失败（忽略）: {}", e),
        }
    } else {
        eprintln!("[gc_spike] bench work 目录不存在，跳过复制");
    }

    // wasmtime 运行
    let wasmtime = match which("wasmtime") {
        Some(p) => p,
        None => {
            eprintln!("[gc_spike] PATH 中无 wasmtime，跳过运行断言（不 fail）");
            return;
        }
    };

    let start = std::time::Instant::now();
    let output = Command::new(&wasmtime)
        .arg("run")
        .arg("--invoke")
        .arg("main")
        .arg(&wasm_path)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[gc_spike] 启动 wasmtime 失败，跳过（不 fail）: {}", e);
            return;
        }
    };
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!(
        "[gc_spike] wasmtime run --invoke main 耗时 {:.1} ms, status={:?}\n  stdout={:?}\n  stderr={:?}",
        elapsed.as_secs_f64() * 1000.0,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    );

    assert!(
        output.status.success(),
        "wasmtime 运行 GC 模块失败: status={:?} stderr={}",
        output.status,
        stderr
    );
    let got = stdout.trim();
    assert_eq!(
        got,
        FINAL_VALUE.to_string(),
        "main 返回值应为 {}，实际 stdout={:?} stderr={:?}",
        FINAL_VALUE,
        got,
        stderr
    );
}
