import argparse
import hashlib
import lzma
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import urllib.request
import zipfile
from collections.abc import Iterable
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from typing import Literal

Platform = Literal["macos", "windows", "linux"]
Command = Sequence[str | Path]


@dataclass
class DependencySource:
    url: str
    sha256: str


@dataclass
class Dependency:
    label: str
    version: str
    directory: str
    strip_root: bool
    os: dict[str, DependencySource]


REPO_ROOT = Path(__file__).resolve().parent.parent
PROJECT = "mobcam"
DISPLAY_NAME = "Mobcam"
AUTHOR = "Erik Moqvist"
EMAIL = "erik.moqvist@gmail.com"
WEBSITE = "https://github.com/eerimoq/mobcam"
BUNDLE_ID = "com.eerimoq.mobcam"
CRATES_DIR = REPO_ROOT / "crates"
DEPS_DIR = REPO_ROOT / ".deps"
RELEASE_DIR = REPO_ROOT / "release"
INSTALL_DIR = RELEASE_DIR / "install"
MACOS_DEPLOYMENT_TARGET = "12.0"
MACOS_TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def read_workspace_version() -> str:
    with open(REPO_ROOT / "Cargo.toml", "rb") as fin:
        manifest: dict[str, Any] = tomllib.load(fin)
        version: str = manifest["workspace"]["package"]["version"]
        return version


VERSION = read_workspace_version()


@dataclass(frozen=True)
class Product:
    name: str
    module: str
    display_name: str
    directory: str

    @property
    def root(self) -> Path:
        return CRATES_DIR / self.directory

    @property
    def packaging_dir(self) -> Path:
        return self.root / "packaging"

    @property
    def install_dir(self) -> Path:
        return INSTALL_DIR / self.directory

    def output_name(self, target_platform: Platform) -> str:
        if target_platform == "macos":
            return f"{self.name}-{VERSION}-macos-universal"
        elif target_platform == "windows":
            return f"{self.name}-{VERSION}-windows-x64"
        else:
            return f"{self.name}-{VERSION}-{platform.machine()}-linux-gnu"

    def values(self, **extra: object) -> dict[str, object]:
        return {
            "PRODUCT": self.name,
            "MODULE": self.module,
            "DISPLAY_NAME": self.display_name,
            "VERSION": VERSION,
            "AUTHOR": AUTHOR,
            "EMAIL": EMAIL,
            "WEBSITE": WEBSITE,
            "BUNDLE_ID": BUNDLE_ID,
            "DEPLOYMENT_TARGET": MACOS_DEPLOYMENT_TARGET,
            "YEAR": time.strftime("%Y"),
            "OBS_PLUGIN": OBS_PLUGIN_NAME,
            "VIRTUALCAM": VIRTUALCAM_NAME,
            **extra,
        }


OBS_PLUGIN_NAME = f"{PROJECT}-obs-plugin"
VIRTUALCAM_NAME = f"{PROJECT}-virtualcam"
OBS_PLUGIN = Product(
    name=OBS_PLUGIN_NAME,
    module=PROJECT,
    display_name=DISPLAY_NAME,
    directory="obs-plugin",
)
VIRTUALCAM = Product(
    name=VIRTUALCAM_NAME,
    module=VIRTUALCAM_NAME,
    display_name=f"{DISPLAY_NAME} virtual camera",
    directory="virtualcam",
)
DATA_DIR = OBS_PLUGIN.root / "data"
OBS_STUDIO_VERSION = "28.0.0"
OBS_STUDIO_URL = "https://github.com/obsproject/obs-studio/archive/refs/tags"
PREBUILT_VERSION = "2026-07-15"
PREBUILT_URL = "https://github.com/obsproject/obs-deps/releases/download"
DEPENDENCIES: list[Dependency] = [
    Dependency(
        label="OBS sources",
        version=OBS_STUDIO_VERSION,
        directory="obs-studio",
        strip_root=True,
        os={
            "macos": DependencySource(
                url=f"{OBS_STUDIO_URL}/{OBS_STUDIO_VERSION}.tar.gz",
                sha256="412a1a26699a6861dbbe93346310d002369c62e00626e8c3a77c127e5e1c06e8",
            ),
            "windows": DependencySource(
                url=f"{OBS_STUDIO_URL}/{OBS_STUDIO_VERSION}.zip",
                sha256="2f54f6f658b7cada48279293c3d5b972835303accddc4e2a739c9e88cdd500cf",
            ),
            "linux": DependencySource(
                url=f"{OBS_STUDIO_URL}/{OBS_STUDIO_VERSION}.tar.gz",
                sha256="412a1a26699a6861dbbe93346310d002369c62e00626e8c3a77c127e5e1c06e8",
            ),
        },
    ),
    Dependency(
        label="Pre-Built obs-deps",
        version=PREBUILT_VERSION,
        directory="prebuilt",
        strip_root=False,
        os={
            "macos": DependencySource(
                url=f"{PREBUILT_URL}/{PREBUILT_VERSION}/macos-deps-{PREBUILT_VERSION}-universal.tar.xz",
                sha256="4ecb4c598dfa853168df6c2a0c4e0ffec8495a81fbd1ba051ef88ecd5e0f7e53",
            ),
            "windows": DependencySource(
                url=f"{PREBUILT_URL}/{PREBUILT_VERSION}/windows-deps-{PREBUILT_VERSION}-x64.zip",
                sha256="6f90e9598fa10cff5ad23cdcfae49b87868c07bf896b02cd464582b4ce2f2ba9",
            ),
        },
    ),
]


