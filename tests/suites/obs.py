import time

from ..utils.generate_device_settings import Resolution
from ..utils.generate_device_settings import VideoCodec
from ..utils.generate_device_settings import stream_settings
from ..utils.moblin import Moblin
from ..utils.obs import Obs
from ..utils.test_case import TestCase


class Record(TestCase):
    def __init__(
        self,
        moblin: Moblin,
        video_codec: VideoCodec,
        resolution: Resolution,
        fps: int,
        buffering: bool = False,
    ):
        name = f"Record{video_codec.name}-{resolution}@{fps}"
        if buffering:
            name += "-Buffering"
        super().__init__(moblin, name)
        self._video_codec = video_codec
        self._resolution = resolution
        self._fps = fps
        self._buffering = buffering

    def setup(self):
        self.moblin.import_settings(
            overrides={"streams": [stream_settings(self._video_codec, self._resolution, self._fps)]}
        )

    def run(self):
        with Obs(self._resolution, self._fps, self.name) as obs:
            obs.create_source(buffering=self._buffering)
            self.moblin.go_live()
            obs.wait_until_connected()
            time.sleep(2)
            recording = obs.record(10)
            self.moblin.end()
        width, height = self._resolution.size()
        self.assert_obs_recording(recording, width=width, height=height, fps=self._fps)


def tests(moblin: Moblin):
    return [
        Record(moblin, VideoCodec.H264, Resolution.FULL_HD, 30),
        Record(moblin, VideoCodec.H264, Resolution.FULL_HD, 60),
        Record(moblin, VideoCodec.H265, Resolution.FULL_HD, 30),
        Record(moblin, VideoCodec.H265, Resolution.FULL_HD, 60),
        Record(moblin, VideoCodec.H265, Resolution.FULL_HD, 30, buffering=True),
    ]
