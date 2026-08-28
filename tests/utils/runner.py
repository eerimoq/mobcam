import argparse
import logging
import shutil
import textwrap
from collections.abc import Callable

import systest

from .config import Config
from .moblin import Moblin
from .utils import FILES_DIR

MakeTests = Callable[[Moblin, argparse.Namespace], list]


class HelpFormatter(argparse.HelpFormatter):
    def _split_lines(self, text: str, width: int) -> list[str]:
        lines = []
        for line in text.splitlines():
            indent = " " * (len(line) - len(line.lstrip()) + 2)
            lines += textwrap.wrap(line, width, subsequent_indent=indent) or [""]
        return lines


def create_parser(description: str) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=description, formatter_class=HelpFormatter)
    return parser


def _remove_previous_run_artifacts():
    shutil.rmtree(FILES_DIR, ignore_errors=True)
    FILES_DIR.mkdir(parents=True)


def run(name: str, parser: argparse.ArgumentParser, make_tests: MakeTests):
    _remove_previous_run_artifacts()
    sequencer = systest.setup(name, parser, add_date_to_log_filename=False)
    sequencer.remove_filtered_testcases = True
    sequencer.compact_output = True
    args = parser.parse_args()
    logging.getLogger("urllib3.connectionpool").setLevel(logging.INFO)
    logging.getLogger("websockets.client").setLevel(logging.INFO)
    config = Config()
    moblin = Moblin(config)
    with moblin:
        moblin.end()
        sequencer.run(*make_tests(moblin, args))
    sequencer.report_and_exit(json=False, dot=False)
