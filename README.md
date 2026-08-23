# OBS MobCam Plugin

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a low
latency camera in OBS Studio, over the USB cable.

## Requirements

- OBS Studio 32.2 or newer.
- Moblin, with the stream URL set to `mobcam://localhost:7790`.
- Moblin's audio codec set to AAC.
- Windows needs the Apple Devices app or iTunes, which installs the Apple Mobile 
  Device Service. Linux needs the `usbmuxd` package.

## Install

Download and install from [releases page](https://github.com/eerimoq/obs-mobcam-plugin/releases):

## Development

The plugin is written in Rust and built by CMake, which drives cargo and then
bundles, signs and packages the result as before. A
[rustup](https://rustup.rs) toolchain is needed rather than any other cargo,
since it is the one that honours `rust-toolchain.toml` and the only one that can
build the second architecture of the macOS universal binary. bindgen generates
the libobs and FFmpeg bindings at build time and needs libclang: from Xcode on
macOS, `libclang-dev` on Ubuntu, and LLVM on Windows.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cmake --preset macos
cmake --build build_macos --config Release
cmake --install build_macos --prefix dist
open dist/mobcam.pkg
```

The parsers, the clock and the format tables are covered by unit tests that do
not need OBS. They need the same headers the build uses:

```shell
export MOBCAM_OBS_INCLUDE_DIRS=.deps/Frameworks/libobs.framework/Headers
export MOBCAM_FFMPEG_INCLUDE_DIRS=.deps/obs-deps-2026-07-15-universal/include
cargo test
cargo clippy --all-targets
```
