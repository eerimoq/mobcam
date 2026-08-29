import json
import logging
import os
import re
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import Self

from systest import ManagedProcess
from systest import wait_until

from .config import MOBCAM_PORT
from .ffmpeg import FfmpegDuplicateFrame
from .ffmpeg import ffmpeg_duplicate_frames
from .generate_device_settings import Resolution
from .obs_websocket import ObsWebSocket
from .utils import FILES_DIR

LOGGER = logging.getLogger(__name__)
LOGGER_OBS = logging.getLogger(__name__ + ".obs")
BINARY = Path("/Applications/OBS.app/Contents/MacOS/OBS")
CONFIG_DIR = Path.home() / "Library" / "Application Support" / "obs-studio"
WEBSOCKET_CONFIG = CONFIG_DIR / "plugin_config" / "obs-websocket" / "config.json"
SENTINEL_DIR = CONFIG_DIR / ".sentinel"
SELECTION_FILES = [CONFIG_DIR / "user.ini", CONFIG_DIR / "global.ini"]
SELECTION_KEYS = ["Profile", "ProfileDir", "SceneCollection", "SceneCollectionFile"]
PROFILE = "MobcamTest"
COLLECTION = "MobcamTest"
SCENE = "MobcamScene"
SOURCE = "Mobcam"
SOURCE_KIND = "mobcam_source"
WEBSOCKET_HOST = "127.0.0.1"
WEBSOCKET_PORT = 4466
WEBSOCKET_PASSWORD = "mobcam"
SHUTDOWN_SECONDS = 30

PROFILE_TEMPLATE = """[General]
Name={name}

[Output]
Mode=Simple
FilenameFormatting={filename}
OverwriteIfExists=true

[SimpleOutput]
FilePath={path}
RecFormat2=mp4
RecEncoder=x264
RecQuality=HQ
RecAudioEncoder=aac
RecTracks=1
Preset=veryfast

[Video]
BaseCX={width}
BaseCY={height}
OutputCX={width}
OutputCY={height}
FPSType=1
FPSInt={fps}
ScaleType=bicubic
ColorFormat=NV12
ColorSpace=709
ColorRange=Partial

[Audio]
SampleRate=48000
ChannelSetup=Stereo
"""


@dataclass
class ObsRecording:
    seconds: float
    video_path: Path
    fps: int
    render_skipped: int
    output_skipped: int

    def duplicates(self) -> list[FfmpegDuplicateFrame]:
        return ffmpeg_duplicate_frames(self.video_path)


