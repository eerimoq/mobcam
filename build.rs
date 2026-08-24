//! Generates the FFI to libobs and FFmpeg, and tells cargo how to link the
//! plugin.
//!
//! The plugin is the whole of the module OBS loads, so cargo does the linking
//! and this file is where the libraries and the headers are resolved. macOS and
//! Windows take both from the `.deps` directory that `build.py deps` fills from
//! buildspec.json; Linux takes them from pkg-config, where the distribution's
//! obs and FFmpeg development packages put them.
//!
//! The headers bindgen reads are therefore the headers of the libraries the
//! plugin is linked against, which is what keeps the generated FFmpeg bindings
//! in step with the FFmpeg that OBS loads at runtime.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// The plugin is one architecture per cargo invocation, so the target is what
/// decides which dependencies are looked for and how they are linked.
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

fn buildspec() -> serde_json::Value {
    println!("cargo:rerun-if-changed=buildspec.json");

    let buildspec = manifest_dir().join("buildspec.json");
    let buildspec = fs::read_to_string(buildspec).expect("buildspec.json is next to Cargo.toml");

    serde_json::from_str(&buildspec).expect("buildspec.json is valid JSON")
}

fn dependency_version(buildspec: &serde_json::Value, dependency: &str) -> String {
    buildspec["dependencies"][dependency]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("buildspec.json has a version for {dependency}"))
        .to_string()
}

/// The directory `build.py deps` downloads and unpacks the prebuilt
/// dependencies into.
fn dependencies_dir() -> PathBuf {
    println!("cargo:rerun-if-env-changed=MOBCAM_DEPS_DIR");

    match env::var("MOBCAM_DEPS_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => manifest_dir().join(".deps"),
    }
}

fn require(path: PathBuf) -> PathBuf {
    assert!(
        path.exists(),
        "{} is missing; run `python3 build.py deps` to download the dependencies",
        path.display()
    );

    path
}

/// obs-config.h includes obsconfig.h, which OBS' own build generates rather
/// than ships. Only the two release state macros are read from it, and a
/// released libobs is neither a beta nor a release candidate.
fn obsconfig_dir() -> PathBuf {
    let dir = out_dir().join("obsconfig");

    fs::create_dir_all(&dir).expect("failed to create the obsconfig directory");
    fs::write(
        dir.join("obsconfig.h"),
        "#pragma once\n\n#define OBS_RELEASE_CANDIDATE 0\n#define OBS_BETA 0\n",
    )
    .expect("failed to write obsconfig.h");

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

/// The libobs symbols are left to be resolved out of the OBS process the plugin
/// is loaded into, which is where they come from at runtime in any case. That is
/// what makes libobs itself unnecessary at link time, and with it the build of
/// obs-studio the plugin would otherwise need just to link. FFmpeg is a real
/// link: obs-deps ships the dylibs, and only the headers of the FFmpeg the
/// plugin is built against can be trusted to describe it.
fn configure_macos(buildspec: &serde_json::Value) -> Vec<PathBuf> {
    let dependencies = dependencies_dir();
    let obs = dependency_version(buildspec, "obs-studio");
    let prebuilt = dependency_version(buildspec, "prebuilt");

    let obs_include = require(dependencies.join(format!("obs-studio-{obs}")).join("libobs"));
    let prebuilt_dir = require(dependencies.join(format!("obs-deps-{prebuilt}-universal")));

    // FFmpeg is linked against the dylibs obs-deps ships, so the plugin records
    // the same @rpath references OBS' own binaries do, and finds them in the
    // Frameworks directory of the application bundle it is loaded from.
    println!("cargo:rustc-link-search=native={}", prebuilt_dir.join("lib").display());
    println!("cargo:rustc-link-lib=dylib=avcodec");
    println!("cargo:rustc-link-lib=dylib=avutil");
    println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");

    vec![obs_include, prebuilt_dir.join("include"), obsconfig_dir()]
}

/// A Windows DLL cannot leave symbols to be resolved at load time the way a
/// macOS bundle can, so libobs needs an import library. Building obs-studio to
/// get one would mean a second build system, so it is generated instead: an
/// import library is a list of names and the DLL they come from, and the names
/// are the ones bindgen just declared.
///
/// cc is only depended on where lib.exe exists, so this half of the file only
/// compiles there. Cross compiling the plugin to Windows was never possible in
/// any case: it needs the Microsoft linker.
#[cfg(windows)]
fn write_import_library(bindings: &Path) {
    let source = fs::read_to_string(bindings).expect("the generated bindings are readable");
    let mut exports = String::from("LIBRARY obs\nEXPORTS\n");
    let mut inside = false;
    // What the symbol is called in the DLL, where bindgen has said that it is
    // not what the item is called in Rust.
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
            // bindgen prefixes the name with a byte the compiler strips again,
            // to keep it from being mangled any further.
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
            // Without DATA the linker would generate a call to the address of
            // the variable rather than a read of the variable itself.
            exports += &format!("    {name} DATA\n");
        }
    }

    let out = out_dir();
    let definition = out.join("obs.def");
    let library = out.join("obs.lib");

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

