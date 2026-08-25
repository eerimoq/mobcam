import argparse
import hashlib
import lzma
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def read_cargo_package():
    with open(ROOT / "Cargo.toml", "rb") as fin:
        return tomllib.load(fin)["package"]


CARGO_PACKAGE = read_cargo_package()
NAME = CARGO_PACKAGE["name"]
DISPLAY_NAME = "Mobcam"
VERSION = CARGO_PACKAGE["version"]
BUNDLE_ID = "com.eerimoq.mobcam"
DEPS_DIR = ROOT / ".deps"
RELEASE_DIR = ROOT / "release"
INSTALL_DIR = RELEASE_DIR / "install"
PACKAGING_DIR = ROOT / "packaging"
DATA_DIR = ROOT / "data"
MACOS_DEPLOYMENT_TARGET = "12.0"
MACOS_TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}
OBS_STUDIO_VERSION = "32.2.2"
OBS_STUDIO_URL = "https://github.com/obsproject/obs-studio/archive/refs/tags"
PREBUILT_VERSION = "2026-07-15"
PREBUILT_URL = "https://github.com/obsproject/obs-deps/releases/download"
DEPENDENCIES = [
    {
        "label": "OBS sources",
        "version": OBS_STUDIO_VERSION,
        "directory": "obs-studio",
        "strip_root": True,
        "os": {
            "macos": {
                "url": f"{OBS_STUDIO_URL}/{OBS_STUDIO_VERSION}.tar.gz",
                "sha256": "35d3cd0979d65664fada7119fdb612eca7c34b61a1623a330caec74bf72626c4",
            },
            "windows": {
                "url": f"{OBS_STUDIO_URL}/{OBS_STUDIO_VERSION}.zip",
                "sha256": "f15f001f1fa526405318835f44f9910046502f496ebc3a30d5296a5018b831aa",
            },
        },
    },
    {
        "label": "Pre-Built obs-deps",
        "version": PREBUILT_VERSION,
        "directory": "prebuilt",
        "strip_root": False,
        "os": {
            "macos": {
                "url": (f"{PREBUILT_URL}/{PREBUILT_VERSION}/macos-deps-{PREBUILT_VERSION}-universal.tar.xz"),
                "sha256": "4ecb4c598dfa853168df6c2a0c4e0ffec8495a81fbd1ba051ef88ecd5e0f7e53",
            },
            "windows": {
                "url": (f"{PREBUILT_URL}/{PREBUILT_VERSION}/windows-deps-{PREBUILT_VERSION}-x64.zip"),
                "sha256": "6f90e9598fa10cff5ad23cdcfae49b87868c07bf896b02cd464582b4ce2f2ba9",
            },
        },
    },
]


class Error(Exception):
    pass


def run(command, **kwargs):
    try:
        return subprocess.run(command, check=True, **kwargs)
    except FileNotFoundError:
        raise Error(f"{command[0]} not found")
    except subprocess.CalledProcessError as error:
        raise Error(f"{command[0]} failed with exit code {error.returncode}")


def host_platform():
    if sys.platform == "darwin":
        return "macos"
    elif sys.platform == "win32":
        return "windows"
    elif sys.platform.startswith("linux"):
        return "linux"
    else:
        raise Error(f"unsupported platform {sys.platform}")


def plugin():
    return {
        "NAME": NAME,
        "DISPLAY_NAME": "Mobcam",
        "VERSION": VERSION,
        "AUTHOR": "Erik Moqvist",
        "EMAIL": "erik.moqvist@gmail.com",
        "WEBSITE": "https://github.com/eerimoq/obs-mobcam-plugin",
        "BUNDLE_ID": BUNDLE_ID,
        "DEPLOYMENT_TARGET": MACOS_DEPLOYMENT_TARGET,
        "YEAR": time.strftime("%Y"),
    }


def render(template, output, **values):
    text = template.read_text(encoding="utf-8")
    for key, value in values.items():
        text = text.replace(f"@{key}@", str(value))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text, encoding="utf-8")


def remove(path):
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif path.exists() or path.is_symlink():
        path.unlink()


