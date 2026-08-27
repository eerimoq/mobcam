import logging
import re
import subprocess
import threading
from dataclasses import dataclass
from pathlib import Path

from .config import Config
from .ffmpeg import FFMPEG_COMMAND
from .process import ManagedProcess
from .utils import FILES_DIR
from .utils import wait_until

LOGGER = logging.getLogger(__name__)
LOGGER_VIRTUALCAM = logging.getLogger(__name__ + ".mobcam-virtualcam")
LOGGER_FFMPEG = logging.getLogger(__name__ + ".ffmpeg")

RE_CONNECTED = re.compile(r"connected to (.+) \(Moblin (\S+)\) on (\S+)")
RE_AUDIO_SPEC = re.compile(r"playing (\d+) Hz (\d+) channel audio into")
RE_WRITING = re.compile(r"writing to (\S+) in ")
RE_VIDEO_INPUT = re.compile(r"Stream #0:0.*: Video: (\w+).*?, (\w+), (\d+)x(\d+)")
RE_AUDIO_INPUT = re.compile(r"Stream #1:0.*: Audio: (\w+).*?, (\d+) Hz, (\w+)")
RE_ERROR = re.compile(r"^error: (.*)")
RE_WARNING = re.compile(r"^warning: (.*)")

DEFAULT_AUDIO_SAMPLE_RATE = 48000
DEFAULT_AUDIO_CHANNELS = 1
FRAME_QUEUE_SIZE = 512
IMAGES_PER_SECOND = 1


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
    audio_path: Path
    images: list[Path]
    frames: int
    duration: float
    window_rates: list[float]

    def fps(self) -> float:
        return self.frames / self.duration


class VirtualCam:
    def __init__(self, config: Config, hardware_decode: bool = True):
        self._config = config
        self._service = config.virtualcam_service()
        self._video_device = config.video_device()
        self._audio_capture_device = config.audio_capture_device()
        self.log = Log()
        command = [
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
            command.append("--no-hardware-decode")
        self._process = ManagedProcess(command, LOGGER_VIRTUALCAM, observer=self.log.add)

    def __enter__(self):
        self._control_service("stop")
        self._process.start()
        self._wait_until_ready()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._stop_process()
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
        audio_path = FILES_DIR / f"{name}.wav"
        images_pattern = FILES_DIR / f"{name}-%03d.jpg"
        frames = self._start(self._frames_command(seconds))
        others = self._start(self._audio_and_images_command(seconds, audio, audio_path, images_pattern))
        frames_output, frames_errors = frames.communicate()
        others_output, others_errors = others.communicate()
        _check(frames, frames_errors, "count the frames of")
        _check(others, others_errors, "record the audio and the images of")
        self._log(frames_errors)
        self._log(others_errors)
        windows = _parse_progress(frames_output)
        return Recording(
            seconds=seconds,
            video=_parse_video_input(frames_errors),
            audio=_parse_audio_input(others_errors),
            audio_path=audio_path,
            images=sorted(FILES_DIR.glob(f"{name}-*.jpg")),
            frames=windows[-1][1] if windows else 0,
            duration=windows[-1][0] if windows else 0,
            window_rates=_window_rates(windows),
        )

    def _frames_command(self, seconds: float) -> list[str]:
        return FFMPEG_COMMAND + [
            *self._video_input(),
            "-progress",
            "pipe:1",
            "-an",
            "-t",
            str(seconds),
            "-vsync",
            "0",
            "-f",
            "null",
            "-",
        ]

    def _audio_and_images_command(
        self,
        seconds: float,
        audio: AudioSpec,
        audio_path: Path,
        images_pattern: Path,
    ) -> list[str]:
        return FFMPEG_COMMAND + [
            *self._video_input(),
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
            "-map",
            "0:v",
            "-t",
            str(seconds),
            "-vf",
            f"fps={IMAGES_PER_SECOND}",
            "-q:v",
            "4",
            str(images_pattern),
            "-map",
            "1:a",
            "-t",
            str(seconds),
            "-c:a",
            "pcm_s16le",
            str(audio_path),
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
        if key == "frame":
            frames = int(value)
        elif key == "out_time_us":
            duration = int(value) / 1_000_000
        elif key == "progress" and duration > 0:
            windows.append((duration, frames))
    return windows


def _window_rates(windows: list[tuple[float, int]]) -> list[float]:
    return [
        (frames - previous_frames) / (duration - previous_duration)
        for (previous_duration, previous_frames), (duration, frames) in zip(windows, windows[1:])
        if duration > previous_duration
    ]


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
