import json
import zipfile
from enum import StrEnum
from pathlib import Path
from uuid import uuid4

from .config import REMOTE_CONTROL_PASSWORD
from .config import Config


class SceneName(StrEnum):
    BACK = "Back"
    FRONT = "Front"


class CameraPosition(StrEnum):
    BACK = "Back"
    FRONT = "Front"


class VideoCodec(StrEnum):
    H264 = "H.264/AVC"
    H265 = "H.265/HEVC"


class AudioCodec(StrEnum):
    AAC = "AAC"
    OPUS = "OPUS"


class Resolution(StrEnum):
    HD = "1280x720"
    FULL_HD = "1920x1080"
    QUAD_HD = "2560x1440"
    ULTRA_HD = "3840x2160"

    def size(self) -> tuple[int, int]:
        width, height = self.split("x")
        return int(width), int(height)


def uuid() -> str:
    return str(uuid4()).upper()


def base_settings(config: Config, remote_control_port: int):
    return {
        "scenes": [
            {
                "name": SceneName.BACK,
                "cameraPosition": CameraPosition.BACK,
                "enabled": True,
            }
        ],
        "remoteControl": {
            "server": {
                "enabled": True,
                "url": f"ws://{config.tester_ip_address()}:{remote_control_port}",
            },
            "password": REMOTE_CONTROL_PASSWORD,
            "hasMigratedAssistant": True,
        },
        "verboseStatuses": True,
        "showAllSettings": True,
        "debug": {"logLevel": "Debug"},
        "show": {"stream": True, "cpu": True, "microphone": True, "cameras": True},
    }


def create_settings_file(settings, output_file: Path, files: dict[str, Path] | None = None):
    with zipfile.ZipFile(output_file, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("settings.json", json.dumps(settings, indent=4))
        for name, path in (files or {}).items():
            archive.write(path, name)
