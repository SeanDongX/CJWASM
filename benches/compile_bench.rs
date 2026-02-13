//! CJWasm 编译器 Criterion 微基准测试
//!
//! 运行: cargo bench
//! 报告: target/criterion/report/index.html
//!
//! 注意: 此报告仅包含 cjwasm 编译器内部管线的性能分析。
//! cjwasm vs cjc 的端到端对比请运行: ./scripts/benchmark.sh

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// ── 测试源代码 ──

const SMALL_SOURCE: &str = include_str!("fixtures/bench_small.cj");
const MEDIUM_SOURCE: &str = include_str!("fixtures/bench_medium.cj");
const LARGE_SOURCE: &str = include_str!("fixtures/bench_large.cj");

// ============================================================
// 1. 端到端编译基准（source → WASM bytes）
// ============================================================

fn bench_compile_e2e(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_端到端编译");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("source_to_wasm", name), source, |b, src| {
            b.iter(|| {
                let wasm = cjwasm::pipeline::compile_source_to_wasm(black_box(src))
                    .expect("编译不应失败");
                black_box(wasm);
            });
        });
    }

    group.finish();
}

// ============================================================
// 2. 词法分析（Lexer）基准
// ============================================================

fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_词法分析");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("tokenize", name), source, |b, src| {
            b.iter(|| {
                let lexer = cjwasm::lexer::Lexer::new(black_box(src));
                let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().expect("词法分析不应失败");
                black_box(tokens);
            });
        });
    }

    group.finish();
}

// ============================================================
// 3. 解析（Parser）基准
// ============================================================

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_语法解析");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        let lexer = cjwasm::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().expect("词法分析不应失败");

        group.throughput(Throughput::Elements(tokens.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("parse_program", name),
            &tokens,
            |b, toks| {
                b.iter(|| {
                    let mut parser = cjwasm::parser::Parser::new(black_box(toks.clone()));
                    let program = parser.parse_program().expect("解析不应失败");
                    black_box(program);
                });
            },
        );
    }

    group.finish();
}

// ============================================================
// 4. 优化器 + 单态化基准
// ============================================================

fn bench_optimizer_monomorph(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_优化器与单态化");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        let program = cjwasm::pipeline::parse_source(source).expect("解析不应失败");

        group.bench_function(BenchmarkId::new("optimize_and_monomorph", name), |b| {
            b.iter(|| {
                let mut p = program.clone();
                cjwasm::optimizer::optimize_program(&mut p);
                cjwasm::monomorph::monomorphize_program(&mut p);
                black_box(p);
            });
        });
    }

    group.finish();
}

// ============================================================
// 5. 代码生成（CodeGen）基准
// ============================================================

fn bench_codegen(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_代码生成");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        let mut program = cjwasm::pipeline::parse_source(source).expect("解析不应失败");
        cjwasm::optimizer::optimize_program(&mut program);
        cjwasm::monomorph::monomorphize_program(&mut program);

        group.bench_with_input(
            BenchmarkId::new("emit_wasm", name),
            &program,
            |b, prog| {
                b.iter(|| {
                    let mut codegen = cjwasm::codegen::CodeGen::new();
                    let wasm = codegen.compile(black_box(prog));
                    black_box(wasm);
                });
            },
        );
    }

    group.finish();
}

// ============================================================
// 6. 输出大小基准（非时间，仅记录）
// ============================================================

fn bench_output_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("cjwasm_输出大小");

    let cases = [
        ("小规模(27行)", SMALL_SOURCE),
        ("中规模(100行)", MEDIUM_SOURCE),
        ("大规模(311行)", LARGE_SOURCE),
    ];

    for (name, source) in &cases {
        group.bench_with_input(
            BenchmarkId::new("wasm_bytes", name),
            source,
            |b, src| {
                b.iter(|| {
                    let wasm = cjwasm::pipeline::compile_source_to_wasm(black_box(src))
                        .expect("编译不应失败");
                    black_box(wasm.len())
                });
            },
        );
    }

    group.finish();
}

// ============================================================
// 注册所有基准组
// ============================================================

criterion_group!(
    benches,
    bench_compile_e2e,
    bench_lexer,
    bench_parser,
    bench_optimizer_monomorph,
    bench_codegen,
    bench_output_size,
);

criterion_main!(benches);
