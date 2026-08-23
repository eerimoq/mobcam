# OBS MobCam Plugin

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a low
latency camera in OBS Studio, over the USB cable.

The phone encodes video and audio and Moblin serves them on a port inside the
device. This plugin reaches that port over usbmux, the same channel Xcode and
`iproxy` use, decodes the streams and hands them to OBS as a source. Nothing
goes over the network, and no separate application has to run alongside OBS.

## Requirements

- OBS Studio 32.2 or newer.
- Moblin, with the stream URL set to `usb://localhost:7777` (the create stream
  wizard has a USB entry under Custom that fills this in).
- Moblin's audio codec set to AAC. It is the only one this transport carries,
  and Moblin sends video alone when it is set to anything else.
- macOS ships usbmuxd, so nothing extra is needed there. Windows needs the Apple
  Devices app or iTunes, which installs the Apple Mobile Device Service. Linux
  needs the `usbmuxd` package.
- A phone that trusts this computer. Plug it in and answer the "Trust This
  Computer?" prompt once.

## Usage

1. Add a **MobCam** source to a scene.
2. Pick the phone from the Device list, or leave it on automatic to take the
   first one attached over USB.
3. Go live in Moblin. Video appears within about a second, and the source shows
   up in the Audio Mixer with the phone's audio.

The plugin keeps trying to connect once a second, so it does not matter whether
OBS or Moblin starts first, and unplugging and replugging the cable recovers on
its own.

### Settings

| Setting | What it does |
|---|---|
| Device | Which phone to connect to, by serial number. Automatic takes the first one attached. |
| Port | The port Moblin listens on, from its stream URL. 7777 unless you changed it. |
| Buffering | Off, the default, shows each frame as it arrives, for the lowest latency. On lets OBS buffer, which is smoother over an uneven feed and lines audio up with video exactly. |
| Show Nothing When Disconnected | Clears the source when the stream ends, instead of leaving the last frame on screen. |
| Disconnect When Not Visible | Drops the connection while the source is hidden. Moblin only encodes while a computer is connected, so this saves phone battery at the cost of a reconnect when the source comes back. |

Video and audio carry capture timestamps from the same clock on the phone, so
they are already in sync when they arrive. With Buffering off OBS shows each
video frame the moment it arrives and pulls the audio along with it, which can
let lip sync wander slightly; turn Buffering on to have OBS line the two up from
the timestamps instead. If the phone's audio is not wanted, mute the source in
the Audio Mixer.

Moblin accepts one computer at a time: a second connection takes over from the
first. Pointing two MobCam sources at the same phone will make them fight over
it.

## Build and install on macOS

```shell
cmake --preset macos
cmake --build build_macos --config Release
cmake --install build_macos --prefix dist
open dist/mobcam.pkg
```

The plugin links the FFmpeg that OBS itself ships, so it has to be built against
the OBS release it runs on. `buildspec.json` pins that release; raising it there
is what moves the plugin to a newer OBS.

## Protocol

The wire protocol is documented in
[moblin/docs/usb-protocol.md](https://github.com/eerimoq/moblin/blob/main/docs/usb-protocol.md),
and `moblin/utils/usb_host.py` is a reference receiver that plays the same
stream with `ffplay`. It is a good first thing to try when the source stays
black: if `usb_host.py` shows nothing either, the problem is not in OBS.
