use cjwasm::pipeline;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("用法: cjwasm <源文件.cj> [更多文件.cj...] [-o 输出文件.wasm]");
        eprintln!("\n示例:");
        eprintln!("  cjwasm hello.cj                    # 输出 hello.wasm");
        eprintln!("  cjwasm hello.cj output.wasm        # 指定输出文件");
        eprintln!("  cjwasm main.cj lib.cj -o app.wasm  # 多文件编译");
        eprintln!("  cjwasm main.cj                     # 自动解析 import 依赖");
        std::process::exit(1);
    }

    // 解析命令行参数
    let mut input_files: Vec<String> = Vec::new();
    let mut output_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "-o" && i + 1 < args.len() {
            output_path = Some(args[i + 1].clone());
            i += 2;
        } else if args[i].ends_with(".wasm") && input_files.len() == 1 && output_path.is_none() {
            output_path = Some(args[i].clone());
            i += 1;
        } else {
            input_files.push(args[i].clone());
            i += 1;
        }
    }

    if input_files.is_empty() {
        eprintln!("错误: 未指定输入文件");
        std::process::exit(1);
    }

    let output = output_path.unwrap_or_else(|| {
        let stem = Path::new(&input_files[0])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        format!("{}.wasm", stem)
    });

    let base_dir = Path::new(&input_files[0])
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let mut programs = Vec::new();
    let mut visited = HashSet::new();

    for file in &input_files {
        let path = Path::new(file);
        let canonical = path.canonicalize().unwrap_or(path.to_path_buf());
        if visited.contains(&canonical) {
            continue;
        }
        visited.insert(canonical);
        let (prog, _source) = match pipeline::parse_file(file) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("错误 ({}): {}", file, e);
                std::process::exit(1);
            }
        };
        programs.push(prog);
    }

    // 自动解析 import 依赖
    let mut import_queue = Vec::new();
    for prog in &programs {
        import_queue.extend(pipeline::collect_import_files(prog, &base_dir, &mut visited));
    }

    while let Some(import_file) = import_queue.pop() {
        let path_str = import_file.to_string_lossy().to_string();
        let (prog, _source) = match pipeline::parse_file(&path_str) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("错误 ({}): {}", path_str, e);
                std::process::exit(1);
            }
        };
        let new_imports = pipeline::collect_import_files(&prog, &base_dir, &mut visited);
        import_queue.extend(new_imports);
        programs.push(prog);
    }

    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        let file_count = programs.len();
        eprintln!("合并 {} 个文件...", file_count);
        pipeline::merge_programs(programs)
    };

    cjwasm::optimizer::optimize_program(&mut program);
    cjwasm::monomorph::monomorphize_program(&mut program);

    let mut codegen = cjwasm::codegen::CodeGen::new();
    let wasm = codegen.compile(&program);

    match fs::write(&output, &wasm) {
        Ok(_) => {
            let files_desc = if input_files.len() > 1 {
                format!("{} 个文件", input_files.len())
            } else {
                input_files[0].clone()
            };
            println!("编译成功: {} -> {}", files_desc, output);
            println!("  大小: {} 字节", wasm.len());
        }
        Err(e) => {
            eprintln!("错误: 无法写入文件 '{}': {}", output, e);
            std::process::exit(1);
        }
    }
}
