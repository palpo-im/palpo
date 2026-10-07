"""Fail closed on incomplete, skipped, failed or missing mixed federation tests."""

import argparse
import json
from pathlib import Path
import sys


def required_cases(path):
    cases = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        package, test = line.split()
        case = ("github.com/matrix-org/complement/" + package, test)
        if case in cases:
            raise ValueError(f"Duplicate required test: {case}")
        cases.add(case)
    return cases


def evaluate(directory):
    required = required_cases(directory / "required-tests.txt")
    allowed_skips = set((directory / "allowed-skips.txt").read_text().splitlines())
    status = int((directory / "exit-code").read_text())
    errors = []
    terminal = {}
    packages = {}
    summaries = []
    output = []
    started = set()
    with (directory / "results.jsonl").open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            # go test may print non-JSON compiler output; exit status and package
            # completion are checked separately. Malformed JSON is never ignored.
            if not line.startswith("{"):
                output.append(line)
                continue
            event = json.loads(line)
            action, package, test = event.get("Action"), event.get("Package"), event.get("Test")
            if "Output" in event:
                output.append(event["Output"])
            if action == "run" and package and test:
                started.add((package, test))
            if action not in ("pass", "fail", "skip"):
                continue
            if not package:
                raise ValueError(f"Missing package on line {line_number}")
            if test:
                key = (package, test)
                if key in terminal:
                    errors.append(f"Duplicate terminal event: {package}/{test}")
                terminal[key] = action
                summaries.append({"Action": action, "Package": package, "Test": test})
                if action == "fail":
                    errors.append(f"Failed test: {package}/{test}")
                elif action == "skip" and test not in allowed_skips:
                    errors.append(f"Unexpected skip: {package}/{test}")
            else:
                packages[package] = action
                if action != "pass":
                    errors.append(f"Package did not pass: {package}: {action}")
    if status != 0:
        errors.append(f"go test exited with status {status}")
    if not terminal or not any(action == "pass" for action in terminal.values()):
        errors.append("No passing tests were reported")
    for package, test in sorted(started - terminal.keys()):
        errors.append(f"Test started but did not complete: {package}/{test}")
    for package in {package for package, _ in terminal} | {package for package, _ in required}:
        if packages.get(package) != "pass":
            errors.append(f"Missing successful package completion: {package}")
    for package, test in sorted(required):
        if terminal.get((package, test)) != "pass":
            errors.append(f"Required test did not pass: {package}/{test}")
    summaries.sort(key=lambda event: (event["Action"] != "fail", event["Action"] == "skip", event["Package"], event["Test"]))
    (directory / "__test_all.result.jsonl").write_text(
        "".join(json.dumps(event) + "\n" for event in summaries), encoding="utf-8"
    )
    (directory / "results.log").write_text("".join(output), encoding="utf-8")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results_dir", type=Path)
    parser.add_argument("--direction", choices=("both", "synapse-palpo", "palpo-synapse"), default="both")
    args = parser.parse_args()
    directions = ("synapse-palpo", "palpo-synapse") if args.direction == "both" else (args.direction,)
    failed = False
    for direction in directions:
        try:
            errors = evaluate(args.results_dir / direction)
        except (OSError, ValueError, TypeError) as exc:
            errors = [f"Incomplete or invalid results: {exc}"]
        for error in errors:
            print(f"{direction}: {error}", file=sys.stderr)
        if not errors:
            print(f"{direction}: all required tests passed")
        failed |= bool(errors)
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
