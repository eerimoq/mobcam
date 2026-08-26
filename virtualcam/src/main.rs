#[cfg(unix)]
mod alsa;
#[cfg(unix)]
mod audio;
#[cfg(unix)]
mod camera;
#[cfg(unix)]
mod convert;
#[cfg(unix)]
mod dynlib;
#[cfg(unix)]
mod options;
#[cfg(unix)]
mod pulse;
#[cfg(unix)]
mod v4l2;

const UNSUPPORTED: &str = "mobcam-virtualcam is only supported on Linux";

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    if !cfg!(target_os = "linux") {
        eprintln!("{UNSUPPORTED}");
        return std::process::ExitCode::FAILURE;
    }
    camera::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!("{UNSUPPORTED}");
    std::process::ExitCode::FAILURE
}
