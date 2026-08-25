use std::env;
use std::path::PathBuf;

/// This executable is only built for real on Linux, where libavcodec and
/// libavutil come from the distribution and the dynamic linker finds them on
/// its own. Everywhere else they come from the downloaded dependencies that
/// `core/build.rs` links against, and `cargo test` needs an rpath to find them.
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"));
    let libraries = manifest_dir
        .parent()
        .expect("the crate lives in the repository")
        .join(".deps")
        .join("prebuilt")
        .join("lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libraries.display());
}
