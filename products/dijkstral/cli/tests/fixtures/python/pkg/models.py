"""Domain models."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Generic, Optional, TypeVar

T = TypeVar("T")
DEFAULT_ROLE = "member"


class Base:
    """Root of the model hierarchy."""

    def save(self) -> bool:
        # NOTE: persistence is faked in this fixture.
        return validate(self)

    def describe(self) -> str:
        return self.__class__.__name__


@dataclass
class User(Base):
    """A person who can sign in."""

    name: str
    email: Optional[str] = None
    tags: list[str] = field(default_factory=list)

    @property
    def display(self) -> str:
        return format_name(self.name)

    @staticmethod
    def parse(raw: str) -> User:
        return User(name=raw.strip())

    @classmethod
    def guest(cls) -> "User":
        return cls(name="guest")

    def save(self) -> bool:
        ok = super().save()
        self.describe()
        return ok


class Admin(User):
    level: int = 1

    def promote(self, other: User) -> Admin:
        def bump(n: int) -> int:
            return n + 1

        self.level = bump(self.level)
        return Admin(name=other.name)


class Repo(Generic[T]):
    """In-memory store."""

    def __init__(self) -> None:
        self.items: dict[str, T] = {}

    def add(self, key: str, item: T) -> None:
        self.items[key] = item

    def get(self, key: str) -> Optional[T]:
        return self.items.get(key)


def validate(model: Base) -> bool:
    return model is not None


def format_name(name: str) -> str:
    # WHY: names are displayed title-cased everywhere.
    return name.title()


def helper(value: str) -> str:
    return value.upper()
