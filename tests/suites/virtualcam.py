import time

from ..utils.generate_device_settings import Resolution
from ..utils.generate_device_settings import VideoCodec
from ..utils.generate_device_settings import stream_settings
from ..utils.moblin import Moblin
from ..utils.test_case import TestCase
from ..utils.virtualcam import VirtualCam


class Stream(TestCase):
    def __init__(self, moblin: Moblin, video_codec: VideoCodec, resolution: Resolution, fps: int):
        super().__init__(moblin, f"Stream{video_codec.name}-{resolution}@{fps}")
        self._video_codec = video_codec
        self._resolution = resolution
        self._fps = fps

    def setup(self):
        self.moblin.import_settings(
            overrides={"streams": [stream_settings(self._video_codec, self._resolution, self._fps)]}
        )

    def run(self):
        with VirtualCam(self.moblin.config) as virtualcam:
            self.moblin.go_live()
            time.sleep(2)
            recording = virtualcam.record(10, self.name)
            self.moblin.end()
        width, height = self._resolution.size()
        self.assert_camera_recording(recording, width=width, height=height, fps=self._fps)


def tests(moblin: Moblin):
    return [
        Stream(moblin, VideoCodec.H264, Resolution.FULL_HD, 30),
        Stream(moblin, VideoCodec.H264, Resolution.FULL_HD, 60),
        Stream(moblin, VideoCodec.H265, Resolution.FULL_HD, 30),
        Stream(moblin, VideoCodec.H265, Resolution.FULL_HD, 60),
    ]
