import logging
from collections.abc import Callable

import systest

from .ffmpeg import detect_silence
from .ffmpeg import ffprobe_audio
from .ffmpeg import ffprobe_format
from .ffmpeg import measure_mean_volume
from .ffmpeg import read_image
from .moblin import Moblin
from .utils import Image
from .utils import wait_until
from .virtualcam import AudioSpec
from .virtualcam import Recording

LOGGER = logging.getLogger(__name__)
PIXEL_FORMATS = ["yuv420p", "nv12"]
MINIMUM_FPS_RATIO = 0.8
MAXIMUM_FPS_RATIO = 1.05
MINIMUM_WINDOW_FPS_RATIO = 0.5
MINIMUM_MEAN_VOLUME_DB = -60.0
SILENCE_NOISE_DB = -60.0
MAXIMUM_SILENCE = 0.5
MAXIMUM_AUDIO_LENGTH_DIFFERENCE = 0.5


class TestCase(systest.TestCase):
    def __init__(self, moblin: Moblin, name: str | None = None):
        super().__init__(name)
        self.moblin = moblin

    def teardown(self):
        self.moblin.end()

    def assert_not_all_black(self, image: Image, minimum_ratio: float = 0.01):
        self.assert_greater(image.non_black_ratio(), minimum_ratio)

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
        self._assert_camera_audio(recording, audio)
        self.assert_greater(len(recording.images), 0)
        self.assert_not_all_black(read_image(recording.images[len(recording.images) // 2]))

    def _assert_camera_format(self, recording: Recording, width: int, height: int):
        if recording.video is None:
            raise Exception("ffmpeg did not report any video input format.")
        LOGGER.info(
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
        LOGGER.info(
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

    def _assert_camera_audio(self, recording: Recording, audio: AudioSpec | None):
        probe = ffprobe_audio(recording.audio_path)
        length = ffprobe_format(recording.audio_path).duration
        mean_volume_db = measure_mean_volume(recording.audio_path)
        LOGGER.info(
            "The microphone delivers %s Hz %s channel audio of %.3f s, mean volume %.1f dB",
            probe.sample_rate,
            probe.channels,
            length,
            mean_volume_db,
        )
        if audio is not None:
            self.assert_equal(probe.sample_rate, audio.sample_rate)
            self.assert_equal(probe.channels, audio.channels)
            if recording.audio is not None:
                self.assert_equal(recording.audio.sample_rate, audio.sample_rate)
                self.assert_equal(recording.audio.channels, audio.channels)
        self.assert_greater(length, recording.seconds - MAXIMUM_AUDIO_LENGTH_DIFFERENCE)
        self.assert_less(length, recording.seconds + MAXIMUM_AUDIO_LENGTH_DIFFERENCE)
        self.assert_greater(mean_volume_db, MINIMUM_MEAN_VOLUME_DB)
        silences = detect_silence(recording.audio_path, SILENCE_NOISE_DB, MAXIMUM_SILENCE)
        for silence in silences:
            LOGGER.info("Silence from %.3f to %.3f seconds", silence.start, silence.end)
        self.assert_equal(len(silences), 0)
