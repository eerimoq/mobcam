import tomllib
from pathlib import Path
from typing import Any

from xdg_base_dirs import xdg_config_home

from .utils import TEST_DIR

REMOTE_CONTROL_PASSWORD = "1234"
MOBCAM_PORT = 7790
MOBCAM_URL = f"mobcam://localhost:{MOBCAM_PORT}"


def find_config_toml() -> Path:
    paths = [TEST_DIR / "config.toml", xdg_config_home() / "mobcam" / "tests" / "config.toml"]
    for path in paths:
        if path.exists():
            return path
    found = " or ".join(f"'{path}'" for path in paths)
    raise Exception(f"No configuration file found. Create {found}.")


class Config:
    def __init__(self) -> None:
        self._config: dict[str, Any] = tomllib.loads(find_config_toml().read_text())

    def remote_control_port(self) -> int:
        port: int = self._config["general"]["remote-control-port"]
        return port

    def tester_ip_address(self) -> str:
        return self._string("general", "tester-ip-address")

    def video_device(self) -> str:
        return self._string("virtualcam", "video-device")

    def audio_playback_device(self) -> str:
        return self._string("virtualcam", "audio-playback-device")

    def audio_capture_device(self) -> str:
        return self._string("virtualcam", "audio-capture-device")

    def _string(self, section: str, key: str) -> str:
        value: str = self._config[section][key]
        return value
