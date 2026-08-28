import tomllib
from pathlib import Path

from xdg_base_dirs import xdg_config_home

from .utils import TEST_DIR

REMOTE_CONTROL_PASSWORD = "1234"
MOBCAM_URL = "mobcam://localhost:7790"


def find_config_toml() -> Path:
    paths = [TEST_DIR / "config.toml", xdg_config_home() / "mobcam" / "tests" / "config.toml"]
    for path in paths:
        if path.exists():
            return path
    found = " or ".join(f"'{path}'" for path in paths)
    raise Exception(f"No configuration file found. Create {found}.")


class Config:
    def __init__(self):
        self.config_toml = find_config_toml()
        self._config = tomllib.loads(self.config_toml.read_text())

    def general(self):
        return self._config["general"]

    def remote_control_port(self) -> int:
        return self.general()["remote-control-port"]

    def tester_ip_address(self) -> str:
        return self.general()["tester-ip-address"]

    def virtualcam_service(self) -> str:
        return self._virtualcam()["service"]

    def video_device(self) -> str:
        return self._virtualcam()["video-device"]

    def audio_playback_device(self) -> str:
        return self._virtualcam()["audio-playback-device"]

    def audio_capture_device(self) -> str:
        return self._virtualcam()["audio-capture-device"]

    def _virtualcam(self):
        return self._config["virtualcam"]
