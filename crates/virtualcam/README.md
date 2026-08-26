# Mobcam Virtual Camera

<p align="center">
  <img src="../../logo/logo-mobcam-no-background.png" alt="Mobcam logo" width="200">
</p>

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a
low latency camera and microphone in every program that can use one, over USB.

Linux only.

## Requirements

- Moblin, with the stream URL set to `mobcam://localhost:7790`.
- Moblin's audio codec set to AAC.
- `usbmuxd`, to talk to the iPhone or iPad over the USB cable.
- The `v4l2loopback` kernel module.
- PulseAudio, PipeWire or ALSA, for the microphone, and its development
  package when building. The video keeps going without any of them.

## Build and install

Install a [rustup](https://rustup.rs) toolchain, the build dependencies and
`usbmuxd`, then connect the device once, unlock it and tap Trust:

```shell
sudo apt install build-essential pkg-config libclang-dev libavcodec-dev \
    libavutil-dev libpulse-dev libasound2-dev usbmuxd
```

`libpulse-dev` and `libasound2-dev` are what the microphone is built against.
Whichever of them `pkg-config` finds is linked in, and a machine that has
neither builds a camera without a microphone.

Build it, either from a clone of the repository or from the
`mobcam-<version>-source.tar.xz` tarball on the
[releases page](https://github.com/eerimoq/mobcam/releases):

```shell
cargo build --locked --release --package mobcam-virtualcam
```

The binary ends up in `target/release/mobcam-virtualcam`. Copy it wherever it
is wanted:

```shell
sudo install -m 755 target/release/mobcam-virtualcam /usr/local/bin
```

Uninstall it by removing the binary again.

## Setting up the camera and the microphone

Install the kernel module and load it:

```shell
sudo apt install v4l2loopback-dkms
sudo modprobe v4l2loopback card_label=Mobcam exclusive_caps=1
```

`exclusive_caps=1` hides the device until something is writing to it, which is
what Chrome, Firefox and most video conferencing programs expect. `card_label`
is the name the camera shows up under.

Create the sink the audio is played into:

```shell
pactl load-module module-null-sink sink_name=Mobcam sink_properties=device.description=Mobcam
```

PulseAudio and PipeWire both take that, and both then offer `Monitor of Mobcam`
as a microphone. Programs record from it the way they record from any other
one. The sink is gone at the next reboot, so put the command in the session
autostart to keep it.

## Usage

Start it and leave it running:

```shell
mobcam-virtualcam
```

It picks the first v4l2loopback device, the `Mobcam` sink and the first attached
iPhone or iPad, waits for Moblin to connect and writes every frame it decodes.
The camera becomes selectable in other programs once the first frame arrives.

`mobcam-virtualcam --list` prints the attached iPhones and iPads, the
v4l2loopback devices and the virtual microphones, and `--help` the rest of the
options, `--device`, `--udid` and `--audio-device` among them to pick which of
each to use.

The video keeps going on its own if the audio cannot be played: nothing is set
up, the sink was never created, `--no-audio` was passed, or neither library was
there to build against. A machine without PulseAudio or PipeWire falls back to
an ALSA loopback device, which `--audio-backend alsa` also picks outright:

```shell
sudo modprobe snd-aloop
```

The audio is written to `plughw:CARD=Loopback,DEV=0`, and programs record it
from `hw:Loopback,1,0`.

## BELABOX

[BELABOX](https://belabox.net) has none of this set up, and no Rust toolchain
either. Run the script below on it, over `ssh`, and it does all of the above:
installs the build dependencies, the toolchain and the `v4l2loopback` module,
builds `snd-aloop` for the BELABOX kernel, which does not ship it, builds and
installs the binary, and runs it as a service:

```shell
curl -fsSL https://raw.githubusercontent.com/eerimoq/mobcam/main/crates/virtualcam/belabox/install.sh | bash
```

It is the same script as `crates/virtualcam/belabox/install.sh` in a clone, and
running it there builds that clone instead of a fresh one.

The camera it creates is `/dev/mobcam`, labelled `Mobcam`, and the microphone
is the `Mobcam` sound card, which belaUI lists among the audio sources and
belacoder reads with `alsasrc device="hw:Mobcam"`. Add a belacoder pipeline
reading from `/dev/mobcam` to stream the camera. Pass `--no-audio` or
`--no-service` to leave either out, and `--help` to see the rest.
