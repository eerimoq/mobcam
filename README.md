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

## Development on macOS

```shell
cmake --preset macos
cmake --build build_macos --config Release
cmake --install build_macos --prefix dist
open dist/mobcam.pkg
```
