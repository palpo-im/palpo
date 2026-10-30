"""Verify that percentage, skip, truncation and duplicate events cannot hide failures."""

import json
from pathlib import Path
import tempfile
import unittest

from complement_regressions import PACKAGES, evaluate, required_cases


class RequiredRegressionGateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.results = self.directory / "results.jsonl"
        self.package = "github.com/matrix-org/complement/tests"
        self.key = (self.package, "TestFixed")

    def write(self, *events, package_status="pass"):
        records = [{"Action": "run", "Package": self.package, "Test": "TestFixed"}]
        records += [{"Package": self.package, **event} for event in events]
        records += [{"Package": package, "Action": package_status} for package in sorted(PACKAGES)]
        self.results.write_text("".join(json.dumps(event) + "\n" for event in records), encoding="utf-8")

    def check(self, **kwargs):
        return evaluate(self.results, {self.key}, **kwargs)

    def test_explicit_pass_succeeds(self):
        self.write({"Action": "pass", "Test": "TestFixed"})
        self.assertEqual(self.check(), [])

    def test_required_failure_cannot_be_hidden_by_other_passing_tests(self):
        self.write({"Action": "fail", "Test": "TestFixed"}, *[
            {"Action": "pass", "Test": f"TestOther{i}"} for i in range(1000)
        ], package_status="fail")
        self.assertTrue(any("Required test" in error for error in self.check()))

    def test_skip_and_missing_fail(self):
        for events in ((), ({"Action": "skip", "Test": "TestFixed"},)):
            with self.subTest(events=events):
                self.write(*events)
                self.assertTrue(self.check())

    def test_duplicate_and_contradictory_terminals_fail(self):
        for action in ("pass", "fail"):
            self.write({"Action": "pass", "Test": "TestFixed"}, {"Action": action, "Test": "TestFixed"})
            self.assertTrue(any("Duplicate" in error for error in self.check()))

    def test_package_identity_is_required(self):
        self.write({"Action": "pass", "Test": "TestFixed", "Package": "wrong/package"})
        self.assertTrue(self.check())

    def test_parent_pass_cannot_hide_failed_or_skipped_child(self):
        for action in ("fail", "skip"):
            self.write({"Action": "pass", "Test": "TestFixed"}, {"Action": action, "Test": "TestFixed/child"})
            self.assertTrue(any("descendant" in error for error in self.check()))

    def test_truncated_package_is_rejected(self):
        self.write({"Action": "pass", "Test": "TestFixed"})
        lines = self.results.read_text().splitlines()
        self.results.write_text("\n".join(lines[:-1]))
        self.assertTrue(any("package completion" in error for error in self.check()))

    def test_malformed_json_is_rejected(self):
        self.results.write_text('{"Action":')
        with self.assertRaises(ValueError):
            self.check()

    def test_empty_or_duplicate_manifests_are_rejected(self):
        with self.assertRaises(ValueError):
            required_cases(self.directory)
        manifest = [{"package": self.package, "test": "TestFixed"}]
        (self.directory / "one.json").write_text(json.dumps(manifest))
        self.assertEqual(required_cases(self.directory), {self.key})
        (self.directory / "two.json").write_text(json.dumps(manifest))
        with self.assertRaises(ValueError):
            required_cases(self.directory)

    def test_clean_suite_requires_zero_failures_and_zero_exit(self):
        self.write({"Action": "pass", "Test": "TestFixed"})
        self.assertEqual(self.check(require_clean=True, exit_code=0), [])
        self.assertTrue(self.check(require_clean=True, exit_code=1))
        self.write({"Action": "pass", "Test": "TestFixed"}, {"Action": "fail", "Test": "TestOther"}, package_status="fail")
        self.assertTrue(self.check(require_clean=True, exit_code=0))


if __name__ == "__main__":
    unittest.main()
