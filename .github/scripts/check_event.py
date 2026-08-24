#!/usr/bin/env python3

"""Decide what the rest of the build should do with this event.

A pull request is only signed and packaged once it asks for testers, and only a
version tag is worth the notarization round trip. Every other job reads these
outputs rather than looking at the event again.
"""

import json
import os
import re
import subprocess

import gha

# What a tag has to look like for the build it produces to be notarized.
VERSION_TAG = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(-(rc|beta).+)?")

# The label that turns a pull request into something people can install.
SEEKING_TESTERS = "Seeking Testers"


def labels(number):
    result = gha.run(
        ["gh", "pr", "view", str(number), "--json", "labels"],
        stdout=subprocess.PIPE,
        text=True,
    )

    return [label["name"] for label in json.loads(result.stdout)["labels"]]


def pull_request_number():
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as fin:
        return json.load(fin)["number"]


def configuration(event):
    if event == "pull_request":
        testers = SEEKING_TESTERS in labels(pull_request_number())

        return {"codesign": testers, "notarize": False, "package": testers}
    elif event == "push":
        tag = VERSION_TAG.fullmatch(os.environ["GITHUB_REF_NAME"])

        return {"codesign": True, "notarize": bool(tag), "package": True}
    elif event == "workflow_dispatch":
        return {"codesign": True, "notarize": False, "package": False}
    elif event == "schedule":
        return {"codesign": True, "notarize": False, "package": True}
    else:
        return {}


def check_event():
    for key, value in configuration(os.environ["GITHUB_EVENT_NAME"]).items():
        gha.output(key, gha.boolean(value))

    spec = gha.buildspec()

    gha.output("commitHash", os.environ["GITHUB_SHA"][:9])
    gha.output("pluginName", spec.get("displayName", spec["name"]))


if __name__ == "__main__":
    gha.main(check_event)
