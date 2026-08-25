use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    fn current() -> Platform {
        match env::var("CARGO_CFG_TARGET_OS")
            .expect("CARGO_CFG_TARGET_OS is set by cargo")
            .as_str()
        {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            "linux" => Platform::Linux,
            other => panic!("unsupported target operating system {other}"),
        }
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"))
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
}

fn target() -> String {
    env::var("TARGET").expect("TARGET is set by cargo")
}

fn dependencies_dir() -> PathBuf {
    manifest_dir().join(".deps")
}

fn require(path: PathBuf) -> PathBuf {
    assert!(
        path.exists(),
        "{} is missing; run `python3 build.py deps` to download the dependencies",
        path.display()
    );
    path
}

fn write_if_changed(path: &Path, contents: &str) {
    if fs::read_to_string(path).is_ok_and(|old| old == contents) {
        return;
    }
    fs::write(path, contents).unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn obsconfig_dir() -> PathBuf {
    let dir = out_dir().join("obsconfig");
    fs::create_dir_all(&dir).expect("failed to create the obsconfig directory");
    write_if_changed(
        &dir.join("obsconfig.h"),
        "#pragma once\n\n#define OBS_RELEASE_CANDIDATE 0\n#define OBS_BETA 0\n",
    );
    dir
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

fn configure_macos() -> Vec<PathBuf> {
    let dependencies = dependencies_dir();
    let obs_include = require(dependencies.join("obs-studio").join("libobs"));
    let prebuilt_include = require(dependencies.join("prebuilt").join("include"));
    println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    vec![obs_include, prebuilt_include, obsconfig_dir()]
}

#[cfg(windows)]
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

#[cfg(windows)]
fn write_import_library(bindings: &Path) {
    let library = out_dir().join("obs.lib");
    if modified(&library) >= modified(bindings) {
        return;
    }
    let source = fs::read_to_string(bindings).expect("the generated bindings are readable");
    let mut exports = String::from("LIBRARY obs\nEXPORTS\n");
    let mut inside = false;
    let mut link_name = None;
    for line in source.lines() {
        let line = line.trim();
        if line.ends_with("extern \"C\" {") {
            inside = true;
        } else if inside && line == "}" {
            inside = false;
        } else if !inside {
            continue;
        } else if let Some(rest) = line.strip_prefix("#[link_name = \"") {
            let (name, _) = rest.split_once('"').expect("a link name is quoted");
            link_name = Some(name.trim_start_matches("\\u{1}").to_string());
        } else if let Some(rest) = line.strip_prefix("pub fn ") {
            let (name, _) = rest.split_once('(').expect("a function declaration has arguments");
            let name = link_name.take().unwrap_or_else(|| name.to_string());
            exports += &format!("    {name}\n");
        } else if let Some(rest) = line
            .strip_prefix("pub static mut ")
            .or_else(|| line.strip_prefix("pub static "))
        {
            let (name, _) = rest.split_once(':').expect("a variable declaration has a type");
            let name = link_name.take().unwrap_or_else(|| name.to_string());
            exports += &format!("    {name} DATA\n");
        }
    }
    let definition = out_dir().join("obs.def");
    fs::write(&definition, exports).expect("failed to write obs.def");
    let mut command = cc::windows_registry::find(&target(), "lib.exe")
        .unwrap_or_else(|| panic!("lib.exe not found; a Visual Studio installation is required"));
    let status = command
        .arg("/nologo")
        .arg(format!("/def:{}", definition.display()))
        .arg(format!("/out:{}", library.display()))
        .arg("/machine:X64")
        .status()
        .expect("failed to run lib.exe");
    assert!(status.success(), "lib.exe failed to generate the libobs import library");
}

#[cfg(not(windows))]
fn write_import_library(_bindings: &Path) {
    panic!("the Windows plugin can only be built on Windows, where lib.exe is");
}

fn configure_windows() -> Vec<PathBuf> {
    let dependencies = dependencies_dir();
    let obs_include = require(dependencies.join("obs-studio").join("libobs"));
    let prebuilt_include = require(dependencies.join("prebuilt").join("include"));
    println!("cargo:rustc-link-search=native={}", out_dir().display());
    println!("cargo:rustc-link-lib=dylib=obs");
    vec![obs_include, prebuilt_include, obsconfig_dir()]
}

fn configure_linux() -> Vec<PathBuf> {
    let packages = ["libobs"];
    for dir in pkg_config(&packages, "--libs-only-L") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    for library in pkg_config(&packages, "--libs-only-l") {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
    let mut include_dirs: Vec<PathBuf> = pkg_config(&packages, "--cflags-only-I")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    include_dirs.push(obsconfig_dir());
    include_dirs
}

fn builder(include_dirs: &[PathBuf]) -> bindgen::Builder {
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target()))
        .clang_arg("-Wno-implicit-function-declaration")
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

fn generate(name: &str, header: &str, builder: bindgen::Builder) -> PathBuf {
    let path = out_dir().join(format!("{name}.rs"));
    let stamp_path = out_dir().join(format!("{name}.stamp"));
    let stamp = format!("{}\n{header}", builder.command_line_flags().join(" "));
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

fn generate_obs(include_dirs: &[PathBuf]) -> PathBuf {
    let builder = builder(include_dirs)
        .allowlist_item("obs_.*")
        .allowlist_item("OBS_.*")
        .allowlist_item("blog")
        .allowlist_item("b(malloc|realloc|free|zalloc|strdup|memdup)")
        .allowlist_item("dstr_.*")
        .allowlist_item("os_.*")
        .allowlist_item("video_.*")
        .allowlist_item("VIDEO_.*")
        .allowlist_item("audio_.*")
        .allowlist_item("AUDIO_.*")
        .allowlist_item("speaker_.*")
        .allowlist_item("SPEAKERS_.*")
        .allowlist_item("LOG_.*")
        .allowlist_item("LIBOBS_API_.*")
        .allowlist_item("MAX_AV_PLANES")
        .allowlist_item("get_audio_planes")
        .allowlist_item("text_lookup_.*")
        .allowlist_item("lookup_t")
        .blocklist_function("blogva");
    generate(
        "obs",
        "#include <obs-module.h>\n#include <util/dstr.h>\n#include <util/platform.h>\n",
        builder,
    )
}

fn main() {
    let platform = Platform::current();
    let include_dirs = match platform {
        Platform::Macos => configure_macos(),
        Platform::Windows => configure_windows(),
        Platform::Linux => configure_linux(),
    };
    let obs_bindings = generate_obs(&include_dirs);
    if let Platform::Windows = platform {
        write_import_library(&obs_bindings);
    }
}
