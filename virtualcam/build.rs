use std::env;
use std::path::PathBuf;

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
