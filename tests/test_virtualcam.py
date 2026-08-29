import argparse

from .suites import virtualcam
from .utils.moblin import Moblin
from .utils.runner import create_parser
from .utils.runner import run
from .utils.test_case import TestCase


def create_suites(moblin: Moblin, _: argparse.Namespace) -> list[list[TestCase]]:
    return [
        virtualcam.tests(moblin),
    ]


def main() -> None:
    parser = create_parser("Run virtual camera tests.")
    run("virtualcam", parser, create_suites)


main()
