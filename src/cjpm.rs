//! cjpm.toml 配置解析与 `cjwasm build` 子命令实现。
//!
//! 兼容仓颉包管理器 (cjpm) 的项目结构，支持：
//! - 解析 cjpm.toml 获取项目元信息
//! - 自动发现 src/ 下的 .cj 源文件
//! - 递归解析 workspace 多模块
//! - 编译输出到 target/ 目录

use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

// ── cjpm.toml 数据结构 ──────────────────────────────────────

/// cjpm.toml 顶层配置
#[derive(Debug, Deserialize)]
pub struct CjpmConfig {
    pub package: Option<PackageConfig>,
    pub workspace: Option<WorkspaceConfig>,
    pub dependencies: Option<toml::Value>,
}

/// [package] 段
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PackageConfig {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub output_type: Option<String>,
    #[serde(default)]
    pub src_dir: Option<String>,
    #[serde(default)]
    pub target_dir: Option<String>,
}

/// [workspace] 段
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub build_members: Vec<String>,
}

// ── 解析与加载 ──────────────────────────────────────────────

/// 从指定目录查找并解析 cjpm.toml
pub fn load_config(project_dir: &Path) -> Result<CjpmConfig, String> {
    let config_path = project_dir.join("cjpm.toml");
    if !config_path.exists() {
        return Err(format!(
            "未找到 cjpm.toml（在 {} 中）",
            project_dir.display()
        ));
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("无法读取 {}: {}", config_path.display(), e))?;

    let config: CjpmConfig =
        toml::from_str(&content).map_err(|e| format!("解析 cjpm.toml 失败: {}", e))?;

    Ok(config)
}

/// 获取源文件目录（默认 src/）
pub fn get_src_dir(config: &CjpmConfig, project_dir: &Path) -> PathBuf {
    if let Some(ref pkg) = config.package {
        if let Some(ref src) = pkg.src_dir {
            if !src.is_empty() {
                return project_dir.join(src);
            }
        }
    }
    project_dir.join("src")
}

/// 获取输出目录（默认 target/）
pub fn get_target_dir(config: &CjpmConfig, project_dir: &Path) -> PathBuf {
    if let Some(ref pkg) = config.package {
        if let Some(ref dir) = pkg.target_dir {
            if !dir.is_empty() {
                return project_dir.join(dir);
            }
        }
    }
    project_dir.join("target")
}

/// 获取输出类型（默认 "executable"）
pub fn get_output_type(config: &CjpmConfig) -> &str {
    if let Some(ref pkg) = config.package {
        if let Some(ref ot) = pkg.output_type {
            return ot.as_str();
        }
    }
    "executable"
}

/// 获取包名称
pub fn get_package_name(config: &CjpmConfig) -> Result<&str, String> {
    config
        .package
        .as_ref()
        .map(|p| p.name.as_str())
        .ok_or_else(|| "cjpm.toml 中缺少 [package] name 字段".to_string())
}

// ── 源文件发现 ──────────────────────────────────────────────

/// 递归收集目录下所有 .cj 文件（按路径排序）
pub fn collect_cj_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_cj_files_recursive(dir, &mut files);
    files.sort();
    files
}

fn collect_cj_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cj_files_recursive(&path, files);
        } else if path.extension().map_or(false, |ext| ext == "cj") {
            files.push(path);
        }
    }
}

// ── build 命令 ──────────────────────────────────────────────

/// build 命令的选项
pub struct BuildOptions {
    /// 项目目录（包含 cjpm.toml 的目录）
    pub project_dir: PathBuf,
    /// 输出文件名（覆盖默认名）
    pub output: Option<String>,
    /// 是否显示详细信息
    pub verbose: bool,
}

