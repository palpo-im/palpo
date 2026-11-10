"""Require zero Complement failures and exact, expiring skip exceptions."""

import argparse
from collections import Counter
from datetime import date, datetime, timezone
import json
from pathlib import Path
import sys

from complement_regressions import PACKAGES


def load_baseline(path, *, today=None, revision=None):
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema") != 1 or not isinstance(data.get("exceptions"), list):
        raise ValueError("Expected Complement baseline schema 1 and exceptions")
    if revision is not None and data.get("complement_revision") != revision:
        raise ValueError("Complement revision differs from the reviewed baseline")
    today = today or datetime.now(timezone.utc).date()
    exceptions = {}
    for item in data["exceptions"]:
        if not isinstance(item, dict):
            raise ValueError("Expected baseline exception object")
        key = (item["package"], item["test"], item["action"])
        if not all(isinstance(value, str) for value in key):
            raise ValueError("Expected string package/test/action")
        if key[0] not in PACKAGES or not key[1].startswith("Test") or key[2] != "skip":
            raise ValueError(f"Invalid baseline case: {key}")
        if key in exceptions:
            raise ValueError(f"Duplicate baseline case: {key}")
        if not all(isinstance(item[field], str) and item[field].strip() for field in ("owner", "reason", "expires")):
            raise ValueError(f"Missing baseline ownership/reason: {key}")
        if date.fromisoformat(item["expires"]) <= today:
            raise ValueError(f"Expired baseline exception: {key}")
        if type(item["count"]) is not int or item["count"] < 1:
            raise ValueError(f"Invalid baseline occurrence count: {key}")
        exceptions[key] = item["count"]
    return exceptions


def evaluate(results, baseline, *, exit_code):
    errors, active, completed = [], {}, Counter()
    observed, counts = Counter(), Counter()
    invocations = 0
    for number, line in enumerate(results.read_text(encoding="utf-8").splitlines(), 1):
        if not line.startswith("{"):
            continue
        event = json.loads(line)
        if not isinstance(event, dict):
            raise ValueError(f"Expected Go event object on line {number}")
        action, package, test = event.get("Action"), event.get("Package"), event.get("Test")
        if action not in ("start", "run", "pass", "fail", "skip"):
            continue
        if not isinstance(package, str) or package not in PACKAGES:
            errors.append(f"Unknown package on line {number}: {package}")
            continue
        if action == "start":
            if package in active:
                errors.append(f"Package restarted before completion: {package}")
            active[package] = {"started": set(), "terminal": {}}
            invocations += 1
            continue
        state = active.get(package)
        if state is None:
            errors.append(f"Result outside package invocation on line {number}: {(package, test, action)}")
            continue
        started, terminal = state["started"], state["terminal"]
        if action == "run":
            if not isinstance(test, str) or not test.startswith("Test"):
                errors.append(f"Invalid test run on line {number}")
            elif test in started:
                errors.append(f"Duplicate test run in package invocation: {(package, test)}")
            else:
                started.add(test)
            continue
        if test:
            if not isinstance(test, str) or not test.startswith("Test"):
                raise ValueError(f"Invalid terminal test on line {number}")
            key = (package, test, action)
            counts[action] += 1
            observed[key] += 1
            if test not in started:
                errors.append(f"Terminal without test run: {key}")
            if test in terminal:
                errors.append(f"Duplicate test terminal in package invocation: {key}")
            terminal[test] = action
            continue
        for missing in sorted(started - terminal.keys()):
            errors.append(f"Test did not complete: {(package, missing)}")
        if not started:
            errors.append(f"Package ran no tests: {package}")
        # Failures always block, including parents with failing descendants.
        # Only named skips may have exceptions; a failed setup/cleanup cannot
        # be excused by a known failing child.
        for name, result in terminal.items():
            if result == "fail":
                errors.append(f"Failed result: {(package, name)}")
            elif result == "skip":
                key = (package, name, result)
                if key not in baseline:
                    errors.append(f"New {result}: {(package, name)}")
        if action != "pass":
            errors.append(f"Package did not pass {package}: {action}")
        completed[package] += 1
        del active[package]
    for package in sorted(PACKAGES):
        if completed[package] == 0:
            errors.append(f"Missing package completion: {package}")
    for package in active:
        errors.append(f"Truncated package invocation: {package}")
    for key, allowed_count in baseline.items():
        if observed[key] > allowed_count:
            errors.append(f"Baseline occurrence count exceeded: {key}: {observed[key]} > {allowed_count}")
    # 137/timeout/cancellation cannot be excused by known failing tests.
    if exit_code != 0:
        errors.append(f"Unexpected go test exit: {exit_code}, expected 0")
    return errors, {**{action: counts[action] for action in ("pass", "fail", "skip")},
                    "package_invocations": invocations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("--exit-code-file", type=Path, required=True)
    parser.add_argument("--revision", help="Actual pinned Complement commit to match against the baseline")
    args = parser.parse_args()
    try:
        errors, counts = evaluate(args.results, load_baseline(args.baseline, revision=args.revision),
                                  exit_code=int(args.exit_code_file.read_text()))
        print(f"Raw terminal events, including parents and every invocation: {counts}")
    except (OSError, ValueError, TypeError, KeyError) as error:
        errors = [f"Incomplete or invalid Complement evidence: {error}"]
    for error in errors:
        print(error, file=sys.stderr)
    if not errors:
        print("Zero Complement failures; exact skip exceptions only")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
