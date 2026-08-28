# System tests

The tests stream from an iPhone or iPad running Moblin to
[Mobcam Virtual Camera](../crates/virtualcam), record the camera and the microphone it creates into
an MP4 file with FFmpeg, and validate the recording.

They run on the Linux machine the device is connected to with a USB cable and drive Moblin 
over its remote control. All commands below are run from the root of a clone of the repository.

```
 iPhone (Moblin)                     Linux machine (mobcam-virtualcam)
      |                                          |
      |  mobcam://localhost:7790 over USB ------>|  /dev/mobcam and the Mobcam sound card
      |                                          |         |
      |<----- remote control over the network ---|      ffmpeg records both
```

# Moblin device configuration

1. Generate the settings.
   ```bash
   make test-generate-device-settings-clipboard
   ```
2. Import them into Moblin, on the device.
3. Connect the device to the machine with a USB cable, unlock it and tap Trust.

# Run the tests

```bash
make test TEST_ARGS="--device iphone16pro"
```

Or test on a BELABOX over SSH (user@belabox.local, no password).

```bash
sudo apt install rsync
make test-belabox TEST_ARGS="--device iphone16pro"
```