def output_name(target_platform):
    if target_platform == "macos":
        return f"{NAME}-{VERSION}-macos-universal"
    elif target_platform == "windows":
        return f"{NAME}-{VERSION}-windows-x64"
    else:
        return f"{NAME}-{VERSION}-{platform.machine()}-linux-gnu"


def download(url, path, sha256):
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


def extract(archive, destination):
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


def extract_stripped(archive, destination):
    staging = destination.with_name(destination.name + ".part")
    remove(staging)
    extract(archive, staging)
    (root,) = staging.iterdir()
    root.replace(destination)
    remove(staging)


def dependencies(target_platform=None):
    target_platform = target_platform or host_platform()
    if target_platform == "linux":
        return
    for dependency in DEPENDENCIES:
        platform = dependency["os"][target_platform]
        url = platform["url"]
        sha256 = platform["sha256"]
        label = f"{dependency['label']} {dependency['version']}"
        directory = DEPS_DIR / dependency["directory"]
        archive = DEPS_DIR / url.rsplit("/", 1)[1]
        marker = DEPS_DIR / f".dependency_{dependency['directory']}.sha256"
        if directory.is_dir() and marker.is_file() and marker.read_text().strip() == sha256:
            continue
        if not archive.is_file():
            download(url, archive, sha256)
        remove(marker)
        remove(directory)
        if dependency["strip_root"]:
            extract_stripped(archive, directory)
        else:
            extract(archive, directory)
        marker.write_text(sha256 + "\n")


def cargo_target_dir():
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))


def cargo_executable():
    directories = []
    if "CARGO_HOME" in os.environ:
        directories.append(Path(os.environ["CARGO_HOME"]) / "bin")
    directories.append(Path.home() / ".cargo" / "bin")
    if "HOMEBREW_PREFIX" in os.environ:
        directories.append(Path(os.environ["HOMEBREW_PREFIX"]) / "opt" / "rustup" / "bin")
    directories += [
        Path("/opt/homebrew/opt/rustup/bin"),
        Path("/usr/local/opt/rustup/bin"),
    ]
    for directory in directories:
        executable = shutil.which("cargo", path=str(directory))
        if executable:
            return Path(executable)
    executable = shutil.which("cargo")
    if executable:
        return Path(executable)
    raise Error("cargo not found; install a toolchain from https://rustup.rs")


def library_name(target_platform, name):
    if target_platform == "macos":
        return f"lib{name}.dylib"
    elif target_platform == "windows":
        return f"{name}.dll"
    else:
        return f"lib{name}.so"


