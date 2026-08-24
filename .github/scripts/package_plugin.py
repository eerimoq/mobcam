#!/usr/bin/env python3

"""Archive the built plugin and build the installer.

Every build is meant to be signed and notarized; each is only turned off when
the runner was given no credentials for it, which is what lets a fork package a
plugin at all.
"""

import gha


def package_plugin():
    arguments = ["--installer"]

    if gha.flag("HAVE_CODESIGN_IDENT"):
        arguments.append("--codesign")

    if gha.flag("HAVE_NOTARIZATION_USER"):
        arguments.append("--notarize")

    gha.python("package", *arguments)


if __name__ == "__main__":
    gha.main(package_plugin)
