"""Require explicit PASS results for the Complement regressions fixed by a PR."""

import argparse
import json
from pathlib import Path
import sys


PACKAGES = {
    "github.com/matrix-org/complement/" + name
    for name in ("tests/csapi", "tests", "tests/msc3930", "tests/msc3967")
}


def required_cases(directory):
    required = set()
    for path in sorted(directory.glob("*.json")):
        cases = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(cases, list) or not cases:
            raise ValueError(f"Expected a nonempty case list: {path}")
        for case in cases:
            package, test = case["package"], case["test"]
            if package not in PACKAGES or not isinstance(test, str) or not test.startswith("Test"):
                raise ValueError(f"Invalid required case: {case}")
            key = (package, test)
            if key in required:
                raise ValueError(f"Duplicate required case: {key}")
            required.add(key)
    if not required:
        raise ValueError("No required regression cases found")
    return required


def evaluate(results, required, *, require_clean=False, exit_code=None):
    errors, terminal, packages, started = [], {}, {}, set()
    for number, line in enumerate(results.read_text(encoding="utf-8").splitlines(), 1):
        # Compiler diagnostics may accompany go test -json. JSON records must
        # still parse, and package/test completion is checked independently.
        if not line.startswith("{"):
            continue
        event = json.loads(line)
        action, package, test = event.get("Action"), event.get("Package"), event.get("Test")
        if action == "run" and package and test:
            started.add((package, test))
        if action not in ("pass", "fail", "skip"):
            continue
        if not package:
            raise ValueError(f"Missing package on line {number}")
        key = (package, test)
        target = terminal if test else packages
        target_key = key if test else package
        if target_key in target:
            errors.append(f"Duplicate terminal event: {key}")
        target[target_key] = action
        if require_clean and action == "fail":
            errors.append(f"Failed result: {key}")
    for package in sorted(PACKAGES):
        if packages.get(package) not in ("pass", "fail"):
            errors.append(f"Missing package completion: {package}")
    for key in sorted(started - terminal.keys()):
        errors.append(f"Test did not complete: {key}")
    for key in sorted(required):
        if key not in started or terminal.get(key) != "pass":
            errors.append(f"Required test did not pass: {key}: {terminal.get(key, 'missing')}")
        # A parent PASS must not hide a failed/skipped required descendant.
        for child, action in terminal.items():
            if child[0] == key[0] and child[1].startswith(key[1] + "/") and action != "pass":
                errors.append(f"Required descendant did not pass: {child}: {action}")
    if require_clean and exit_code != 0:
        errors.append(f"go test did not exit successfully: {exit_code}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("manifest_dir", type=Path)
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--exit-code-file", type=Path)
    args = parser.parse_args()
    try:
        required = required_cases(args.manifest_dir)
        status = int(args.exit_code_file.read_text()) if args.exit_code_file else None
        errors = evaluate(args.results, required, require_clean=args.require_clean, exit_code=status)
    except (OSError, ValueError, TypeError, KeyError) as exc:
        errors = [f"Incomplete or invalid results: {exc}"]
    for error in errors:
        print(error, file=sys.stderr)
    if not errors:
        print(f"All {len(required)} required Complement regression cases explicitly passed")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
