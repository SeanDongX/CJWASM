//! L1 标准库模块全量测试
//!
//! 验证所有 L1 模块（std.io, std.binary, std.console, std.overflow, std.crypto,
//! std.deriving, std.ast, std.argopt, std.sort, std.ref, std.unicode）在存在
//! third_party vendor 时能正确解析到对应 .cj 文件。

use std::path::Path;

fn repo_vendor_dir() -> Option<std::path::PathBuf> {
    let repo = std::env::var("CARGO_MANIFEST_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    cjwasm::pipeline::get_vendor_std_dir(&repo)
}

/// 全量：所有 L1 顶层模块在 vendor 下均应能解析到至少一个 .cj 文件
#[test]
fn test_l1_all_top_modules_resolve_from_vendor() {
    let vendor = match repo_vendor_dir() {
        Some(v) => v,
        None => {
            eprintln!("跳过: 未找到 vendor 目录 (third_party/cangjie_runtime/std/libs/std 或 CJWASM_STD_PATH)");
            return;
        }
    };
    let bases: &[&Path] = &[];
    let modules = cjwasm::pipeline::l1_std_top_modules();
    assert!(!modules.is_empty(), "L1 模块列表不应为空");

    for &name in modules {
        let module_path = ["std".to_string(), name.to_string()];
        let files = cjwasm::pipeline::resolve_import_to_files(
            &module_path,
            bases,
            Some(vendor.as_path()),
        );
        assert!(
            !files.is_empty(),
            "L1 模块 std.{} 应从 vendor 解析到至少一个 .cj 文件 (vendor={})",
            name,
            vendor.display()
        );
        assert!(
            files.iter().all(|p| p.extension().map_or(false, |e| e == "cj")),
            "L1 std.{} 解析结果应均为 .cj 文件: {:?}",
            name,
            files
        );
    }
}

/// 各 L1 模块单独解析测试（便于定位失败模块）
#[test]
fn test_l1_std_io_resolve() {
    assert_l1_module_resolves("io");
}

#[test]
fn test_l1_std_binary_resolve() {
    assert_l1_module_resolves("binary");
}

#[test]
fn test_l1_std_console_resolve() {
    assert_l1_module_resolves("console");
}

#[test]
fn test_l1_std_overflow_resolve() {
    assert_l1_module_resolves("overflow");
}

#[test]
fn test_l1_std_crypto_resolve() {
    assert_l1_module_resolves("crypto");
}

#[test]
fn test_l1_std_deriving_resolve() {
    assert_l1_module_resolves("deriving");
}

#[test]
fn test_l1_std_ast_resolve() {
    assert_l1_module_resolves("ast");
}

#[test]
fn test_l1_std_argopt_resolve() {
    assert_l1_module_resolves("argopt");
}

#[test]
fn test_l1_std_sort_resolve() {
    assert_l1_module_resolves("sort");
}

#[test]
fn test_l1_std_ref_resolve() {
    assert_l1_module_resolves("ref");
}

#[test]
fn test_l1_std_unicode_resolve() {
    assert_l1_module_resolves("unicode");
}

fn assert_l1_module_resolves(name: &str) {
    let vendor = match repo_vendor_dir() {
        Some(v) => v,
        None => {
            eprintln!("跳过 test_l1_std_{}_resolve: 未找到 vendor", name);
            return;
        }
    };
    let module_path = ["std".to_string(), name.to_string()];
    let bases: &[&Path] = &[];
    let files = cjwasm::pipeline::resolve_import_to_files(
        &module_path,
        bases,
        Some(vendor.as_path()),
    );
    assert!(
        !files.is_empty(),
        "std.{} 应从 vendor 解析到至少一个 .cj (vendor={})",
        name,
        vendor.display()
    );
    assert!(
        files.iter().all(|p| p.extension().map_or(false, |e| e == "cj")),
        "std.{} 解析结果应均为 .cj: {:?}",
        name,
        files
    );
}

/// L1 子包解析：std.crypto.digest、std.crypto.cipher 等
#[test]
fn test_l1_std_subpackage_resolve() {
    let vendor = match repo_vendor_dir() {
        Some(v) => v,
        None => return,
    };
    let bases: &[&Path] = &[];
    let subpackages = [
        ["std".to_string(), "crypto".to_string(), "digest".to_string()],
        ["std".to_string(), "crypto".to_string(), "cipher".to_string()],
    ];
    for module_path in &subpackages {
        let files = cjwasm::pipeline::resolve_import_to_files(
            module_path,
            bases,
            Some(vendor.as_path()),
        );
        let name = module_path.join(".");
        assert!(
            !files.is_empty(),
            "L1 子包 {} 应从 vendor 解析到至少一个 .cj (vendor={})",
            name,
            vendor.display()
        );
    }
}

/// 通过 collect_import_files 收集：含多个 L1 import 的 program 应拉取到多份 vendor 文件
#[test]
fn test_l1_collect_import_files_multi_module() {
    let vendor = match repo_vendor_dir() {
        Some(v) => v,
        None => return,
    };
    let source = r#"
        package test.l1
        import std.binary
        import std.sort
        import std.unicode
        func main(): Int64 { return 0 }
    "#;
    let program = cjwasm::pipeline::parse_source(source).expect("解析应成功");
    let bases: &[&Path] = &[];
    let mut visited = std::collections::HashSet::new();
    let files = cjwasm::pipeline::collect_import_files(
        &program,
        bases,
        &mut visited,
        Some(vendor.as_path()),
    );
    assert!(
        files.len() >= 3,
        "import std.binary + std.sort + std.unicode 应至少解析出 3 个包的文件，得到 {} 个",
        files.len()
    );
    assert!(
        files.iter().all(|p| p.extension().map_or(false, |e| e == "cj")),
        "collect 结果应均为 .cj"
    );
}
