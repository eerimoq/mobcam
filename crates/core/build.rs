use std::env;
use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the core crate lives in the repository")
        .to_path_buf()
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
}

fn target() -> String {
    env::var("TARGET").expect("TARGET is set by cargo")
}

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is set by cargo")
}

fn prebuilt_dir() -> PathBuf {
    let path = repo_root().join(".deps").join("prebuilt");
    assert!(
        path.exists(),
        "{} is missing; run `python3 scripts/build.py deps` to download the dependencies",
        path.display()
    );
    path
}

fn pkg_config(packages: &[&str], flags: &str) -> Vec<String> {
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

fn configure_prebuilt() -> Vec<PathBuf> {
    let prebuilt_dir = prebuilt_dir();
    println!("cargo:rustc-link-search=native={}", prebuilt_dir.join("lib").display());
    println!("cargo:rustc-link-lib=dylib=avcodec");
    println!("cargo:rustc-link-lib=dylib=avutil");
    // The unit tests run outside any bundle, so they need to find the dylibs.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", prebuilt_dir.join("lib").display());
    vec![prebuilt_dir.join("include")]
}

fn configure_pkg_config() -> Vec<PathBuf> {
    let packages = ["libavcodec", "libavutil"];
    for dir in pkg_config(&packages, "--libs-only-L") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    for library in pkg_config(&packages, "--libs-only-l") {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
    pkg_config(&packages, "--cflags-only-I")
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

fn generate_ffmpeg(include_dirs: &[PathBuf]) {
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target()))
        .derive_default(true)
        .generate_comments(false)
        .layout_tests(false)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .allowlist_item("av_.*")
        .allowlist_item("avcodec_.*")
        .allowlist_item("AV_.*")
        .allowlist_item("AV(Codec|Packet|Frame|Buffer|Pixel|Sample|HWDevice|Rational|Channel|Dictionary|Class|Color|Media|Profile|Discard|Field|Chroma|Audio).*")
        .allowlist_item("AVERROR.*")
        .allowlist_item("FF_.*");
    for dir in include_dirs {
        builder = builder.clang_arg(format!("-I{}", dir.display()));
    }
    let path = out_dir().join("ffmpeg.rs");
    let stamp_path = out_dir().join("ffmpeg.stamp");
    let header = "#include <libavcodec/avcodec.h>\n\
                  #include <libavutil/channel_layout.h>\n\
                  #include <libavutil/hwcontext.h>\n\
                  #include <libavutil/pixdesc.h>\n\
                  #include <libavutil/samplefmt.h>\n";
    let stamp = format!("{}\n{header}", builder.command_line_flags().join(" "));
    if path.exists() && std::fs::read_to_string(&stamp_path).is_ok_and(|old| old == stamp) {
        return;
    }
    builder
        .header_contents("ffmpeg.h", header)
        .generate()
        .unwrap_or_else(|error| panic!("failed to generate ffmpeg bindings: {error}"))
        .write_to_file(&path)
        .unwrap_or_else(|error| panic!("failed to write ffmpeg.rs: {error}"));
    std::fs::write(&stamp_path, stamp).expect("failed to write ffmpeg.stamp");
}

fn main() {
    let include_dirs = match target_os().as_str() {
        "macos" | "windows" => configure_prebuilt(),
        "linux" => configure_pkg_config(),
        other => panic!("unsupported target operating system {other}"),
    };
    generate_ffmpeg(&include_dirs);
}
