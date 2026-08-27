import time
from collections.abc import Callable
from pathlib import Path

TEST_DIR = Path(__file__).parent.parent.resolve()
FILES_DIR = TEST_DIR / "files"


def wait_until(
    check: Callable[[], bool],
    description: str,
    timeout: float = 60,
    ignore_errors: bool = False,
):
    end_time = time.monotonic() + timeout
    while time.monotonic() < end_time:
        try:
            if check():
                return
        except Exception:
            if not ignore_errors:
                raise
        time.sleep(0.5)
    raise Exception(f"Timeout waiting for {description}")
