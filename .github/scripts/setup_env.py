#!/usr/bin/env python3

"""Give the runner what it needs before cargo is called.

What has to be installed differs per platform; the outputs the workflow names
its artifacts after do not, and come straight out of buildspec.json.
"""

import gha

# The Xcode the plugin is built with. The runner image ships several, and the
# default is not necessarily the one the deployment target expects.
XCODE = "/Applications/Xcode_26.6.app/Contents/Developer"

# libobs and FFmpeg come from the distribution on Ubuntu, which is what OBS
# itself is built against there. bindgen needs libclang.
UBUNTU_PACKAGES = [
    "libclang-dev",
    "libobs-dev",
    "libavcodec-dev",
    "libavutil-dev",
    "libsimde-dev",
    "pkg-config",
]


def setup_macos():
    gha.run(["sudo", "xcode-select", "--switch", XCODE])


def setup_linux():
    gha.run(["sudo", "add-apt-repository", "--yes", "ppa:obsproject/obs-studio"])
    gha.run(["sudo", "apt-get", "--quiet", "update"])
    gha.run(
        ["sudo", "apt-get", "--quiet", "--yes", "--no-install-recommends", "install"]
        + UBUNTU_PACKAGES
    )


def setup_windows():
    # build.py downloads everything the Windows build needs itself.
    pass


def setup_env():
    {
        "macos": setup_macos,
        "linux": setup_linux,
        "windows": setup_windows,
    }[gha.host_platform()]()

    spec = gha.buildspec()

    gha.output("pluginName", spec["name"])
    gha.output("pluginVersion", spec["version"])


if __name__ == "__main__":
    gha.main(setup_env)
