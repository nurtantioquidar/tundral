"""Version arithmetic for releases."""


def current():
    return parse(open("VERSION").read())


def parse(text):
    return tuple(int(p) for p in text.strip().split("."))


def bump(version, kind):
    major, minor, patch = version
    if kind == "major":
        return (major + 1, 0, 0)
    if kind == "minor":
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)
