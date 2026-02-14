use cjwasm::pipeline;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn print_usage() {
    eprintln!("用法: cjwasm <命令|源文件> [选项]");
    eprintln!();
    eprintln!("命令:");
    eprintln!("  build            读取 cjpm.toml，编译当前工程为 WASM");
    eprintln!("  init <名称>      初始化新的仓颉 WASM 项目");
    eprintln!();
    eprintln!("直接编译:");
    eprintln!("  cjwasm <源文件.cj> [更多文件.cj...] [-o 输出文件.wasm]");
    eprintln!();
    eprintln!("示例:");
    eprintln!("  cjwasm build                       # 编译 cjpm 工程");
    eprintln!("  cjwasm build -o app.wasm            # 指定输出文件");
    eprintln!("  cjwasm build -v                     # 显示详细信息");
    eprintln!("  cjwasm init myproject               # 初始化新项目");
    eprintln!("  cjwasm hello.cj                     # 编译单文件");
    eprintln!("  cjwasm hello.cj -o hello.wasm       # 指定输出文件");
    eprintln!("  cjwasm main.cj lib.cj -o app.wasm   # 多文件编译");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "build" => cmd_build(&args[2..]),
        "init" => cmd_init(&args[2..]),
        "-h" | "--help" | "help" => {
            print_usage();
        }
        _ => {
            // 兼容旧行为：直接编译 .cj 文件
            cmd_compile(&args[1..]);
        }
    }
}

// ── build 子命令 ─────────────────────────────────────────────

fn cmd_build(args: &[String]) {
    let mut output: Option<String> = None;
    let mut verbose = false;
    let mut project_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                if i + 1 < args.len() {
                    output = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("错误: -o 需要指定输出文件名");
                    std::process::exit(1);
                }
            }
            "-v" | "--verbose" | "-V" => {
                verbose = true;
                i += 1;
            }
            "-p" | "--path" => {
                if i + 1 < args.len() {
                    project_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("错误: -p 需要指定项目路径");
                    std::process::exit(1);
                }
            }
            "-h" | "--help" => {
                eprintln!("用法: cjwasm build [选项]");
                eprintln!();
                eprintln!("读取 cjpm.toml 配置，编译当前仓颉工程为 WASM。");
                eprintln!();
                eprintln!("选项:");
                eprintln!("  -o, --output <文件>    指定输出文件名");
                eprintln!("  -p, --path <目录>      指定项目目录（默认当前目录）");
                eprintln!("  -v, --verbose          显示详细编译信息");
                eprintln!("  -h, --help             显示帮助");
                std::process::exit(0);
            }
            _ => {
                eprintln!("错误: 未知选项 '{}'", args[i]);
                eprintln!("运行 'cjwasm build --help' 查看帮助");
                std::process::exit(1);
            }
        }
    }

    let opts = cjwasm::cjpm::BuildOptions {
        project_dir,
        output,
        verbose,
    };

    match cjwasm::cjpm::build(&opts) {
        Ok(result) => {
            println!(
                "cjwasm build 成功: {} ({} 个源文件) -> {}",
                result.package_name,
                result.source_files,
                result.output_path.display()
            );
            println!("  大小: {} 字节", result.wasm_size);
        }
        Err(e) => {
            eprintln!("cjwasm build 失败: {}", e);
            std::process::exit(1);
        }
    }
}

// ── init 子命令 ──────────────────────────────────────────────

fn cmd_init(args: &[String]) {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        eprintln!("用法: cjwasm init <项目名称>");
        eprintln!();
        eprintln!("在当前目录下创建新的仓颉 WASM 项目，生成：");
        eprintln!("  cjpm.toml       项目配置文件");
        eprintln!("  src/main.cj     入口源文件");
        if args.is_empty() {
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let name = &args[0];
    let project_dir = env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(name);

    match cjwasm::cjpm::init(&project_dir, name) {
        Ok(()) => {
            println!("项目已创建: {}/", name);
            println!("  cjpm.toml");
            println!("  src/main.cj");
            println!();
            println!("开始使用:");
            println!("  cd {}", name);
            println!("  cjwasm build");
        }
        Err(e) => {
            eprintln!("初始化失败: {}", e);
            std::process::exit(1);
        }
    }
}

// ── 直接编译（兼容旧行为）────────────────────────────────────

fn cmd_compile(args: &[String]) {
    // 解析命令行参数
    let mut input_files: Vec<String> = Vec::new();
    let mut output_path: Option<String> = None;
    let mut i = 0;
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
