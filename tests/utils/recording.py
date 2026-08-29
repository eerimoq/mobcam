from dataclasses import dataclass
from pathlib import Path


@dataclass
class Recording:
    seconds: float
    video_path: Path
