from .suites import obs
from .utils.runner import create_parser
from .utils.runner import run


def create_suites(moblin, _):
    return [
        obs.tests(moblin),
    ]


def main():
    parser = create_parser("Run OBS Studio plugin tests.")
    run("obs", parser, create_suites)


main()
