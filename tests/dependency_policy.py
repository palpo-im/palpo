"""Reject broad or expired cargo-deny advisory exceptions before auditing."""

import argparse
from datetime import date, datetime, timezone
import json
from pathlib import Path
import re
import sys
import tomllib


def validate(config, policy, *, today=None):
    today = today or datetime.now(timezone.utc).date()
    if not isinstance(config, dict) or not isinstance(policy, dict):
        raise ValueError("Expected configuration and advisory policy objects")
    if policy.get("schema") != 1 or not isinstance(policy.get("exceptions"), list):
        raise ValueError("Expected advisory policy schema 1 with an exceptions list")
    metadata = {}
    for item in policy["exceptions"]:
        if not isinstance(item, dict):
            raise ValueError("Expected advisory exception object")
        advisory = item["id"]
        if not isinstance(advisory, str) or not re.fullmatch(r"RUSTSEC-\d{4}-\d{4}", advisory):
            raise ValueError(f"Expected exact RustSec ID: {advisory}")
        if advisory in metadata:
            raise ValueError(f"Duplicate advisory exception: {advisory}")
        if not isinstance(item["owner"], str) or not item["owner"].strip():
            raise ValueError(f"Missing exception owner: {advisory}")
        if not isinstance(item["reason"], str) or not item["reason"].strip():
            raise ValueError(f"Missing exception reason: {advisory}")
        if not isinstance(item["expires"], str):
            raise ValueError(f"Invalid expiry date: {advisory}")
        expires = date.fromisoformat(item["expires"])
        if expires <= today:
            raise ValueError(f"Expired advisory exception: {advisory} ({expires})")
        metadata[advisory] = item
    ignored = {}
    advisories = config.get("advisories", {})
    if not isinstance(advisories, dict) or not isinstance(advisories.get("ignore", []), list):
        raise ValueError("Expected advisories.ignore list")
    if type(advisories.get("version")) is not int or advisories["version"] != 2:
        raise ValueError("Advisory policy must use version 2")
    if advisories.get("unsound") != "all":
        raise ValueError("Unsoundness must remain blocking for all dependencies")
    for item in advisories.get("ignore", []):
        if not isinstance(item, dict) or set(item) - {"id", "reason", "expiry", "allow"}:
            raise ValueError("Only documented exact-ID advisory exceptions are allowed")
        advisory = item["id"]
        if advisory in ignored:
            raise ValueError(f"Duplicate cargo-deny advisory exception: {advisory}")
        if advisory not in metadata or item.get("reason") != metadata[advisory]["reason"]:
            raise ValueError(f"Missing or inconsistent exception metadata: {advisory}")
        ignored[advisory] = item
    if ignored.keys() != metadata.keys():
        raise ValueError("Exception metadata and cargo-deny ignore IDs differ")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=Path("deny.toml"))
    parser.add_argument("--policy", type=Path, default=Path("tests/security/advisory-exceptions.json"))
    args = parser.parse_args()
    try:
        validate(tomllib.loads(args.config.read_text(encoding="utf-8")),
                 json.loads(args.policy.read_text(encoding="utf-8")))
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f"Invalid dependency advisory policy: {error}", file=sys.stderr)
        return 1
    print("Advisory exceptions are exact, owned, justified and unexpired")
    return 0


if __name__ == "__main__":
    sys.exit(main())
