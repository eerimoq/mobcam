import json
import logging
import os
import re
import signal
import subprocess
import time
from base64 import b64encode
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from uuid import uuid4

from systest import ManagedProcess
from systest import wait_until
from websockets.sync.client import ClientConnection
from websockets.sync.client import connect

from .config import MOBCAM_PORT
from .ffmpeg import FfmpegDuplicateFrame
from .ffmpeg import ffmpeg_duplicate_frames
from .generate_device_settings import Resolution
from .utils import FILES_DIR
from .utils import Log

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
HOST = "127.0.0.1"
WEBSOCKET_PORT = 4466
WEBSOCKET_PASSWORD = "mobcam"
RPC_VERSION = 1
OP_IDENTIFY = 1
OP_IDENTIFIED = 2
OP_REQUEST = 6
OP_REQUEST_RESPONSE = 7
SHUTDOWN_SECONDS = 30
CONNECTED_RE = re.compile(r"connected to .* on ")

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


def _authentication(password: str, salt: str, challenge: str) -> str:
    secret = b64encode(sha256((password + salt).encode()).digest()).decode()
    return b64encode(sha256((secret + challenge).encode()).digest()).decode()


class ObsWebSocket:
    def __init__(self, port: int, password: str):
        self._url = f"ws://{HOST}:{port}"
        self._password = password
        self._connection: ClientConnection | None = None

    def connect(self):
        connection = connect(self._url, max_size=None, open_timeout=5)
        try:
            hello = json.loads(connection.recv())["d"]
            identify: dict[str, int | str] = {"rpcVersion": RPC_VERSION, "eventSubscriptions": 0}
            authentication = hello.get("authentication")
            if authentication is not None:
                identify["authentication"] = _authentication(
                    self._password, authentication["salt"], authentication["challenge"]
                )
            connection.send(json.dumps({"op": OP_IDENTIFY, "d": identify}))
            message = json.loads(connection.recv())
            if message["op"] != OP_IDENTIFIED:
                raise Exception(f"Unexpected message {message} when identifying")
        except BaseException:
            connection.close()
            raise
        self._connection = connection

    def close(self):
        if self._connection is not None:
            self._connection.close()
            self._connection = None

    def request(self, request_type: str, data: dict | None = None) -> dict:
        if self._connection is None:
            raise Exception("Not connected to OBS")
        request_id = str(uuid4())
        self._connection.send(
            json.dumps(
                {
                    "op": OP_REQUEST,
                    "d": {
                        "requestType": request_type,
                        "requestId": request_id,
                        "requestData": data or {},
                    },
                }
            )
        )
        while True:
            message = json.loads(self._connection.recv())
            if message["op"] != OP_REQUEST_RESPONSE or message["d"]["requestId"] != request_id:
                continue
            response = message["d"]
            status = response["requestStatus"]
            if not status["result"]:
                raise Exception(f"{request_type} failed: {status}")
            return response.get("responseData") or {}


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
    def __init__(self, resolution: Resolution, fps: int, name: str):
        self._resolution = resolution
        self._fps = fps
        self._name = name
        self.log = Log()
        self._client = ObsWebSocket(WEBSOCKET_PORT, WEBSOCKET_PASSWORD)
        self._process: ManagedProcess | None = None
        self._websocket_config: bytes | None = None
        self._selection: dict[Path, str] = {}

    def __enter__(self):
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

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._stop_recording()
        self._client.close()
        self._stop_process()
        self._restore_websocket()
        _restore_selection(self._selection)
        _remove_sentinel()

    def create_source(self, hardware_decode: bool = True, buffering: bool = False):
        scenes = self._client.request("GetSceneList")["scenes"]
        if not any(scene["sceneName"] == SCENE for scene in scenes):
            self._client.request("CreateScene", {"sceneName": SCENE})
        self._client.request("SetCurrentProgramScene", {"sceneName": SCENE})
        self._client.request(
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
        )

    def wait_until_connected(self):
        LOGGER.info("Waiting for the Mobcam source to connect to the device...")
        wait_until(self._is_connected, "the source to connect to the device")

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

    def _is_connected(self) -> bool:
        if self._process is None or not self._process.is_running():
            raise Exception("OBS not running")
        return self.log.match(CONNECTED_RE) is not None

    def _is_not_recording(self) -> bool:
        return not self._client.request("GetRecordStatus")["outputActive"]

    def _write_profile(self):
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

    def _write_collection(self):
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

    def _enable_websocket(self):
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

    def _restore_websocket(self):
        if self._websocket_config is None:
            WEBSOCKET_CONFIG.unlink(missing_ok=True)
        else:
            WEBSOCKET_CONFIG.write_bytes(self._websocket_config)
            self._websocket_config = None

    def _start_process(self):
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
            observer=self.log.add,
            ready=self._wait_until_ready,
        )
        self._process.start()

    def _wait_until_ready(self):
        LOGGER.info("Waiting for OBS to start...")

        def check() -> bool:
            try:
                self._client.connect()
                self._client.request("GetVersion")
            except BaseException:
                self._client.close()
                raise
            return True

        wait_until(check, "OBS to start", ignore_errors=True)

    def _stop_recording(self):
        try:
            if self._client.request("GetRecordStatus")["outputActive"]:
                self._client.request("StopRecord")
        except Exception:
            pass

    def _stop_process(self):
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


def _check_machine():
    if not BINARY.is_file():
        raise Exception(f"No OBS Studio in {BINARY}")
    if not CONFIG_DIR.is_dir():
        raise Exception(f"No OBS Studio configuration in {CONFIG_DIR}; start OBS Studio once")
    found = subprocess.run(["pgrep", "-x", "OBS"], check=False, capture_output=True)
    if found.returncode == 0:
        raise Exception("OBS Studio is already running; close it before running the tests")


def _read_selection() -> dict[Path, str]:
    return {path: path.read_text() for path in SELECTION_FILES if path.is_file()}


def _restore_selection(selection: dict[Path, str]):
    for path, text in selection.items():
        if not path.is_file():
            continue
        current = path.read_text()
        for key in SELECTION_KEYS:
            found = re.search(rf"^{key}=(.*)$", text, re.MULTILINE)
            if found is None:
                continue
            line = f"{key}={found.group(1)}"
            current = re.sub(rf"^{key}=.*$", lambda _, line=line: line, current, count=1, flags=re.MULTILINE)
        path.write_text(current)


def _remove_sentinel():
    if SENTINEL_DIR.is_dir():
        for path in SENTINEL_DIR.iterdir():
            path.unlink(missing_ok=True)
