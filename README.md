<p align="center">
  <img src="logo/logo-mobcam-no-background.png" alt="Mobcam logo" width="200">
</p>

# Mobcam

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a
low latency camera on the computer, over a USB cable.

This repository holds two products doing that in two different ways:

| Product | Where | What it is |
| --- | --- | --- |
| [Mobcam OBS plugin](obs-plugin/) | `obs-plugin/` | A source type in OBS Studio. macOS, Windows and Linux. |
| [Mobcam virtual camera](virtualcam/) | `virtualcam/` | A v4l2loopback camera and a virtual microphone, for every program that is not OBS Studio. Linux. |

Each is installed and used on its own, and each has its own page above with the
requirements, the install steps and the usage. They share `core/`, the crate
with the USB transport, the Moblin protocol and the decoding, and they are
built, packaged and released together.

Both need Moblin with the stream URL set to `mobcam://localhost:7790` and the
audio codec set to AAC, and both need the operating system to be able to talk
to the connected iPhone or iPad. What that takes differs per operating system
and is on the product pages.

## Releases

Every release is on the
[releases page](https://github.com/eerimoq/mobcam/releases). One tag releases
both products, and the name of a file says which product it belongs to:

```
mobcam-obs-plugin-<version>-macos-universal.pkg
mobcam-obs-plugin-<version>-windows-x64-Installer.exe
mobcam-obs-plugin-<version>-windows-x64.zip
mobcam-obs-plugin-<version>-x86_64-linux-gnu.deb
mobcam-obs-plugin-<version>-x86_64-linux-gnu.tar.xz
mobcam-virtualcam-<version>-x86_64-linux-gnu.deb
mobcam-virtualcam-<version>-x86_64-linux-gnu.tar.xz
mobcam-<version>-source.tar.xz
```

## Layout

```
core/         mobcam-core, the USB transport, the Moblin protocol and the decoding
obs-plugin/   mobcam-obs-plugin, the OBS Studio plugin and everything it is packaged from
virtualcam/   mobcam-virtualcam, the virtual camera and microphone
logo/         the logo
build.py      builds, packages and installs both products
ci.py         what the GitHub Actions workflow runs
```

A product directory holds its own sources, its own `build.rs` and its own
`packaging/` templates, so nothing but `core/` and the build scripts is shared.

## Development

Everything is written in Rust, in a cargo workspace of three crates: `core`
with what both products need, `obs-plugin` with the OBS Studio plugin on top of
it, and `virtualcam` with the virtual camera. cargo compiles and links them,
and `build.py` does everything around that: the dependencies, the macOS bundle,
the code signing and the installers.

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
Windows and builds every product the host operating system has one for: the
plugin everywhere, the virtual camera on Linux. Each is staged under
`release/install/<product directory>`. `build.py install` copies them into the
OBS plugin directory and the binary directory of the current user. The archives
and the installers the releases are made of come from `build.py package
--installer`.

On Linux there is nothing to download: libobs and FFmpeg come from the
distribution, which needs `libobs-dev`, `libavcodec-dev`, `libavutil-dev`,
`libsimde-dev` and `pkg-config` installed. `mobcam-virtualcam` needs none of
libobs, so `cargo build -p mobcam-virtualcam` gets by with the FFmpeg packages
alone; it loads libpulse and libasound with `dlopen` at run time, so neither is
needed to build it, and it runs on a machine that has only one of them.

The dependencies are all `build.py` needs, so cargo can be run directly too:

```shell
cargo build
cargo clippy --workspace --all-targets
cargo test --workspace
cargo fmt
```
