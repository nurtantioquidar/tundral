"""Release helper for the shop library."""

from tools.versions import bump, current


def release(kind):
    version = bump(current(), kind)
    print(f"releasing {version}")
    return version


def main():
    release("patch")
