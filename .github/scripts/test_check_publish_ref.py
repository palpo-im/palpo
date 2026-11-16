import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check-publish-ref.py")
ROOT = SCRIPT.resolve().parents[2]
FIRST = "a" * 40
SECOND = "b" * 40


class PublicationRefTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.reply = self.root / "reply.json"
        self.calls = self.root / "calls.jsonl"
        self.output = self.root / "output"
        fake = self.root / "gh"
        fake.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            "with pathlib.Path(os.environ['PALPO_TEST_CALLS']).open('a') as f:\n"
            "    f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "sys.stdout.write(pathlib.Path(os.environ['PALPO_TEST_REPLY']).read_text())\n"
            "sys.stderr.write(os.environ['GH_TOKEN'])\n"
            "sys.exit(int(os.environ.get('PALPO_TEST_RETURN_CODE', '0')))\n"
        )
        fake.chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=str(self.root) + os.pathsep + os.environ.get("PATH", ""),
            GH_TOKEN="test-secret-never-print",
            GITHUB_REF="refs/heads/main",
            GITHUB_SHA=FIRST,
            GITHUB_REPOSITORY="palpo-im/palpo",
            GITHUB_OUTPUT=str(self.output),
            PALPO_TEST_CALLS=str(self.calls),
            PALPO_TEST_REPLY=str(self.reply),
        )
        self.respond(FIRST)

    def respond(self, revision, ref="refs/heads/main"):
        self.reply.write_text(json.dumps({"ref": ref, "object": {"type": "commit", "sha": revision}}))

    def check(self, *, success=True, publish=True):
        self.output.write_text("")
        result = subprocess.run(
            [sys.executable, str(SCRIPT)], env=self.env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=5,
        )
        self.assertEqual(result.returncode, 0 if success else 1, result.stderr)
        outputs = dict(line.split("=", 1) for line in self.output.read_text().splitlines())
        self.assertEqual(outputs["publish"], str(publish).lower())
        self.assertNotIn(self.env["GH_TOKEN"], result.stdout + result.stderr)
        return result

    def test_current_main_and_next_use_read_only_ref_lookup(self):
        for branch in ("main", "next"):
            with self.subTest(branch=branch):
                self.env["GITHUB_REF"] = f"refs/heads/{branch}"
                self.respond(FIRST, self.env["GITHUB_REF"])
                self.check()
                call = json.loads(self.calls.read_text().splitlines()[-1])
                self.assertEqual(call, ["api", "--method", "GET", f"repos/palpo-im/palpo/git/ref/heads/{branch}"])

    def test_stale_main_and_next_skip_publication(self):
        for branch in ("main", "next"):
            with self.subTest(branch=branch):
                self.env["GITHUB_REF"] = f"refs/heads/{branch}"
                self.respond(SECOND, self.env["GITHUB_REF"])
                result = self.check(publish=False)
                self.assertIn("Skipping stale publication", result.stdout)

    def test_branch_advance_rejects_a_late_old_run(self):
        self.check()
        self.respond(SECOND)
        self.env["GITHUB_SHA"] = SECOND
        self.check()
        self.env["GITHUB_SHA"] = FIRST
        self.check(publish=False)
        self.assertEqual(len(self.calls.read_text().splitlines()), 3)

    def test_tag_and_nonmoving_branch_preserve_existing_publication_without_api(self):
        self.env["PALPO_TEST_RETURN_CODE"] = "1"
        self.reply.write_text("not a response")
        for ref in ("refs/tags/v1.2.3", "refs/heads/release/example"):
            with self.subTest(ref=ref):
                self.env["GITHUB_REF"] = ref
                self.check()
        self.assertFalse(self.calls.exists())

    def test_api_failure_does_not_open_gate_or_echo_credentials(self):
        self.env["PALPO_TEST_RETURN_CODE"] = "1"
        self.check(success=False, publish=False)

    def test_malformed_response_never_opens_gate(self):
        for payload in ("not json", "null", "[]", "{}", json.dumps({"ref": "refs/heads/main"})):
            with self.subTest(payload=payload):
                self.reply.write_text(payload)
                self.check(success=False, publish=False)

    def test_wrong_ref_object_type_and_sha_never_open_gate(self):
        for ref, kind, revision in (
            ("refs/heads/next", "commit", FIRST),
            ("refs/heads/main", "tag", FIRST),
            ("refs/heads/main", "commit", None),
            ("refs/heads/main", "commit", "truncated"),
        ):
            with self.subTest(ref=ref, kind=kind, revision=revision):
                self.reply.write_text(json.dumps({"ref": ref, "object": {"type": kind, "sha": revision}}))
                self.check(success=False, publish=False)

    def test_missing_or_malformed_context_never_opens_gate(self):
        for key, value in (("GITHUB_REF", ""), ("GITHUB_SHA", "short"), ("GITHUB_REPOSITORY", "../other?bad")):
            with self.subTest(key=key):
                previous = self.env[key]
                self.env[key] = value
                self.check(success=False, publish=False)
                self.env[key] = previous
        del self.env["GITHUB_SHA"]
        self.check(success=False, publish=False)
        self.assertFalse(self.calls.exists())

    def test_workflow_holds_lock_and_guards_entire_publication_chain(self):
        text = (ROOT / ".github/workflows/release-docker-images.yml").read_text()
        merge = text.split("\n  merge:\n", 1)[1]
        self.assertIn("group: image-publication-${{ github.ref }}-${{ matrix.repository }}", merge)
        self.assertIn("cancel-in-progress: false", merge)
        self.assertIn("queue: max", merge)
        self.assertLess(merge.index("concurrency:"), merge.index("steps:"))
        steps = merge.split("      - ")[1:]
        guard = next(i for i, step in enumerate(steps) if "id: publish_ref" in step)
        self.assertIn("GH_TOKEN: ${{ github.token }}", steps[guard])
        self.assertIn("python3 .github/scripts/check-publish-ref.py", steps[guard])
        for step in steps[guard + 1:]:
            with self.subTest(step=step.splitlines()[0]):
                self.assertIn("steps.publish_ref.outputs.publish == 'true'", step)
        self.assertIn("type=raw,value=latest,enable=${{ github.ref == 'refs/heads/main' }}", merge)
        self.assertIn("type=raw,value=next,enable=${{ github.ref == 'refs/heads/next' }}", merge)
        self.assertIn("type=pep440,pattern={{raw}}", merge)
        self.assertIn("type=sha", merge)


if __name__ == "__main__":
    unittest.main()
