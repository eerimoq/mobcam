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
        self.config_toml = find_config_toml()
        self._config: dict[str, Any] = tomllib.loads(self.config_toml.read_text())

    def general(self) -> dict[str, Any]:
        general: dict[str, Any] = self._config["general"]
        return general

    def remote_control_port(self) -> int:
        port: int = self.general()["remote-control-port"]
        return port

    def tester_ip_address(self) -> str:
        address: str = self.general()["tester-ip-address"]
        return address

    def virtualcam_service(self) -> str:
        service: str = self._virtualcam()["service"]
        return service

    def video_device(self) -> str:
        device: str = self._virtualcam()["video-device"]
        return device

    def audio_playback_device(self) -> str:
        device: str = self._virtualcam()["audio-playback-device"]
        return device

    def audio_capture_device(self) -> str:
        device: str = self._virtualcam()["audio-capture-device"]
        return device

    def _virtualcam(self) -> dict[str, Any]:
        virtualcam: dict[str, Any] = self._config["virtualcam"]
        return virtualcam
