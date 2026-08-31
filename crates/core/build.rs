use mobcam_build::{dependencies_dir, generate, link_pkg_config, require, target_os};
use std::path::PathBuf;

const HEADER: &str = "#include <libavcodec/avcodec.h>\n\
                      #include <libavutil/channel_layout.h>\n\
                      #include <libavutil/hwcontext.h>\n\
                      #include <libavutil/pixdesc.h>\n\
                      #include <libavutil/samplefmt.h>\n";

fn configure_prebuilt() -> Vec<PathBuf> {
    let prebuilt_dir = require(dependencies_dir().join("prebuilt"));
    let lib_dir = prebuilt_dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=avcodec");
    println!("cargo:rustc-link-lib=dylib=avutil");
    // The unit tests run outside any bundle, so they need to find the dylibs.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    vec![prebuilt_dir.join("include")]
}

fn main() {
    let include_dirs = match target_os().as_str() {
        "macos" | "windows" => configure_prebuilt(),
        "linux" => link_pkg_config(&["libavcodec", "libavutil"]),
        other => panic!("unsupported target operating system {other}"),
    };
    let builder = mobcam_build::builder(&include_dirs)
        .allowlist_item("av_.*")
        .allowlist_item("avcodec_.*")
        .allowlist_item("AV_.*")
        .allowlist_item("AV(Codec|Packet|Frame|Buffer|Pixel|Sample|HWDevice|Rational|Channel|Dictionary|Class|Color|Media|Profile|Discard|Field|Chroma|Audio).*")
        .allowlist_item("AVERROR.*")
        .allowlist_item("FF_.*")
        .allowlist_item("LIBAV(CODEC|UTIL)_VERSION.*")
        .allowlist_item("avutil_version");
    generate("ffmpeg", HEADER, builder, "");
}
