from systest_moblin import test_case

from .moblin import Moblin


class TestCase(test_case.TestCase):
    def __init__(self, moblin: Moblin, name: str | None = None) -> None:
        super().__init__(name)
        self.moblin = moblin

    def teardown(self) -> None:
        self.moblin.end()
