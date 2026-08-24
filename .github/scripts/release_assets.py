#!/usr/bin/env python3

"""Turn the downloaded build artifacts into the files a release is made of.

Every artifact arrives in a directory of its own; the release wants the files
themselves side by side, and a checksum for each of them to use as its body.
"""

import hashlib
import os
import shutil
from pathlib import Path

import gha

# The suffixes worth releasing, per artifact the build uploads. Anything else
# in an artifact directory, debug symbols above all, stays where it is.
VARIANTS = {
    "windows-x64": ["zip", "exe"],
    "macos-universal": ["tar.xz", "pkg"],
    "ubuntu-24.04-x86_64": ["tar.xz", "deb"],
    "sources": ["tar.xz"],
}

CHECKSUMS = "CHECKSUMS.txt"


def collect(root, commit_hash):
    for variant, suffixes in VARIANTS.items():
        for directory in sorted(root.glob(f"*-{variant}-{commit_hash}")):
            for suffix in suffixes:
                for path in sorted(directory.glob(f"*.{suffix}")):
                    print(f"    {path.relative_to(root)}")
                    shutil.move(path, root / path.name)


def checksums(root):
    suffixes = {suffix for suffixes in VARIANTS.values() for suffix in suffixes}
    lines = ["### Checksums"]

    for path in sorted(root.iterdir()):
        if not any(path.name.endswith(f".{suffix}") for suffix in suffixes):
            continue

        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        lines.append(f"    {path.name}: {digest}")

    (root / CHECKSUMS).write_text("\n".join(lines) + "\n", encoding="utf-8")


def release_assets():
    root = Path(os.environ["GITHUB_WORKSPACE"])

    collect(root, os.environ["GITHUB_SHA"][:9])
    checksums(root)

    print((root / CHECKSUMS).read_text(encoding="utf-8"))


if __name__ == "__main__":
    gha.main(release_assets)
