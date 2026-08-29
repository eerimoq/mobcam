import json
import logging
import re
import subprocess
import time
from dataclasses import dataclass
from dataclasses import field
from fractions import Fraction
from functools import cache
from pathlib import Path
from typing import Any

LOGGER = logging.getLogger(__name__)
FFMPEG = "ffmpeg"
FFPROBE = "ffprobe"
FFMPEG_COMMAND = [FFMPEG, "-hide_banner", "-nostdin", "-nostats", "-y"]
DUPLICATE_FRAME_RE = re.compile(r"drop pts:\d+ pts_time:([\d.]+) drop_count:(\d+)")


def _run_logged(command: list[str], text: bool) -> subprocess.CompletedProcess[Any]:
    started = time.monotonic()
    try:
        return subprocess.run(command, check=True, capture_output=True, text=text)
    finally:
        LOGGER.debug("Command (%.3f s): %s", time.monotonic() - started, " ".join(command))


def _run(command: list[str]) -> subprocess.CompletedProcess[str]:
    return _run_logged(command, True)


def ffprobe_run(path: Path, *args: str) -> dict[str, Any]:
    command = [
        FFPROBE,
        "-of",
        "json",
        *args,
        str(path),
    ]
    output: dict[str, Any] = json.loads(_run(command).stdout)
    return output


def ffmpeg_run(*args: str) -> subprocess.CompletedProcess[str]:
    return _run(FFMPEG_COMMAND + [*args])


@cache
def ffmpeg_supports(kind: str, name: str) -> bool:
    proc = subprocess.run(
        [FFMPEG, "-hide_banner", "-loglevel", "quiet", f"-{kind}"],
        check=False,
        capture_output=True,
        text=True,
    )
    return re.search(rf"\b{re.escape(name)}\b", proc.stdout) is not None


def video_encoder() -> str:
    return "h264_rkmpp" if ffmpeg_supports("encoders", "h264_rkmpp") else "libx264"


def _frame_pts(frame: dict[str, Any]) -> float:
    pts: Any = frame.get("pts_time", frame.get("pkt_pts_time"))
    return float(pts)


@dataclass
class FfprobeVideoOutputFrame:
    pts: float
    picture_type: str

    def __init__(self, frame: dict[str, Any]) -> None:
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

    def __init__(self, frame: dict[str, Any]) -> None:
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


@dataclass
class FfmpegDuplicateFrame:
    pts: float
    count: int


def ffmpeg_duplicate_frames(path: Path) -> list[FfmpegDuplicateFrame]:
    output = _run(
        FFMPEG_COMMAND
        + [
            "-loglevel",
            "debug",
            "-i",
            str(path),
            "-map",
            "0:v:0",
            "-an",
            "-vf",
            "mpdecimate",
            "-f",
            "null",
            "-",
        ]
    ).stderr
    return [
        FfmpegDuplicateFrame(pts=float(found.group(1)), count=int(found.group(2)))
        for found in DUPLICATE_FRAME_RE.finditer(output)
    ]


def ffprobe_video(path: Path) -> FfprobeVideoOutput:
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


def _get_fps(stream: dict[str, Any], name: str) -> Fraction | None:
    try:
        return Fraction(stream[name])
    except Exception:
        return None


def ffprobe_audio(path: Path) -> FfprobeAudioOutput:
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


def ffprobe_format(path: Path) -> FfprobeFormatOutput:
    output = ffprobe_run(path, "-show_entries", "format=duration,start_time")
    return FfprobeFormatOutput(
        duration=float(output["format"]["duration"]),
        start_time=float(output["format"].get("start_time", 0)),
    )


def ffprobe(path: Path) -> FfprobeOutput:
    return FfprobeOutput(
        video=ffprobe_video(path),
        audio=ffprobe_audio(path),
        format=ffprobe_format(path),
    )
