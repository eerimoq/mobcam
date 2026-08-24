#!/usr/bin/env python3

import argparse
import base64
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from build import (  # noqa: E402
    MACOS_TARGETS,
    Error,
    host_platform,
    macos_paths,
    run,
    NAME,
    VERSION,
    DISPLAY_NAME
)

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
CREDENTIALS = [
    ("MACOS_SIGNING_IDENTITY", "CODESIGN_IDENT"),
    ("MACOS_SIGNING_INSTALLER_IDENTITY", "CODESIGN_IDENT_INSTALLER"),
    ("MACOS_SIGNING_CERT", None),
    ("MACOS_NOTARIZATION_USERNAME", "CODESIGN_IDENT_USER"),
    ("MACOS_NOTARIZATION_PASSWORD", "CODESIGN_IDENT_PASS"),
]
KEYCHAIN_TIMEOUT = "21600"
KEYCHAIN_TOOLS = ["/usr/bin/codesign", "/usr/bin/security", "/usr/bin/xcrun"]
VARIANTS = {
    "windows-x64": ["zip", "exe"],
    "macos-universal": ["tar.xz", "pkg"],
    "ubuntu-24.04-x86_64": ["tar.xz", "deb"],
}
CHECKSUMS = "CHECKSUMS.txt"


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
            ["sudo", "apt-get", "--quiet", "--yes", "--no-install-recommends", "install"]
            + UBUNTU_PACKAGES
        )
    run(["rustup", "show", "active-toolchain"])
    if TARGETS[platform]:
        run(["rustup", "target", "add"] + TARGETS[platform])
    return platform


def import_certificate(password):
    temporary = Path(os.environ["RUNNER_TEMP"])
    certificate = temporary / "build_certificate.p12"
    certificate.write_bytes(base64.b64decode(os.environ["MACOS_SIGNING_CERT"]))
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
            os.environ.get("MACOS_SIGNING_CERT_PASSWORD", ""),
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
        ["security", "set-key-partition-list", "-S", "apple-tool:,apple:",
         "-k", password, keychain],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    run(["security", "list-keychain", "-d", "user", "-s", keychain, "login-keychain"])


def codesigning():
    given = {name: os.environ.get(name, "") for name, _ in CREDENTIALS}
    for name, variable in CREDENTIALS:
        if variable:
            os.environ[variable] = given[name]
    sign = all(given[name] for name, _ in CREDENTIALS[:3])
    notarize = sign and all(given[name] for name, _ in CREDENTIALS[3:])
    if sign:
        import_certificate(os.urandom(16).hex())
    else:
        print("    no signing credentials; building unsigned", flush=True)
    return sign, notarize


def verify_universal_binary(name):
    _, binary, _ = macos_paths(name)
    result = run(["lipo", "-archs", binary], stdout=subprocess.PIPE, text=True)
    archs = result.stdout.split()
    print(f"    {name} architectures: {' '.join(archs)}")
    missing = [arch for arch in ARCHITECTURES if arch not in archs]
    if missing:
        raise Error(f"{name} is missing the {' and '.join(missing)} slice")


def lint():
    setup()
    run(["cargo", "clippy", "--all-targets", "--", "--deny", "warnings"])
    run(["cargo", "fmt", "--check"])


def build():
    platform = setup()
    sign, notarize = codesigning() if platform == "macos" else (False, False)
    build_py("build", *(["--codesign"] if sign else []))
    if platform == "macos":
        verify_universal_binary(NAME)
    build_py(
        "package",
        "--installer",
        *(["--codesign"] if sign else []),
        *(["--notarize"] if notarize else []),
    )
    output("pluginName", NAME)
    output("pluginVersion", VERSION)
    output("commitHash", os.environ.get("GITHUB_SHA", "")[:9])


def release():
    root = Path(os.environ["GITHUB_WORKSPACE"])
    commit_hash = os.environ["GITHUB_SHA"][:9]
    suffixes = {suffix for suffixes in VARIANTS.values() for suffix in suffixes}
    for variant, variant_suffixes in VARIANTS.items():
        for directory in sorted(root.glob(f"*-{variant}-{commit_hash}")):
            for suffix in variant_suffixes:
                for path in sorted(directory.glob(f"*.{suffix}")):
                    print(f"    {path.relative_to(root)}")
                    shutil.move(path, root / path.name)
    lines = ["### Checksums"]
    for path in sorted(root.iterdir()):
        if any(path.name.endswith(f".{suffix}") for suffix in suffixes):
            lines.append(f"    {path.name}: {hashlib.sha256(path.read_bytes()).hexdigest()}")
    (root / CHECKSUMS).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print((root / CHECKSUMS).read_text(encoding="utf-8"))
    output("pluginName", DISPLAY_NAME)


def main():
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparser = subparsers.add_parser("lint")
    subparser.set_defaults(function=lint)
    subparser = subparsers.add_parser("build")
    subparser.set_defaults(function=build)
    subparser = subparsers.add_parser("release")
    subparser.set_defaults(function=release)
    arguments = parser.parse_args()
    try:
        arguments.function()
    except Error as error:
        print(f"::error::{error}", flush=True)
        sys.exit(2)


if __name__ == "__main__":
    main()
