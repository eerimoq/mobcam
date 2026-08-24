#!/usr/bin/env python3

"""Build, bundle and package the MobCam OBS plugin.

    python3 build.py deps        download the prebuilt dependencies
    python3 build.py build       compile the plugin and stage it in release/
    python3 build.py package     archive the staged plugin, and build installers
    python3 build.py install     install the staged plugin for the current user
    python3 build.py clean       remove release/ and the cargo target directory

cargo compiles and links the plugin itself, so what is left for this script is
everything around it: the dependencies buildspec.json names, the macOS bundle,
the code signing and the installers. buildspec.json is the single source of
truth for the plugin name, version and dependency versions; build.rs checks the
version in Cargo.toml against it.
"""

import argparse
import hashlib
import json
import lzma
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DEPS_DIR = ROOT / ".deps"
RELEASE_DIR = ROOT / "release"

# Everything that would be installed, laid out exactly as it is installed. The
# archives and the installers are all made from this one tree.
INSTALL_DIR = RELEASE_DIR / "install"

PACKAGING_DIR = ROOT / "packaging"
DATA_DIR = ROOT / "data"

# The oldest macOS the plugin runs on. Both the bundle and the installer state
# it, so the installer refuses to install what would not load.
MACOS_DEPLOYMENT_TARGET = "12.0"

# OBS ships a universal application, so the plugin has to be universal too.
# cargo builds one architecture at a time; lipo joins the two afterwards.
MACOS_TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}

# The names of the archives to download and of the directories they are
# unpacked into, per platform and dependency. The obs-studio source is only
# needed for the libobs headers: nothing links against libobs on macOS, and on
# Windows build.rs generates the import library from the headers themselves.
DEPENDENCIES = {
    "macos": {
        "prebuilt": ("macos-deps-{version}-universal.tar.xz", "obs-deps-{version}-universal"),
        "obs-studio": ("{version}.tar.gz", "obs-studio-{version}"),
    },
    "windows": {
        "prebuilt": ("windows-deps-{version}-x64.zip", "obs-deps-{version}-x64"),
        "obs-studio": ("{version}.zip", "obs-studio-{version}"),
    },
}


class Error(Exception):
    """Anything the user can fix, reported without a traceback."""


def log(message):
    print(f"==> {message}", flush=True)


def run(command, **kwargs):
    command = [str(argument) for argument in command]
    print(f"    {' '.join(command)}", flush=True)

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


def buildspec():
    with open(ROOT / "buildspec.json", encoding="utf-8") as fin:
        return json.load(fin)


def plugin(spec):
    """The values the bundle, the packages and the installers are filled in
    with, all of them from buildspec.json."""

    name = spec["name"]

    return {
        "NAME": name,
        "DISPLAY_NAME": spec.get("displayName", name),
        "VERSION": spec["version"],
        "AUTHOR": spec["author"],
        "EMAIL": spec["email"],
        "WEBSITE": spec["website"],
        "BUNDLE_ID": spec["platformConfig"]["macos"]["bundleId"],
        "DEPLOYMENT_TARGET": MACOS_DEPLOYMENT_TARGET,
        "YEAR": time.strftime("%Y"),
    }


def render(template, output, **values):
    """Fill an @KEY@ template in. The templates are the installer scripts and
    the bundle's Info.plist, none of which cargo knows anything about."""

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


def output_name(spec, target_platform):
    """The name every artifact for a platform is built around. The release
    workflow picks the files up by these names."""

    version = spec["version"]
    name = spec["name"]

    if target_platform == "macos":
        return f"{name}-{version}-macos-universal"
    elif target_platform == "windows":
        return f"{name}-{version}-windows-x64"
    else:
        return f"{name}-{version}-{platform.machine()}-linux-gnu"


def download(url, path, sha256):
    log(f"Downloading {url}")

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
            f"{url} does not have the hash buildspec.json expects:\n"
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
            # The dependency archives are symlink farms, which the data filter
            # keeps as long as they stay inside the archive, as these do.
            if sys.version_info >= (3, 12):
                tar_file.extractall(destination, filter="data")
            else:
                tar_file.extractall(destination)


