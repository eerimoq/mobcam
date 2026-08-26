# Mobcam

<p align="center">
  <img src="logo/logo-mobcam-no-background.png" alt="Mobcam logo" width="200">
</p>

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a
low latency camera on the computer, over a USB cable.

This repository holds two products doing that in two different ways:

| Product | What it is |
| --- | --- |
| [Mobcam OBS Plugin](crates/obs-plugin/) | A source type in OBS Studio. macOS, Windows and Linux. |
| [Mobcam Virtual Camera](crates/virtualcam/) | A v4l2loopback camera and a virtual microphone, for every program that is not OBS Studio. Linux. |

## Development

A [rustup](https://rustup.rs) toolchain is needed rather than any other cargo,
since it is the one that honours `rust-toolchain.toml` and the only one that can
build the second architecture of the macOS universal binary.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
make build
make install
```
