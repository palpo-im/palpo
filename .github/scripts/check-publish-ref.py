#!/usr/bin/env python3
"""Fence moving image tags after the publisher job acquires its concurrency lock."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys


def main():
    try:
        output = Path(os.environ["GITHUB_OUTPUT"])
        # Leave a closed gate even when the lookup or response validation fails.
        with output.open("a") as stream:
            stream.write("publish=false\n")
        ref = os.environ["GITHUB_REF"]
        revision = os.environ["GITHUB_SHA"]
        repository = os.environ["GITHUB_REPOSITORY"]
        if not re.fullmatch(r"refs/(?:heads|tags)/[^\s]+", ref):
            raise ValueError("invalid publication ref")
        if not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise ValueError("invalid publication revision")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("invalid publication repository")

        publish = True
        if ref in ("refs/heads/main", "refs/heads/next"):
            branch = ref.removeprefix("refs/heads/")
            result = subprocess.run(
                ["gh", "api", "--method", "GET", f"repos/{repository}/git/ref/heads/{branch}"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=30,
                check=False,
            )
            if result.returncode != 0:
                # Do not echo CLI stderr: authentication/debug output may be sensitive.
                raise ValueError("branch ref lookup failed")
            payload = json.loads(result.stdout)
            if not isinstance(payload, dict) or payload.get("ref") != ref:
                raise ValueError("branch ref lookup returned an unexpected ref")
            target = payload.get("object")
            if (
                not isinstance(target, dict)
                or target.get("type") != "commit"
                or not isinstance(target.get("sha"), str)
                or not re.fullmatch(r"[0-9a-f]{40}", target["sha"])
            ):
                raise ValueError("branch ref lookup returned an invalid commit")
            publish = target["sha"] == revision

        with output.open("a") as stream:
            stream.write(f"publish={str(publish).lower()}\n")
        if publish:
            print(f"Publication allowed for {ref} at {revision}.")
        else:
            print(f"Skipping stale publication for {ref} at {revision}.")
        return 0
    except (KeyError, OSError, ValueError, TypeError, subprocess.SubprocessError) as error:
        print(f"Publication ref check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
