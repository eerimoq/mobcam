import time

from ..utils.generate_device_settings import Resolution
from ..utils.generate_device_settings import VideoCodec
from ..utils.generate_device_settings import stream_settings
from ..utils.moblin import Moblin
from ..utils.test_case import TestCase
from ..utils.virtualcam import VirtualCam


class VirtualcamStream(TestCase):
    def __init__(self, moblin: Moblin, video_codec: VideoCodec, resolution: Resolution, fps: int):
        super().__init__(moblin, f"Stream{resolution}@{fps}{video_codec.name}")
        self._video_codec = video_codec
        self._resolution = resolution
        self._fps = fps

    def setup(self) -> None:
        self.moblin.import_settings(
            overrides={"streams": [stream_settings(self._video_codec, self._resolution, self._fps)]}
        )

    def run(self) -> None:
        with VirtualCam(self.moblin.config) as virtualcam:
            self.moblin.go_live()
            time.sleep(2)
            recording = virtualcam.record(10, self.name)
            self.moblin.end()
        width, height = self._resolution.size()
        self.assert_recording(recording, width, height, self._fps)


def tests(moblin: Moblin) -> list[TestCase]:
    return [
        VirtualcamStream(moblin, VideoCodec.H264, Resolution.FULL_HD, 30),
        VirtualcamStream(moblin, VideoCodec.H264, Resolution.FULL_HD, 60),
        VirtualcamStream(moblin, VideoCodec.H265, Resolution.FULL_HD, 30),
        VirtualcamStream(moblin, VideoCodec.H265, Resolution.FULL_HD, 60),
    ]
