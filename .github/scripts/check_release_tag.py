#!/usr/bin/env python3

"""Decide whether the pushed tag makes a release.

Only a plain version tag does; anything else pushed as a tag is left alone, and
the rest of the release job is skipped.
"""

import os
import re

import gha

RELEASE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
PRERELEASE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+-(beta|rc)[0-9]*")


def check_release_tag():
    tag = os.environ["GITHUB_REF_NAME"]

    if RELEASE.fullmatch(tag):
        prerelease = False
    elif PRERELEASE.fullmatch(tag):
        prerelease = True
    else:
        gha.output("validTag", "false")
        return

    gha.output("validTag", "true")
    gha.output("prerelease", gha.boolean(prerelease))
    gha.output("version", tag)


if __name__ == "__main__":
    gha.main(check_release_tag)
