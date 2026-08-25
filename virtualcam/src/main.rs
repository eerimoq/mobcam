//! Use an iPhone or iPad running Moblin as a camera in any program, over USB.

#[cfg(target_os = "linux")]
mod alsa;
#[cfg(target_os = "linux")]
mod audio;
#[cfg(target_os = "linux")]
mod camera;
#[cfg(target_os = "linux")]
mod convert;
#[cfg(target_os = "linux")]
mod dynlib;
#[cfg(target_os = "linux")]
mod options;
#[cfg(target_os = "linux")]
mod pulse;
#[cfg(target_os = "linux")]
mod v4l2;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    camera::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("mobcam-virtualcam is only supported on Linux");
    std::process::ExitCode::FAILURE
}
