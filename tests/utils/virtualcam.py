import logging
import re
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path

from .config import Config
from .ffmpeg import AUDIO_ENCODER
from .ffmpeg import MOBCAM_FFMPEG_COMMAND
from .ffmpeg import VIDEO_ENCODER
from .ffmpeg import extract_audio
from .ffmpeg import ffprobe_video
from .process import ManagedProcess
from .utils import FILES_DIR
from .utils import wait_until

LOGGER = logging.getLogger(__name__)
LOGGER_VIRTUALCAM = logging.getLogger(__name__ + ".mobcam-virtualcam")
LOGGER_FFMPEG = logging.getLogger(__name__ + ".ffmpeg")

RE_CONNECTED = re.compile(r"connected to (.+) \(Moblin (\S+)\) on (\S+)")
RE_AUDIO_SPEC = re.compile(r"playing (\d+) Hz (\d+) channel audio into")
RE_WRITING = re.compile(r"writing to (\S+)")
RE_VIDEO_INPUT = re.compile(r"Stream #0:0.*: Video: (\w+).*?, (\w+), (\d+)x(\d+)")
RE_AUDIO_INPUT = re.compile(r"Stream #1:0.*: Audio: (\w+).*?, (\d+) Hz, (\w+)")
RE_ERROR = re.compile(r"^error: (.*)")
RE_WARNING = re.compile(r"^warning: (.*)")

DEFAULT_AUDIO_SAMPLE_RATE = 48000
DEFAULT_AUDIO_CHANNELS = 1
FRAME_QUEUE_SIZE = 512
VIDEO_BITRATE = "10M"
VIDEO_TIME_BASE = "1/90000"
DECIMATE_FILTER = "mpdecimate=hi=64:lo=32:frac=0.01"
START_ATTEMPTS = 3
RETRY_SECONDS = 2
CAMERA_MODULE = "v4l2loopback"
WINDOW_SECONDS = 0.5
AUDIO_BITRATE = "128k"


class Log:
    def __init__(self):
        self._lock = threading.Lock()
        self._lines: list[str] = []

    def add(self, line: str):
        with self._lock:
            self._lines.append(line)

    def lines(self) -> list[str]:
        with self._lock:
            return list(self._lines)

    def match(self, pattern: re.Pattern) -> re.Match | None:
        for line in self.lines():
            found = pattern.search(line)
            if found is not None:
                return found
        return None

    def matches(self, pattern: re.Pattern) -> list[str]:
        return [found.group(1) for line in self.lines() if (found := pattern.search(line)) is not None]

    def errors(self) -> list[str]:
        return self.matches(RE_ERROR)

    def warnings(self) -> list[str]:
        return self.matches(RE_WARNING)


@dataclass
class AudioSpec:
    sample_rate: int
    channels: int


@dataclass
class VideoInputFormat:
    codec: str
    pixel_format: str
    width: int
    height: int


@dataclass
class Recording:
    seconds: float
    video: VideoInputFormat | None
    audio: AudioSpec | None
    video_path: Path
    audio_path: Path
    frames: int
    duration: float
    distinct_frames: int
    distinct_duration: float
    window_rates: list[float]

    def fps(self) -> float:
        return self.frames / self.duration

    def distinct_fps(self) -> float:
        return self.distinct_frames / self.distinct_duration

    def duplicate_ratio(self) -> float:
        return 1 - self.distinct_frames / self.frames


