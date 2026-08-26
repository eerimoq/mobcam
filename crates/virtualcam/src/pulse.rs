use std::path::PathBuf;

#[cfg_attr(pulse, path = "pulse/supported.rs")]
#[cfg_attr(not(pulse), path = "pulse/unsupported.rs")]
mod stream;

pub use stream::Stream;

unsafe extern "C" {
    fn getuid() -> u32;
}

fn socket() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { getuid() })));
    runtime_dir.join("pulse").join("native")
}

pub fn available() -> bool {
    cfg!(pulse) && (std::env::var_os("PULSE_SERVER").is_some() || socket().exists())
}
