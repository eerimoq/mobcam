#!/usr/bin/env python3

"""Build the plugin, signed if the runner was given an identity to sign it
with."""

import gha


def build_plugin():
    arguments = []

    if gha.flag("HAVE_CODESIGN_IDENT"):
        arguments.append("--codesign")

    gha.python("build", *arguments)


if __name__ == "__main__":
    gha.main(build_plugin)
