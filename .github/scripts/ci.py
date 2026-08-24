#!/usr/bin/env python3

import argparse
import base64
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from build import DISPLAY_NAME
from build import MACOS_TARGETS
from build import NAME
from build import VERSION
from build import Error
from build import host_platform
from build import macos_paths
from build import run

UBUNTU_PACKAGES = [
    "libclang-dev",
    "libobs-dev",
    "libavcodec-dev",
    "libavutil-dev",
    "libsimde-dev",
    "pkg-config",
]
TARGETS = {
    "macos": sorted(MACOS_TARGETS.values()),
    "windows": ["x86_64-pc-windows-msvc"],
    "linux": [],
}
ARCHITECTURES = sorted(MACOS_TARGETS)
KEYCHAIN_TIMEOUT = "21600"
KEYCHAIN_TOOLS = ["/usr/bin/codesign", "/usr/bin/security", "/usr/bin/xcrun"]
VARIANTS = {
    "windows-x64": ["zip", "exe"],
    "macos-universal": ["tar.xz", "pkg"],
    "ubuntu-24.04-x86_64": ["tar.xz", "deb"],
}


def output(name, value):
    path = os.environ.get("GITHUB_OUTPUT")
    if path is None:
        print(f"{name}={value}", flush=True)
        return
    with open(path, "a", encoding="utf-8") as fout:
        fout.write(f"{name}={value}\n")


def build_py(*arguments):
    return run([sys.executable, ROOT / "build.py", *arguments])


def setup():
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
    run(["rustup", "show", "active-toolchain"])
    if TARGETS[platform]:
        run(["rustup", "target", "add"] + TARGETS[platform])
    return platform


def import_certificate(args, password):
    temporary = Path(os.environ["RUNNER_TEMP"])
    certificate = temporary / "build_certificate.p12"
    certificate.write_bytes(base64.b64decode(args.codesign_certificate))
    keychain = temporary / "app-signing.keychain-db"
    tools = [argument for tool in KEYCHAIN_TOOLS for argument in ("-T", tool)]
    run(["security", "create-keychain", "-p", password, keychain])
    run(["security", "set-keychain-settings", "-lut", KEYCHAIN_TIMEOUT, keychain])
    run(["security", "unlock-keychain", "-p", password, keychain])
    run(
        [
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
        + tools,
    )
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


def codesigning(args):
    import_certificate(args, os.urandom(16).hex())
    return True


def verify_universal_binary(name):
    _, binary, _ = macos_paths(name)
    result = run(["lipo", "-archs", binary], stdout=subprocess.PIPE, text=True)
    archs = result.stdout.split()
    print(f"    {name} architectures: {' '.join(archs)}")
    missing = [arch for arch in ARCHITECTURES if arch not in archs]
    if missing:
        raise Error(f"{name} is missing the {' and '.join(missing)} slice")


def lint(_):
    setup()
    run([sys.executable, "-m", "pip", "install", "--requirement", ROOT / "requirements.txt"])
    for target in ["style-check", "lint", "spell-check"]:
        run(["make", "--directory", ROOT, target])


def build(args):
    platform = setup()
    if platform == "macos":
        import_certificate(args, os.urandom(16).hex())
        build_arguments = [
            "--codesign-application-identity",
            args.codesign_application_identity,
        ]
        package_arguments = build_arguments
        package_arguments += [
            "--codesign-installer-identity",
            args.codesign_installer_identity,
        ]
        package_arguments += [
            "--notarization-user",
            args.notarization_user,
            "--notarization-password",
            args.notarization_password,
        ]
        build_py("build", *build_arguments)
        verify_universal_binary(NAME)
        build_py("package", "--installer", *package_arguments)
    else:
        build_py("build")
        build_py("package", "--installer")
    output("pluginName", NAME)
    output("pluginVersion", VERSION)


def release(_):
    root = Path(os.environ["GITHUB_WORKSPACE"])
    for variant, variant_suffixes in VARIANTS.items():
        for directory in sorted(root.glob(f"*-{variant}")):
            for suffix in variant_suffixes:
                for path in sorted(directory.glob(f"*.{suffix}")):
                    print(f"    {path.relative_to(root)}")
                    shutil.move(path, root / path.name)
    output("pluginName", DISPLAY_NAME)


def main():
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparser = subparsers.add_parser("lint")
    subparser.set_defaults(function=lint)
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
