import json
import logging
import sys
import tempfile
import threading
import time
from base64 import b64encode
from pathlib import Path
from types import TracebackType
from typing import Any
from typing import Self

from systest import ManagedProcess
from systest import wait_until
from websockets.sync.client import ClientConnection
from websockets.sync.client import connect

from .config import REMOTE_CONTROL_PASSWORD
from .config import Config
from .generate_device_settings import SceneName
from .generate_device_settings import base_settings
from .generate_device_settings import create_settings_file

LOGGER = logging.getLogger(__name__)
LOGGER_ASSISTANT = logging.getLogger(__name__ + ".assistant")
LOGGER_EVENTS = logging.getLogger(__name__ + ".events")
HOST = "127.0.0.1"


class AssistantEvents:
    def __init__(self, port: int) -> None:
        self._port = port
        self._stopped = threading.Event()
        self._connection: ClientConnection | None = None
        self._thread = threading.Thread(target=self._listen, daemon=True)
        self._state_lock = threading.Lock()
        self._state: dict[str, Any] = {}

    def state(self) -> dict[str, Any]:
        with self._state_lock:
            return dict(self._state)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stopped.set()
        connection = self._connection
        if connection is not None:
            connection.close()
        self._thread.join(timeout=5)

    def _listen(self) -> None:
        while not self._stopped.is_set():
            try:
                with connect(f"ws://{HOST}:{self._port}/events", max_size=None) as connection:
                    self._connection = connection
                    for message in connection:
                        self._handle_message(message)
            except Exception:
                pass
            finally:
                self._connection = None
            self._stopped.wait(1)

    def _handle_message(self, message: str | bytes) -> None:
        for kind, data in json.loads(message).items():
            if kind == "log":
                LOGGER_EVENTS.debug("%s", data["entry"])
            else:
                if kind == "state":
                    with self._state_lock:
                        self._state.update(data["data"])
                LOGGER_EVENTS.debug("%s: %s", kind, data)


class Moblin:
    def __init__(self, config: Config) -> None:
        self.config = config
        self._remote_control_port = config.remote_control_port()
        self._server = ManagedProcess(
            [
                sys.executable,
                "-u",
                "-m",
                "moblin_assistant",
                "--port",
                str(self._remote_control_port),
                "run",
                "--password",
                REMOTE_CONTROL_PASSWORD,
            ],
            LOGGER_ASSISTANT,
            ready=self._wait_until_streamer_is_connected,
        )
        self._events = AssistantEvents(self._remote_control_port)

    def __enter__(self) -> Self:
        self._server.start()
        self._events.start()
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        self._events.stop()
        self._server.stop()

    def import_settings(self, overrides: dict[str, Any], files: dict[str, Path] | None = None) -> None:
        settings = base_settings(self.config, self._remote_control_port)
        settings.update(overrides)
        with tempfile.TemporaryDirectory() as settings_dir:
            settings_file = Path(settings_dir) / "settings.zip"
            create_settings_file(settings, settings_file, files)
            try:
                self._request(
                    {"importSettings": {"data": b64encode(settings_file.read_bytes()).decode("utf-8")}}
                )
            except Exception:
                pass
            time.sleep(2)

    def set_scene(self, name: SceneName) -> None:
        self._request({"setScene": {"id": self._get_settings_id("scenes", name)}})

    def set_muted(self, on: bool) -> None:
        self._request({"setMute": {"on": on}})

    def get_state(self) -> dict[str, Any]:
        return self._events.state()

    def go_live(self) -> None:
        self._request({"setLive": {"on": True}})

    def end(self) -> None:
        self._request({"setLive": {"on": False}})

    def ping(self) -> None:
        self._get_settings()

    def get_status(self) -> dict[str, Any]:
        status: dict[str, Any] = self._request({"getStatus": {}})["data"]["getStatus"]
        return status

    def _request(self, data: dict[str, Any]) -> dict[str, Any]:
        url = f"ws://{HOST}:{self._remote_control_port}/client"
        with connect(url, max_size=None) as server:
            server.send(json.dumps({"type": "request", "data": data}))
            response: dict[str, Any] = json.loads(server.recv())["data"]
            return response

    def _get_settings(self) -> dict[str, Any]:
        settings: dict[str, Any] = self._request({"getSettings": {}})["data"]["getSettings"]["data"]
        return settings

    def _get_settings_id(self, kind: str, name: str) -> str:
        for item in self._get_settings()[kind]:
            if item["name"] == name:
                item_id: str = item["id"]
                return item_id
        raise Exception(f"Unknown {kind} item {name}")

    def _wait_until_streamer_is_connected(self) -> None:
        LOGGER.info(
            "Waiting for a remote control streamer to connect to port %d...",
            self._remote_control_port,
        )

        def check() -> bool:
            self.ping()
            return True

        wait_until(check, "streamer to connect", ignore_errors=True)
        LOGGER.info("Remote control streamer connected")