class Error(Exception):
    pass


def run(command: Command, **kwargs: Any) -> subprocess.CompletedProcess[Any]:
    return subprocess.run(command, check=True, **kwargs)


def host_platform() -> Platform:
    if sys.platform == "darwin":
        return "macos"
    elif sys.platform == "win32":
        return "windows"
    elif sys.platform.startswith("linux"):
        return "linux"
    else:
        raise Error(f"unsupported platform {sys.platform}")


def render(template: Path, output: Path, **values: object) -> None:
    text = template.read_text(encoding="utf-8")
    for key, value in values.items():
        text = text.replace(f"@{key}@", str(value))
    missing = sorted(set(re.findall(r"@[A-Z_]+@", text)))
    if missing:
        raise Error(f"{template} has placeholders nothing filled in: {', '.join(missing)}")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text, encoding="utf-8")


def remove(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif path.exists() or path.is_symlink():
        path.unlink()


def source_name() -> str:
    return f"{PROJECT}-{VERSION}-source"


def download(url: str, path: Path, sha256: str) -> None:
    digest = hashlib.sha256()
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".part")
    with urllib.request.urlopen(url) as response, open(temporary, "wb") as fout:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            fout.write(chunk)
    if digest.hexdigest() != sha256:
        temporary.unlink()
        raise Error(
            f"{url} does not have the hash DEPENDENCIES expects:\n"
            f"  expected {sha256}\n"
            f"  actual   {digest.hexdigest()}"
        )
    temporary.replace(path)


def extract(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as zip_file:
            zip_file.extractall(destination)
    else:
        with tarfile.open(archive) as tar_file:
            if sys.version_info >= (3, 12):
                tar_file.extractall(destination, filter="data")
            else:
                tar_file.extractall(destination)


def extract_stripped(archive: Path, destination: Path) -> None:
    staging = destination.with_name(destination.name + ".part")
    remove(staging)
    extract(archive, staging)
    (root,) = staging.iterdir()
    root.replace(destination)
    remove(staging)


def dependencies(target_platform: Platform | None = None) -> None:
    target_platform = target_platform or host_platform()
    for dependency in DEPENDENCIES:
        source = dependency.os.get(target_platform)
        if source is None:
            continue
        url = source.url
        sha256 = source.sha256
        directory = DEPS_DIR / dependency.directory
        archive = DEPS_DIR / url.rsplit("/", 1)[1]
        marker = DEPS_DIR / f".dependency_{dependency.directory}.sha256"
        if directory.is_dir() and marker.is_file() and marker.read_text().strip() == sha256:
            continue
        if not archive.is_file():
            download(url, archive, sha256)
        remove(marker)
        remove(directory)
        if dependency.strip_root:
            extract_stripped(archive, directory)
        else:
            extract(archive, directory)
        marker.write_text(sha256 + "\n")


def cargo_target_dir() -> Path:
    return Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))


def library_name(target_platform: Platform, name: str) -> str:
    if target_platform == "macos":
        return f"lib{name}.dylib"
    elif target_platform == "windows":
        return f"{name}.dll"
    else:
        return f"lib{name}.so"


