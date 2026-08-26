# Mobcam virtual camera

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a
low latency camera and microphone in every program that can use one, over USB,
without OBS Studio.

It reads the same video and audio over the same USB cable as the
[Mobcam OBS plugin](../obs-plugin/), but writes them to a
[v4l2loopback](https://github.com/umlaeute/v4l2loopback) device and a virtual
microphone instead of into OBS Studio, so every program that can use a camera
can use the iPhone or iPad, whether or not OBS Studio is running.

Linux only.

## Requirements

- Moblin, with the stream URL set to `mobcam://localhost:7790`.
- Moblin's audio codec set to AAC.
- `usbmuxd`, to talk to the iPhone or iPad over the USB cable.
- The `v4l2loopback` kernel module.
- PulseAudio, PipeWire or ALSA, for the microphone. The video keeps going
  without any of them.

## Install

Every release is on the
[releases page](https://github.com/eerimoq/mobcam/releases). On Debian and
Ubuntu, download `mobcam-virtualcam-<version>-x86_64-linux-gnu.deb` and install
it:

```shell
sudo apt install ./mobcam-virtualcam-<version>-x86_64-linux-gnu.deb
```

On other distributions, download
`mobcam-virtualcam-<version>-x86_64-linux-gnu.tar.xz` and unpack it into `/usr`:

```shell
sudo tar -xf mobcam-virtualcam-<version>-x86_64-linux-gnu.tar.xz -C /usr
```

Install `usbmuxd` as well, connect the device once, unlock it and tap Trust:

```shell
sudo apt install usbmuxd
```

Uninstall the package with `sudo apt remove mobcam-virtualcam`, or, for the
tarball, remove `/usr/bin/mobcam-virtualcam`.

Earlier releases shipped the virtual camera and the OBS Studio plugin together
in one `mobcam` package. Installing this one replaces it.

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
pactl load-module module-null-sink sink_name=Mobcam \
    sink_properties=device.description=Mobcam
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
Stop it with Ctrl-C.

`mobcam-virtualcam --list` prints the attached iPhones and iPads, the
v4l2loopback devices and the virtual microphones, and `--help` the rest of the
options, `--device`, `--udid` and `--audio-device` among them to pick which of
each to use.

The video keeps going on its own if the audio cannot be played: nothing is set
up, the sink was never created, or `--no-audio` was passed. A machine without
PulseAudio or PipeWire falls back to an ALSA loopback device, which
`--audio-backend alsa` also picks outright:

```shell
sudo modprobe snd-aloop
```

The audio is written to `plughw:CARD=Loopback,DEV=0`, and programs record it
from `hw:Loopback,1,0`.

## Development

See the [development section](../README.md#development) of the repository.
