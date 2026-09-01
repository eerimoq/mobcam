import time

from systest_moblin.ffmpeg import FfmpegVideoCodec

from ..utils.generate_device_settings import AudioCodec
from ..utils.generate_device_settings import Resolution
from ..utils.generate_device_settings import VideoCodec
from ..utils.generate_device_settings import stream_settings
from ..utils.moblin import Moblin
from ..utils.test_case import TestCase
from ..utils.utils import FILES_DIR
from ..utils.virtualcam import VirtualCam


class VirtualcamStream(TestCase):
    def __init__(
        self,
        moblin: Moblin,
        video_codec: VideoCodec,
        resolution: Resolution,
        fps: int,
        audio_codec: AudioCodec,
    ):
        name = f"Stream{resolution}@{fps}{video_codec.name}{audio_codec.name.capitalize()}"
        super().__init__(moblin, name)
        self._video_codec = video_codec
        self._resolution = resolution
        self._fps = fps
        self._audio_codec = audio_codec

    def setup(self) -> None:
        self.moblin.import_settings(
            overrides={
                "streams": [
                    stream_settings(self._video_codec, self._resolution, self._fps, self._audio_codec)
                ]
            }
        )

    def run(self) -> None:
        with VirtualCam(self.moblin.config) as virtualcam:
            self.moblin.go_live()
            time.sleep(2)
            recording = virtualcam.record(10, self.name)
            self.moblin.end()
        width, height = self._resolution.size()
        self.assert_recording(
            recording.video_path,
            FILES_DIR,
            has_qr_codes=False,
            duplicated_frames_crops=None,
            width=width,
            height=height,
            fps=self._fps,
            video_codec=FfmpegVideoCodec.H264,
            channels=2,
            check_video_presentation_time_stamps=False,
            check_picture_types=False,
            check_audio_presentation_time_stamps=False,
        )


def tests(moblin: Moblin) -> list[TestCase]:
    return [
        VirtualcamStream(moblin, VideoCodec.H264, Resolution.FULL_HD, 30, AudioCodec.AAC),
        VirtualcamStream(moblin, VideoCodec.H264, Resolution.FULL_HD, 60, AudioCodec.AAC),
        VirtualcamStream(moblin, VideoCodec.H265, Resolution.FULL_HD, 30, AudioCodec.AAC),
        VirtualcamStream(moblin, VideoCodec.H265, Resolution.FULL_HD, 60, AudioCodec.AAC),
        VirtualcamStream(moblin, VideoCodec.H265, Resolution.FULL_HD, 30, AudioCodec.OPUS),
    ]
