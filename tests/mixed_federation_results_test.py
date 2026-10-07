"""Regression tests for the CI gate; no Docker or homeserver required."""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from mixed_federation_results import evaluate


class ResultsGateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.package = "github.com/matrix-org/complement/tests/palpo_mixed"
        (self.directory / "required-tests.txt").write_text("tests/palpo_mixed TestMixedPublicRooms\n")
        (self.directory / "allowed-skips.txt").write_text("TestExisting/known_skip\n")
        (self.directory / "exit-code").write_text("0\n")

    def events(self, *events):
        (self.directory / "results.jsonl").write_text(
            "".join(json.dumps({"Package": self.package, **event}) + "\n" for event in events)
        )

    def passing(self, *extra):
        self.events(
            {"Action": "run", "Test": "TestMixedPublicRooms"},
            {"Action": "pass", "Test": "TestMixedPublicRooms"},
            *extra,
            {"Action": "pass"},
        )

    def test_complete_run_passes(self):
        self.passing()
        self.assertEqual(evaluate(self.directory), [])

    def test_known_subtest_skip_passes(self):
        self.passing({"Action": "skip", "Test": "TestExisting/known_skip"})
        self.assertEqual(evaluate(self.directory), [])

    def test_new_skip_fails(self):
        self.passing({"Action": "skip", "Test": "TestMixedPublicRooms/new_skip"})
        self.assertTrue(any("Unexpected skip" in error for error in evaluate(self.directory)))

    def test_missing_required_test_fails(self):
        self.events({"Action": "pass", "Test": "TestUnrelated"}, {"Action": "pass"})
        self.assertTrue(any("Required test" in error for error in evaluate(self.directory)))

    def test_skipped_required_test_fails_even_if_allowed(self):
        (self.directory / "allowed-skips.txt").write_text("TestMixedPublicRooms\n")
        self.events({"Action": "skip", "Test": "TestMixedPublicRooms"}, {"Action": "pass"})
        self.assertTrue(any("Required test" in error for error in evaluate(self.directory)))

    def test_nonzero_exit_fails_even_with_passing_events(self):
        self.passing()
        (self.directory / "exit-code").write_text("1\n")
        self.assertTrue(any("exited with status" in error for error in evaluate(self.directory)))

    def test_package_failure_without_failed_test_fails(self):
        self.events({"Action": "pass", "Test": "TestMixedPublicRooms"}, {"Action": "fail"})
        self.assertTrue(any("Package did not pass" in error for error in evaluate(self.directory)))

    def test_timeout_or_truncated_run_fails(self):
        self.events({"Action": "run", "Test": "TestMixedPublicRooms"})
        self.assertTrue(any("did not complete" in error for error in evaluate(self.directory)))

    def test_failed_subtest_cannot_be_hidden_by_parent_pass(self):
        self.passing({"Action": "fail", "Test": "TestMixedPublicRooms/GET"})
        self.assertTrue(any("Failed test" in error for error in evaluate(self.directory)))

    def test_no_tests_fails(self):
        self.events({"Action": "pass"})
        self.assertTrue(any("No passing tests" in error for error in evaluate(self.directory)))

    def test_duplicate_terminal_events_fail(self):
        self.passing({"Action": "pass", "Test": "TestMixedPublicRooms"})
        self.assertTrue(any("Duplicate terminal" in error for error in evaluate(self.directory)))

    def test_malformed_json_fails(self):
        (self.directory / "results.jsonl").write_text('{"Action":')
        with self.assertRaises(ValueError):
            evaluate(self.directory)

    def test_missing_results_fail(self):
        with self.assertRaises(OSError):
            evaluate(self.directory)

    def test_cli_requires_both_directions(self):
        self.passing()
        one_direction = self.directory / "palpo-synapse"
        one_direction.mkdir()
        for path in list(self.directory.iterdir()):
            if path.is_file():
                path.rename(one_direction / path.name)
        script = Path(__file__).with_name("mixed_federation_results.py")
        result = subprocess.run([sys.executable, str(script), str(self.directory)], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("synapse-palpo", result.stderr)

    def test_cli_accepts_two_complete_directions(self):
        self.passing()
        one_direction = self.directory / "palpo-synapse"
        one_direction.mkdir()
        for path in list(self.directory.iterdir()):
            if path.is_file():
                path.rename(one_direction / path.name)
        shutil.copytree(one_direction, self.directory / "synapse-palpo")
        script = Path(__file__).with_name("mixed_federation_results.py")
        result = subprocess.run([sys.executable, str(script), str(self.directory)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
