from datetime import date
import unittest

from dependency_policy import validate


class DependencyPolicyTest(unittest.TestCase):
    def setUp(self):
        self.item = {"id": "RUSTSEC-2026-0001", "owner": "@maintainer",
                     "reason": "Temporary, reviewed unreachable code path", "expires": "2026-12-01"}
        self.policy = {"schema": 1, "exceptions": [self.item]}
        self.config = {"advisories": {"version": 2, "unsound": "all", "ignore": [{"id": self.item["id"], "reason": self.item["reason"]}]}}

    def check(self):
        validate(self.config, self.policy, today=date(2026, 11, 10))

    def test_empty_policy_and_exact_owned_exception(self):
        validate({"advisories": {"version": 2, "unsound": "all", "ignore": []}}, {"schema": 1, "exceptions": []})
        self.check()

    def test_expired_and_expiry_day_are_rejected(self):
        for expiry in ("2026-11-09", "2026-11-10"):
            self.item["expires"] = expiry
            with self.assertRaises(ValueError):
                self.check()

    def test_missing_owner_reason_or_precise_id_is_rejected(self):
        for field, value in (("owner", ""), ("reason", ""), ("id", "RUSTSEC-*")):
            old = self.item[field]
            self.item[field] = value
            with self.assertRaises(ValueError):
                self.check()
            self.item[field] = old

    def test_untracked_broad_and_duplicate_ignores_are_rejected(self):
        for ignores in ([self.item["id"]], [{"crate": "unsafe-dependency"}],
                        [{"id": "RUSTSEC-2026-9999", "reason": "unknown"}],
                        self.config["advisories"]["ignore"] * 2):
            self.config["advisories"]["ignore"] = ignores
            with self.assertRaises((ValueError, KeyError)):
                self.check()

    def test_stale_metadata_is_rejected(self):
        self.config["advisories"]["ignore"] = []
        with self.assertRaises(ValueError):
            self.check()

    def test_malformed_policy_roots_entries_and_dates_are_rejected(self):
        for policy in ([], None, {"schema": 1, "exceptions": "all"},
                       {"schema": 1, "exceptions": [None]},
                       {"schema": 1, "exceptions": [{**self.item, "expires": 123}]}):
            with self.subTest(policy=policy), self.assertRaises(ValueError):
                validate(self.config, policy, today=date(2026, 11, 10))

    def test_advisory_policy_cannot_disable_transitive_unsoundness(self):
        for field, value in (("version", 1), ("version", True), ("unsound", "none"),
                             ("unsound", "workspace"), ("unsound", None)):
            with self.subTest(field=field, value=value):
                config = {"advisories": {**self.config["advisories"], field: value}}
                with self.assertRaises(ValueError):
                    validate(config, self.policy, today=date(2026, 11, 10))


if __name__ == "__main__":
    unittest.main()