def dependencies(target_platform=None):
    """Download and unpack what buildspec.json names for the platform.

    A marker file next to each unpacked dependency records the hash it was
    unpacked from, so a dependency whose version changed is replaced rather than
    unpacked on top of the old one.
    """

    target_platform = target_platform or host_platform()

    if target_platform == "linux":
        log("Linux uses the distribution's libobs and FFmpeg; nothing to download")
        return

    spec = buildspec()
    hash_key = "macos" if target_platform == "macos" else "windows-x64"

    for dependency, (archive_name, directory_name) in DEPENDENCIES[target_platform].items():
        data = spec["dependencies"][dependency]
        version = data["version"]
        sha256 = data["hashes"][hash_key]
        archive = DEPS_DIR / archive_name.format(version=version)
        directory = DEPS_DIR / directory_name.format(version=version)
        marker = DEPS_DIR / f".dependency_{dependency}.sha256"

        if directory.is_dir() and marker.is_file() and marker.read_text().strip() == sha256:
            log(f"{data['label']} {version} is up to date")
            continue

        if not archive.is_file():
            if dependency == "obs-studio":
                url = f"{data['baseUrl']}/{archive.name}"
            else:
                url = f"{data['baseUrl']}/{version}/{archive.name}"

            download(url, archive, sha256)

        log(f"Unpacking {data['label']} {version}")
        remove(directory)

        # The obs-studio archives carry their own top level directory; the
        # obs-deps ones unpack straight into the prefix they describe.
        if dependency == "obs-studio":
            extract(archive, DEPS_DIR)
        else:
            extract(archive, directory)

        marker.write_text(sha256 + "\n")


def cargo_target_dir():
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))


def cargo_executable():
    """Find the rustup managed cargo, in preference to any other on the PATH.

    It is the one that honours rust-toolchain.toml, and the only one that has
    the second architecture of the macOS universal binary installed. A cargo
    from a package manager has neither, and quietly builds something else.
    """

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
    """What cargo calls the shared library it builds, which is the [lib] name in
    Cargo.toml with the platform's prefix and suffix around it."""

    if target_platform == "macos":
        return f"lib{name}.dylib"
    elif target_platform == "windows":
        return f"{name}.dll"
    else:
        return f"lib{name}.so"


