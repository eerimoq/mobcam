# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Mobcam turns an iPhone or iPad running [Moblin](https://github.com/eerimoq/moblin) into a low latency
camera on a computer, over USB. Moblin streams to `mobcam://localhost:7790`; the host reaches the
device through `usbmuxd` (Apple Mobile Device Service on Windows), decodes with FFmpeg and hands the
frames to a consumer. Two consumers ship from this repository:

| Crate | Product |
| --- | --- |
| `crates/obs-plugin` | A `Mobcam` source type in OBS Studio. macOS, Windows, Linux. `cdylib`. |
| `crates/virtualcam` | `mobcam-virtualcam`, a v4l2loopback camera plus a virtual microphone. Linux only. Binary. |
| `crates/core` | Everything both share: usbmux transport, wire protocol, FFmpeg decoding. |

## Commands

Build needs a [rustup](https://rustup.rs) toolchain, not a distro cargo: it is the one that honours
`rust-toolchain.toml` and the only one that can build the second arch of the macOS universal binary.

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin   # macOS only
make build       # scripts/build.py deps && scripts/build.py build
make install     # macOS/Linux only; installs into the user's OBS plugin dir (+ ~/.local/bin)
make package     # release artifacts, with installers
make clean       # removes release/ and target/
```

Quality gates, all four of which CI runs (`scripts/ci.py style_and_lint`):

```shell
make style        # cargo fmt, isort, ruff format
make style-check
make lint         # clippy --deny warnings, ruff check, mypy --strict
make test         # cargo test --workspace
make spell-check  # codespell
```

Every tool is invoked with an explicit config out of `.config/`. Plain `cargo fmt` uses the wrong
width — always `cargo fmt -- --config-path .config/rustfmt.toml`, or just `make style`.

A single test: `cargo test -p mobcam-virtualcam convert::tests::every_other_row_and_column_is_kept`. Tests live in
`crates/virtualcam` (`convert.rs`, `audio.rs`, `alsa.rs`); `mobcam-obs-plugin` sets `test = false`
because it is a bare `cdylib` against libobs.

Python tooling comes from `scripts/requirements.txt` (`.venv/` is already set up in a clone).

## Dependencies and code generation

`python scripts/build.py deps` downloads into `.deps/`, which is gitignored:

- `.deps/obs-studio` — libobs headers of the *oldest* supported OBS Studio (28.0.0), used on every
  platform including Linux, so the plugin loads in every newer OBS as well. Do not switch to the
  distribution's headers.
- `.deps/prebuilt` — obs-deps FFmpeg for macOS and Windows. Linux gets FFmpeg and libobs from
  `pkg-config` instead.

The macOS and Windows plugins ship those FFmpeg libraries rather than borrowing the ones OBS Studio
installed, so a struct layout never differs between the headers bindgen read and the library that
ends up loaded. On macOS `build_macos()` copies them into the bundle's `Contents/Frameworks` and
renames them to `mobcam-lib*.dylib`: dyld keys loaded images on the install name, and OBS Studio has
already loaded `@rpath/libavcodec.dylib` from its own `Frameworks` by the time a plugin is dlopened,
so a copy under the original name is silently ignored in favour of OBS Studio's. Each library is
signed before the bundle is, and `verify_macos()` fails the build if anything in it still needs an
`@rpath` library the bundle does not carry, links something outside `/usr/lib` and
`/System/Library`, or searches for libraries anywhere but inside the bundle. Belt and braces,
`obs_module_load()` calls `ffmpeg::version::check()` and refuses to register the source unless the
loaded libavcodec and libavutil have the major version the plugin was built against and are no
older than it — which is the only guard on Linux, where the libraries come from the distribution.

Each crate's `build.rs` runs bindgen at build time and writes `$OUT_DIR/{ffmpeg,obs}.rs`, included by
`ffmpeg/sys.rs` and `obs/sys.rs`. Generation is skipped when a `.stamp` file matches the flags,
header text and the OBS source hash, so touching an allowlist in `build.rs` regenerates and nothing
else does. On Windows there is no libobs import library, so `obs-plugin/build.rs` parses the
generated bindings into an `obs.def` and calls `lib.exe`.

`virtualcam/build.rs` probes `pkg-config` for `libpulse-simple` and `alsa` and emits `cfg(pulse)` /
`cfg(alsa)` — these are build-script cfgs, not Cargo features, so `#[cfg(pulse)]` code is only
compiled where the library was present at build time. A machine with neither builds a camera without
a microphone.

The workspace version in the root `Cargo.toml` is the single source of truth; `scripts/build.py`
reads it out with `tomllib` and it flows into every packaging template (`packaging/**/*.in`,
rendered by `render()`).

## Architecture

The pipeline is the same for both products:

```
usbmux::Stream  →  session::stream()  →  protocol::unpack_*  →  Decoder  →  Sink
```

- `core/usbmux.rs` speaks the plist protocol of `usbmuxd` (a unix socket on Unix, TCP 127.0.0.1:27015
  on Windows — see `usbmux/unix.rs` and `usbmux/windows.rs`), lists devices and connects a port. All
  reads take an `&dyn Abort` and poll at 100 ms so a stopping source or a `SIGINT` gets out.
- `core/protocol.rs` is the Moblin wire format: a 5 byte header (kind, big-endian length) then hello,
  video/audio config and frame messages. Pure `unpack_*` functions returning `Option`, borrowing from
  the caller's buffer.
- `core/session.rs` owns the message loop and defines `Handler: Sink` — the one trait a consumer
  implements: `hello()` for the device name, plus `Sink::video`/`Sink::audio` for decoded frames.
- `core/decoder.rs` holds one `Stream` for video and one for audio. `Codec::decoders_for()` returns
  every decoder for the codec id, sorted so hardware comes first when hardware decoding is on and
  last when it is off, and the decoder walks that list until one opens. Hardware frames are either
  mapped (only for devices where `maps_cheaply()`, currently `rkmpp` and `drm`) or downloaded, with
  the choice made once from the first frame and cached in `Access`.
- `core/ffmpeg/` is the safe wrapper over the generated `sys` bindings; nothing outside it should
  touch raw FFmpeg pointers.
- `core/logging.rs` is a `OnceLock<fn(Level, &str)>` set by each product (`obs::install_logger`,
  `write_log` in virtualcam), used through the `log!` macro.

**OBS plugin.** `obs/` wraps libobs the same way `ffmpeg/` wraps FFmpeg: `Source`, `Data`,
`Properties`, `Frame`/`Audio`. `source.rs` is the source type — a worker thread reconnects every
second, an `Output` implements `Sink` and pushes into libobs, and `Clock` re-anchors OBS timestamps
on any PTS jump over five seconds. Every `extern "C"` entry point goes through `panic::guard` so a
Rust panic never unwinds into C. UI strings are keys looked up via `obs::text()` against
`data/locale/en-US.ini` — add both when adding a setting.

**Virtual camera.** `camera.rs` is the main loop, `convert.rs` the pixel format conversions,
`audio.rs` the resampling and the backend choice. Frames are queued into v4l2loopback rather than
written to it, because a `write()` makes the kernel stamp the buffer with the time of the write,
while `VIDIOC_QBUF` carries a timestamp of the caller's choosing: `core/clock.rs` turns the device's
presentation timestamp into one on the monotonic clock, the same `Clock` the OBS plugin anchors OBS
timestamps with. The buffer index has to be a strict rotation, as the output `VIDIOC_DQBUF` of
v4l2loopback hands back the buffer that was queued last. Frames are also spaced out before they are
queued, to three quarters of the frame interval the timestamps carry: v4l2loopback fast-forwards a
reader that has fallen more than two frames behind, so a writer that empties a burst of frames
straight into the camera loses all but the last of them, whatever the buffer count.

The platform-specific modules follow one pattern: `v4l2.rs`, `pulse.rs` and `alsa.rs` hold the
portable types and `#[cfg_attr(..., path = "...")]` in either a `supported.rs` or an
`unsupported.rs`. The unsupported halves are uninhabited enums whose
`open()` returns an error, which is what keeps this Linux-only crate compiling on macOS — keep that
working when adding to any backend.

## CI and releases

`.github/workflows/all.yaml` runs style-and-lint on Ubuntu and builds on macOS, Ubuntu and Windows
through `scripts/ci.py`, which installs the OS packages, calls into `scripts/build.py` and, on macOS,
imports the signing certificate and notarizes. Pushing a `X.Y.Z` tag publishes a GitHub release from
the uploaded artifacts.

## Coding convertions

- Do not write comments or docstrings.