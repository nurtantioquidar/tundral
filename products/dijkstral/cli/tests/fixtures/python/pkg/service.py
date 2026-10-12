"""Service layer exercising every import and call form."""
import json
import os.path as osp
from collections import defaultdict
from typing import TYPE_CHECKING

from . import models
from .models import *
from .models import Repo, User as U
from .util import helper as util_helper, slugify

if TYPE_CHECKING:
    from .models import Admin


def helper(value: str) -> str:
    """Duplicate name: also defined in models and util."""
    return value.lower()


class UserService:
    def __init__(self, repo: Repo[U]) -> None:
        self.repo = repo
        self.cache = defaultdict(list)

    def register(self, name: str) -> U:
        user = U(name=name)
        user.save()
        self.repo.add(slugify(name), user)
        models.validate(user)
        return user

    def promote(self, admin: "Admin", user: User) -> None:
        admin.promote(user)

    def export(self) -> str:
        names = sorted(self.repo.items, key=helper)
        mapped = list(map(util_helper, names))
        return json.dumps(mapped)


def run(path: str) -> UserService:
    service = UserService(Repo())
    service.register(osp.basename(path))
    format_name("x")
    unknown_function()
    return service
