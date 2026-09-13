#!/usr/bin/env python3
"""Answer a harness's clarifying questions from a ground-truth specification.

Used as Butterfly's `--answers-cmd` in the private suite's clarify experiment: the worker sees
only the raw commit title, the oracle holds the rewritten statement (what a well-informed user
knows), and answers strictly from it. Reads the questions on stdin, prints the answer on stdout.

    clarify_oracle.py --truth statement.txt [--model deepseek-flash] [--log answers.log]

Needs DEEPSEEK_API_KEY. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.request

SYSTEM = (
    "You are the engineer who wrote the ticket below and you know the codebase. Another "
    "engineer is about to implement it and asks you questions. Answer each question precisely "
    "and briefly, using ONLY the specification. Give exact names, signatures, messages and "
    "behaviours when the specification states them. When the specification does not settle a "
    "question, reply 'Not specified; your assumption is fine.' Never invent details, never "
    "describe how to implement, and do not paste the whole specification."
)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--truth", required=True)
    p.add_argument("--model", default="deepseek-flash")
    p.add_argument("--base-url", default="https://api.deepseek.com")
    p.add_argument("--log")
    args = p.parse_args()
    key = os.environ.get("DEEPSEEK_API_KEY")
    if not key:
        print("DEEPSEEK_API_KEY is not set", file=sys.stderr)
        return 2
    truth = open(args.truth).read()
    questions = sys.stdin.read()
    body = {
        "model": args.model,
        "temperature": 0,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {
                "role": "user",
                "content": f"SPECIFICATION:\n{truth}\n\nQUESTIONS:\n{questions}",
            },
        ],
    }
    req = urllib.request.Request(
        f"{args.base_url}/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {key}"},
    )
    with urllib.request.urlopen(req, timeout=180) as resp:
        data = json.load(resp)
    answer = data["choices"][0]["message"]["content"].strip()
    if args.log:
        with open(args.log, "a") as fh:
            fh.write(f"### QUESTIONS\n{questions}\n### ANSWER\n{answer}\n\n")
    print(answer)
    return 0


if __name__ == "__main__":
    sys.exit(main())