def cargo_build_library(
    target_platform: Platform, product: Product, targets: Sequence[str | None]
) -> list[Path]:
    environment = dict(os.environ)
    libraries: list[Path] = []
    if target_platform == "macos":
        environment["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    for target in targets:
        command = ["cargo", "build", "--locked", "--profile", "release", "--package", product.name]
        if target is not None:
            command += ["--target", target]
        run(command, cwd=REPO_ROOT, env=environment)
        directory = cargo_target_dir()
        if target is not None:
            directory /= target
        libraries.append(directory / "release" / library_name(target_platform, product.module))
    return libraries


def cargo_build_binary(product: Product) -> Path:
    run(
        ["cargo", "build", "--locked", "--profile", "release", "--package", product.name],
        cwd=REPO_ROOT,
    )
    return cargo_target_dir() / "release" / product.module


def copy_data(destination: Path) -> None:
    shutil.copytree(DATA_DIR, destination, dirs_exist_ok=True)


def codesign(path: Path, identity: str) -> None:
    identity = identity or "-"
    command: list[str | Path] = [
        "codesign",
        "--force",
        "--sign",
        identity,
        "--options",
        "runtime",
    ]
    if identity != "-":
        command.append("--timestamp")
    run(command + [path])


def macos_paths() -> tuple[Path, Path, Path]:
    module = OBS_PLUGIN.module
    bundle = OBS_PLUGIN.install_dir / f"{module}.plugin"
    return (
        bundle,
        bundle / "Contents" / "MacOS" / module,
        OBS_PLUGIN.install_dir / f"{module}.plugin.dSYM",
    )


def build_macos(identity: str) -> None:
    bundle, binary, symbols = macos_paths()
    libraries = cargo_build_library("macos", OBS_PLUGIN, sorted(MACOS_TARGETS.values()))
    remove(bundle)
    binary.parent.mkdir(parents=True)
    run(["lipo", "-create", *libraries, "-output", binary])
    run(["install_name_tool", "-id", f"@rpath/{OBS_PLUGIN.module}", binary])
    render(
        OBS_PLUGIN.packaging_dir / "macos" / "Info.plist.in",
        bundle / "Contents" / "Info.plist",
        **OBS_PLUGIN.values(),
    )
    copy_data(bundle / "Contents" / "Resources")
    remove(symbols)
    run(["dsymutil", binary, "-o", symbols])
    run(["strip", "-x", binary])
    codesign(bundle, identity)


def build_linux_obs_plugin() -> None:
    (library,) = cargo_build_library("linux", OBS_PLUGIN, [None])
    install = OBS_PLUGIN.install_dir
    remove(install)
    library_dir = install / "lib" / f"{platform.machine()}-linux-gnu" / "obs-plugins"
    library_dir.mkdir(parents=True)
    shutil.copy2(library, library_dir / f"{OBS_PLUGIN.module}.so")
    copy_data(install / "share" / "obs" / "obs-plugins" / OBS_PLUGIN.module)


def build_linux_virtualcam() -> None:
    binary = cargo_build_binary(VIRTUALCAM)
    install = VIRTUALCAM.install_dir
    remove(install)
    binary_dir = install / "bin"
    binary_dir.mkdir(parents=True)
    shutil.copy2(binary, binary_dir / VIRTUALCAM.module)


def build_linux() -> None:
    build_linux_obs_plugin()
    build_linux_virtualcam()


def build_windows() -> None:
    (library,) = cargo_build_library("windows", OBS_PLUGIN, [None])
    root = OBS_PLUGIN.install_dir / OBS_PLUGIN.module
    binary_dir = root / "bin" / "64bit"
    remove(root)
    binary_dir.mkdir(parents=True)
    shutil.copy2(library, binary_dir / f"{OBS_PLUGIN.module}.dll")
    symbols = library.with_suffix(".pdb")
    if symbols.is_file():
        shutil.copy2(symbols, binary_dir / f"{OBS_PLUGIN.module}.pdb")
    copy_data(root / "data")


def build(args: argparse.Namespace) -> None:
    target_platform = host_platform()
    INSTALL_DIR.mkdir(parents=True, exist_ok=True)
    if target_platform == "macos":
        build_macos(args.codesign_application_identity)
    elif target_platform == "windows":
        build_windows()
    else:
        build_linux()


def tar_xz(archive: Path, directory: Path, members: Iterable[str]) -> None:
    archive.parent.mkdir(parents=True, exist_ok=True)
    remove(archive)
    with tarfile.open(archive, "w:xz") as tar_file:
        for member in members:
            tar_file.add(directory / member, arcname=member)


def package_macos(args: argparse.Namespace) -> None:
    base = OBS_PLUGIN.output_name("macos")
    bundle, _, symbols = macos_paths()
    if not bundle.is_dir():
        raise Error("no staged plugin found; run `python3 scripts/build.py build` first")
    if args.installer:
        package_macos_installer(args, base)
    else:
        tar_xz(RELEASE_DIR / f"{base}.tar.xz", OBS_PLUGIN.install_dir, [bundle.name])
    if symbols.is_dir():
        tar_xz(RELEASE_DIR / f"{base}-dSYMs.tar.xz", OBS_PLUGIN.install_dir, [symbols.name])


def package_macos_installer(args: argparse.Namespace, base: str) -> None:
    staging = RELEASE_DIR / "installer"
    root = staging / "root" / "Library" / "Application Support" / "obs-studio" / "plugins"
    bundle, _, _ = macos_paths()
    remove(staging)
    root.mkdir(parents=True)
    shutil.copytree(bundle, root / bundle.name, symlinks=True)
    run(
        [
            "pkgbuild",
            "--identifier",
            BUNDLE_ID,
            "--version",
            VERSION,
            "--root",
            staging / "root",
            staging / f"{OBS_PLUGIN.module}.pkg",
        ]
    )
    distribution = staging / "distribution.xml"
    render(
        OBS_PLUGIN.packaging_dir / "macos" / "distribution.xml.in",
        distribution,
        **OBS_PLUGIN.values(),
    )
    resources = staging / "resources"
    resources.mkdir(parents=True)
    shutil.copy2(OBS_PLUGIN.packaging_dir / "macos" / "background.png", resources / "background.png")
    package = RELEASE_DIR / f"{base}.pkg"
    unsigned = staging / f"{OBS_PLUGIN.module}-distribution.pkg"
    run(
        [
            "productbuild",
            "--distribution",
            distribution,
            "--package-path",
            staging,
            "--resources",
            resources,
            unsigned,
        ]
    )
    remove(package)
    if args.codesign_installer_identity:
        run(
            [
                "productsign",
                "--sign",
                args.codesign_installer_identity,
                unsigned,
                package,
            ]
        )
    else:
        unsigned.replace(package)
    remove(staging)
    if args.notarization_user or args.notarization_password:
        notarize(package, OBS_PLUGIN.name, args)


def notarize(package: Path, name: str, args: argparse.Namespace) -> None:
    user = args.notarization_user
    password = args.notarization_password
    team = args.codesign_application_identity.rpartition("(")[2].rstrip(")")
    if not (user and password and team):
        raise Error(
            "notarization needs --notarization-user, --notarization-password "
            "and a team in --codesign-application-identity"
        )
    profile = f"{name}-Codesign-Password"
    run(
        [
            "xcrun",
            "notarytool",
            "store-credentials",
            profile,
            "--apple-id",
            user,
            "--team-id",
            team,
            "--password",
            password,
        ]
    )
    run(
        [
            "xcrun",
            "notarytool",
            "submit",
            package,
            "--keychain-profile",
            profile,
            "--wait",
        ]
    )
    run(["xcrun", "stapler", "staple", package])


def package_linux(args: argparse.Namespace) -> None:
    if not (OBS_PLUGIN.install_dir / "lib").is_dir():
        raise Error("no staged plugin found; run `python3 scripts/build.py build` first")
    plugin_base = OBS_PLUGIN.output_name("linux")
    tar_xz(RELEASE_DIR / f"{plugin_base}.tar.xz", OBS_PLUGIN.install_dir, ["lib", "share"])
    source_tarball()
    if args.installer:
        package_deb(OBS_PLUGIN, plugin_base)


def package_deb(product: Product, base: str) -> None:
    staging = RELEASE_DIR / f"deb-{product.directory}"
    remove(staging)
    shutil.copytree(product.install_dir, staging / "usr", symlinks=True)
    architecture = subprocess.run(
        ["dpkg", "--print-architecture"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    render(
        product.packaging_dir / "linux" / "control.in",
        staging / "DEBIAN" / "control",
        **product.values(ARCHITECTURE=architecture),
    )
    package = RELEASE_DIR / f"{base}.deb"
    remove(package)
    run(["dpkg-deb", "--build", "--root-owner-group", staging, package])
    remove(staging)


def source_tarball() -> None:
    base = source_name()
    archive = RELEASE_DIR / f"{base}.tar.xz"
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    sources = run(
        ["git", "archive", f"--prefix={base}/", "--format=tar", "HEAD"],
        capture_output=True,
        cwd=REPO_ROOT,
    ).stdout
    with lzma.open(archive, "wb") as fout:
        fout.write(sources)


def package_windows(args: argparse.Namespace) -> None:
    base = OBS_PLUGIN.output_name("windows")
    root = OBS_PLUGIN.install_dir / OBS_PLUGIN.module
    if not root.is_dir():
        raise Error("no staged plugin found; run `python3 scripts/build.py build` first")
    archive = RELEASE_DIR / f"{base}.zip"
    remove(archive)
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zip_file:
        for path in sorted(root.rglob("*")):
            if path.is_file():
                zip_file.write(path, path.relative_to(OBS_PLUGIN.install_dir))
    if args.installer:
        package_windows_installer(base)


def find_inno_setup() -> str:
    compiler = shutil.which("iscc")
    if compiler:
        return compiler
    candidates = [
        Path(os.environ.get("ProgramFiles(x86)", "")) / "Inno Setup 6" / "ISCC.exe",
        Path(os.environ.get("ProgramFiles", "")) / "Inno Setup 6" / "ISCC.exe",
    ]
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    raise Error("Inno Setup (ISCC.exe) not found; install it from https://jrsoftware.org/isinfo.php")


def package_windows_installer(base: str) -> None:
    script = RELEASE_DIR / "installer.iss"
    render(
        OBS_PLUGIN.packaging_dir / "windows" / "installer.iss.in",
        script,
        **OBS_PLUGIN.values(
            SOURCE_DIR=REPO_ROOT,
            PACKAGING_DIR=OBS_PLUGIN.packaging_dir,
            INSTALL_DIR=OBS_PLUGIN.install_dir,
            OUTPUT_DIR=RELEASE_DIR,
            OUTPUT_NAME=f"{base}-Installer",
        ),
    )
    run([find_inno_setup(), script, f"/DReleaseDir={OBS_PLUGIN.install_dir}"])
    remove(script)


def package(args: argparse.Namespace) -> None:
    target_platform = host_platform()
    if target_platform == "macos":
        package_macos(args)
    elif target_platform == "windows":
        package_windows(args)
    else:
        package_linux(args)


def install_macos() -> None:
    destination = Path.home() / "Library/Application Support/obs-studio/plugins"
    source = macos_paths()[0]
    destination.mkdir(parents=True, exist_ok=True)
    remove(destination / source.name)
    shutil.copytree(source, destination / source.name, symlinks=True)


def install_linux() -> None:
    destination = Path.home() / ".config" / "obs-studio" / "plugins" / OBS_PLUGIN.module
    remove(destination)
    (destination / "bin" / "64bit").mkdir(parents=True)
    shutil.copy2(
        OBS_PLUGIN.install_dir
        / "lib"
        / f"{platform.machine()}-linux-gnu"
        / "obs-plugins"
        / f"{OBS_PLUGIN.module}.so",
        destination / "bin" / "64bit" / f"{OBS_PLUGIN.module}.so",
    )
    copy_data(destination / "data")
    binaries = Path.home() / ".local" / "bin"
    binaries.mkdir(parents=True, exist_ok=True)
    shutil.copy2(VIRTUALCAM.install_dir / "bin" / VIRTUALCAM.module, binaries / VIRTUALCAM.module)


def install(_: argparse.Namespace) -> None:
    target_platform = host_platform()
    if target_platform == "macos":
        install_macos()
    elif target_platform == "linux":
        install_linux()
    else:
        raise Error("installing is only supported on macOS and Linux")


def clean(_: argparse.Namespace) -> None:
    for path in [RELEASE_DIR, cargo_target_dir()]:
        if path.exists():
            remove(path)


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    deps_parser = subparsers.add_parser("deps")
    deps_parser.set_defaults(function=lambda args: dependencies())
    build_parser = subparsers.add_parser("build")
    build_parser.add_argument("--codesign-application-identity")
    build_parser.set_defaults(function=build)
    package_parser = subparsers.add_parser("package")
    package_parser.add_argument("--installer", action="store_true")
    package_parser.add_argument("--codesign-application-identity")
    package_parser.add_argument("--codesign-installer-identity")
    package_parser.add_argument("--notarization-user")
    package_parser.add_argument("--notarization-password")
    package_parser.set_defaults(function=package)
    install_parser = subparsers.add_parser("install")
    install_parser.set_defaults(function=install)
    clean_parser = subparsers.add_parser("clean")
    clean_parser.set_defaults(function=clean)
    args = parser.parse_args()
    args.function(args)


if __name__ == "__main__":
    main()
