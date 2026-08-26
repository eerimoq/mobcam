# Mobcam

<p align="center">
  <img src="logo/logo-mobcam-no-background.png" alt="Mobcam logo" width="200">
</p>

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a
low latency camera on the computer, over a USB cable.

This repository holds two products doing that in two different ways:

| Product | What it is |
| --- | --- |
| [Mobcam OBS plugin](crates/obs-plugin/) | A source type in OBS Studio. macOS, Windows and Linux. |
| [Mobcam virtual camera](crates/virtualcam/) | A v4l2loopback camera and a virtual microphone, for every program that is not OBS Studio. Linux. |

## Development

A [rustup](https://rustup.rs) toolchain is needed rather than any other cargo,
since it is the one that honours `rust-toolchain.toml` and the only one that can
build the second architecture of the macOS universal binary.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
make build
make install
```

### Layout

```
crates/core/         mobcam-core, the USB transport, the Moblin protocol and the decoding
crates/obs-plugin/   mobcam-obs-plugin, the OBS Studio plugin and everything it is packaged from
crates/virtualcam/   mobcam-virtualcam, the virtual camera and microphone
logo/                the logo
scripts/             build.py builds and installs both products, and packages the
                     OBS plugin, and ci.py is what the GitHub Actions workflow runs
```
