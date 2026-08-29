import re
import threading
from pathlib import Path

TEST_DIR = Path(__file__).parent.parent.resolve()
ROOT_DIR = TEST_DIR.parent
FILES_DIR = TEST_DIR / "files"


class Log:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._lines: list[str] = []

    def add(self, line: str) -> None:
        with self._lock:
            self._lines.append(line)

    def lines(self) -> list[str]:
        with self._lock:
            return list(self._lines)

    def match(self, pattern: re.Pattern[str]) -> re.Match[str] | None:
        for line in self.lines():
            found = pattern.search(line)
            if found is not None:
                return found
        return None
