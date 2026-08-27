# System tests

The tests stream from an iPhone or iPad running Moblin to
[Mobcam Virtual Camera](../crates/virtualcam), record the camera and the microphone it creates into
an MP4 file with FFmpeg, and validate the recording.

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
That is also what builds the FFmpeg in `/opt/mobcam/ffmpeg/bin`, the one that encodes video in the
RK3588 hardware and reads ALSA, which the tests record the camera and the microphone with, and count
the frames it repeated with. It also hands `/dev/mpp_service` to the `video` group, which the
machine ships to root alone; without that nothing but root encodes in the hardware. The FFmpeg of the machine, the one in the path, reads everything that
came out. The tests need `sudo` without a password, as they stop the `mobcam-virtualcam` service and
run the binary themselves.

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

Everything a run captured is left in `tests/files`, one MP4 and the WAV taken out of it per test,
and the log in `logs/test.log`. `make test-remote` copies both back from the machine.

# What is validated

Every test streams for ten seconds and then checks that

- the camera delivers I420 or NV12 frames of the resolution Moblin is streaming,
- the frames keep coming at close to the frame rate Moblin is streaming, in every half second of the
  ten,
- the MP4 the camera and the microphone were recorded into holds H.264 of that resolution and AAC
  of the sample rate `mobcam-virtualcam` logged, ten seconds of both,
- the microphone delivers the sample rate `mobcam-virtualcam` logged and at least as many channels
  as it plays, for the whole ten seconds,
- the audio is neither silent nor interrupted, and
- `mobcam-virtualcam` logged no errors or warnings.

The frame rate is measured from the timestamps of the recorded frames, which the capture stamps with
the wall clock as it reads them, and not from what `ffmpeg` reports while it runs: the hardware
encoder hands packets back in bursts, so its progress counter reads 0 frames in one half second and
60 in the next while the camera is delivering 30 a second the whole way through. The timestamps come
off the recording with `ffprobe`, and the recording is encoded with an explicit `-enc_time_base:v
1/90000` so they survive the trip: left to itself `ffmpeg` gives the encoder the frame rate of the
camera as its time base, which rounds every wall clock timestamp to the nearest 1/30 of a second
and turns a steady 30 frames a second into pairs sharing a timestamp separated by gaps of two
frame times.

Recording costs close to nothing because `h264_rkmpp` encodes in the video unit of the RK3588.
Encoding 1080p60 in software costs the BELABOX around eight frames a second, which is more than the
rate being measured can afford, so the FFmpeg of the machine is not the one that records. A second
capture, running at the same time, counts the frames that are not repeats of the one before with
`mpdecimate` and encodes nothing at all. The audio the silence and the volume checks read is a WAV
taken out of the MP4 once the recording is over, as ALSA hands the loopback to one reader at a time
and the recording is the one holding it.

The microphone is the capture half of an `snd-aloop` loopback, which hands a mono stream over as
stereo, so the channel count is only checked to be at least the one `mobcam-virtualcam` plays and to
survive the trip from ALSA into the MP4 unchanged.

`mpdecimate` runs with far tighter thresholds than its defaults, `hi=64:lo=32:frac=0.01`, and on the
frames the camera delivers rather than on the recording. A frame the camera repeated is the same
buffer read twice, identical to the byte, which any threshold catches, while the defaults are loose
enough to call a still scene one long duplicate: pointed at a grey wall, a camera whose picture only
moves by its own noise keeps 1 frame of 90 with the defaults and all 90 with these. Encoding the
frames first would blur the distinction the other way, as a duplicate comes back out of H.264
slightly changed.

The distinct frame rate is logged and nothing else, for now. A device pointed at a scene that does
not move sends a picture that does not change, which arrives as frame after frame the decoder gives
back byte for byte the same, so the count says as much about what the camera was pointed at as about
Mobcam. It is there to be read, and to be turned into a check of a camera that froze once the tests
stream something that moves.

The frame rate is only checked to be in the neighbourhood of the one Moblin streams, never to be
exactly it. The camera of the device delivers fewer frames than it is asked for in dim light, 55 of
60 in a normally lit room, and how many is nothing the tests can control. A regression that drops
every other frame is still caught.

# Troubleshooting

`mobcam-virtualcam stopped: error: /dev/mobcam cannot be written to as a camera` means the
v4l2loopback device is held by something else, or was left behind by a program that was killed while
writing to it. Every test reloads the module when it is done, so a run leaves the camera in a state
`mobcam-virtualcam.service` can take, and a test that finds it stuck anyway reloads it and tries
again, twice, before giving up: that is the `reloading v4l2loopback and trying again` warning in the
log. Reload it by hand with:

```bash
sudo systemctl stop mobcam-virtualcam
sudo modprobe -r v4l2loopback
sudo modprobe v4l2loopback
sudo systemctl start mobcam-virtualcam
```
