import logging
from collections.abc import Callable

import systest
from systest import wait_until

from .common.ffmpeg import FfprobeVideoOutput
from .common.ffmpeg import ffmpeg_duplicate_frames
from .common.ffmpeg import ffprobe_audio
from .common.ffmpeg import ffprobe_format
from .common.ffmpeg import ffprobe_video
from .moblin import Moblin
from .recording import Recording

LOGGER = logging.getLogger(__name__)
MAXIMUM_LENGTH_DIFFERENCE = 1.0
FRAME_COUNT_TOLERANCE_RATIO = 0.05
PTS_DELTA_TOLERANCE_RATIO = 0.2
PTS_DELTA_SETTLE_SECONDS = 0.25
WORST_PTS_DELTAS = 5
DUPLICATE_SETTLE_SECONDS = 0.5
WORST_DUPLICATES = 5


class TestCase(systest.TestCase):
    def __init__(self, moblin: Moblin, name: str | None = None) -> None:
        super().__init__(name)
        self.moblin = moblin

    def teardown(self) -> None:
        self.moblin.end()

    def wait_until(self, check: Callable[[], bool]) -> None:
        wait_until(check, "condition to be true")

    def assert_recording(
        self,
        recording: Recording,
        width: int,
        height: int,
        fps: int,
        duplicate_frames: bool = False,
    ) -> None:
        video = ffprobe_video(recording.video_path)
        audio = ffprobe_audio(recording.video_path)
        length = ffprobe_format(recording.video_path).duration
        self._assert_pts_deltas(video, fps)
        if duplicate_frames:
            self._assert_duplicate_frames(recording, length)
        self.assert_equal(video.codec, "h264")
        self.assert_equal(video.width, width)
        self.assert_equal(video.height, height)
        self.assert_equal(audio.codec, "aac")
        self.assert_greater(length, recording.seconds - MAXIMUM_LENGTH_DIFFERENCE)
        self.assert_less(length, recording.seconds + MAXIMUM_LENGTH_DIFFERENCE)
        expected_frames = fps * length
        self.assert_greater(len(video.frames), (1 - FRAME_COUNT_TOLERANCE_RATIO) * expected_frames)
        self.assert_less(len(video.frames), (1 + FRAME_COUNT_TOLERANCE_RATIO) * expected_frames)

    def _assert_duplicate_frames(self, recording: Recording, length: float) -> None:
        all_duplicates = ffmpeg_duplicate_frames(recording.video_path)
        duplicates = [
            duplicate
            for duplicate in all_duplicates
            if DUPLICATE_SETTLE_SECONDS < duplicate.pts < length - DUPLICATE_SETTLE_SECONDS
        ]
        for duplicate in duplicates[:WORST_DUPLICATES]:
            LOGGER.info(
                "Duplicated frame %.3f s into the recording, %d in a row",
                duplicate.pts,
                duplicate.count,
            )
        self.assert_equal(len(duplicates), 0)

    def _assert_pts_deltas(self, video: FfprobeVideoOutput, fps: int) -> None:
        timestamps = sorted(frame.pts for frame in video.frames)
        deltas = [after - before for before, after in zip(timestamps, timestamps[1:])]
        if len(deltas) == 0:
            return
        expected = 1 / fps
        tolerance = PTS_DELTA_TOLERANCE_RATIO * expected
        start = timestamps[0]
        offsets = [timestamp - start for timestamp in timestamps[1:]]
        end = offsets[-1] - PTS_DELTA_SETTLE_SECONDS
        outliers = [
            (delta, offset)
            for delta, offset in zip(deltas, offsets)
            if abs(delta - expected) > tolerance and PTS_DELTA_SETTLE_SECONDS < offset < end
        ]
        worst = sorted(outliers, key=lambda outlier: -abs(outlier[0] - expected))
        for delta, offset in worst[:WORST_PTS_DELTAS]:
            LOGGER.info("PTS delta %.2f ms at %.3f s into the recording", 1000 * delta, offset)
        self.assert_equal(len(outliers), 0)
