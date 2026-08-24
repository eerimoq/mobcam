#!/usr/bin/env python3

"""Put the Apple credentials the runner was given where the signing tools look
for them.

The certificate goes into a keychain made for this run alone. Credentials that
are absent simply turn the part of the build that needs them off, which is what
lets a fork build the plugin without any of them.
"""

import base64
import os
import subprocess
from pathlib import Path

import gha

# Long enough for a build, short enough that a leaked keychain is worthless.
KEYCHAIN_TIMEOUT = "21600"

# The tools that are allowed to use the imported key without prompting.
KEYCHAIN_TOOLS = ["/usr/bin/codesign", "/usr/bin/security", "/usr/bin/xcrun"]


def secret(name):
    return os.environ.get(name, "")


def team(identity):
    """The team id Apple puts at the end of a signing identity, as in
    'Developer ID Application: Someone (TEAMID)'."""

    return identity.rsplit(" ", 1)[-1].replace("(", "").replace(")", "")


def import_certificate(temporary, password):
    certificate = temporary / "build_certificate.p12"
    certificate.write_bytes(base64.b64decode(secret("MACOS_SIGNING_CERT")))

    keychain = temporary / "app-signing.keychain-db"

    with gha.group("Keychain setup"):
        gha.run(["security", "create-keychain", "-p", password, keychain], quiet=True)
        gha.run(["security", "set-keychain-settings", "-lut", KEYCHAIN_TIMEOUT, keychain])
        gha.run(["security", "unlock-keychain", "-p", password, keychain], quiet=True)

        arguments = []

        for tool in KEYCHAIN_TOOLS:
            arguments += ["-T", tool]

        gha.run(
            [
                "security",
                "import",
                certificate,
                "-P",
                secret("MACOS_SIGNING_CERT_PASSWORD"),
                "-A",
                "-t",
                "cert",
                "-f",
                "pkcs12",
                "-k",
                keychain,
            ]
            + arguments,
            quiet=True,
        )

        gha.run(
            ["security", "set-key-partition-list", "-S", "apple-tool:,apple:",
             "-k", password, keychain],
            quiet=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        gha.run(["security", "list-keychain", "-d", "user", "-s", keychain, "login-keychain"])


def setup_codesigning():
    identity = secret("MACOS_SIGNING_IDENTITY")
    installer = secret("MACOS_SIGNING_IDENTITY_INSTALLER")

    if not (identity and installer and secret("MACOS_SIGNING_CERT")):
        gha.output("haveCodesignIdent", "false")
        gha.output("haveNotarizationUser", "false")
        return

    # The keychain lives and dies with this run, so nothing outside it needs to
    # know the password.
    import_certificate(Path(os.environ["RUNNER_TEMP"]), os.urandom(16).hex())

    gha.output("haveCodesignIdent", "true")
    gha.output("codesignIdent", identity)
    gha.output("installerIdent", installer)
    gha.output("codesignTeam", team(identity))

    notarization = secret("MACOS_NOTARIZATION_USERNAME") and secret("MACOS_NOTARIZATION_PASSWORD")
    gha.output("haveNotarizationUser", gha.boolean(notarization))


if __name__ == "__main__":
    gha.main(setup_codesigning)
