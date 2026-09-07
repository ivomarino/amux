#!/usr/bin/env python3
"""Exercise the installed hook entry point with real disposable Git state."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


def fixture_environment():
    # A cwd does not override GIT_DIR/GIT_INDEX_FILE inherited from a hook.
    # These fixtures own their Git configuration and never use a caller's
    # repository pointers, transport settings, or injected config.
    return {k: v for k, v in os.environ.items()
            if not k.startswith(("GIT_", "AMUX_"))}


class ObservedOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.repo = Path(self.scratch.name) / "repo"
        self.repo.mkdir()
        (Path(self.scratch.name) / "home").mkdir()
        self.paths = [
            "research/ops-success-plan-20260907.md",
            "research/ops-success-matrix-20260907.json",
            "research/ops-success-results-20260907.md",
        ]
        self.git("init", "-q")
        self.git("config", "core.hooksPath", "/dev/null")
        self.git("config", "user.name", "Guard fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        for rel in self.paths:
            file = self.repo / rel
            file.parent.mkdir(exist_ok=True)
            file.write_text("baseline\n")
        self.git("add", "--", *self.paths)
        self.git("commit", "-qm", "fixture baseline")
        for rel in self.paths:
            (self.repo / rel).write_text("baseline\nauthored change\n")
        self.git("add", "--", *self.paths)
        self.observations = [{
            "path": rel, "mine_age_secs": None,
            "observers": [{"session": "reader-lane", "age_secs": 500}],
            "provenance": "observed", "establishes_ownership": False,
            "why": "A moving mtime does not identify the writer.",
        } for rel in self.paths]

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True,
                              env=fixture_environment(), capture_output=True, text=True)

    def invoke(self, response, extra_env=None):
        hook = Path(os.environ.get("STAGED_GUARD_HOOK") or
                    Path(__file__).with_name("amux-staged-guard"))
        env = fixture_environment()
        env.update(AMUX_SESSION="author-lane", AMUX_URL="https://guard.invalid",
                   AMUX_HOME=str(Path(self.scratch.name) / "home"))
        env.update(extra_env or {})
        called = []

        def transport(req, **kwargs):
            called.append((req.full_url, json.loads(req.data)))
            self.assertTrue(req.full_url.endswith("/api/git/staged-guard"))
            return io.BytesIO(json.dumps(response).encode())

        before_cwd = os.getcwd()
        with patch.dict(os.environ, env, clear=True):
            loader = importlib.machinery.SourceFileLoader("_ownership_guard", str(hook))
            spec = importlib.util.spec_from_loader(loader.name, loader)
            mod = importlib.util.module_from_spec(spec)
            loader.exec_module(mod)
            stderr = io.StringIO()
            try:
                os.chdir(self.repo)
                with patch.object(mod.urllib.request, "urlopen", transport), contextlib.redirect_stderr(stderr):
                    result = mod.main()
            finally:
                os.chdir(before_cwd)
        self.assertEqual(len(called), 1)
        self.assertEqual(set(called[0][1]["paths"]), set(self.paths))
        return result, stderr.getvalue()

    def test_observation_only_files_report_uncertainty_without_a_false_owner(self):
        result, output = self.invoke({"ok": True, "observations": self.observations})
        self.assertEqual(result, 0)
        self.assertIn("ownership unverified for 3 staged path(s)", output)
        for rel in self.paths:
            self.assertIn(rel, output)
        self.assertNotIn("edited by session 'reader-lane'", output)
        self.assertNotIn("COMMIT BLOCKED", output)

    def test_real_writer_still_blocks_beside_an_observation(self):
        result, output = self.invoke({"ok": True, "observations": self.observations,
            "foreign": [{"path": self.paths[0], "owner": "writer-lane",
                         "provenance": "firsthand", "age_secs": 1000,
                         "why": "A recorded edit belongs to writer-lane."}]})
        self.assertEqual(result, 1)
        self.assertIn("writer-lane", output)
        self.assertIn("COMMIT BLOCKED", output)
        self.assertNotIn("edited by session 'reader-lane'", output)

    def test_blind_cotenant_block_does_not_invent_an_owner(self):
        result, output = self.invoke({"ok": True, "observations": self.observations,
            "foreign": [{"path": self.paths[0], "owner": "", "age_secs": 0,
                         "why": "A cotenant is invisible; ownership is unverified."}]})
        self.assertEqual(result, 1)
        self.assertIn("COMMIT BLOCKED", output)
        self.assertIn("ownership is unverified", output)
        self.assertNotIn("edited by session ''", output)
        self.assertNotIn("staged files were edited by OTHER", output)

    def test_old_response_without_observations_keeps_the_normal_pass(self):
        result, output = self.invoke({"ok": True})
        self.assertEqual(result, 0)
        self.assertNotIn("ownership unverified", output)
        self.assertNotIn("MTIME OBSERVATIONS", output)

    def test_own_observation_on_dirty_path_does_not_invent_a_writer(self):
        path = self.paths[0]
        (self.repo / path).write_text("baseline\nauthored change\nunrecorded change\n")
        result, output = self.invoke({
            "ok": True,
            "observations": [{
                "path": path, "mine_age_secs": 500, "observers": [],
                "provenance": "observed", "establishes_ownership": False,
            }],
            "shared": [{
                "path": path, "owner": "(unknown)", "peer": False,
                "age_secs": 0, "has_unstaged_changes": True,
                "mine_provenance": "observed", "their_provenance": "none",
            }],
        }, extra_env={"AMUX_PARTIAL_STAGE": "1"})
        self.assertEqual(result, 0)
        self.assertIn("partial stage DECLARED for 1 path", output)
        ledger = Path(self.scratch.name) / "home" / "staged-guard-divergence.jsonl"
        self.assertEqual(json.loads(ledger.read_text())["resolution"], "declared-partial")
        self.assertIn("ownership unverified for 1 staged path", output)
        self.assertNotIn("is yours", output)
        self.assertNotIn("no other session edited it", output)
        self.assertNotIn("THEIR claim is a recorded write", output)

    def test_caller_git_variables_cannot_redirect_the_disposable_fixture(self):
        sentinel = Path(self.scratch.name) / "caller-repo"
        sentinel.mkdir()
        # The known-positive caller repository is independently bootstrapped;
        # never point the hostile variables at the developer's real checkout.
        clean = {k: v for k, v in os.environ.items()
                 if not k.startswith(("GIT_", "AMUX_"))}

        def caller_git(*args, env=clean):
            return subprocess.run(
                ["git", "-c", "core.hooksPath=/dev/null", *args],
                cwd=sentinel, env=env, check=True, capture_output=True, text=True)

        caller_git("init", "-q")
        (sentinel / "caller-only.txt").write_text("caller staged content\n")
        caller_git("add", "--", "caller-only.txt")
        index = sentinel / ".git" / "index"
        initial_index = index.read_bytes()
        hostile = {
            "GIT_DIR": str(sentinel / ".git"),
            "GIT_WORK_TREE": str(sentinel),
            "GIT_INDEX_FILE": str(index),
        }
        control = subprocess.run(
            ["git", "diff", "--cached", "--name-only"], cwd=self.repo,
            env={**clean, **hostile}, check=True, capture_output=True, text=True)
        self.assertEqual(control.stdout.splitlines(), ["caller-only.txt"])
        with patch.dict(os.environ, hostile):
            actual_repo = self.git("rev-parse", "--show-toplevel").stdout.strip()
            self.assertEqual(Path(actual_repo).resolve(), self.repo.resolve())
            self.git("add", "--", self.paths[0])
            result, output = self.invoke({"ok": True})
        self.assertEqual(result, 0, output)
        self.assertEqual(index.read_bytes(), initial_index)
        self.assertEqual(caller_git("diff", "--cached", "--name-only").stdout.splitlines(),
                         ["caller-only.txt"])


if __name__ == "__main__":
    unittest.main()
