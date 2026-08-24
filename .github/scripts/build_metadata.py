#!/usr/bin/env python3

"""The values more than one job needs, worked out once.

Whether to sign, notarize and package is not among them: every build does all
three, as far as the credentials the runner was given allow.
"""

import os

import gha


def build_metadata():
    spec = gha.buildspec()

    gha.output("commitHash", os.environ["GITHUB_SHA"][:9])
    gha.output("pluginName", spec.get("displayName", spec["name"]))


if __name__ == "__main__":
    gha.main(build_metadata)
