/// 生成 Tauri 在编译期需要的资源与配置代码。
use std::{collections::BTreeSet, fs, path::Path};

/// 编译 Electron 版使用的 LMDB v2 只读兼容库并生成 Tauri 构建信息。
fn main() {
    compile_legacy_lmdb();
    tauri_build::build();
}

/// 为 LMDB v2 符号增加项目命名空间，避免与 heed 的 LMDB v1 冲突。
fn compile_legacy_lmdb() {
    let vendor = Path::new("vendor/lmdb-v2");
    let sources = [vendor.join("mdb.c"), vendor.join("midl.c")];
    let mut symbols = BTreeSet::new();

    // 从实际 C 源和头文件提取全部 mdb_ 标识符，内部调用也会同步重命名。
    for path in sources
        .iter()
        .chain([vendor.join("lmdb.h"), vendor.join("midl.h")].iter())
    {
        let content = fs::read_to_string(path).expect("LMDB v2 vendor source must exist");
        symbols.extend(extract_mdb_symbols(&content));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("chacha8.c").display()
    );

    let mut build = cc::Build::new();
    build
        .files(sources)
        .file(vendor.join("chacha8.c"))
        .include(vendor)
        .define("MDB_MAXKEYSIZE", "0")
        .warnings(false);
    for symbol in symbols {
        build.define(&symbol, Some(format!("ztools_{symbol}").as_str()));
    }
    build.compile("ztools_lmdb_v2");

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=ntdll");
        println!("cargo:rustc-link-lib=synchronization");
    }
}

/// 提取 C 文本中以 mdb_ 开头的标识符集合。
fn extract_mdb_symbols(content: &str) -> BTreeSet<String> {
    content
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| token.starts_with("mdb_") && token.len() > 4)
        .map(str::to_owned)
        .collect()
}