def cargo_build(target_platform, name, debug, targets):
    """Build the plugin, one cargo invocation per architecture, and return the
    libraries that came out."""

    profile = "dev" if debug else "release"
    profile_directory = "debug" if debug else "release"
    cargo = cargo_executable()
    environment = dict(os.environ)
    libraries = []

    # The cargo shim runs a bare "rustc" from the PATH, so a different rust
    # installation earlier on it would quietly supply the compiler, and with it
    # a toolchain that has no second architecture installed.
    environment["PATH"] = os.pathsep.join([str(cargo.parent), environment.get("PATH", "")])

    if target_platform == "macos":
        environment["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET

    for target in targets:
        command = [cargo, "build", "--locked", "--profile", profile]

        if target is not None:
            command += ["--target", target]

        log(f"Building {target or 'the plugin'}")
        run(command, cwd=ROOT, env=environment)

        directory = cargo_target_dir()

        if target is not None:
            directory /= target

        libraries.append(directory / profile_directory / library_name(target_platform, name))

    return libraries


def copy_data(destination):
    """The locale files and anything else next to them, which OBS looks for in
    the plugin's data directory."""

    if not DATA_DIR.is_dir():
        return

    shutil.copytree(DATA_DIR, destination, dirs_exist_ok=True)


def codesign(path, identity=None, entitlements=None):
    """Sign for the hardened runtime, which notarization requires. An ad-hoc
    signature is what a build without credentials gets, and is enough for the
    plugin to load on Apple silicon."""

    identity = identity or os.environ.get("CODESIGN_IDENT") or "-"
    command = ["codesign", "--force", "--sign", identity, "--options", "runtime"]

    if identity != "-":
        # An ad-hoc signature cannot carry a trusted timestamp.
        command.append("--timestamp")

    if entitlements is not None:
        command += ["--entitlements", entitlements]

    run(command + [path])


def build_macos(spec, debug):
    """Assemble the plugin bundle, which is what OBS loads on macOS: the
    universal binary, the data files and an Info.plist naming the two."""

    values = plugin(spec)
    name = values["NAME"]
    bundle = INSTALL_DIR / f"{name}.plugin"
    binary = bundle / "Contents" / "MacOS" / name

    libraries = cargo_build("macos", name, debug, sorted(MACOS_TARGETS.values()))

    remove(bundle)
    binary.parent.mkdir(parents=True)

    log("Creating the universal binary")
    run(["lipo", "-create", *libraries, "-output", binary])

    # cargo has no notion of where the library ends up, so it records the path
    # it was built at. Nothing loads the plugin by that name, but it should not
    # name a directory on the build machine either.
    run(["install_name_tool", "-id", f"@rpath/{name}", binary])

    render(
        PACKAGING_DIR / "macos" / "Info.plist.in",
        bundle / "Contents" / "Info.plist",
        **values,
    )
    copy_data(bundle / "Contents" / "Resources")

    # A debug build keeps its symbols in the binary, and must not leave the
    # symbols of an earlier release build behind for the packaging to pick up.
    symbols = INSTALL_DIR / f"{name}.plugin.dSYM"
    remove(symbols)

    if not debug:
        log("Separating the debug symbols")
        run(["dsymutil", binary, "-o", symbols])
        run(["strip", "-x", binary])

    log("Signing the plugin")
    codesign(bundle, entitlements=entitlements_file())

    return bundle


def entitlements_file():
    entitlements = PACKAGING_DIR / "macos" / "entitlements.plist"

    return entitlements if entitlements.is_file() else None


def build_linux(spec, debug):
    """Stage the shared object and its data the way a package installs them,
    under the same prefix OBS is installed with."""

    name = spec["name"]
    (library,) = cargo_build("linux", name, debug, [None])

    # The Debian style multiarch directory, which is where OBS looks for
    # plugins on the distributions this is packaged for.
    library_dir = INSTALL_DIR / "lib" / f"{platform.machine()}-linux-gnu" / "obs-plugins"

    remove(INSTALL_DIR / "lib")
    remove(INSTALL_DIR / "share")
    library_dir.mkdir(parents=True)

    # OBS loads plugins by file name, without the lib prefix cargo gives them.
    shutil.copy2(library, library_dir / f"{name}.so")
    copy_data(INSTALL_DIR / "share" / "obs" / "obs-plugins" / name)

    return library_dir / f"{name}.so"


def build_windows(spec, debug):
    """Stage the DLL, its debug symbols and its data in the layout OBS expects
    a plugin directory to have."""

    name = spec["name"]
    (library,) = cargo_build("windows", name, debug, [None])

    root = INSTALL_DIR / name
    binary_dir = root / "bin" / "64bit"

    remove(root)
    binary_dir.mkdir(parents=True)
    shutil.copy2(library, binary_dir / f"{name}.dll")

    symbols = library.with_suffix(".pdb")

    if symbols.is_file():
        shutil.copy2(symbols, binary_dir / f"{name}.pdb")

    copy_data(root / "data")

    return binary_dir / f"{name}.dll"


def build(arguments):
    dependencies()

    if arguments.codesign and not os.environ.get("CODESIGN_IDENT"):
        raise Error("--codesign needs a signing identity in CODESIGN_IDENT")

    spec = buildspec()
    target_platform = host_platform()

    INSTALL_DIR.mkdir(parents=True, exist_ok=True)

    if target_platform == "macos":
        artifact = build_macos(spec, arguments.debug)
    elif target_platform == "windows":
        artifact = build_windows(spec, arguments.debug)
    else:
        artifact = build_linux(spec, arguments.debug)

    log(f"Built {artifact}")


def tar_xz(archive, directory, members):
    """The archives are the same tar.xz the plugin has always been distributed
    as, so an existing installation can be updated by unpacking one."""

    log(f"Creating {archive.name}")
    archive.parent.mkdir(parents=True, exist_ok=True)
    remove(archive)

    with tarfile.open(archive, "w:xz") as tar_file:
        for member in members:
            tar_file.add(directory / member, arcname=member)


def package_macos(spec, arguments):
    values = plugin(spec)
    name = values["NAME"]
    base = output_name(spec, "macos")
    bundle = INSTALL_DIR / f"{name}.plugin"
    symbols = INSTALL_DIR / f"{name}.plugin.dSYM"

    if not bundle.is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")

    if arguments.installer:
        package_macos_installer(spec, arguments, base)
    else:
        tar_xz(RELEASE_DIR / f"{base}.tar.xz", INSTALL_DIR, [bundle.name])

    if symbols.is_dir():
        tar_xz(RELEASE_DIR / f"{base}-dSYMs.tar.xz", INSTALL_DIR, [symbols.name])


def package_macos_installer(spec, arguments, base):
    """Build the installer package: a component package holding the bundle,
    wrapped in a distribution package that refuses too old a macOS."""

    values = plugin(spec)
    name = values["NAME"]
    staging = RELEASE_DIR / "installer"
    root = staging / "root" / "Library" / "Application Support" / "obs-studio" / "plugins"

    remove(staging)
    root.mkdir(parents=True)
    shutil.copytree(
        INSTALL_DIR / f"{name}.plugin",
        root / f"{name}.plugin",
        symlinks=True,
    )

    log("Building the installer package")
    run(
        [
            "pkgbuild",
            "--identifier",
            values["BUNDLE_ID"],
            "--version",
            values["VERSION"],
            "--root",
            staging / "root",
            staging / f"{name}.pkg",
        ]
    )

    distribution = staging / "distribution.xml"
    render(PACKAGING_DIR / "macos" / "distribution.xml.in", distribution, **values)

    package = RELEASE_DIR / f"{base}.pkg"
    unsigned = staging / f"{name}-distribution.pkg"

    run(
        [
            "productbuild",
            "--distribution",
            distribution,
            "--package-path",
            staging,
            unsigned,
        ]
    )

    remove(package)

    installer_identity = os.environ.get("CODESIGN_IDENT_INSTALLER")

    if arguments.codesign:
        if not installer_identity:
            raise Error("--codesign needs an installer identity in CODESIGN_IDENT_INSTALLER")

        log("Signing the installer package")
        run(["productsign", "--sign", installer_identity, unsigned, package])
    else:
        unsigned.replace(package)

    remove(staging)

    if arguments.notarize:
        notarize(package, name)


def notarize(package, name):
    """Hand the package to Apple and staple the ticket to it, so that Gatekeeper
    accepts it without asking."""

    user = os.environ.get("CODESIGN_IDENT_USER")
    password = os.environ.get("CODESIGN_IDENT_PASS")
    team = os.environ.get("CODESIGN_TEAM")

    if not team:
        identity = os.environ.get("CODESIGN_IDENT", "")
        team = identity.rpartition("(")[2].rstrip(")")

    if not (user and password and team):
        raise Error("notarization needs CODESIGN_IDENT_USER, CODESIGN_IDENT_PASS and a team")

    profile = f"{name}-Codesign-Password"

    log("Notarizing the installer package")
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


def package_linux(spec, arguments):
    base = output_name(spec, "linux")
    name = spec["name"]

    if not (INSTALL_DIR / "lib").is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")

    tar_xz(RELEASE_DIR / f"{base}.tar.xz", INSTALL_DIR, ["lib", "share"])
    source_tarball(spec)

    if arguments.installer:
        package_deb(spec, base, name)


def package_deb(spec, base, name):
    """Build the Debian package, which is the staged tree under /usr plus the
    control file describing it."""

    values = plugin(spec)
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

    log(f"Creating {package.name}")
    run(["dpkg-deb", "--build", "--root-owner-group", staging, package])
    remove(staging)


def source_tarball(spec):
    """The sources the release carries alongside the binaries, straight out of
    git so that nothing untracked ends up in them."""

    values = plugin(spec)
    base = f"{values['NAME']}-{values['VERSION']}-source"
    archive = RELEASE_DIR / f"{base}.tar.xz"

    log(f"Creating {archive.name}")
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)

    sources = subprocess.run(
        ["git", "archive", f"--prefix={base}/", "--format=tar", "HEAD"],
        check=True,
        capture_output=True,
        cwd=ROOT,
    ).stdout

    with lzma.open(archive, "wb") as fout:
        fout.write(sources)