/// 执行 build 命令
pub fn build(opts: &BuildOptions) -> Result<BuildResult, String> {
    let config = load_config(&opts.project_dir)?;
    let pkg_name = get_package_name(&config)?;
    let src_dir = get_src_dir(&config, &opts.project_dir);
    let target_dir = get_target_dir(&config, &opts.project_dir);

    if opts.verbose {
        eprintln!("包名: {}", pkg_name);
        eprintln!("源目录: {}", src_dir.display());
        eprintln!("目标目录: {}", target_dir.display());
    }

    // 检查源目录
    if !src_dir.exists() {
        return Err(format!("源目录不存在: {}", src_dir.display()));
    }

    // 收集源文件
    let cj_files = collect_cj_files(&src_dir);
    if cj_files.is_empty() {
        return Err(format!(
            "在 {} 中未找到 .cj 源文件",
            src_dir.display()
        ));
    }

    if opts.verbose {
        eprintln!("发现 {} 个源文件:", cj_files.len());
        for f in &cj_files {
            eprintln!("  {}", f.display());
        }
    }

    // 解析所有源文件
    let mut programs = Vec::new();
    let mut visited = HashSet::new();

    for file in &cj_files {
        let canonical = file.canonicalize().unwrap_or(file.clone());
        if visited.contains(&canonical) {
            continue;
        }
        visited.insert(canonical);

        let path_str = file.to_string_lossy().to_string();
        let (prog, _source) = crate::pipeline::parse_file(&path_str)?;
        programs.push(prog);
    }

    // 自动解析 import 依赖（在 src 目录和项目根目录中搜索）
    let search_dirs = [src_dir.clone(), opts.project_dir.clone()];
    let mut import_queue = Vec::new();
    for prog in &programs {
        for base in &search_dirs {
            import_queue.extend(crate::pipeline::collect_import_files(
                prog,
                base,
                &mut visited,
            ));
        }
    }

    while let Some(import_file) = import_queue.pop() {
        let path_str = import_file.to_string_lossy().to_string();
        let (prog, _source) = crate::pipeline::parse_file(&path_str)?;
        for base in &search_dirs {
            let new_imports = crate::pipeline::collect_import_files(&prog, base, &mut visited);
            import_queue.extend(new_imports);
        }
        programs.push(prog);
    }

    let file_count = programs.len();

    // 合并 AST
    let mut program = if programs.len() == 1 {
        programs.into_iter().next().unwrap()
    } else {
        if opts.verbose {
            eprintln!("合并 {} 个文件...", file_count);
        }
        crate::pipeline::merge_programs(programs)
    };

    // 注入 JSON 标准库（如果有 import stdx.encoding.json）
    let needs_json_stdlib = program.imports.iter().any(|imp| {
        let path = imp.module_path.join(".");
        path.starts_with("stdx.encoding.json")
    });
    if needs_json_stdlib {
        let json_stdlib = crate::stdlib::json::generate_json_stdlib();
        json_stdlib.inject_into(&mut program);
    }
    if program.imports.iter().any(|imp| imp.module_path.join(".").starts_with("std.time")) {
        crate::stdlib::time::generate_time_stdlib().inject_into(&mut program);
    }

    // 优化 + 单态化
    crate::optimizer::optimize_program(&mut program);
    crate::monomorph::monomorphize_program(&mut program);

    // 代码生成
    let mut codegen = crate::codegen::CodeGen::new();
    let wasm = codegen.compile(&program);

    // 确定输出路径
    let output_dir = target_dir.join("wasm");
    fs::create_dir_all(&output_dir)
        .map_err(|e| format!("无法创建输出目录 {}: {}", output_dir.display(), e))?;

    let output_name = opts
        .output
        .clone()
        .unwrap_or_else(|| format!("{}.wasm", pkg_name));

    let output_path = if output_name.contains('/') || output_name.contains('\\') {
        PathBuf::from(&output_name)
    } else {
        output_dir.join(&output_name)
    };

    // 如果输出路径有父目录，确保它存在
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建目录 {}: {}", parent.display(), e))?;
    }

    fs::write(&output_path, &wasm)
        .map_err(|e| format!("无法写入 {}: {}", output_path.display(), e))?;

    Ok(BuildResult {
        package_name: pkg_name.to_string(),
        source_files: file_count,
        output_path,
        wasm_size: wasm.len(),
    })
}

/// build 成功后的结果
#[derive(Debug)]
pub struct BuildResult {
    pub package_name: String,
    pub source_files: usize,
    pub output_path: PathBuf,
    pub wasm_size: usize,
}

// ── init 命令 ────────────────────────────────────────────────

/// 初始化一个新的 cjpm 兼容项目
pub fn init(project_dir: &Path, name: &str) -> Result<(), String> {
    // 创建目录结构
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| format!("无法创建目录 {}: {}", src_dir.display(), e))?;

    // 生成 cjpm.toml
    let toml_path = project_dir.join("cjpm.toml");
    if toml_path.exists() {
        return Err("cjpm.toml 已存在".to_string());
    }

    let toml_content = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
description = ""
output-type = "executable"
src-dir = ""
target-dir = ""
"#
    );
    fs::write(&toml_path, toml_content)
        .map_err(|e| format!("无法写入 {}: {}", toml_path.display(), e))?;

    // 生成 src/main.cj
    let main_path = src_dir.join("main.cj");
    if !main_path.exists() {
        let main_content = format!(
            r#"package {name}

func main(): Int64 {{
    println("Hello, {name}!")
    return 0
}}
"#
        );
        fs::write(&main_path, main_content)
            .map_err(|e| format!("无法写入 {}: {}", main_path.display(), e))?;
    }

    Ok(())
}

