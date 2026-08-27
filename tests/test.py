from .suites import virtualcam
from .utils.runner import create_parser
from .utils.runner import run


def create_suites(moblin, _):
    return [
        virtualcam.tests(moblin),
    ]


def main():
    parser = create_parser("Run tests.")
    run("test", parser, create_suites)


main()
