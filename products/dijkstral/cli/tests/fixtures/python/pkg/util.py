import re
from functools import wraps


def traced(fn):
    @wraps(fn)
    def wrapper(*args, **kwargs):
        return fn(*args, **kwargs)

    return wrapper


@traced
def slugify(text: str) -> str:
    return re.sub(r"\W+", "-", helper(text))


def helper(value: str) -> str:
    return value.strip()
