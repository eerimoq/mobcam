import json
import zipfile
from enum import StrEnum
from pathlib import Path
from typing import Any
from uuid import uuid4

from .config import MOBCAM_URL
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


def base_settings(config: Config, remote_control_port: int) -> dict[str, Any]:
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
        "debug": {
            "logLevel": "Debug",
            "builtinAudioAndVideoDelay": 0,
            "builtinAudioAndVideoDelay70msMigrated": True,
        },
        "show": {"stream": True, "cpu": True, "microphone": True, "cameras": True},
    }


def stream_settings(video_codec: VideoCodec, resolution: Resolution, fps: int) -> dict[str, Any]:
    return {
        "id": uuid(),
        "name": "Mobcam",
        "enabled": True,
        "url": MOBCAM_URL,
        "codec": video_codec,
        "audioCodec": AudioCodec.AAC,
        "resolution": resolution,
        "fps": fps,
        "bitrate": 5_000_000,
        "bitrateRateControl": "CBR",
    }


def create_settings_file(
    settings: dict[str, Any], output_file: Path, files: dict[str, Path] | None = None
) -> None:
    with zipfile.ZipFile(output_file, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("settings.json", json.dumps(settings, indent=4))
        for name, path in (files or {}).items():
            archive.write(path, name)
