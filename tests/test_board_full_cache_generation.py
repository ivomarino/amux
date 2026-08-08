"""A write landing during a board_full rebuild must not be undone by it (AF-12).

_board_changed() nulls _sse_cache["board_full"]["data"] WITHOUT taking _sse_cache_lock.
A rebuild already inside the lock therefore finishes afterwards, writes its PRE-write
result back, and stamps it fresh — silently undoing the invalidation. Filtered board GETs
then serve a board missing a committed card for up to TTL*5 (10s).

Reproduced live before fixing: 5 trials with the write timed into an in-flight rebuild,
1 came back stale (desc_len 2227 expected, 2222 served). That reproduction is
probabilistic — a ~20% hit rate means "0 stale in 7 trials" has a ~21% chance of being
luck — so the guard is pinned here deterministically instead.

The fix copies `board`'s existing guard, and the SHAPE matters: rebuild-until-stable,
bounded, NOT discard-if-moved. The sibling's own comment records that discarding on every
bump caused the 2026-08-02 outage — under a write burst the cache never became fresh and
every client full-built its own board (2 cores pinned, 90s reads). A build STARTED after
the latest bump is correct to commit.
"""
import importlib.util
import os
import sys
from pathlib import Path

import pytest

SERVER_PATH = str(Path(__file__).parent.parent / "amux-server.py")


@pytest.fixture
def srv(tmp_path):
    home = tmp_path / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_bfgen", None)
        spec = importlib.util.spec_from_file_location("amux_server_bfgen", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_bfgen"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_a_write_during_the_rebuild_is_not_committed_over(srv):
    """THE RACE. _load_board is made to simulate a write landing mid-flight by bumping
    the generation on its first call — exactly what _board_changed() does. The rebuild
    must notice and re-read rather than committing the stale snapshot."""
    bf = srv._sse_cache["board_full"]
    bf["data"] = None
    bf["time"] = 0
    bf["gen"] = 0
    calls = {"n": 0}

    def fake_load(done_limit=0):
        calls["n"] += 1
        if calls["n"] == 1:
            # a write lands while this first load is in flight
            bf["gen"] = bf.get("gen", 0) + 1
            return ["STALE"]
        return ["FRESH"]

    srv._load_board = fake_load
    out = srv._board_full_cached()
    assert calls["n"] >= 2, (
        "the rebuild never re-read after a write bumped the generation — it is "
        "committing a snapshot taken before the write (AF-12)")
    assert out == ["FRESH"], "returned the stale snapshot: %r" % (out,)
    assert bf["data"] == ["FRESH"], (
        "the CACHE was poisoned with the pre-write snapshot; every filtered GET for the "
        "next TTL*5 serves a board missing that write")


def test_a_quiet_rebuild_loads_exactly_once(srv):
    """Counter-case, and NOT a fix-detector — it passes against a pre-fix specimen too,
    because the old code also loaded once. It guards THIS change: without it, "always
    re-read" would pass the test above while
    doubling the cost of every cold filtered GET — the expensive path this cache exists
    to eliminate."""
    bf = srv._sse_cache["board_full"]
    bf["data"] = None
    bf["time"] = 0
    bf["gen"] = 0
    calls = {"n": 0}

    def fake_load(done_limit=0):
        calls["n"] += 1
        return ["OK"]

    srv._load_board = fake_load
    assert srv._board_full_cached() == ["OK"]
    assert calls["n"] == 1, "no write landed, yet the board was loaded %d times" % calls["n"]


def test_a_sustained_write_burst_still_commits_rather_than_starving(srv):
    """THE OUTAGE GUARD, and also not a fix-detector (pre-fix there was no loop to be
    unbounded). It guards THIS change's retry from starving.

    If every generation bump discarded the build, a board under
    continuous writes would never cache — which is the 2026-08-02 failure the sibling's
    comment records. The loop is bounded, so a burst commits a near-fresh board."""
    bf = srv._sse_cache["board_full"]
    bf["data"] = None
    bf["time"] = 0
    bf["gen"] = 0
    calls = {"n": 0}

    def fake_load(done_limit=0):
        calls["n"] += 1
        bf["gen"] = bf.get("gen", 0) + 1     # a write lands during EVERY rebuild
        return ["ATTEMPT-%d" % calls["n"]]

    srv._load_board = fake_load
    out = srv._board_full_cached()
    assert calls["n"] <= 4, "unbounded retry under a write burst: %d loads" % calls["n"]
    assert bf["data"] is not None and out is not None, (
        "gave up without caching anything — under sustained writes every client then "
        "full-builds its own board, which is the outage this bound exists to prevent")


def test_board_changed_bumps_the_generation(srv):
    """The guard is only live if the invalidator actually moves the counter. Nulling
    `data` alone is what left the race open."""
    bf = srv._sse_cache["board_full"]
    bf["gen"] = 7
    bf["data"] = ["something"]
    srv._board_changed()
    assert bf["data"] is None, "_board_changed no longer nulls board_full"
    assert bf.get("gen", 0) > 7, (
        "_board_changed did not bump board_full's generation, so an in-flight rebuild "
        "cannot detect the write (AF-12)")
