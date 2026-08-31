use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

pub use bindgen;

pub fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
}

pub fn target() -> String {
    env::var("TARGET").expect("TARGET is set by cargo")
}

pub fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is set by cargo")
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the crate lives in the repository")
        .to_path_buf()
}

pub fn dependencies_dir() -> PathBuf {
    repo_root().join(".deps")
}

pub fn require(path: PathBuf) -> PathBuf {
    assert!(
        path.exists(),
        "{} is missing; run `python3 scripts/build.py deps` to download the dependencies",
        path.display()
    );
    path
}

pub fn write_if_changed(path: &Path, contents: &str) {
    if fs::read_to_string(path).is_ok_and(|old| old == contents) {
        return;
    }
    fs::write(path, contents).unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

pub fn pkg_config(packages: &[&str], flags: &str) -> Vec<String> {
    let output = Command::new("pkg-config")
        .arg(flags)
        .args(packages)
        .output()
        .expect("pkg-config is installed");
    assert!(
        output.status.success(),
        "pkg-config {flags} {} failed: {}",
        packages.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout)
        .expect("pkg-config prints UTF-8")
        .split_whitespace()
        .map(|flag| flag[2..].to_string())
        .collect()
}

pub fn link_pkg_config(packages: &[&str]) -> Vec<PathBuf> {
    for dir in pkg_config(packages, "--libs-only-L") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    for library in pkg_config(packages, "--libs-only-l") {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
    pkg_config(packages, "--cflags-only-I")
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

pub fn builder(include_dirs: &[PathBuf]) -> bindgen::Builder {
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target()))
        .derive_default(true)
        .generate_comments(false)
        .layout_tests(false)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false);
    for dir in include_dirs {
        builder = builder.clang_arg(format!("-I{}", dir.display()));
    }
    builder
}

pub fn generate(name: &str, header: &str, builder: bindgen::Builder, sources: &str) -> PathBuf {
    let path = out_dir().join(format!("{name}.rs"));
    let stamp_path = out_dir().join(format!("{name}.stamp"));
    let stamp = format!("{}\n{header}\n{sources}", builder.command_line_flags().join(" "));
    if path.exists() && fs::read_to_string(&stamp_path).is_ok_and(|old| old == stamp) {
        return path;
    }
    builder
        .header_contents(&format!("{name}.h"), header)
        .generate()
        .unwrap_or_else(|error| panic!("failed to generate {name} bindings: {error}"))
        .write_to_file(&path)
        .unwrap_or_else(|error| panic!("failed to write {name}.rs: {error}"));
    write_if_changed(&stamp_path, &stamp);
    path
}
