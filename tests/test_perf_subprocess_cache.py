"""Performance regression: tmux session cache and env file caching.

Guards against the AMUX-2644 class of regression: background loops that
independently shell out to tmux, producing O(sessions * loops) subprocess
forks per tick. The fix is a shared cache with a TTL; this test verifies
the caching layer works correctly so a future change can't silently
revert to per-call subprocess forks.

Runs without tmux — the cache layer is tested in isolation via monkeypatching.
"""

import importlib.util
import os
import sys
import tempfile
import time
from pathlib import Path
from unittest.mock import patch

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv():
    os.environ.setdefault("AMUX_HOME", tempfile.mkdtemp())
    spec = importlib.util.spec_from_file_location("amux_server", SERVER_PATH)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_server"] = mod
    spec.loader.exec_module(mod)
    return mod


class TestTmuxSessionsCache:
    def test_cache_returns_set(self, srv):
        with patch("subprocess.run") as mock_run:
            mock_run.return_value = type("R", (), {
                "returncode": 0,
                "stdout": "session-a\nsession-b\nsession-c\n"
            })()
            srv._tmux_sessions_cache = (0.0, set())
            result = srv._tmux_sessions_set()
            assert result == {"session-a", "session-b", "session-c"}
            assert mock_run.call_count == 1

    def test_cache_reuses_within_ttl(self, srv):
        srv._tmux_sessions_cache = (time.time(), {"cached-a", "cached-b"})
        result = srv._tmux_sessions_set()
        assert result == {"cached-a", "cached-b"}

    def test_cache_expires_after_ttl(self, srv):
        srv._tmux_sessions_cache = (time.time() - 10, {"stale"})
        with patch("subprocess.run") as mock_run:
            mock_run.return_value = type("R", (), {
                "returncode": 0,
                "stdout": "fresh-a\nfresh-b\n"
            })()
            result = srv._tmux_sessions_set()
            assert result == {"fresh-a", "fresh-b"}
            assert mock_run.call_count == 1

    def test_concurrent_calls_share_one_subprocess(self, srv):
        import threading
        srv._tmux_sessions_cache = (0.0, set())
        call_count = [0]
        original_run = __import__("subprocess").run

        def counting_run(*a, **kw):
            call_count[0] += 1
            time.sleep(0.05)
            return type("R", (), {
                "returncode": 0,
                "stdout": "s1\ns2\n"
            })()

        with patch("subprocess.run", side_effect=counting_run):
            threads = [threading.Thread(target=srv._tmux_sessions_set) for _ in range(10)]
            for t in threads:
                t.start()
            for t in threads:
                t.join()
        # The lock should ensure at most 2 calls (one winner, possibly one
        # that checked before the lock but lost the race)
        assert call_count[0] <= 2, f"Expected <=2 subprocess calls, got {call_count[0]}"


class TestEnvFileCache:
    def test_caches_by_mtime(self, srv):
        with tempfile.NamedTemporaryFile(mode="w", suffix=".env", delete=False) as f:
            f.write('FOO="bar"\nBAZ=qux\n')
            f.flush()
            path = Path(f.name)
        try:
            r1 = srv.parse_env_file(path)
            assert r1 == {"FOO": "bar", "BAZ": "qux"}

            r2 = srv.parse_env_file(path)
            assert r2 is r1

            time.sleep(0.05)
            path.write_text('CHANGED="yes"\n')
            os.utime(path, (time.time() + 1, time.time() + 1))

            r3 = srv.parse_env_file(path)
            assert r3 == {"CHANGED": "yes"}
            assert r3 is not r1
        finally:
            path.unlink()

    def test_missing_file_returns_empty(self, srv):
        result = srv.parse_env_file(Path("/nonexistent/path.env"))
        assert result == {}
