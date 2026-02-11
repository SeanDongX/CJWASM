use cjwasm::codegen::CodeGen;
use cjwasm::lexer::Lexer;
use cjwasm::parser::Parser;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("用法: cjwasm <源文件.cj> [输出文件.wasm]");
        eprintln!("\n示例:");
        eprintln!("  cjwasm hello.cj              # 输出 hello.wasm");
        eprintln!("  cjwasm hello.cj output.wasm  # 指定输出文件");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = if args.len() >= 3 {
        args[2].clone()
    } else {
        let stem = Path::new(input_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        format!("{}.wasm", stem)
    };

    // 读取源文件
    let source = match fs::read_to_string(input_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("错误: 无法读取文件 '{}': {}", input_path, e);
            std::process::exit(1);
        }
    };

    // 词法分析
    let lexer = Lexer::new(&source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = match tokens {
        Ok(t) => t,
        Err(e) => {
            eprintln!("词法错误: {}", e);
            std::process::exit(1);
        }
    };

    // 语法分析
    let mut parser = Parser::new(tokens);
    let mut program = match parser.parse_program() {
        Ok(p) => p,
        Err(e) => {
            let (line, col) = cjwasm::parser::line_column_from_source(&source, e.byte_start);
            eprintln!("语法错误: {} (行 {} 列 {})", e, line, col);
            std::process::exit(1);
        }
    };

    // 优化（常量折叠等）
    cjwasm::optimizer::optimize_program(&mut program);

    // 代码生成
    let mut codegen = CodeGen::new();
    let wasm = codegen.compile(&program);

    // 写入输出文件
    match fs::write(&output_path, &wasm) {
        Ok(_) => {
            println!("编译成功: {} -> {}", input_path, output_path);
            println!("  大小: {} 字节", wasm.len());
        }
        Err(e) => {
            eprintln!("错误: 无法写入文件 '{}': {}", output_path, e);
            std::process::exit(1);
        }
    }
}
