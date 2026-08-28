import logging
import statistics
from collections.abc import Callable

import systest
from systest import wait_until

from .ffmpeg import ffprobe_audio
from .ffmpeg import ffprobe_format
from .ffmpeg import ffprobe_video
from .moblin import Moblin
from .virtualcam import Recording

LOGGER = logging.getLogger(__name__)
PIXEL_FORMATS = ["yuv420p", "nv12"]
MINIMUM_FPS_RATIO = 0.8
MAXIMUM_FPS_RATIO = 1.05
MINIMUM_WINDOW_FPS_RATIO = 0.5
MAXIMUM_VIDEO_LENGTH_DIFFERENCE = 0.5
MINIMUM_MEAN_VOLUME_DB = -60.0
SILENCE_NOISE_DB = -60.0
MAXIMUM_SILENCE = 0.5
MAXIMUM_AUDIO_LENGTH_DIFFERENCE = 0.5
PTS_DELTA_TOLERANCE_RATIO = 0.2
PTS_DELTA_SETTLE_SECONDS = 0.25
WORST_PTS_DELTAS = 5


class TestCase(systest.TestCase):
    def __init__(self, moblin: Moblin, name: str | None = None):
        super().__init__(name)
        self.moblin = moblin

    def teardown(self):
        self.moblin.end()

    def wait_until(self, check: Callable[[], bool]):
        wait_until(check, "condition to be true")

    def assert_camera_recording(self, recording: Recording, width: int, height: int, fps: int):
        self._assert_pts_deltas(recording, fps)
        video = ffprobe_video(recording.video_path)
        recorded_audio = ffprobe_audio(recording.video_path)
        length = ffprobe_format(recording.video_path).duration
        self.assert_equal(video.codec, "h264")
        self.assert_equal(video.width, width)
        self.assert_equal(video.height, height)
        self.assert_equal(recorded_audio.codec, "aac")
        self.assert_greater(length, recording.seconds - MAXIMUM_VIDEO_LENGTH_DIFFERENCE)
        self.assert_less(length, recording.seconds + MAXIMUM_VIDEO_LENGTH_DIFFERENCE)

    def _assert_pts_deltas(self, recording: Recording, fps: int):
        deltas = recording.pts_deltas()
        if len(deltas) == 0:
            return
        expected = 1 / fps
        tolerance = PTS_DELTA_TOLERANCE_RATIO * expected
        start = recording.timestamps[0]
        offsets = [timestamp - start for timestamp in recording.timestamps[1:]]
        outliers = [
            (delta, offset)
            for delta, offset in zip(deltas, offsets)
            if abs(delta - expected) > tolerance and offset > PTS_DELTA_SETTLE_SECONDS
        ]
        LOGGER.debug(
            "PTS delta expected %.2f ms, mean %.2f ms, median %.2f ms, min %.2f ms, max %.2f ms, "
            "standard deviation %.2f ms, %s of %s deltas more than %.0f %% off after the first "
            "%.2f s",
            1000 * expected,
            1000 * statistics.mean(deltas),
            1000 * statistics.median(deltas),
            1000 * min(deltas),
            1000 * max(deltas),
            1000 * statistics.pstdev(deltas),
            len(outliers),
            len(deltas),
            100 * PTS_DELTA_TOLERANCE_RATIO,
            PTS_DELTA_SETTLE_SECONDS,
        )
        worst = sorted(outliers, key=lambda outlier: -abs(outlier[0] - expected))
        for delta, offset in worst[:WORST_PTS_DELTAS]:
            LOGGER.info("PTS delta %.2f ms at %.3f s into the recording", 1000 * delta, offset)
        self.assert_equal(len(outliers), 0)
