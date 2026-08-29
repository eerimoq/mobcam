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
