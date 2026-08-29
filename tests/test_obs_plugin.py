import argparse

from .suites import obs_plugin
from .utils.moblin import Moblin
from .utils.runner import create_parser
from .utils.runner import run
from .utils.test_case import TestCase


def create_suites(moblin: Moblin, _: argparse.Namespace) -> list[list[TestCase]]:
    return [
        obs_plugin.tests(moblin),
    ]


def main() -> None:
    parser = create_parser("Run OBS Studio plugin tests.")
    run("obs", parser, create_suites)


main()
