from .suites import obs_plugin
from .utils.runner import create_parser
from .utils.runner import run


def create_suites(moblin, _):
    return [
        obs_plugin.tests(moblin),
    ]


def main():
    parser = create_parser("Run OBS Studio plugin tests.")
    run("obs", parser, create_suites)


main()
