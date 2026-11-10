from datetime import date
import json
from pathlib import Path
import tempfile
import unittest

from complement_baseline import evaluate, load_baseline
from complement_regressions import PACKAGES


class ComplementBaselineTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.results = Path(self.tmp.name) / "results.jsonl"
        self.package = "github.com/matrix-org/complement/tests"
        self.key = (self.package, "TestKnown", "skip")

    def write(self, tests=(), *, status="pass", extra=()):
        rows = []
        for package in sorted(PACKAGES):
            rows.append({"Action": "start", "Package": package})
            cases = tests if package == self.package else (("TestSentinel", "pass"),)
            for test, action in cases:
                rows.extend([{"Action": "run", "Package": package, "Test": test},
                             {"Action": action, "Package": package, "Test": test}])
            rows.append({"Action": status if package == self.package else "pass", "Package": package})
        rows.extend(extra)
        self.results.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")

    def check(self, baseline=None, exit_code=0):
        return evaluate(self.results, baseline or {}, exit_code=exit_code)[0]

    def test_exact_skip_is_allowed_and_raw_parent_counts_are_retained(self):
        self.write((("TestKnown", "skip"), ("TestParent/Child", "pass"), ("TestParent", "pass")))
        errors, counts = evaluate(self.results, {self.key: 1}, exit_code=0)
        self.assertEqual(errors, [])
        self.assertEqual(counts["pass"], 5)
        self.assertEqual(counts["skip"], 1)

    def test_new_skip_or_failure_cannot_be_diluted_by_passing_tests(self):
        for action in ("skip", "fail"):
            self.write((("TestKnown", "skip"), ("TestNew", action)) + tuple(
                (f"TestPassing{i}", "pass") for i in range(1000)), status="fail" if action == "fail" else "pass")
            self.assertTrue(any("New " in error or "Failed result" in error for error in self.check({self.key: 1}, exit_code=int(action == "fail"))))

    def test_skip_does_not_authorize_failure_or_same_name_in_another_package(self):
        self.write((("TestKnown", "fail"),), status="fail")
        self.assertTrue(self.check({self.key: 1}, exit_code=1))
        self.write((("TestKnown", "skip"),))
        wrong = ("github.com/matrix-org/complement/tests/csapi", "TestKnown", "skip")
        self.assertTrue(self.check({wrong: 1}))

    def test_known_skip_can_be_fixed(self):
        self.write((("TestKnown", "pass"),))
        self.assertEqual(self.check({self.key: 1}), [])

    def test_failures_always_block_even_with_a_parent_or_leaf_waiver(self):
        self.write((("TestParent/Child", "fail"), ("TestParent", "fail")), status="fail")
        baseline = {(self.package, "TestParent/Child", "fail"): 1}
        self.assertTrue(self.check(baseline, exit_code=1))
        self.assertTrue(self.check({(self.package, "TestParent", "fail"): 1}, exit_code=1))
        self.assertTrue(self.check(baseline, exit_code=137))

    def test_truncation_missing_runs_duplicate_terminals_and_empty_packages_fail(self):
        self.write((("TestKnown", "pass"),))
        lines = self.results.read_text().splitlines()
        for changed in (lines[:-1], [line for line in lines if '"Action": "run"' not in line],
                        lines[:-1] + [lines[-2], lines[-1]]):
            self.results.write_text("\n".join(changed))
            self.assertTrue(self.check())
        self.write(())
        self.assertTrue(self.check())

    def test_running_test_cannot_disappear_behind_a_package_pass(self):
        self.write((("TestKnown", "pass"),))
        lines = self.results.read_text().splitlines()
        self.results.write_text("\n".join(line for line in lines if not ('"Action": "pass"' in line and '"Test": "TestKnown"' in line)))
        self.assertTrue(any("did not complete" in error for error in self.check()))

    def test_same_test_names_in_separate_completed_invocations_are_counted(self):
        self.write((("TestKnown", "pass"),), extra=[
            {"Action": "start", "Package": self.package},
            {"Action": "run", "Package": self.package, "Test": "TestKnown"},
            {"Action": "pass", "Package": self.package, "Test": "TestKnown"},
            {"Action": "pass", "Package": self.package},
        ])
        errors, counts = evaluate(self.results, {}, exit_code=0)
        self.assertEqual(errors, [])
        self.assertEqual(counts["pass"], 5)
        self.assertEqual(counts["package_invocations"], 5)

    def test_repeated_known_skip_does_not_gain_another_exception(self):
        self.write((("TestKnown", "skip"),), extra=[
            {"Action": "start", "Package": self.package},
            {"Action": "run", "Package": self.package, "Test": "TestKnown"},
            {"Action": "skip", "Package": self.package, "Test": "TestKnown"},
            {"Action": "pass", "Package": self.package},
        ])
        self.assertTrue(any("count exceeded" in error for error in self.check({self.key: 1})))

    def test_malformed_json_and_unexplained_exit_fail(self):
        self.write((("TestKnown", "pass"),))
        self.assertTrue(self.check(exit_code=1))
        self.results.write_text('{"Action":')
        with self.assertRaises(ValueError):
            self.check()

    def test_baseline_requires_owner_reason_expiry_and_exact_unique_identity(self):
        path = Path(self.tmp.name) / "baseline.json"
        item = {"package": self.package, "test": "TestKnown", "action": "skip", "count": 1,
                "owner": "@maintainer", "reason": "Unsupported optional capability", "expires": "2026-12-10"}
        def load(items):
            path.write_text(json.dumps({"schema": 1, "exceptions": items}))
            return load_baseline(path, today=date(2026, 11, 10))
        self.assertEqual(load([item]), {self.key: 1})
        with self.assertRaises(ValueError):
            load([item, item])
        for field, value in (("owner", ""), ("reason", ""), ("expires", "2026-11-10"),
                             ("package", "another/package"), ("action", "pass"), ("action", "fail"), ("count", True)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                load([{**item, field: value}])

    def test_malformed_baseline_roots_fields_and_revision_mismatch_fail(self):
        path = Path(self.tmp.name) / "baseline.json"
        for data in ([], None, {"schema": 1, "exceptions": "all"},
                     {"schema": 1, "exceptions": [None]},
                     {"schema": 1, "exceptions": [{"package": [], "test": "TestX", "action": "skip"}]}):
            path.write_text(json.dumps(data))
            with self.subTest(data=data), self.assertRaises(ValueError):
                load_baseline(path, today=date(2026, 11, 10))
        path.write_text(json.dumps({"schema": 1, "exceptions": [], "complement_revision": "old"}))
        with self.assertRaises(ValueError):
            load_baseline(path, revision="changed", today=date(2026, 11, 10))


if __name__ == "__main__":
    unittest.main()
