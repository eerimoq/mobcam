# System tests

The tests stream from an iPhone or iPad running Moblin to Mobcam over USB and validate what comes out
the other end. They run on the machine the device is connected to and drive Moblin over its remote
control, which the device reaches over the network.

There is one runner per product. All commands below are run from the root of a clone of the
repository. Each runner writes its log to `logs/obs.log` or `logs/virtualcam.log` and its recordings
to `tests/files/`, which is emptied when a runner starts.

# Configuration

Copy `tests/config.example.toml` to `tests/config.toml` and set `tester-ip-address` to the address of
the machine the tests run on, the one the device connects back to.

# Moblin device configuration

1. Generate the settings.
   ```bash
   just test-generate-device-settings-clipboard
   ```
2. Import them into Moblin, on the device.
3. Connect the device to the machine with a USB cable, unlock it and tap Trust.

# OBS plugin

Records a scene with a [Mobcam source](../crates/obs-plugin) into an MP4 file with OBS Studio and
checks that OBS never had to render the same frame twice, which is what a source that stalls or
arrives late looks like once OBS has re-timed everything it writes.

```
 iPhone (Moblin)                     macOS machine (OBS Studio)
      |                                          |
      |  mobcam://localhost:7790 over USB ------>|  the Mobcam source in a scene
      |                                          |         |
      |<----- remote control over the network ---|      OBS Studio records it
```

macOS only. OBS Studio must be in `/Applications/OBS.app` with the plugin installed
(`just install`), must have been started once so it has a configuration, and must not be running when
the tests start. The tests create and use their own `MobcamTest` profile and scene collection and
leave the ones already there alone, and enable the obs-websocket server while they run.

Point the device at something that moves. A still scene gives a recording of identical frames, which
is exactly what the duplicated frame check looks for, and every test fails.

```bash
just test-obs-plugin
```

# Virtual camera

Records the camera and the microphone [Mobcam Virtual Camera](../crates/virtualcam) creates into an
MP4 file with FFmpeg and checks the recorded frames are spaced by the frame interval the device
streams at, which is what the timestamps the camera is fed carry.

```
 iPhone (Moblin)                     Linux machine (mobcam-virtualcam)
      |                                          |
      |  mobcam://localhost:7790 over USB ------>|  /dev/mobcam and the Mobcam sound card
      |                                          |         |
      |<----- remote control over the network ---|      ffmpeg records both
```

Linux only, on a machine set up by [install.sh](../scripts/belabox/install.sh). The tests start their
own `mobcam-virtualcam`, so the service must not be running.

```bash
just test-virtualcam
```

Or on a BELABOX over SSH (user@belabox.local, no password), which builds, stops the service and
copies the logs and the recordings back when it is done.

```bash
sudo apt install rsync
just test-virtualcam-belabox
```

## BELABOX tips and tricks

### Passwordless ssh

```bash
ssh-copy-id user@belabox.local
```

### Permanently enable ssh

```bash
sudo systemctl enable ssh
```

### Passwordless sudo

```bash
sudo visudo
```

and add this line at the end of the file

```bash
user ALL=(ALL) NOPASSWD: ALL
```