fn configure_windows(buildspec: &serde_json::Value) -> Vec<PathBuf> {
    let dependencies = dependencies_dir();
    let obs = dependency_version(buildspec, "obs-studio");
    let prebuilt = dependency_version(buildspec, "prebuilt");

    let obs_include = require(dependencies.join(format!("obs-studio-{obs}")).join("libobs"));
    let prebuilt_dir = require(dependencies.join(format!("obs-deps-{prebuilt}-x64")));

    println!("cargo:rustc-link-search=native={}", prebuilt_dir.join("lib").display());
    println!("cargo:rustc-link-lib=dylib=avcodec");
    println!("cargo:rustc-link-lib=dylib=avutil");
    println!("cargo:rustc-link-search=native={}", out_dir().display());
    println!("cargo:rustc-link-lib=dylib=obs");

    vec![obs_include, prebuilt_dir.join("include"), obsconfig_dir()]
}

/// Linux has no prebuilt dependencies of its own: libobs and FFmpeg are the
/// distribution's, which is what OBS itself is built against there.
fn configure_linux() -> Vec<PathBuf> {
    let packages = ["libobs", "libavcodec", "libavutil"];

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
    // cargo builds one triple at a time and bindgen does not infer it. Without
    // this the x86_64 half of a macOS universal build would be generated with
    // arm64 struct layouts, which links cleanly and then misreads every field.
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={}", target()))
        .derive_default(true)
        .generate_comments(false)
        .layout_tests(false)
        // Enum handling: OBS and FFmpeg both pass enums across the ABI, and a
        // value neither header knows about must not become an invalid Rust
        // enum, so they stay plain integer constants.
        .default_enum_style(bindgen::EnumVariation::Consts)
        // Without this every constant is prefixed with the name of the enum it
        // came from, so AV_PIX_FMT_NV12 would have to be spelled
        // AVPixelFormat_AV_PIX_FMT_NV12. The C names are what the FFmpeg and
        // OBS documentation uses, so they are what the code should use too.
        .prepend_enum_name(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    for dir in include_dirs {
        builder = builder.clang_arg(format!("-I{}", dir.display()));
    }

    builder
}

fn write(bindings: bindgen::Bindings, name: &str) -> PathBuf {
    let path = out_dir().join(name);

    bindings
        .write_to_file(&path)
        .unwrap_or_else(|error| panic!("failed to write {name}: {error}"));

    path
}

fn generate_obs(include_dirs: &[PathBuf]) -> PathBuf {
    let bindings = builder(include_dirs)
        .header_contents(
            "obs.h",
            "#include <obs-module.h>\n#include <util/dstr.h>\n#include <util/platform.h>\n",
        )
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
        // Takes a va_list, which is awkward to build from Rust and unnecessary:
        // logging goes through the variadic blog() with a preformatted "%s".
        .blocklist_function("blogva")
        .generate()
        .expect("failed to generate libobs bindings");

    write(bindings, "obs.rs")
}

fn generate_ffmpeg(include_dirs: &[PathBuf]) {
    let bindings = builder(include_dirs)
        .header_contents(
            "ffmpeg.h",
            "#include <libavcodec/avcodec.h>\n\
             #include <libavutil/channel_layout.h>\n\
             #include <libavutil/hwcontext.h>\n\
             #include <libavutil/pixdesc.h>\n\
             #include <libavutil/samplefmt.h>\n",
        )
        .allowlist_item("av_.*")
        .allowlist_item("avcodec_.*")
        .allowlist_item("AV_.*")
        .allowlist_item("AV(Codec|Packet|Frame|Buffer|Pixel|Sample|HWDevice|Rational|Channel|Dictionary|Class|Color|Media|Profile|Discard|Field|Chroma|Audio).*")
        .allowlist_item("AVERROR.*")
        .allowlist_item("FF_.*")
        .generate()
        .expect("failed to generate FFmpeg bindings");

    write(bindings, "ffmpeg.rs");
}

/// buildspec.json is the single source of truth for the plugin version; build.py
/// reads it for the bundle and the installers. Cargo cannot, so the two are
/// checked against each other here rather than left to drift.
fn check_version(buildspec: &serde_json::Value) {
    let version = buildspec["version"]
        .as_str()
        .expect("buildspec.json has a top level version");
    let cargo = env!("CARGO_PKG_VERSION");

    assert_eq!(
        version, cargo,
        "buildspec.json and Cargo.toml disagree about the plugin version"
    );
}

fn main() {
    let buildspec = buildspec();

    check_version(&buildspec);

    let platform = Platform::current();

    // Both sets of headers go to both generators. libobs' headers reach for
    // simde, which obs-deps ships alongside FFmpeg rather than alongside
    // libobs, and this is the same union of include paths OBS itself is built
    // with.
    let include_dirs = match platform {
        Platform::Macos => configure_macos(&buildspec),
        Platform::Windows => configure_windows(&buildspec),
        Platform::Linux => configure_linux(),
    };

    let obs_bindings = generate_obs(&include_dirs);

    generate_ffmpeg(&include_dirs);

    if let Platform::Windows = platform {
        write_import_library(&obs_bindings);
    }
}
