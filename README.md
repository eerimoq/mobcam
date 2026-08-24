# OBS Mobcam Plugin

Use an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) as a low
latency camera in OBS Studio over USB.

## Requirements

- OBS Studio 32.2 or newer.
- Moblin, with the stream URL set to `mobcam://localhost:7790`.
- Moblin's audio codec set to AAC.

The iPhone or iPad is connected over a USB cable, so the computer must be able
to talk to it. Each operating system needs something different for that; see the
install section below.

## Install

Every release is on the [releases page](https://github.com/eerimoq/obs-mobcam-plugin/releases). 
Download the file for your operating system and follow the steps below. Quit OBS Studio before
installing and start it again afterwards.

### Windows

The Apple Mobile Device Service is needed to talk to the iPhone or iPad. Both
the Apple Devices app from the Microsoft Store and iTunes install it, so install
either one. Connect the device once, unlock it and tap Trust.

Then install the plugin:

1. Download `mobcam-<version>-windows-x64-Installer.exe`.
2. Run it and accept the elevation prompt by clicking "More Info" and then "Run
   Anyway".
3. Select the folder OBS Studio is installed in, normally
   `C:\Program Files\obs-studio`. The installer suggests the folder it finds and
   warns if the selected one does not hold `bin\64bit\obs64.exe`. The plugin is
   installed into `obs-plugins\64bit` and `data\obs-plugins\mobcam` in that
   folder.

To install by hand instead, download `mobcam-<version>-windows-x64.zip` and
unpack it into `C:\ProgramData\obs-studio\plugins`, so that the plugin ends up
in `C:\ProgramData\obs-studio\plugins\mobcam\bin\64bit`.

Uninstall the plugin from Settings, Apps, Installed apps.

### Linux

`usbmuxd` is needed to talk to the iPhone or iPad. On Debian and Ubuntu:

```shell
sudo apt install usbmuxd
```

Then install the plugin. On Debian and Ubuntu, download
`mobcam-<version>-x86_64-linux-gnu.deb` and install it:

```shell
sudo apt install ./mobcam-<version>-x86_64-linux-gnu.deb
```

On other distributions, download `mobcam-<version>-x86_64-linux-gnu.tar.xz` and
unpack it into `/usr`:

```shell
sudo tar -xf mobcam-<version>-x86_64-linux-gnu.tar.xz -C /usr
```

Connect the device once, unlock it and tap Trust.

Both ways install the plugin for the distribution's OBS Studio. An OBS Studio
installed as a Flatpak or a Snap looks for its plugins inside its own sandbox
and will not find it.

Uninstall the package with `sudo apt remove mobcam`, or, for the tarball, remove
`/usr/lib/x86_64-linux-gnu/obs-plugins/mobcam.so` and
`/usr/share/obs/obs-plugins/mobcam`.

### macOS

macOS 12 or newer. Nothing extra has to be installed to talk to the iPhone or
iPad.

1. Download `mobcam-<version>-macos-universal.pkg`.
2. Open it and follow the installer. The plugin is signed and notarized, and is
   installed into `~/Library/Application Support/obs-studio/plugins` for the
   current user.

The first time the device is connected, unlock it and tap Trust.

Uninstall the plugin by removing
`~/Library/Application Support/obs-studio/plugins/mobcam.plugin`.

### Verifying the install

Start OBS Studio and add a source. The plugin loaded if `Mobcam` is in the list
of source types.

## Development

The plugin is written in Rust. cargo compiles and links it, and `build.py`
does everything around that: the dependencies, the macOS bundle, the code
signing and the installers.

A [rustup](https://rustup.rs) toolchain is needed rather than any other cargo,
since it is the one that honours `rust-toolchain.toml` and the only one that can
build the second architecture of the macOS universal binary. bindgen generates
the libobs and FFmpeg bindings at build time and needs libclang: from Xcode on
macOS, `libclang-dev` on Ubuntu, and LLVM on Windows.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
python3 build.py build
python3 build.py install
```

`build.py build` downloads the dependencies it names into `.deps` on macOS and
Windows, builds the plugin and stages it under `release/install`. `build.py
install` copies it into the OBS plugin directory of the current user. The
archives and the installers the releases are made of come from `build.py
package --installer`.

On Linux there is nothing to download: libobs and FFmpeg come from the
distribution, which needs `libobs-dev`, `libavcodec-dev`, `libavutil-dev`,
`libsimde-dev` and `pkg-config` installed.

The dependencies are all `build.py` needs, so cargo can be run directly too:

```shell
cargo build
cargo clippy --all-targets
cargo fmt
```