// ── 测试 ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_project(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("cjwasm_cjpm_test_{}", name));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        tmp
    }

    #[test]
    fn test_load_config_missing() {
        let tmp = std::env::temp_dir().join("cjwasm_cjpm_test_missing");
        let _ = fs::create_dir_all(&tmp);
        let result = load_config(&tmp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未找到 cjpm.toml"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_config_basic() {
        let tmp = create_test_project("load_basic");
        let toml = r#"
[package]
name = "hello"
version = "1.0.0"
output-type = "executable"
"#;
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();

        let config = load_config(&tmp).unwrap();
        let pkg = config.package.unwrap();
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, Some("1.0.0".to_string()));
        assert_eq!(pkg.output_type, Some("executable".to_string()));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_config_with_src_dir() {
        let tmp = create_test_project("src_dir");
        let toml = r#"
[package]
name = "myapp"
src-dir = "lib"
target-dir = "out"
"#;
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();

        let config = load_config(&tmp).unwrap();
        assert_eq!(get_src_dir(&config, &tmp), tmp.join("lib"));
        assert_eq!(get_target_dir(&config, &tmp), tmp.join("out"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_get_defaults() {
        let tmp = create_test_project("defaults");
        let toml = r#"
[package]
name = "test"
"#;
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();

        let config = load_config(&tmp).unwrap();
        assert_eq!(get_src_dir(&config, &tmp), tmp.join("src"));
        assert_eq!(get_target_dir(&config, &tmp), tmp.join("target"));
        assert_eq!(get_output_type(&config), "executable");
        assert_eq!(get_package_name(&config).unwrap(), "test");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_cj_files() {
        let tmp = create_test_project("collect");
        let src = tmp.join("src");
        fs::write(src.join("main.cj"), "func main(): Int64 { return 0 }").unwrap();
        fs::write(src.join("lib.cj"), "func helper(): Int64 { return 1 }").unwrap();
        fs::create_dir_all(src.join("utils")).unwrap();
        fs::write(
            src.join("utils/math.cj"),
            "func add(a: Int64, b: Int64): Int64 { return a + b }",
        )
        .unwrap();

        let files = collect_cj_files(&src);
        assert_eq!(files.len(), 3);
        // 应该按路径排序
        assert!(files[0].to_string_lossy().contains("lib.cj"));
        assert!(files[1].to_string_lossy().contains("main.cj"));
        assert!(files[2].to_string_lossy().contains("math.cj"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_cj_files_empty() {
        let tmp = create_test_project("collect_empty");
        let files = collect_cj_files(&tmp.join("src"));
        assert!(files.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_init() {
        let tmp = create_test_project("init");
        let _ = fs::remove_dir_all(&tmp);

        init(&tmp, "myproject").unwrap();

        assert!(tmp.join("cjpm.toml").exists());
        assert!(tmp.join("src/main.cj").exists());

        let config = load_config(&tmp).unwrap();
        assert_eq!(get_package_name(&config).unwrap(), "myproject");

        let main_src = fs::read_to_string(tmp.join("src/main.cj")).unwrap();
        assert!(main_src.contains("myproject"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_init_already_exists() {
        let tmp = create_test_project("init_exists");
        fs::write(tmp.join("cjpm.toml"), "[package]\nname = \"x\"").unwrap();

        let result = init(&tmp, "y");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("已存在"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_full() {
        let tmp = create_test_project("build_full");
        let toml = r#"
[package]
name = "testapp"
version = "0.1.0"
output-type = "executable"
"#;
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();
        fs::write(
            tmp.join("src/main.cj"),
            "func main(): Int64 { return 42 }",
        )
        .unwrap();

        let opts = BuildOptions {
            project_dir: tmp.clone(),
            output: None,
            verbose: false,
        };
        let result = build(&opts).unwrap();
        assert_eq!(result.package_name, "testapp");
        assert_eq!(result.source_files, 1);
        assert!(result.output_path.exists());
        assert!(result.wasm_size > 0);
        assert!(result
            .output_path
            .to_string_lossy()
            .contains("testapp.wasm"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_multi_file() {
        let tmp = create_test_project("build_multi");
        let toml = r#"
[package]
name = "multitest"
"#;
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();
        fs::write(
            tmp.join("src/main.cj"),
            "func main(): Int64 { return add(10, 20) }",
        )
        .unwrap();
        fs::write(
            tmp.join("src/math.cj"),
            "func add(a: Int64, b: Int64): Int64 { return a + b }",
        )
        .unwrap();

        let opts = BuildOptions {
            project_dir: tmp.clone(),
            output: None,
            verbose: false,
        };
        let result = build(&opts).unwrap();
        assert_eq!(result.source_files, 2);
        assert!(result.output_path.exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_custom_output() {
        let tmp = create_test_project("build_output");
        let toml = "[package]\nname = \"app\"\n";
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();
        fs::write(
            tmp.join("src/main.cj"),
            "func main(): Int64 { return 0 }",
        )
        .unwrap();

        let opts = BuildOptions {
            project_dir: tmp.clone(),
            output: Some("custom.wasm".to_string()),
            verbose: false,
        };
        let result = build(&opts).unwrap();
        assert!(result
            .output_path
            .to_string_lossy()
            .contains("custom.wasm"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_no_src() {
        let tmp = create_test_project("build_no_src");
        let _ = fs::remove_dir_all(tmp.join("src"));
        let toml = "[package]\nname = \"nosrc\"\n";
        fs::write(tmp.join("cjpm.toml"), toml).unwrap();

        let opts = BuildOptions {
            project_dir: tmp.clone(),
            output: None,
            verbose: false,
        };
        let result = build(&opts);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("源目录不存在"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
