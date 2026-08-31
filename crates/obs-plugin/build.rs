use mobcam_build::{dependencies_dir, generate, link_pkg_config, out_dir, require, target_os, write_if_changed};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const HEADER: &str = "#include <obs-module.h>\n#include <util/dstr.h>\n#include <util/platform.h>\n";

enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    fn current() -> Platform {
        match target_os().as_str() {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            "linux" => Platform::Linux,
            other => panic!("unsupported target operating system {other}"),
        }
    }
}

fn obs_sources_hash() -> String {
    let marker = dependencies_dir().join(".dependency_obs-studio.sha256");
    println!("cargo:rerun-if-changed={}", marker.display());
    fs::read_to_string(&marker).unwrap_or_default().trim().to_string()
}

fn obs_include_dir() -> PathBuf {
    require(dependencies_dir().join("obs-studio").join("libobs"))
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

fn configure_bundled() -> Vec<PathBuf> {
    let prebuilt_include = require(dependencies_dir().join("prebuilt").join("include"));
    vec![obs_include_dir(), prebuilt_include, obsconfig_dir()]
}

fn configure_macos() -> Vec<PathBuf> {
    println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../Frameworks");
    configure_bundled()
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
    let mut command = cc::windows_registry::find(&mobcam_build::target(), "lib.exe")
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
    println!("cargo:rustc-link-search=native={}", out_dir().display());
    println!("cargo:rustc-link-lib=dylib=obs");
    configure_bundled()
}

fn configure_linux() -> Vec<PathBuf> {
    link_pkg_config(&["libobs"]);
    // The headers of the oldest supported OBS Studio, not the ones the
    // distribution installs, so that the plugin loads in every newer one as
    // well.
    vec![obs_include_dir(), obsconfig_dir()]
}

fn generate_obs(include_dirs: &[PathBuf]) -> PathBuf {
    let builder = mobcam_build::builder(include_dirs)
        .clang_arg("-Wno-implicit-function-declaration")
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
    generate("obs", HEADER, builder, &obs_sources_hash())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
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