def package_windows(spec, arguments):
    base = output_name(spec, "windows")
    name = spec["name"]

    if not (INSTALL_DIR / name).is_dir():
        raise Error("no staged plugin found; run `python3 build.py build` first")

    archive = RELEASE_DIR / f"{base}.zip"
    remove(archive)

    log(f"Creating {archive.name}")
    RELEASE_DIR.mkdir(parents=True, exist_ok=True)

    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zip_file:
        for path in sorted((INSTALL_DIR / name).rglob("*")):
            if path.is_file():
                zip_file.write(path, path.relative_to(INSTALL_DIR))

    if arguments.installer:
        package_windows_installer(spec, base)


def find_inno_setup():
    """Inno Setup is preinstalled on the GitHub runners, but not always on the
    PATH, so fall back to the places its installer puts it."""

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


def package_windows_installer(spec, base):
    values = plugin(spec)
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

    log("Building the installer")
    run([find_inno_setup(), script, f"/DReleaseDir={INSTALL_DIR}"])
    remove(script)


def package(arguments):
    spec = buildspec()
    target_platform = host_platform()

    if target_platform == "macos":
        package_macos(spec, arguments)
    elif target_platform == "windows":
        package_windows(spec, arguments)
    else:
        package_linux(spec, arguments)


def install(arguments):
    """Copy the staged plugin into the OBS plugin directory of the current
    user, which is where a plugin under development belongs."""

    spec = buildspec()
    name = spec["name"]
    target_platform = host_platform()

    if target_platform == "macos":
        destination = Path.home() / "Library/Application Support/obs-studio/plugins"
        source = INSTALL_DIR / f"{name}.plugin"
        destination.mkdir(parents=True, exist_ok=True)
        remove(destination / source.name)
        shutil.copytree(source, destination / source.name, symlinks=True)
        log(f"Installed {destination / source.name}")
    elif target_platform == "linux":
        destination = Path.home() / ".config" / "obs-studio" / "plugins" / name
        remove(destination)
        (destination / "bin" / "64bit").mkdir(parents=True)
        shutil.copy2(
            INSTALL_DIR / "lib" / f"{platform.machine()}-linux-gnu" / "obs-plugins" / f"{name}.so",
            destination / "bin" / "64bit" / f"{name}.so",
        )
        copy_data(destination / "data")
        log(f"Installed {destination}")
    else:
        raise Error("installing is only supported on macOS and Linux; use the installer instead")


