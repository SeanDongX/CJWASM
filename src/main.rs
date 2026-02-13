use cjwasm::codegen::CodeGen;
use cjwasm::lexer::Lexer;
use cjwasm::parser::Parser;
use cjwasm::ast::Program;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// 解析单个 .cj 文件为 Program AST
fn parse_file(path: &str) -> (Program, String) {
    let source = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("错误: 无法读取文件 '{}': {}", path, e);
            std::process::exit(1);
        }
    };

    let lexer = Lexer::new(&source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = match tokens {
        Ok(t) => t,
        Err(e) => {
            eprintln!("词法错误 ({}): {}", path, e);
            std::process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => {
            let (line, col) = cjwasm::parser::line_column_from_source(&source, e.byte_start);
            eprintln!("语法错误 ({}): {} (行 {} 列 {})", path, e, line, col);
            std::process::exit(1);
        }
    };

    (program, source)
}

/// 合并多个 Program AST 为一个
fn merge_programs(programs: Vec<Program>) -> Program {
    let mut merged = Program {
        module_name: None,
        imports: vec![],
        structs: vec![],
        interfaces: vec![],
        classes: vec![],
        enums: vec![],
        functions: vec![],
        extends: vec![],
    };

    for prog in programs {
        // 使用第一个有 module_name 的作为模块名
        if merged.module_name.is_none() && prog.module_name.is_some() {
            merged.module_name = prog.module_name;
        }
        merged.imports.extend(prog.imports);
        merged.structs.extend(prog.structs);
        merged.interfaces.extend(prog.interfaces);
        merged.classes.extend(prog.classes);
        merged.enums.extend(prog.enums);
        merged.functions.extend(prog.functions);
        merged.extends.extend(prog.extends);
    }

    merged
}

/// 根据 import 路径解析文件路径
/// 例如: import math.utils -> 搜索 math/utils.cj 或 math_utils.cj
fn resolve_import_path(module_path: &[String], base_dir: &Path) -> Option<PathBuf> {
    // 策略 1: 将模块路径转为目录路径 math.utils -> math/utils.cj
    let dir_path = base_dir.join(
        module_path.iter().cloned().collect::<Vec<_>>().join("/")
    ).with_extension("cj");
    if dir_path.exists() {
        return Some(dir_path);
    }

    // 策略 2: 使用下划线连接 math.utils -> math_utils.cj
    let underscore_path = base_dir.join(
        module_path.iter().cloned().collect::<Vec<_>>().join("_")
    ).with_extension("cj");
    if underscore_path.exists() {
        return Some(underscore_path);
    }

    // 策略 3: 在 src/ 子目录中查找
    let src_dir = base_dir.join("src");
    if src_dir.exists() {
        let src_path = src_dir.join(
            module_path.iter().cloned().collect::<Vec<_>>().join("/")
        ).with_extension("cj");
        if src_path.exists() {
            return Some(src_path);
        }
    }

    None
}

/// 递归解析 import 依赖
fn collect_import_files(
    main_program: &Program,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut import_files = Vec::new();

    for import in &main_program.imports {
        if let Some(resolved) = resolve_import_path(&import.module_path, base_dir) {
            let canonical = resolved.canonicalize().unwrap_or(resolved.clone());
            if !visited.contains(&canonical) {
                visited.insert(canonical.clone());
                import_files.push(resolved);
            }
        }
        // 如果找不到文件，静默跳过（可能是标准库 import）
    }

    import_files
}

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

    // 解析命令行参数：支持多个 .cj 文件和 -o 输出选项
    let mut input_files: Vec<String> = Vec::new();
    let mut output_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "-o" && i + 1 < args.len() {
            output_path = Some(args[i + 1].clone());
            i += 2;
        } else if args[i].ends_with(".wasm") && input_files.len() == 1 && output_path.is_none() {
            // 向后兼容: cjwasm hello.cj output.wasm
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

    // 解析主文件
    let base_dir = Path::new(&input_files[0])
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let mut programs = Vec::new();
    let mut visited = HashSet::new();

    // 解析所有明确指定的文件
    for file in &input_files {
        let path = Path::new(file);
        let canonical = path.canonicalize().unwrap_or(path.to_path_buf());
        if visited.contains(&canonical) {
            continue;
        }
        visited.insert(canonical);
        let (prog, _source) = parse_file(file);
        programs.push(prog);
    }

    // 自动解析 import 依赖（从所有已解析的程序中查找 import）
    let mut import_queue: Vec<PathBuf> = Vec::new();
    for prog in &programs {
        import_queue.extend(collect_import_files(prog, &base_dir, &mut visited));
    }

    while let Some(import_file) = import_queue.pop() {
        let path_str = import_file.to_string_lossy().to_string();
        let (prog, _source) = parse_file(&path_str);
        // 递归查找新 import
        let new_imports = collect_import_files(&prog, &base_dir, &mut visited);
        import_queue.extend(new_imports);
        programs.push(prog);
    }

    // 合并所有 Program AST
    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        let file_count = programs.len();
        eprintln!("合并 {} 个文件...", file_count);
        merge_programs(programs)
    };

    // 优化（常量折叠等）
    cjwasm::optimizer::optimize_program(&mut program);

    // 泛型单态化：生成特化版本并替换调用点
    cjwasm::monomorph::monomorphize_program(&mut program);

    let mut codegen = CodeGen::new();
    let wasm = codegen.compile(&program);

    // 写入输出文件
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
