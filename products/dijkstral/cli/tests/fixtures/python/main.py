#!/usr/bin/env python3
"""Entry point."""
import sys

import pkg
from pkg import run
from pkg.service import UserService as Svc


def main(argv: list[str]) -> int:
    svc: Svc = run(argv[0])
    pkg.run("again")
    return 0 if svc else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
