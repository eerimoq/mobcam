import platform
import shutil
import subprocess
import sys
from pathlib import Path

from .config import Config


def _is_executable_in_path(name: str) -> bool:
    return shutil.which(name) is not None


def _is_readable(path: Path) -> bool:
    try:
        with path.open("rb"):
            return True
    except OSError:
        return False


def _has_passwordless_sudo() -> bool:
    return subprocess.run(["sudo", "-n", "true"], check=False, capture_output=True).returncode == 0


def _check_machine() -> list[str]:
    if platform.system() != "Linux":
        return ["The tests must run on the Linux machine the device is connected to"]
    config = Config()
    missing = []
    video_device = Path(config.video_device())
    if not video_device.exists():
        missing.append(f"{video_device} does not exist; load the v4l2loopback module")
    elif not _is_readable(video_device):
        missing.append(f"{video_device} is not readable; add the user to the video group")
    if not Path(config.virtualcam_binary()).exists():
        missing.append(f"{config.virtualcam_binary()} does not exist; install mobcam-virtualcam")
    if not _has_passwordless_sudo():
        missing.append("sudo asks for a password; add the user to the sudoers with NOPASSWD")
    return missing


def check_dependencies():
    missing_dependencies = []
    for executable in ["ffmpeg", "ffprobe"]:
        if not _is_executable_in_path(executable):
            missing_dependencies.append(f"{executable} executable not found")
    missing_dependencies += _check_machine()
    if len(missing_dependencies) > 0:
        print("--- Missing dependencies ---")
        print()
        for missing_dependency in missing_dependencies:
            print("  -", missing_dependency)
        print()
        sys.exit(1)
