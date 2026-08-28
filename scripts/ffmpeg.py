#!/usr/bin/env python3

import argparse
import hashlib
import os
import shutil
import zipfile
from pathlib import Path

from build import DEPS_DIR
from build import MACOS_DEPLOYMENT_TARGET
from build import MACOS_TARGETS
from build import PROJECT
from build import RELEASE_DIR
from build import Command
from build import Error
from build import Platform
from build import download
from build import extract_stripped
from build import host_platform
from build import remove
from build import run
from build import tar_xz

FFMPEG_VERSION = "8.0"
FFMPEG_URL = f"https://ffmpeg.org/releases/ffmpeg-{FFMPEG_VERSION}.tar.xz"
FFMPEG_SHA256 = "b2751fccb6cc4c77708113cd78b561059b6fa904b24162fa0be2d60273d27b8e"
BUILD_DIR = DEPS_DIR / "ffmpeg-build"
SOURCE_DIR = BUILD_DIR / f"ffmpeg-{FFMPEG_VERSION}"
LIBRARIES = ["avcodec", "avutil"]
DECODERS = ["h264", "hevc", "aac"]
PARSERS = ["h264", "hevc", "aac"]
MACOS_HWACCELS = ["h264_videotoolbox", "hevc_videotoolbox"]
WINDOWS_HWACCELS = [
    "h264_d3d11va",
    "h264_d3d11va2",
    "h264_dxva2",
    "hevc_d3d11va",
    "hevc_d3d11va2",
    "hevc_dxva2",
]
LICENSES = ["COPYING.LGPLv2.1", "COPYING.LGPLv3", "LICENSE.md", "CREDITS"]
MEMBERS = ["include", "lib", "licenses", "link.txt"]
COMMON_FLAGS = [
    "--disable-everything",
    "--disable-autodetect",
    "--enable-static",
    "--disable-shared",
    "--enable-pic",
    "--disable-programs",
    "--disable-doc",
    "--disable-avdevice",
    "--disable-avformat",
    "--disable-avfilter",
    "--disable-swscale",
    "--disable-swresample",
    "--disable-network",
    "--disable-iconv",
    "--disable-zlib",
    "--disable-lzma",
    "--disable-bzlib",
    "--disable-debug",
    f"--enable-decoder={','.join(DECODERS)}",
    f"--enable-parser={','.join(PARSERS)}",
]


def output_name(target_platform: Platform) -> str:
    if target_platform == "macos":
        return f"{PROJECT}-ffmpeg-{FFMPEG_VERSION}-macos-universal"
    elif target_platform == "windows":
        return f"{PROJECT}-ffmpeg-{FFMPEG_VERSION}-windows-x64"
    else:
        raise Error(f"{target_platform} uses the distribution's FFmpeg, nothing to build")


def source() -> Path:
    if SOURCE_DIR.is_dir():
        return SOURCE_DIR
    archive = BUILD_DIR / FFMPEG_URL.rsplit("/", 1)[1]
    if not archive.is_file():
        download(FFMPEG_URL, archive, FFMPEG_SHA256)
    extract_stripped(archive, SOURCE_DIR)
    return SOURCE_DIR


def platform_flags(target_platform: Platform, arch: str) -> list[str]:
    if target_platform == "macos":
        target = f"-arch {arch} -mmacosx-version-min={MACOS_DEPLOYMENT_TARGET}"
        flags = [
            "--enable-videotoolbox",
            f"--enable-hwaccel={','.join(MACOS_HWACCELS)}",
            f"--arch={arch}",
            f"--extra-cflags={target}",
            f"--extra-ldflags={target}",
        ]
        if arch != os.uname().machine:
            flags.append("--enable-cross-compile")
        return flags
    return [
        "--toolchain=msvc",
        "--enable-d3d11va",
        "--enable-dxva2",
        f"--enable-hwaccel={','.join(WINDOWS_HWACCELS)}",
    ]


def bash(command: str, cwd: Path) -> None:
    shell: Command = ["bash", "-c", command]
    if host_platform() == "windows":
        shell = [msys2_bash(), "-lc", command]
    run(shell, cwd=cwd)


def msys2_bash() -> str:
    for candidate in [os.environ.get("MSYS2_BASH"), "C:/msys64/usr/bin/bash.exe"]:
        if candidate and Path(candidate).is_file():
            return candidate
    found = shutil.which("bash")
    if found is None:
        raise Error("MSYS2 is required to configure FFmpeg on Windows; set MSYS2_BASH to its bash")
    return found


def require_nasm() -> None:
    if shutil.which("nasm") is None:
        raise Error("nasm is required to assemble the x86 code; install it and build again")


def configure_and_make(target_platform: Platform, arch: str, prefix: Path) -> None:
    if arch in ["x86_64", "x86"]:
        require_nasm()
    build = BUILD_DIR / f"build-{arch}"
    remove(build)
    build.mkdir(parents=True)
    remove(prefix)
    flags = [f"--prefix={prefix}", *COMMON_FLAGS, *platform_flags(target_platform, arch)]
    if target_platform == "macos":
        os.environ["MACOSX_DEPLOYMENT_TARGET"] = MACOS_DEPLOYMENT_TARGET
    configure = " ".join([str(source() / "configure"), *[f"'{flag}'" for flag in flags]])
    bash(configure, build)
    bash(f"make -j{os.cpu_count() or 4}", build)
    bash("make install", build)


