#!/usr/bin/env python3
"""ATE-92: real process/lock lifetimes, preserved artifacts, and idle retry."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('cargo-target-guard.py').resolve()
spec = importlib.util.spec_from_file_location('guard', SCRIPT)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def quiet(_):
    return [], 1


class CargoReclaimTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.root = self.base / 'rust-build-target'
        self.artifacts = self.root / 'debug' / 'deps'
        self.artifacts.mkdir(parents=True)
        self.marker = self.artifacts / 'artifact'
        self.marker.write_text('must survive active builds')

    def mutate(self, action='clear', path=None, destination=None, probe=quiet, protected=()):
        return guard.mutate(action, path or self.root, destination, [self.root], protected,
                            process_probe=probe)

    def test_active_build_lease_blocks_clear_and_rotation_then_idle_retry_reclaims(self):
        # The actual sanctioned wrapper with a deterministic Cargo executable.
        # It execs into the tool so the test pins lease inheritance, not a parent
        # shell that happens to remain alive during a subprocess.
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        # Exercise the Linux scope command without requiring a CI user manager.
        scope = bin_dir / 'systemd-run'
        scope.write_text('#!/bin/sh\nwhile [ "$1" != -- ]; do shift; done\nshift\nexec "$@"\n')
        scope.chmod(0o755)
        cargo = bin_dir / 'cargo'
        cargo.write_text('#!/bin/sh\nprintf "ready\\n"\nread -r release\n')
        cargo.chmod(0o755)
        env = {**os.environ, 'PATH': str(bin_dir) + ':' + os.environ['PATH'],
               'CARGO_TARGET_DIR': str(self.root)}
        proc = subprocess.Popen(['bash', str(SCRIPT.with_name('safe-cargo.sh')), 'check'],
                                env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, text=True)
        try:
            self.assertEqual(proc.stdout.readline().strip(), 'ready')
            for action, path, dest in [('clear', self.root, None),
                                       ('move', self.artifacts, self.base / 'staged')]:
                with self.assertRaisesRegex(guard.Deferred, 'lock busy'):
                    self.mutate(action, path, dest)
                self.assertTrue(self.marker.exists())
        finally:
            proc.communicate('finished\n', timeout=10)
        self.assertEqual(proc.returncode, 0)
        self.mutate()
        self.assertFalse(self.marker.exists())
        self.assertTrue((self.root / 'debug' / '.cargo-lock').exists())
        self.assertTrue(guard.lease_path(self.root).exists())

    def test_native_cargo_lock_blocks_unwrapped_build(self):
        lock = self.root / 'debug' / '.cargo-lock'
        fd = guard.open_lock(lock)
        try:
            with self.assertRaisesRegex(guard.Deferred, 'lock busy'):
                self.mutate()
            self.assertTrue(self.marker.exists())
        finally:
            os.close(fd)
        before = lock.stat().st_ino
        self.mutate()
        self.assertEqual(lock.stat().st_ino, before)
        self.assertFalse(self.marker.exists())

    def test_direct_test_binary_without_cargo_parent_prevents_deletion(self):
        binary = self.artifacts / 'orphan-test-0123456789abcdef'
        source = self.base / 'test.c'
        source.write_text('#include <unistd.h>\nint main(void) { sleep(60); return 0; }\n')
        subprocess.run(['cc', str(source), '-o', str(binary)], check=True, capture_output=True)
        proc = subprocess.Popen([str(binary)])
        try:
            active, count = guard.active_processes([self.root])
            self.assertIn(str(proc.pid), active)
            self.assertGreater(count, 0)
            with self.assertRaises(guard.Deferred):
                self.mutate(probe=guard.active_processes)
            self.assertTrue(binary.exists())
        finally:
            proc.terminate()
            proc.wait(timeout=10)
        self.mutate()
        self.assertFalse(binary.exists())

    def test_unmeasured_probe_fails_closed_and_idle_move_restore_purge_work(self):
        def failed(_):
            raise guard.Deferred('process probe unmeasured')
        with self.assertRaisesRegex(guard.Deferred, 'unmeasured'):
            self.mutate(probe=failed)
        staged = self.base / 'quarantine' / 'deps'
        self.mutate('move', self.artifacts, staged)
        self.assertTrue((staged / 'artifact').exists())
        self.mutate('move', staged, self.artifacts)
        self.assertTrue(self.marker.exists())
        self.mutate('move', self.artifacts, staged)
        with self.assertRaisesRegex(guard.Deferred, 'unmeasured'):
            self.mutate('purge', staged, protected=[self.artifacts], probe=failed)
        self.assertTrue(staged.exists())
        self.mutate('purge', staged, protected=[self.artifacts])
        self.assertFalse(staged.exists())

    def test_cargo_lock_directories_never_rotate(self):
        for path in (self.root, self.root / 'debug'):
            with self.assertRaisesRegex(guard.Deferred, 'cannot rotate'):
                self.mutate('move', path, self.base / 'staged')
        self.assertTrue(self.marker.exists())

    def test_fixture_probe_seam_cannot_authorize_production_mutation(self):
        env = {**os.environ, 'AMUX_CARGO_GUARD_TEST_FIXTURE': str(self.base),
               'AMUX_RS_DISK_CLEAR_ONLY': '1'}
        result = subprocess.run([sys.executable, str(SCRIPT), 'clear', '--target',
                                 str(self.root), '--path', str(self.root)],
                                env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 75)
        self.assertIn('invalid temporary fixture', result.stdout)
        self.assertTrue(self.marker.exists())

    def test_valid_fixture_cannot_move_or_restore_across_its_boundary(self):
        with tempfile.TemporaryDirectory(prefix='amux-cargo-guard-test.') as fixture_dir:
            fixture = Path(fixture_dir).resolve()
            target = fixture / 'target'
            artifacts = target / 'debug' / 'deps'
            artifacts.mkdir(parents=True)
            marker = artifacts / 'artifact'
            marker.write_text('fixture must stay inside its boundary')
            env = {**os.environ, 'AMUX_CARGO_GUARD_TEST_FIXTURE': str(fixture),
                   'AMUX_RS_DISK_CLEAR_ONLY': '1'}
            outside = self.base / 'outside'
            for action, extra in [
                    ('move', ['--destination', str(outside)]),
                    ('purge', ['--protected-path', str(outside)])]:
                with self.subTest(action=action):
                    result = subprocess.run([sys.executable, str(SCRIPT), action,
                                             '--target', str(target), '--path', str(artifacts),
                                             *extra], env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 75, result.stdout)
                    self.assertIn('invalid temporary fixture', result.stdout)
                    self.assertTrue(marker.exists())
                    self.assertFalse(outside.exists())

    def test_command_line_mentions_do_not_become_builds(self):
        proc = subprocess.Popen(['sh', '-c', 'sleep 30 # cargo rustc build'])
        try:
            active, _ = guard.active_processes([self.root])
            self.assertNotIn(str(proc.pid), active)
        finally:
            proc.terminate()
            proc.wait(timeout=10)


if __name__ == '__main__':
    unittest.main()
