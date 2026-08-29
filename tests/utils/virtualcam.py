import logging
import re
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import Self

from systest import ManagedProcess

from .config import Config
from .ffmpeg import ffmpeg_run
from .ffmpeg import ffprobe_video
from .ffmpeg import video_encoder
from .utils import FILES_DIR
from .utils import ROOT_DIR
from .utils import Log

LOGGER = logging.getLogger(__name__)
BINARY = str(ROOT_DIR / "target" / "release" / "mobcam-virtualcam")


@dataclass
class Recording:
    seconds: float
    video_path: Path
    timestamps: list[float]

    @property
    def frames(self) -> int:
        return len(self.timestamps)

    @property
    def duration(self) -> float:
        return self.timestamps[-1] - self.timestamps[0] if len(self.timestamps) > 1 else 0

    def fps(self) -> float:
        return self.frames / self.duration

    def pts_deltas(self) -> list[float]:
        return [after - before for before, after in zip(self.timestamps, self.timestamps[1:])]


class VirtualCam:
    def __init__(self, config: Config) -> None:
        self._config = config
        self._service = config.virtualcam_service()
        self._video_device = config.video_device()
        self._audio_capture_device = config.audio_capture_device()
        self.log = Log()
        self._process: ManagedProcess | None = None

    def __enter__(self) -> Self:
        self._start_process()
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        self._stop_process()

    def record(self, seconds: float, name: str) -> Recording:
        video_path = FILES_DIR / f"{name}.mp4"
        ffmpeg_run(
            *self._video_input(),
            *self._audio_input(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-t",
            str(seconds),
            "-fps_mode",
            "passthrough",
            "-enc_time_base:v",
            "1/90000",
            "-c:v",
            video_encoder(),
            "-b:v",
            "5M",
            "-c:a",
            "aac",
            str(video_path),
        )
        timestamps = sorted(frame.pts for frame in ffprobe_video(video_path).frames)
        return Recording(seconds=seconds, video_path=video_path, timestamps=timestamps)

    def _start_process(self) -> None:
        self._process = ManagedProcess(
            [
                BINARY,
                "--debug",
                "--device",
                self._video_device,
                "--audio-backend",
                "alsa",
                "--audio-device",
                self._config.audio_playback_device(),
            ],
            LOGGER,
            observer=self.log.add,
        )
        self._process.start()

    def _stop_process(self) -> None:
        if self._process is not None:
            self._process.stop()
            self._process = None

    def _is_running_and(self, pattern: re.Pattern[str]) -> bool:
        if self._process is None or not self._process.is_running():
            raise Exception("mobcam-virtualcam not running")
        return self.log.match(pattern) is not None

    def _video_input(self) -> list[str]:
        return ["-f", "v4l2", "-i", self._video_device]

    def _audio_input(self) -> list[str]:
        return [
            "-f",
            "alsa",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-i",
            self._audio_capture_device,
        ]