class Obs:
    def __init__(self, resolution: Resolution, fps: int, name: str) -> None:
        self._resolution = resolution
        self._fps = fps
        self._name = name
        self._client = ObsWebSocket(WEBSOCKET_HOST, WEBSOCKET_PORT, WEBSOCKET_PASSWORD)
        self._process: ManagedProcess | None = None
        self._websocket_config: bytes | None = None
        self._selection: dict[Path, str] = {}
        self._scene_item_id = 0

    def __enter__(self) -> Self:
        _check_machine()
        self._write_profile()
        self._write_collection()
        self._enable_websocket()
        self._selection = _read_selection()
        try:
            self._start_process()
        except BaseException:
            self._restore_websocket()
            _restore_selection(self._selection)
            raise
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        self._stop_recording()
        self._client.close()
        self._stop_process()
        self._restore_websocket()
        _restore_selection(self._selection)
        _remove_sentinel()

    def create_source(self, hardware_decode: bool = True, buffering: bool = False) -> None:
        scenes = self._client.request("GetSceneList")["scenes"]
        if not any(scene["sceneName"] == SCENE for scene in scenes):
            self._client.request("CreateScene", {"sceneName": SCENE})
        self._client.request("SetCurrentProgramScene", {"sceneName": SCENE})
        self._scene_item_id = self._client.request(
            "CreateInput",
            {
                "sceneName": SCENE,
                "inputName": SOURCE,
                "inputKind": SOURCE_KIND,
                "inputSettings": {
                    "device": "",
                    "port": MOBCAM_PORT,
                    "hardware_decode": hardware_decode,
                    "buffering": buffering,
                    "clear_on_disconnect": True,
                    "disconnect_when_hidden": False,
                },
            },
        )["sceneItemId"]

    def wait_until_video(self) -> None:
        LOGGER.debug("Waiting for the Mobcam source to output video...")
        wait_until(self._has_video, "the source to output video")

    def record(self, seconds: float) -> ObsRecording:
        before = self._client.request("GetStats")
        self._client.request("StartRecord")
        time.sleep(seconds)
        video_path = Path(self._client.request("StopRecord")["outputPath"])
        wait_until(self._is_not_recording, "the recording to stop")
        after = self._client.request("GetStats")
        return ObsRecording(
            seconds=seconds,
            video_path=video_path,
            fps=self._fps,
            render_skipped=after["renderSkippedFrames"] - before["renderSkippedFrames"],
            output_skipped=after["outputSkippedFrames"] - before["outputSkippedFrames"],
        )

    def _has_video(self) -> bool:
        if self._process is None or not self._process.is_running():
            raise Exception("OBS not running")
        transform = self._client.request(
            "GetSceneItemTransform", {"sceneName": SCENE, "sceneItemId": self._scene_item_id}
        )["sceneItemTransform"]
        size = (transform["sourceWidth"], transform["sourceHeight"])
        return size == self._resolution.size()

    def _is_not_recording(self) -> bool:
        return not self._client.request("GetRecordStatus")["outputActive"]

    def _write_profile(self) -> None:
        width, height = self._resolution.size()
        directory = CONFIG_DIR / "basic" / "profiles" / PROFILE
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "basic.ini").write_text(
            PROFILE_TEMPLATE.format(
                name=PROFILE,
                filename=self._name,
                path=FILES_DIR,
                width=width,
                height=height,
                fps=self._fps,
            )
        )

    def _write_collection(self) -> None:
        directory = CONFIG_DIR / "basic" / "scenes"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"{COLLECTION}.json").write_text(
            json.dumps(
                {
                    "name": COLLECTION,
                    "sources": [],
                    "scene_order": [],
                    "groups": [],
                    "transitions": [],
                    "current_transition": "Fade",
                    "transition_duration": 300,
                },
                indent=4,
            )
        )

    def _enable_websocket(self) -> None:
        WEBSOCKET_CONFIG.parent.mkdir(parents=True, exist_ok=True)
        if WEBSOCKET_CONFIG.exists():
            self._websocket_config = WEBSOCKET_CONFIG.read_bytes()
        WEBSOCKET_CONFIG.write_text(
            json.dumps(
                {
                    "alerts_enabled": False,
                    "auth_required": True,
                    "first_load": False,
                    "server_enabled": True,
                    "server_password": WEBSOCKET_PASSWORD,
                    "server_port": WEBSOCKET_PORT,
                },
                indent=4,
            )
        )

    def _restore_websocket(self) -> None:
        if self._websocket_config is None:
            WEBSOCKET_CONFIG.unlink(missing_ok=True)
        else:
            WEBSOCKET_CONFIG.write_bytes(self._websocket_config)
            self._websocket_config = None

    def _start_process(self) -> None:
        self._process = ManagedProcess(
            [
                str(BINARY),
                "--multi",
                "--collection",
                COLLECTION,
                "--profile",
                PROFILE,
                "--disable-updater",
                "--disable-missing-files-check",
            ],
            LOGGER_OBS,
            ready=self._wait_until_ready,
        )
        self._process.start()

    def _wait_until_ready(self) -> None:
        LOGGER.debug("Waiting for OBS to start...")

        def check() -> bool:
            try:
                self._client.connect()
                self._client.request("GetVersion")
            except BaseException:
                self._client.close()
                raise
            return True

        wait_until(check, "OBS to start", ignore_errors=True)

    def _stop_recording(self) -> None:
        try:
            if self._client.request("GetRecordStatus")["outputActive"]:
                self._client.request("StopRecord")
        except Exception:
            pass

    def _stop_process(self) -> None:
        process = self._process
        if process is None:
            return
        pid = process.pid()
        if pid is not None and process.is_running():
            try:
                os.kill(pid, signal.SIGINT)
                wait_until(lambda: not process.is_running(), "OBS to exit", timeout=SHUTDOWN_SECONDS)
            except Exception:
                LOGGER.warning("OBS did not exit on its own, killing it")
        process.stop()
        self._process = None


def _check_machine() -> None:
    if not BINARY.is_file():
        raise Exception(f"No OBS Studio in {BINARY}")
    if not CONFIG_DIR.is_dir():
        raise Exception(f"No OBS Studio configuration in {CONFIG_DIR}; start OBS Studio once")
    found = subprocess.run(["pgrep", "-x", "OBS"], check=False, capture_output=True)
    if found.returncode == 0:
        raise Exception("OBS Studio is already running; close it before running the tests")


def _read_selection() -> dict[Path, str]:
    return {path: path.read_text() for path in SELECTION_FILES if path.is_file()}


def _restore_selection(selection: dict[Path, str]) -> None:
    for path, text in selection.items():
        if not path.is_file():
            continue
        current = path.read_text()
        for key in SELECTION_KEYS:
            found = re.search(rf"^{key}=(.*)$", text, re.MULTILINE)
            if found is None:
                continue
            line = f"{key}={found.group(1)}"
            current = re.sub(rf"^{key}=.*$", lambda _: line, current, count=1, flags=re.MULTILINE)
        path.write_text(current)


def _remove_sentinel() -> None:
    if SENTINEL_DIR.is_dir():
        for path in SENTINEL_DIR.iterdir():
            path.unlink(missing_ok=True)
