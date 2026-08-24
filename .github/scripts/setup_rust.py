#!/usr/bin/env python3

"""Install the toolchain and every target this platform's plugin is built for."""

import gha

# macOS ships a universal plugin, so both slices have to be buildable.
TARGETS = {
    "macos": ["aarch64-apple-darwin", "x86_64-apple-darwin"],
    "windows": ["x86_64-pc-windows-msvc"],
    "linux": [],
}


def setup_rust():
    # rust-toolchain.toml decides the toolchain; the targets are added to
    # whichever one that is, which is the one cargo will use here.
    gha.run(["rustup", "show", "active-toolchain"])

    targets = TARGETS[gha.host_platform()]

    if targets:
        gha.run(["rustup", "target", "add"] + targets)


if __name__ == "__main__":
    gha.main(setup_rust)
