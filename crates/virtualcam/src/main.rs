mod alsa;
mod audio;
mod camera;
mod convert;
mod options;
mod pulse;
mod v4l2;

fn main() -> std::process::ExitCode {
    camera::main()
}
