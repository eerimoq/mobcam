import logging
import statistics
from collections.abc import Callable

import systest

from .ffmpeg import AUDIO_CODEC
from .ffmpeg import VIDEO_CODEC
from .ffmpeg import detect_silence
from .ffmpeg import ffprobe_audio
from .ffmpeg import ffprobe_format
from .ffmpeg import ffprobe_video
from .ffmpeg import measure_mean_volume
from .moblin import Moblin
from .utils import wait_until
from .virtualcam import AudioSpec
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

    def assert_camera_recording(
        self,
        recording: Recording,
        width: int,
        height: int,
        fps: int,
        audio: AudioSpec | None = None,
    ):
        self._assert_camera_format(recording, width, height)
        self._assert_camera_frames(recording, fps)
        self._assert_pts_deltas(recording, fps)
        self._assert_recorded_file(recording, width, height, audio)
        self._assert_camera_audio(recording, audio)

    def _assert_camera_format(self, recording: Recording, width: int, height: int):
        if recording.video is None:
            raise Exception("ffmpeg did not report any video input format.")
        LOGGER.debug(
            "The camera delivers %s %sx%s",
            recording.video.pixel_format,
            recording.video.width,
            recording.video.height,
        )
        self.assert_in(recording.video.pixel_format, PIXEL_FORMATS)
        self.assert_equal(recording.video.width, width)
        self.assert_equal(recording.video.height, height)

    def _assert_camera_frames(self, recording: Recording, fps: int):
        self.assert_greater(len(recording.window_rates), 0)
        slowest = min(recording.window_rates)
        LOGGER.debug(
            "%s frames in %.3f s, %.2f fps, slowest %.1f fps of the %s windows",
            recording.frames,
            recording.duration,
            recording.fps(),
            slowest,
            len(recording.window_rates),
        )
        self.assert_greater(recording.fps(), MINIMUM_FPS_RATIO * fps)
        self.assert_less(recording.fps(), MAXIMUM_FPS_RATIO * fps)
        self.assert_greater(slowest, MINIMUM_WINDOW_FPS_RATIO * fps)

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

    def _assert_recorded_file(
        self,
        recording: Recording,
        width: int,
        height: int,
        audio: AudioSpec | None,
    ):
        video = ffprobe_video(recording.video_path)
        recorded_audio = ffprobe_audio(recording.video_path)
        length = ffprobe_format(recording.video_path).duration
        LOGGER.debug(
            "%s holds %s frames of %s %sx%s and %s Hz %s channel %s of %.3f s, %s MiB",
            recording.video_path.name,
            len(video.frames),
            video.codec,
            video.width,
            video.height,
            recorded_audio.sample_rate,
            recorded_audio.channels,
            recorded_audio.codec,
            length,
            round(recording.video_path.stat().st_size / 1024 / 1024, 1),
        )
        self.assert_equal(video.codec, VIDEO_CODEC)
        self.assert_equal(video.width, width)
        self.assert_equal(video.height, height)
        self.assert_equal(recorded_audio.codec, AUDIO_CODEC)
        if audio is not None:
            self.assert_equal(recorded_audio.sample_rate, audio.sample_rate)
            self.assert_greater_equal(recorded_audio.channels, audio.channels)
        self.assert_greater(length, recording.seconds - MAXIMUM_VIDEO_LENGTH_DIFFERENCE)
        self.assert_less(length, recording.seconds + MAXIMUM_VIDEO_LENGTH_DIFFERENCE)

    def _assert_camera_audio(self, recording: Recording, audio: AudioSpec | None):
        probe = ffprobe_audio(recording.audio_path)
        length = ffprobe_format(recording.audio_path).duration
        mean_volume_db = measure_mean_volume(recording.audio_path)
        LOGGER.debug(
            "The microphone delivers %s Hz %s channel audio of %.3f s, mean volume %.1f dB",
            probe.sample_rate,
            probe.channels,
            length,
            mean_volume_db,
        )
        if audio is not None:
            self.assert_equal(probe.sample_rate, audio.sample_rate)
            self.assert_greater_equal(probe.channels, audio.channels)
            if recording.audio is not None:
                self.assert_equal(recording.audio.sample_rate, audio.sample_rate)
                self.assert_equal(recording.audio.channels, probe.channels)
        self.assert_greater(length, recording.seconds - MAXIMUM_AUDIO_LENGTH_DIFFERENCE)
        self.assert_less(length, recording.seconds + MAXIMUM_AUDIO_LENGTH_DIFFERENCE)
        self.assert_greater(mean_volume_db, MINIMUM_MEAN_VOLUME_DB)
        silences = detect_silence(recording.audio_path, SILENCE_NOISE_DB, MAXIMUM_SILENCE)
        for silence in silences:
            LOGGER.info("Silence from %.3f to %.3f seconds", silence.start, silence.end)
        self.assert_equal(len(silences), 0)