class VirtualCam:
    def __init__(self, config: Config, hardware_decode: bool = True):
        self._config = config
        self._service = config.virtualcam_service()
        self._video_device = config.video_device()
        self._audio_capture_device = config.audio_capture_device()
        self.log = Log()
        self._command = [
            "sudo",
            "-n",
            config.virtualcam_binary(),
            "--device",
            self._video_device,
            "--audio-backend",
            "alsa",
            "--audio-device",
            config.audio_playback_device(),
        ]
        if not hardware_decode:
            self._command.append("--no-hardware-decode")
        self._process = ManagedProcess(self._command, LOGGER_VIRTUALCAM, observer=self.log.add)

    def __enter__(self):
        self._control_service("stop")
        self._start_process()
        return self

    def _start_process(self):
        for attempt in range(START_ATTEMPTS):
            self.log = Log()
            self._process = ManagedProcess(self._command, LOGGER_VIRTUALCAM, observer=self.log.add)
            self._process.start()
            try:
                self._wait_until_ready()
                return
            except Exception:
                self._process.stop()
                if attempt + 1 == START_ATTEMPTS:
                    raise
                LOGGER.warning(
                    "mobcam-virtualcam did not take %s; reloading %s and trying again",
                    self._video_device,
                    CAMERA_MODULE,
                )
                self._reload_camera()
                time.sleep(RETRY_SECONDS)

    def _reload_camera(self):
        for command in [
            ["sudo", "-n", "modprobe", "-r", CAMERA_MODULE],
            ["sudo", "-n", "modprobe", CAMERA_MODULE],
            ["sudo", "-n", "udevadm", "settle"],
        ]:
            proc = subprocess.run(command, check=False, capture_output=True, text=True)
            if proc.returncode != 0:
                LOGGER.warning("%s failed: %s", " ".join(command), proc.stderr.strip())
                return

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._stop_process()
        self._reload_camera()
        self._control_service("start")

    def _wait_until_ready(self, timeout: float = 10):
        wait_until(
            lambda: self._is_running_and(RE_WRITING),
            f"mobcam-virtualcam to open {self._video_device}",
            timeout=timeout,
        )

    def _is_running_and(self, pattern: re.Pattern) -> bool:
        if not self._process.is_running():
            errors = self.log.errors() or self.log.lines()[-3:]
            raise Exception("mobcam-virtualcam stopped: " + "; ".join(errors))
        return self.log.match(pattern) is not None

    def wait_until_connected(self, timeout: float = 60):
        wait_until(
            lambda: self._is_running_and(RE_CONNECTED),
            f"mobcam-virtualcam to connect to the device and Moblin to start streaming to "
            f"{self._video_device}",
            timeout=timeout,
        )
        LOGGER.info("Connected to %s", self.device_name())

    def device_name(self) -> str | None:
        found = self.log.match(RE_CONNECTED)
        return None if found is None else found.group(1)

    def audio_spec(self) -> AudioSpec | None:
        found = self.log.match(RE_AUDIO_SPEC)
        if found is None:
            return None
        return AudioSpec(int(found.group(1)), int(found.group(2)))

    def is_running(self) -> bool:
        return self._process.is_running()

    def record(self, seconds: float, name: str) -> Recording:
        audio = self.audio_spec() or AudioSpec(DEFAULT_AUDIO_SAMPLE_RATE, DEFAULT_AUDIO_CHANNELS)
        video_path = FILES_DIR / f"{name}.mp4"
        audio_path = FILES_DIR / f"{name}.wav"
        recording = self._start(self._recording_command(seconds, audio, video_path))
        distinct = self._start(self._distinct_frames_command(seconds))
        recording_output, recording_errors = recording.communicate()
        distinct_output, distinct_errors = distinct.communicate()
        self._log(recording_errors)
        self._log(distinct_errors)
        _check(recording, recording_errors, "record")
        _check(distinct, distinct_errors, "count the distinct frames of")
        extract_audio(video_path, audio_path)
        timestamps = sorted(frame.pts for frame in ffprobe_video(video_path).frames)
        distinct_windows = _parse_progress(distinct_output)
        return Recording(
            seconds=seconds,
            video=_parse_video_input(recording_errors),
            audio=_parse_audio_input(recording_errors),
            video_path=video_path,
            audio_path=audio_path,
            frames=len(timestamps),
            duration=timestamps[-1] - timestamps[0] if len(timestamps) > 1 else 0,
            distinct_frames=distinct_windows[-1][1] if distinct_windows else 0,
            distinct_duration=distinct_windows[-1][0] if distinct_windows else 0,
            window_rates=_window_rates(timestamps),
        )

    def _recording_command(self, seconds: float, audio: AudioSpec, path: Path) -> list[str]:
        return MOBCAM_FFMPEG_COMMAND + [
            *self._video_input(),
            *self._audio_input(audio),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-t",
            str(seconds),
            "-fps_mode",
            "passthrough",
            "-enc_time_base:v",
            VIDEO_TIME_BASE,
            "-c:v",
            VIDEO_ENCODER,
            "-b:v",
            VIDEO_BITRATE,
            "-c:a",
            AUDIO_ENCODER,
            "-b:a",
            AUDIO_BITRATE,
            str(path),
        ]

    def _distinct_frames_command(self, seconds: float) -> list[str]:
        return MOBCAM_FFMPEG_COMMAND + [
            *self._video_input(),
            "-progress",
            "pipe:1",
            "-an",
            "-t",
            str(seconds),
            "-vf",
            DECIMATE_FILTER,
            "-fps_mode",
            "passthrough",
            "-f",
            "null",
            "-",
        ]

    def _audio_input(self, audio: AudioSpec) -> list[str]:
        return [
            "-thread_queue_size",
            str(FRAME_QUEUE_SIZE),
            "-f",
            "alsa",
            "-ar",
            str(audio.sample_rate),
            "-ac",
            str(audio.channels),
            "-i",
            self._audio_capture_device,
        ]

    def _video_input(self) -> list[str]:
        return [
            "-thread_queue_size",
            str(FRAME_QUEUE_SIZE),
            "-use_wallclock_as_timestamps",
            "1",
            "-f",
            "v4l2",
            "-i",
            self._video_device,
        ]

    def _start(self, command: list[str]) -> subprocess.Popen:
        LOGGER_FFMPEG.debug("Command: %s", " ".join(command))
        return subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def _log(self, output: str):
        for line in output.splitlines():
            LOGGER_FFMPEG.debug("%s", line)

    def _control_service(self, action: str):
        if self._service == "":
            return
        proc = subprocess.run(
            ["sudo", "-n", "systemctl", action, self._service],
            check=False,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            LOGGER.warning("Failed to %s %s: %s", action, self._service, proc.stderr.strip())

    def _stop_process(self):
        subprocess.run(
            ["sudo", "-n", "pkill", "-TERM", "-f", self._config.virtualcam_binary()],
            check=False,
            capture_output=True,
        )
        try:
            wait_until(lambda: not self._process.is_running(), "mobcam-virtualcam to stop", timeout=5)
        except Exception:
            LOGGER.warning("mobcam-virtualcam did not stop; killing it")
        self._process.stop()


def _check(process: subprocess.Popen, errors: str, what: str):
    if process.returncode == 0:
        return
    tail = "\n".join(errors.splitlines()[-5:])
    raise Exception(f"ffmpeg failed to {what} the camera:\n{tail}")


def _parse_progress(output: str) -> list[tuple[float, int]]:
    windows = []
    frames = 0
    duration = 0.0
    for line in output.splitlines():
        key, _, value = line.partition("=")
        number = _parse_number(value)
        if key == "frame" and number is not None:
            frames = number
        elif key == "out_time_us" and number is not None:
            duration = number / 1_000_000
        elif key == "progress" and duration > 0:
            windows.append((duration, frames))
    return windows


def _parse_number(value: str) -> int | None:
    try:
        return int(value)
    except ValueError:
        return None


def _window_rates(timestamps: list[float]) -> list[float]:
    if len(timestamps) < 2:
        return []
    start = timestamps[0]
    windows = int((timestamps[-1] - start) / WINDOW_SECONDS)
    counts = [0] * windows
    for timestamp in timestamps:
        window = int((timestamp - start) / WINDOW_SECONDS)
        if window < windows:
            counts[window] += 1
    return [count / WINDOW_SECONDS for count in counts]


def _parse_video_input(output: str) -> VideoInputFormat | None:
    found = RE_VIDEO_INPUT.search(output)
    if found is None:
        return None
    return VideoInputFormat(
        codec=found.group(1),
        pixel_format=found.group(2),
        width=int(found.group(3)),
        height=int(found.group(4)),
    )


def _parse_audio_input(output: str) -> AudioSpec | None:
    found = RE_AUDIO_INPUT.search(output)
    if found is None:
        return None
    return AudioSpec(sample_rate=int(found.group(2)), channels=1 if found.group(3) == "mono" else 2)
