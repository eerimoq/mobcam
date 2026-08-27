use std::env;
use std::path::PathBuf;
use std::process::Command;

/// The audio backends, as the `cfg` they enable and the package `pkg-config` knows them by.
const BACKENDS: [(&str, &str); 2] = [("pulse", "libpulse-simple"), ("alsa", "alsa")];

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is set by cargo")
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the crate lives in the repository")
        .to_path_buf()
}

fn configure_prebuilt() {
    let libraries = repo_root().join(".deps").join("prebuilt").join("lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libraries.display());
}

/// Link against `package` if the machine has it, and say whether it did.
fn link(package: &str) -> bool {
    let Ok(output) = Command::new("pkg-config").args(["--libs", package]).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let flags = String::from_utf8_lossy(&output.stdout);
    for flag in flags.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(library) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib=dylib={library}");
        }
    }
    true
}

fn configure_audio() {
    let mut found = false;
    for (backend, package) in BACKENDS {
        if link(package) {
            println!("cargo:rustc-cfg={backend}");
            found = true;
        }
    }
    if !found {
        println!(
            "cargo:warning=neither PulseAudio nor ALSA was found; mobcam-virtualcam is built \
             without audio support. Install libpulse-dev or libasound2-dev and build again to \
             play the audio into a virtual microphone."
        );
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    for (backend, _) in BACKENDS {
        println!("cargo:rustc-check-cfg=cfg({backend})");
    }
    match target_os().as_str() {
        "macos" => configure_prebuilt(),
        "linux" => configure_audio(),
        _ => {}
    }
}
