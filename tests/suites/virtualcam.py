import time

from ..utils.config import MOBCAM_URL
from ..utils.generate_device_settings import AudioCodec
from ..utils.generate_device_settings import Resolution
from ..utils.generate_device_settings import VideoCodec
from ..utils.generate_device_settings import uuid
from ..utils.moblin import Moblin
from ..utils.test_case import TestCase
from ..utils.virtualcam import VirtualCam

RECORDING_SECONDS = 10
SETTLE_SECONDS = 2
BITRATE = 5_000_000


def stream_settings(video_codec: VideoCodec, resolution: Resolution, fps: int):
    return {
        "id": uuid(),
        "name": "Mobcam",
        "enabled": True,
        "url": MOBCAM_URL,
        "codec": video_codec,
        "audioCodec": AudioCodec.AAC,
        "resolution": resolution,
        "fps": fps,
        "bitrate": BITRATE,
        "bitrateRateControl": "CBR",
    }


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
            virtualcam.wait_until_connected()
            time.sleep(SETTLE_SECONDS)
            recording = virtualcam.record(RECORDING_SECONDS, self.name)
            width, height = self._resolution.size()
            self.assert_camera_recording(
                recording,
                width=width,
                height=height,
                fps=self._fps,
                audio=virtualcam.audio_spec(),
            )
            self.assert_equal(virtualcam.log.errors(), [])
            self.assert_equal(virtualcam.log.warnings(), [])


def tests(moblin: Moblin):
    return [
        Stream(moblin, VideoCodec.H264, Resolution.FULL_HD, 30),
        Stream(moblin, VideoCodec.H264, Resolution.FULL_HD, 60),
        Stream(moblin, VideoCodec.H265, Resolution.FULL_HD, 30),
        Stream(moblin, VideoCodec.H265, Resolution.FULL_HD, 60),
    ]