def cargo_build(target_platform, name, debug, targets):
    profile = "dev" if debug else "release"
    profile_directory = "debug" if debug else "release"
    cargo = cargo_executable()
    environment = dict(os.environ)
    libraries = []
    environment["PATH"] = os.pathsep.join([str(cargo.parent), environment.get("PATH", "")])
    if target_platform == "macos":
        environment["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    for target in targets:
        command = [cargo, "build", "--locked", "--profile", profile]
        if target is not None:
            command += ["--target", target]
        run(command, cwd=ROOT, env=environment)
        directory = cargo_target_dir()
        if target is not None:
            directory /= target
        libraries.append(directory / profile_directory / library_name(target_platform, name))
    return libraries


def copy_data(destination):
    shutil.copytree(DATA_DIR, destination, dirs_exist_ok=True)


def codesign(path, identity):
    identity = identity or "-"
    command = ["codesign", "--force", "--sign", identity, "--options", "runtime"]
    if identity != "-":
        command.append("--timestamp")
    run(command + [path])


def macos_paths(name):
    bundle = INSTALL_DIR / f"{name}.plugin"
    return (
        bundle,
        bundle / "Contents" / "MacOS" / name,
        INSTALL_DIR / f"{name}.plugin.dSYM",
    )


def build_macos(debug, identity):
    bundle, binary, symbols = macos_paths(NAME)
    libraries = cargo_build("macos", NAME, debug, sorted(MACOS_TARGETS.values()))
    remove(bundle)
    binary.parent.mkdir(parents=True)
    run(["lipo", "-create", *libraries, "-output", binary])
    run(["install_name_tool", "-id", f"@rpath/{NAME}", binary])
    render(
        PACKAGING_DIR / "macos" / "Info.plist.in",
        bundle / "Contents" / "Info.plist",
        **plugin(),
    )
    copy_data(bundle / "Contents" / "Resources")
    remove(symbols)
    if not debug:
        run(["dsymutil", binary, "-o", symbols])
        run(["strip", "-x", binary])
    codesign(bundle, identity)
    return bundle


def build_linux(debug):
    (library,) = cargo_build("linux", NAME, debug, [None])
    library_dir = INSTALL_DIR / "lib" / f"{platform.machine()}-linux-gnu" / "obs-plugins"
    remove(INSTALL_DIR / "lib")
    remove(INSTALL_DIR / "share")
    library_dir.mkdir(parents=True)
    shutil.copy2(library, library_dir / f"{NAME}.so")
    copy_data(INSTALL_DIR / "share" / "obs" / "obs-plugins" / NAME)
    return library_dir / f"{NAME}.so"


def build_windows(debug):
    (library,) = cargo_build("windows", NAME, debug, [None])
    root = INSTALL_DIR / NAME
    binary_dir = root / "bin" / "64bit"
    remove(root)
    binary_dir.mkdir(parents=True)
    shutil.copy2(library, binary_dir / f"{NAME}.dll")
    symbols = library.with_suffix(".pdb")
    if symbols.is_file():
        shutil.copy2(symbols, binary_dir / f"{NAME}.pdb")
    copy_data(root / "data")
    return binary_dir / f"{NAME}.dll"


def build(args):
    target_platform = host_platform()
    INSTALL_DIR.mkdir(parents=True, exist_ok=True)
    if target_platform == "macos":
        artifact = build_macos(args.debug, args.codesign_application_identity)
    elif target_platform == "windows":
        artifact = build_windows(args.debug)
    else:
        artifact = build_linux(args.debug)


def tar_xz(archive, directory, members):
    archive.parent.mkdir(parents=True, exist_ok=True)
    remove(archive)
    with tarfile.open(archive, "w:xz") as tar_file:
        for member in members:
            tar_file.add(directory / member, arcname=member)


def package_macos(args):
    base = output_name("macos")
    bundle, _, symbols = macos_paths(NAME)
    if not bundle.is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")
    if args.installer:
        package_macos_installer(args, base)
    else:
        tar_xz(RELEASE_DIR / f"{base}.tar.xz", INSTALL_DIR, [bundle.name])
    if symbols.is_dir():
        tar_xz(RELEASE_DIR / f"{base}-dSYMs.tar.xz", INSTALL_DIR, [symbols.name])


def package_macos_installer(args, base):
    values = plugin()
    name = values["NAME"]
    staging = RELEASE_DIR / "installer"
    root = staging / "root" / "Library" / "Application Support" / "obs-studio" / "plugins"
    bundle, _, _ = macos_paths(name)
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
            staging / f"{name}.pkg",
        ]
    )
    distribution = staging / "distribution.xml"
    render(PACKAGING_DIR / "macos" / "distribution.xml.in", distribution, **values)
    resources = staging / "resources"
    resources.mkdir(parents=True)
    shutil.copy2(PACKAGING_DIR / "macos" / "background.png", resources / "background.png")
    package = RELEASE_DIR / f"{base}.pkg"
    unsigned = staging / f"{name}-distribution.pkg"
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
        notarize(package, name, args)


def notarize(package, name, args):
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


def package_linux(args):
    base = output_name("linux")
    if not (INSTALL_DIR / "lib").is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")
    tar_xz(RELEASE_DIR / f"{base}.tar.xz", INSTALL_DIR, ["lib", "share"])
    source_tarball()
    if args.installer:
        package_deb(base, NAME)


