#!/usr/bin/env python3

"""Archive the built plugin, and build the installer when the event asks for
one.

Signing and notarization are each turned off unless both the event wants them
and the runner has the credentials for them, which is what lets a fork package
a plugin at all.
"""

import gha


def package_plugin():
    arguments = []

    if gha.flag("PACKAGE"):
        arguments.append("--installer")

    if gha.flag("HAVE_CODESIGN_IDENT"):
        arguments.append("--codesign")

    if gha.flag("NOTARIZE") and gha.flag("HAVE_NOTARIZATION_USER"):
        arguments.append("--notarize")

    gha.python("package", *arguments)


if __name__ == "__main__":
    gha.main(package_plugin)
