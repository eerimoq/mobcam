import json
import logging
import math
import re
import subprocess
import time
from dataclasses import dataclass
from dataclasses import field
from fractions import Fraction
from pathlib import Path

LOGGER = logging.getLogger(__name__)
FFMPEG_COMMAND = ["ffmpeg", "-hide_banner", "-nostdin", "-nostats", "-y"]
MOBCAM_FFMPEG_DIR = Path("/opt/mobcam/ffmpeg/bin")
MOBCAM_FFMPEG = str(MOBCAM_FFMPEG_DIR / "ffmpeg")
MOBCAM_FFMPEG_COMMAND = [MOBCAM_FFMPEG, "-hide_banner", "-nostdin", "-nostats", "-y"]
VIDEO_ENCODER = "h264_rkmpp"
VIDEO_CODEC = "h264"
AUDIO_ENCODER = "aac"
AUDIO_CODEC = "aac"
RE_VOLUME_DETECT = re.compile(r"(n_samples|mean_volume|max_volume): (-?[\d.]+|-?inf)")
RE_SILENCE_DETECT = re.compile(r"silence_(start|end): (-?[\d.]+)")


def _run_logged(command: list[str], text: bool):
    started = time.monotonic()
    try:
        return subprocess.run(command, check=True, capture_output=True, text=text)
    finally:
        LOGGER.debug("Command (%.3f s): %s", time.monotonic() - started, " ".join(command))


def _run(command: list[str]):
    return _run_logged(command, True)


def ffprobe_run(path: Path, *args):
    command = [
        "ffprobe",
        "-of",
        "json",
        *args,
        str(path),
    ]
    output = _run(command).stdout
    return json.loads(output)


def ffmpeg_run(*args):
    return _run(FFMPEG_COMMAND + [*args])


def mobcam_ffmpeg_supports(kind: str, name: str) -> bool:
    proc = subprocess.run(
        [MOBCAM_FFMPEG, "-hide_banner", "-loglevel", "quiet", f"-{kind}"],
        check=False,
        capture_output=True,
        text=True,
    )
    return re.search(rf"\b{re.escape(name)}\b", proc.stdout) is not None


def _frame_pts(frame) -> float:
    return float(frame.get("pts_time", frame.get("pkt_pts_time")))


@dataclass
class FfprobeVideoOutputFrame:
    pts: float
    picture_type: str

    def __init__(self, frame):
        self.pts = _frame_pts(frame)
        self.picture_type = frame["pict_type"]


@dataclass
class FfprobeVideoOutput:
    codec: str
    width: int
    height: int
    real_base_fps: Fraction | None
    average_fps: Fraction | None
    frames: list[FfprobeVideoOutputFrame]


@dataclass
class FfprobeAudioOutputFrame:
    pts: float
    channels: int
    number_of_samples: int

    def __init__(self, frame):
        self.pts = _frame_pts(frame)
        self.channels = frame["channels"]
        self.number_of_samples = frame["nb_samples"]


@dataclass
class FfprobeAudioOutput:
    codec: str = ""
    sample_rate: int = 0
    channels: int = 0
    channel_layout: str = ""
    frames: list[FfprobeAudioOutputFrame] = field(default_factory=list)


@dataclass
class FfprobeFormatOutput:
    duration: float
    start_time: float


@dataclass
class FfprobeOutput:
    video: FfprobeVideoOutput
    audio: FfprobeAudioOutput
    format: FfprobeFormatOutput


def ffprobe_video(path: Path):
    output = ffprobe_run(
        path,
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name,width,height,r_frame_rate,avg_frame_rate:frame=pict_type,pts_time,pkt_pts_time",
    )
    stream = output["streams"][0]
    frames = [FfprobeVideoOutputFrame(frame) for frame in output["frames"]]
    return FfprobeVideoOutput(
        codec=stream["codec_name"],
        width=stream["width"],
        height=stream["height"],
        real_base_fps=_get_fps(stream, "r_frame_rate"),
        average_fps=_get_fps(stream, "avg_frame_rate"),
        frames=frames,
    )


def _get_fps(stream, name: str) -> Fraction | None:
    try:
        return Fraction(stream[name])
    except Exception:
        return None


def ffprobe_audio(path) -> FfprobeAudioOutput:
    output = ffprobe_run(
        path,
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=codec_name,sample_rate,channels,channel_layout"
        ":frame=nb_samples,pts_time,pkt_pts_time,channels",
    )
    streams = output["streams"]
    if len(streams) == 0:
        return FfprobeAudioOutput()
    stream = streams[0]
    frames = [FfprobeAudioOutputFrame(frame) for frame in output["frames"]]
    return FfprobeAudioOutput(
        codec=stream["codec_name"],
        sample_rate=int(stream["sample_rate"]),
        channels=stream["channels"],
        channel_layout=stream.get("channel_layout", ""),
        frames=frames,
    )


def ffprobe_format(path):
    output = ffprobe_run(path, "-show_entries", "format=duration,start_time")
    return FfprobeFormatOutput(
        duration=float(output["format"]["duration"]),
        start_time=float(output["format"].get("start_time", 0)),
    )


def ffprobe(path: Path):
    return FfprobeOutput(
        video=ffprobe_video(path),
        audio=ffprobe_audio(path),
        format=ffprobe_format(path),
    )


def extract_audio(path: Path, audio_path: Path):
    ffmpeg_run("-i", str(path), "-vn", "-c:a", "pcm_s16le", str(audio_path))


def measure_mean_volume(path: Path) -> float:
    return _measure_volume(path, "mean_volume")


def _measure_volume(path: Path, name: str) -> float:
    output = ffmpeg_run(
        "-i",
        str(path),
        "-vn",
        "-af",
        "volumedetect",
        "-f",
        "null",
        "-",
    ).stderr
    for found_name, value in RE_VOLUME_DETECT.findall(output):
        if found_name == name:
            return float(value)
    return -math.inf


@dataclass
class Silence:
    start: float
    end: float


def detect_silence(path: Path, noise_db: float, minimum_duration: float) -> list[Silence]:
    output = ffmpeg_run(
        "-i",
        str(path),
        "-vn",
        "-af",
        f"silencedetect=noise={noise_db}dB:duration={minimum_duration}",
        "-f",
        "null",
        "-",
    ).stderr
    silences = []
    start = None
    for kind, value in RE_SILENCE_DETECT.findall(output):
        if kind == "start":
            start = float(value)
        elif start is not None:
            silences.append(Silence(start, float(value)))
            start = None
    if start is not None:
        silences.append(Silence(start, ffprobe_format(path).duration))
    return silences