def clean(arguments):
    for path in [RELEASE_DIR, cargo_target_dir()]:
        if path.exists():
            log(f"Removing {path}")
            remove(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subparsers = parser.add_subparsers(dest="command", required=True)

    deps_parser = subparsers.add_parser("deps", help="download the prebuilt dependencies")
    deps_parser.set_defaults(function=lambda arguments: dependencies())

    build_parser = subparsers.add_parser("build", help="compile and stage the plugin")
    build_parser.add_argument("--debug", action="store_true", help="build without optimizations")
    build_parser.add_argument(
        "--codesign",
        action="store_true",
        help="sign with CODESIGN_IDENT rather than ad-hoc (macOS)",
    )
    build_parser.set_defaults(function=build)

    package_parser = subparsers.add_parser("package", help="archive the staged plugin")
    package_parser.add_argument(
        "--installer",
        action="store_true",
        help="build the installer package instead of the archive",
    )
    package_parser.add_argument(
        "--codesign",
        action="store_true",
        help="sign the installer with CODESIGN_IDENT_INSTALLER (macOS)",
    )
    package_parser.add_argument(
        "--notarize",
        action="store_true",
        help="submit the installer package to Apple for notarization (macOS)",
    )
    package_parser.set_defaults(function=package)

    install_parser = subparsers.add_parser("install", help="install for the current user")
    install_parser.set_defaults(function=install)

    clean_parser = subparsers.add_parser("clean", help="remove the build output")
    clean_parser.set_defaults(function=clean)

    arguments = parser.parse_args()

    try:
        arguments.function(arguments)
    except Error as error:
        sys.exit(f"error: {error}")


if __name__ == "__main__":
    main()
