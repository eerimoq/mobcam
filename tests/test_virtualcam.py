from .suites import virtualcam
from .utils.runner import create_parser
from .utils.runner import run


def create_suites(moblin, _):
    return [
        virtualcam.tests(moblin),
    ]


def main():
    parser = create_parser("Run virtual camera tests.")
    run("virtualcam", parser, create_suites)


main()
