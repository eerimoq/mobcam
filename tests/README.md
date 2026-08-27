# System tests

The tests stream from an iPhone or iPad running Moblin to
[Mobcam Virtual Camera](../crates/virtualcam), record the camera and the microphone it creates with
FFmpeg and validate the recording.

They run on the Linux machine the device is connected to with a USB cable, a
[BELABOX](https://belabox.net) in particular, and drive Moblin over its remote control. All commands
below are run from the root of a clone of the repository.

```
 iPhone (Moblin)                     Linux machine (mobcam-virtualcam)
      |                                          |
      |  mobcam://localhost:7790 over USB ------>|  /dev/mobcam and the Mobcam sound card
      |                                          |         |
      |<----- remote control over the network ---|      ffmpeg records both
```

# Prerequisites

Mobcam Virtual Camera has to be installed on the machine already, which
[crates/virtualcam/belabox/install.sh](../crates/virtualcam/belabox/install.sh) does on a BELABOX.
The tests need `sudo` without a password, as they stop the `mobcam-virtualcam` service and run the
binary themselves.

Set the rest up with

```bash
./tests/belabox/setup.sh
```

It installs FFmpeg and the Python the tests run on, puts the user in the `video` and `audio` groups,
which is what makes the camera and the microphone readable, and creates `.venv`. Log out and in
again afterwards for the new groups to take.

# Configuration

`tests/belabox/setup.sh` copies `tests/config.example.toml` to `tests/config.toml`. Edit it to match
the machine. `tests/config.toml` is used if it exists, otherwise
`$XDG_CONFIG_HOME/mobcam/tests/config.toml`.

`tester-ip-address` is the address of the machine itself, the one Moblin's remote control connects
back to. The `[virtualcam]` section is the binary, the service and the devices it is started with;
`audio-playback-device` is what `mobcam-virtualcam` plays into and `audio-capture-device` the other
half of the same loopback, which the tests record from.

# Moblin device configuration

1. Generate the settings.
   ```bash
   make test-generate-device-settings-stdout
   ```
2. Import them into Moblin, on the device.
3. Connect the device to the machine with a USB cable, unlock it and tap Trust.

Each test imports the stream it needs on top of these settings, so the stream URL, the codec, the
resolution and the frame rate do not have to be set by hand.

# Run the tests

On the machine:

```bash
make test TEST_ARGS="--device iphone16pro"
make test TEST_ARGS="--device iphone16pro StreamH265-1920x1080@60"
```

From another machine, which copies the working tree over and runs the tests there:

```bash
make test-remote TEST_ARGS="--device iphone16pro"
make test-remote BELABOX=user@belabox.local TEST_ARGS="--device iphone16pro"
```

Everything a run captured is left in `tests/files`, one WAV and one JPEG per second per test, and
the log in `logs/test.log`. `make test-remote` copies both back from the machine.

# What is validated

Every test streams for ten seconds and then checks that

- the camera delivers I420 or NV12 frames of the resolution Moblin is streaming,
- the frames keep coming at close to the frame rate Moblin is streaming, in every half second of the
  ten,
- the microphone delivers the sample rate and the channel count `mobcam-virtualcam` logged, for the
  whole ten seconds,
- the audio is neither silent nor interrupted, and the video is not black, and
- `mobcam-virtualcam` logged no errors or warnings.

The frame rate is measured by a capture that only counts frames, never encodes: encoding 1080p60 in
software costs the BELABOX around eight frames a second, which is more than the rate being measured
can afford. A second capture, running at the same time, writes the audio and one JPEG a second,
which is what the silence and the black checks read.

The frame rate is only checked to be in the neighbourhood of the one Moblin streams, never to be
exactly it. The camera of the device delivers fewer frames than it is asked for in dim light, 55 of
60 in a normally lit room, and how many is nothing the tests can control. A regression that drops
every other frame is still caught.

# Troubleshooting

`mobcam-virtualcam stopped: error: /dev/mobcam cannot be written to as a camera` means the
v4l2loopback device is held by something else, or was left behind by a program that was killed while
writing to it. Reload the module:

```bash
sudo systemctl stop mobcam-virtualcam
sudo modprobe -r v4l2loopback
sudo modprobe v4l2loopback
sudo systemctl start mobcam-virtualcam
```