def package_deb(base, name):
    values = plugin()
    staging = RELEASE_DIR / "deb"
    remove(staging)
    shutil.copytree(INSTALL_DIR, staging / "usr", symlinks=True)
    architecture = subprocess.run(
        ["dpkg", "--print-architecture"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    render(
        PACKAGING_DIR / "linux" / "control.in",
        staging / "DEBIAN" / "control",
        ARCHITECTURE=architecture,
        **values,
    )
    package = RELEASE_DIR / f"{base}.deb"
    remove(package)
    run(["dpkg-deb", "--build", "--root-owner-group", staging, package])
    remove(staging)


def source_tarball():
    values = plugin()
    base = f"{values['NAME']}-{values['VERSION']}-source"
    archive = RELEASE_DIR / f"{base}.tar.xz"
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    sources = subprocess.run(
        ["git", "archive", f"--prefix={base}/", "--format=tar", "HEAD"],
        check=True,
        capture_output=True,
        cwd=ROOT,
    ).stdout
    with lzma.open(archive, "wb") as fout:
        fout.write(sources)


def package_windows(args):
    base = output_name("windows")
    if not (INSTALL_DIR / NAME).is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")
    archive = RELEASE_DIR / f"{base}.zip"
    remove(archive)
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zip_file:
        for path in sorted((INSTALL_DIR / NAME).rglob("*")):
            if path.is_file():
                zip_file.write(path, path.relative_to(INSTALL_DIR))
    if args.installer:
        package_windows_installer(base)


def find_inno_setup():
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


def package_windows_installer(base):
    values = plugin()
    script = RELEASE_DIR / "installer.iss"
    render(
        PACKAGING_DIR / "windows" / "installer.iss.in",
        script,
        SOURCE_DIR=ROOT,
        INSTALL_DIR=INSTALL_DIR,
        OUTPUT_DIR=RELEASE_DIR,
        OUTPUT_NAME=f"{base}-Installer",
        **values,
    )
    run([find_inno_setup(), script, f"/DReleaseDir={INSTALL_DIR}"])
    remove(script)


def package(args):
    target_platform = host_platform()
    if target_platform == "macos":
        package_macos(args)
    elif target_platform == "windows":
        package_windows(args)
    else:
        package_linux(args)


def install(_):
    target_platform = host_platform()
    if target_platform == "macos":
        destination = Path.home() / "Library/Application Support/obs-studio/plugins"
        source, _, _ = macos_paths(NAME)
        destination.mkdir(parents=True, exist_ok=True)
        remove(destination / source.name)
        shutil.copytree(source, destination / source.name, symlinks=True)
    elif target_platform == "linux":
        destination = Path.home() / ".config" / "obs-studio" / "plugins" / NAME
        remove(destination)
        (destination / "bin" / "64bit").mkdir(parents=True)
        shutil.copy2(
            INSTALL_DIR / "lib" / f"{platform.machine()}-linux-gnu" / "obs-plugins" / f"{NAME}.so",
            destination / "bin" / "64bit" / f"{NAME}.so",
        )
        copy_data(destination / "data")
    else:
        raise Error("installing is only supported on macOS and Linux; use the installer instead")


def clean(_):
    for path in [RELEASE_DIR, cargo_target_dir()]:
        if path.exists():
            remove(path)


def main():
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    deps_parser = subparsers.add_parser("deps")
    deps_parser.set_defaults(function=lambda args: dependencies())
    build_parser = subparsers.add_parser("build")
    build_parser.add_argument("--debug", action="store_true")
    build_parser.add_argument(
        "--codesign-application-identity",
        default="",
        help="macOS application signing identity; ad-hoc signed when omitted",
    )
    build_parser.set_defaults(function=build)
    package_parser = subparsers.add_parser("package")
    package_parser.add_argument("--installer", action="store_true")
    package_parser.add_argument(
        "--codesign-application-identity",
        default="",
        help="macOS application signing identity; the notarization team id is taken from it",
    )
    package_parser.add_argument(
        "--codesign-installer-identity",
        default="",
        help="macOS installer signing identity; the installer is unsigned when omitted",
    )
    package_parser.add_argument(
        "--notarization-user",
        default="",
        help="Apple ID to notarize the installer with; not notarized when omitted",
    )
    package_parser.add_argument(
        "--notarization-password",
        default="",
        help="app-specific password for --notarization-user",
    )
    package_parser.set_defaults(function=package)
    install_parser = subparsers.add_parser("install")
    install_parser.set_defaults(function=install)
    clean_parser = subparsers.add_parser("clean")
    clean_parser.set_defaults(function=clean)
    args = parser.parse_args()
    try:
        args.function(args)
    except Error as error:
        sys.exit(f"error: {error}")


if __name__ == "__main__":
    main()
