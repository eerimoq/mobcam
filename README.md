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

The plugin is written in Rust. cargo compiles and links it, and `build.py`
does everything around that: the dependencies, the macOS bundle, the code
signing and the installers.

A [rustup](https://rustup.rs) toolchain is needed rather than any other cargo,
since it is the one that honours `rust-toolchain.toml` and the only one that can
build the second architecture of the macOS universal binary. bindgen generates
the libobs and FFmpeg bindings at build time and needs libclang: from Xcode on
macOS, `libclang-dev` on Ubuntu, and LLVM on Windows.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
python3 build.py build
python3 build.py install
```

`build.py build` downloads the dependencies it names into `.deps` on macOS and
Windows, builds the plugin and stages it under `release/install`. `build.py
install` copies it into the OBS plugin directory of the current user. The
archives and the installers the releases are made of come from `build.py
package --installer`.

On Linux there is nothing to download: libobs and FFmpeg come from the
distribution, which needs `libobs-dev`, `libavcodec-dev`, `libavutil-dev`,
`libsimde-dev` and `pkg-config` installed.

The dependencies are all `build.py` needs, so cargo can be run directly too:

```shell
cargo build
cargo clippy --all-targets
cargo fmt
```
