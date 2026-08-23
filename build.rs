//! Generates the FFI to libobs and FFmpeg.
//!
//! The include directories come from CMake, which has already resolved them
//! through `find_package(libobs)` and the FFmpeg lookup in CMakeLists.txt. That
//! indirection is the point: the headers bindgen reads are guaranteed to be the
//! headers the plugin is linked against, which is what keeps the generated
//! FFmpeg bindings in step with the FFmpeg that OBS loads at runtime.

use std::env;
use std::path::PathBuf;

/// CMake hands these over as bar separated path lists. A bar rather than a
/// semicolon because CMake would otherwise treat the value as a list and split
/// it into separate arguments on the way here, and rather than a colon because
/// Windows paths contain those.
fn include_dirs(variable: &str) -> Vec<String> {
    println!("cargo:rerun-if-env-changed={variable}");

    match env::var(variable) {
        Ok(value) => value
            .split('|')
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn builder(include_dirs: &[String]) -> bindgen::Builder {
    // cargo builds one triple at a time and bindgen does not infer it. Without
    // this the x86_64 half of a macOS universal build would be generated with
    // arm64 struct layouts, which links cleanly and then misreads every field.
    let target = env::var("TARGET").expect("TARGET is set by cargo");

    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("--target={target}"))
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
        builder = builder.clang_arg(format!("-I{dir}"));
    }

    builder
}

fn write(bindings: bindgen::Bindings, name: &str) {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    bindings
        .write_to_file(out.join(name))
        .unwrap_or_else(|error| panic!("failed to write {name}: {error}"));
}

fn generate_obs(include_dirs: &[String]) {
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

    write(bindings, "obs.rs");
}

fn generate_ffmpeg(include_dirs: &[String]) {
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

/// buildspec.json is the single source of truth for the plugin version; CMake
/// reads it in bootstrap.cmake. Cargo cannot, so the two are checked against
/// each other here rather than left to drift.
fn check_version() {
    println!("cargo:rerun-if-changed=buildspec.json");

    let buildspec = std::fs::read_to_string("buildspec.json").expect("buildspec.json is next to Cargo.toml");
    let buildspec: serde_json::Value = serde_json::from_str(&buildspec).expect("buildspec.json is valid JSON");

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
    // Both sets go to both generators. libobs' headers reach for simde, which
    // obs-deps ships alongside FFmpeg rather than alongside libobs, and this is
    // the same union of include paths CMake compiles the C side with.
    let mut dirs = include_dirs("MOBCAM_OBS_INCLUDE_DIRS");

    dirs.extend(include_dirs("MOBCAM_FFMPEG_INCLUDE_DIRS"));

    generate_obs(&dirs);
    generate_ffmpeg(&dirs);
    check_version();
}
