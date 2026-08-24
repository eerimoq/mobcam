#!/usr/bin/env python3

"""Check that the macOS plugin really is universal.

A cargo build only ever produces one architecture, so a slip in the build would
ship an arm64 only plugin that Intel users cannot load. Nothing else in the
build would notice.
"""

import subprocess

import gha

ARCHITECTURES = ["arm64", "x86_64"]


def verify_universal_binary():
    name = gha.buildspec()["name"]
    binary = gha.ROOT / "release" / "install" / f"{name}.plugin" / "Contents" / "MacOS" / name

    result = gha.run(["lipo", "-archs", binary], stdout=subprocess.PIPE, text=True)
    archs = result.stdout.split()

    print(f"{name} architectures: {' '.join(archs)}")

    missing = [arch for arch in ARCHITECTURES if arch not in archs]

    if missing:
        raise gha.Error(f"{name} is missing the {' and '.join(missing)} slice")


if __name__ == "__main__":
    gha.main(verify_universal_binary)
