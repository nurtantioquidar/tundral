import pytest

from pkg.models import User
from pkg.service import UserService, run


class TestUserService:
    def test_register(self):
        service = run("/tmp/alice")
        assert isinstance(service, UserService)

    def test_parse(self):
        assert User.parse(" bob ").name == "bob"


@pytest.fixture
def service():
    return run("/tmp/x")


def test_run(service):
    assert service.repo
