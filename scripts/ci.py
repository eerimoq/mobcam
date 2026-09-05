#!/usr/bin/env python3

import argparse
import base64
import os
import shutil
import subprocess
import sys
from pathlib import Path

from build import DISPLAY_NAME
from build import MACOS_TARGETS
from build import PROJECT
from build import REPO_ROOT
from build import VERSION
from build import Error
from build import Platform
from build import Signing
from build import build_products
from build import dependencies
from build import host_platform
from build import package_products
from build import run

UBUNTU_PACKAGES = [
    "libclang-dev",
    "libobs-dev",
    "libavcodec-dev",
    "libavutil-dev",
    "libsimde-dev",
    "libpulse-dev",
    "libasound2-dev",
    "pkg-config",
    "just",
]
TARGETS: dict[Platform, list[str]] = {
    "macos": sorted(MACOS_TARGETS.values()),
    "windows": ["x86_64-pc-windows-msvc"],
    "linux": [],
}
KEYCHAIN_TIMEOUT = "21600"
KEYCHAIN_TOOLS = ["/usr/bin/codesign", "/usr/bin/security", "/usr/bin/xcrun"]
VARIANTS: dict[str, list[str]] = {
    "windows-x64": ["zip", "exe"],
    "macos-universal": ["tar.xz", "pkg"],
    "ubuntu-24.04-x86_64": ["tar.xz", "deb"],
}


def output(name: str, value: str) -> None:
    path = os.environ.get("GITHUB_OUTPUT")
    if path is None:
        return
    with open(path, "a", encoding="utf-8") as fout:
        fout.write(f"{name}={value}\n")


def setup() -> Platform:
    platform = host_platform()
    if platform == "linux":
        run(["sudo", "add-apt-repository", "--yes", "ppa:obsproject/obs-studio"])
        run(["sudo", "apt-get", "--quiet", "update"])
        run(
            [
                "sudo",
                "apt-get",
                "--quiet",
                "--yes",
                "--no-install-recommends",
                "install",
            ]
            + UBUNTU_PACKAGES
        )
    if TARGETS[platform]:
        run(["rustup", "target", "add"] + TARGETS[platform])
    return platform


def import_certificate(args: argparse.Namespace, password: str) -> None:
    temporary = Path(os.environ["RUNNER_TEMP"])
    certificate = temporary / "build_certificate.p12"
    certificate.write_bytes(base64.b64decode(args.codesign_certificate))
    keychain = temporary / "app-signing.keychain-db"
    tools = [argument for tool in KEYCHAIN_TOOLS for argument in ("-T", tool)]
    run(["security", "create-keychain", "-p", password, keychain])
    run(["security", "set-keychain-settings", "-lut", KEYCHAIN_TIMEOUT, keychain])
    run(["security", "unlock-keychain", "-p", password, keychain])
    command: list[str | Path] = [
        "security",
        "import",
        certificate,
        "-P",
        args.codesign_certificate_password,
        "-A",
        "-t",
        "cert",
        "-f",
        "pkcs12",
        "-k",
        keychain,
    ]
    run([*command, *tools])
    run(
        [
            "security",
            "set-key-partition-list",
            "-S",
            "apple-tool:,apple:",
            "-k",
            password,
            keychain,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    run(["security", "list-keychain", "-d", "user", "-s", keychain, "login-keychain"])


def style_and_lint(_: argparse.Namespace) -> None:
    setup()
    dependencies()
    run([sys.executable, "-m", "pip", "install", "--requirement", REPO_ROOT / "scripts" / "requirements.txt"])
    for target in ["style-check", "lint", "unit-test", "spell-check"]:
        run(["just", "--justfile", REPO_ROOT / "justfile", target])


def build(args: argparse.Namespace) -> None:
    platform = setup()
    dependencies()
    if platform == "macos":
        import_certificate(args, os.urandom(16).hex())
        build_products(args.codesign_application_identity)
        package_products(
            installer=True,
            signing=Signing(
                args.codesign_application_identity,
                args.codesign_installer_identity,
                args.notarization_user,
                args.notarization_password,
            ),
        )
    else:
        build_products()
        package_products(installer=True)
    output("name", PROJECT)
    output("version", VERSION)


def release(_: argparse.Namespace) -> None:
    root = Path(os.environ["GITHUB_WORKSPACE"])
    for variant, variant_suffixes in VARIANTS.items():
        for directory in sorted(root.glob(f"*-{variant}")):
            for suffix in variant_suffixes:
                for path in sorted(directory.glob(f"*.{suffix}")):
                    print(f"    {path.relative_to(root)}")
                    shutil.move(path, root / path.name)
    output("displayName", DISPLAY_NAME)


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparser = subparsers.add_parser("style_and_lint")
    subparser.set_defaults(function=style_and_lint)
    subparser = subparsers.add_parser("build")
    subparser.add_argument("--codesign-application-identity")
    subparser.add_argument("--codesign-installer-identity")
    subparser.add_argument("--codesign-certificate")
    subparser.add_argument("--codesign-certificate-password")
    subparser.add_argument("--notarization-user")
    subparser.add_argument("--notarization-password")
    subparser.set_defaults(function=build)
    subparser = subparsers.add_parser("release")
    subparser.set_defaults(function=release)
    args = parser.parse_args()
    try:
        args.function(args)
    except Error as error:
        print(f"::error::{error}", flush=True)
        sys.exit(2)


if __name__ == "__main__":
    main()