def library_files(prefix: Path) -> list[Path]:
    libraries = []
    for name in LIBRARIES:
        matches = sorted(
            path
            for path in (prefix / "lib").iterdir()
            if path.stem in [name, f"lib{name}"] and path.suffix in [".a", ".lib"]
        )
        if not matches:
            raise Error(f"FFmpeg did not install a static library for {name} in {prefix / 'lib'}")
        libraries.append(matches[0])
    return libraries


def link_libraries(prefix: Path) -> list[str]:
    libraries: list[str] = []
    for name in LIBRARIES:
        text = (prefix / "lib" / "pkgconfig" / f"lib{name}.pc").read_text(encoding="utf-8")
        for line in text.splitlines():
            if not line.startswith(("Libs:", "Libs.private:")):
                continue
            flags = line.split(":", 1)[1].split()
            index = 0
            while index < len(flags):
                flag = flags[index]
                if flag == "-framework":
                    index += 1
                    libraries.append(f"framework={flags[index]}")
                elif flag.startswith("-l") and flag[2:] not in LIBRARIES:
                    libraries.append(f"dylib={flag[2:]}")
                index += 1
    return sorted(set(libraries))


def stage(target_platform: Platform, prefix: Path, staging: Path) -> None:
    remove(staging)
    shutil.copytree(prefix / "include", staging / "include")
    (staging / "lib").mkdir(parents=True)
    for library in library_files(prefix):
        shutil.copy2(library, staging / "lib" / library.name)
    licenses = staging / "licenses"
    licenses.mkdir()
    for name in LICENSES:
        path = source() / name
        if path.is_file():
            shutil.copy2(path, licenses / name)
    libraries = link_libraries(prefix)
    (staging / "link.txt").write_text("".join(f"{library}\n" for library in libraries), encoding="utf-8")
    print(f"{target_platform} links against {' '.join(libraries)}")


def build_macos(staging: Path) -> None:
    prefixes = {}
    for arch in sorted(MACOS_TARGETS):
        prefix = BUILD_DIR / f"prefix-{arch}"
        configure_and_make("macos", arch, prefix)
        prefixes[arch] = prefix
    universal = BUILD_DIR / "prefix-universal"
    remove(universal)
    shutil.copytree(prefixes["arm64"], universal)
    for library in library_files(prefixes["arm64"]):
        slices = [prefix / "lib" / library.name for prefix in prefixes.values()]
        run(["lipo", "-create", *slices, "-output", universal / "lib" / library.name])
    stage("macos", universal, staging)


def build_windows(staging: Path) -> None:
    prefix = BUILD_DIR / "prefix-x64"
    configure_and_make("windows", "x86_64", prefix)
    stage("windows", prefix, staging)


def zip_directory(archive: Path, directory: Path) -> None:
    archive.parent.mkdir(parents=True, exist_ok=True)
    remove(archive)
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zip_file:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                zip_file.write(path, path.relative_to(directory))


def archive_path() -> Path:
    target_platform = host_platform()
    base = output_name(target_platform)
    suffix = "tar.xz" if target_platform == "macos" else "zip"
    return RELEASE_DIR / f"{base}.{suffix}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as fin:
        while chunk := fin.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def build(_: argparse.Namespace) -> None:
    target_platform = host_platform()
    staging = BUILD_DIR / "staging"
    BUILD_DIR.mkdir(parents=True, exist_ok=True)
    archive = archive_path()
    if target_platform == "macos":
        build_macos(staging)
        tar_xz(archive, staging, MEMBERS)
    else:
        build_windows(staging)
        zip_directory(archive, staging)
    print(f"wrote {archive}")
    print(f"sha256 {sha256(archive)}  <- put this in DEPENDENCIES in scripts/build.py")


def install(_: argparse.Namespace) -> None:
    staging = BUILD_DIR / "staging"
    archive = archive_path()
    if not staging.is_dir() or not archive.is_file():
        raise Error("nothing built; run `python3 scripts/ffmpeg.py build` first")
    destination = DEPS_DIR / "ffmpeg"
    remove(destination)
    shutil.copytree(staging, destination)
    marker = DEPS_DIR / ".dependency_ffmpeg.sha256"
    marker.write_text(sha256(archive), encoding="utf-8")
    print(f"installed {destination}")


def clean(_: argparse.Namespace) -> None:
    remove(BUILD_DIR)


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    build_parser = subparsers.add_parser("build")
    build_parser.set_defaults(function=build)
    install_parser = subparsers.add_parser("install")
    install_parser.set_defaults(function=install)
    clean_parser = subparsers.add_parser("clean")
    clean_parser.set_defaults(function=clean)
    args = parser.parse_args()
    args.function(args)


if __name__ == "__main__":
    main()
