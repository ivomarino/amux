"""One lane's browser call must not block every other lane's (AC-289).

Filed by amux-cloud, confirmed by reading rather than proposed: `_bu_driver_lock` was
a single module-level Lock, and `_bu_driver_call` held it across, in order,
subprocess.Popen, a blocking stdout.readline(), stdin.write+flush, a
select.select(..., timeout_s) whose timeout DEFAULTS TO 45, and another readline().
So one lane's slow or hung browser call serialised every lane's browser operation for
up to 45 seconds.

The function's docstring said "Serialized per session" the entire time. That sentence
is why it survived review: it described the intended design so exactly that reading it
answered the question, and the lock two lines below it was module-level.

The serialisation itself is REAL and must survive any fix — a driver is one subprocess
behind one stdin/stdout pipe pair, so two concurrent calls to the SAME driver would
interleave and corrupt the line protocol. Per-session is the invariant; global was the
accident. Both halves are asserted here, because a "fix" that removed the locking
entirely would pass a test that only checked for parallelism.

Scope, so nobody over-claims it from this file: this lock is taken on browser paths
only. It cannot make /api/board return HTTP 000 the way the slog lock did (AC-174),
and it is emphatically not the host-level wedges (AC-268/AC-288) — no Python lock
stops sshd answering.
"""

import re
import threading
import time
from pathlib import Path

import pytest

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _fn(name):
    """A top-level function's source, by indentation."""
    m = re.search(r"^def %s\(" % re.escape(name), SRC, re.M)
    assert m, "%s is gone" % name
    start = m.start()
    nxt = re.search(r"^(?:def |class )", SRC[start + 1:], re.M)
    return SRC[start:start + 1 + (nxt.start() if nxt else len(SRC))]


# ───────────────────────── the scope of the lock ─────────────────────────────

def test_driver_call_does_not_hold_the_GLOBAL_lock():
    """THE REGRESSION. `_bu_driver_call` must not wrap its body in the module-level
    lock — that is what made one lane's 45s timeout everyone's 45s timeout."""
    body = _fn("_bu_driver_call")
    assert "_bu_lock_for(" in body, (
        "_bu_driver_call no longer takes a per-session lock — either the fix was "
        "reverted or the locking was removed entirely, and the second is worse")
    # The global lock may still appear for SHORT registry sections; what must not
    # exist is a `with _bu_driver_lock:` wrapping the blocking work.
    blocking = ("subprocess.Popen", "readline()", "_select.select")
    for stmt in re.finditer(r"^ {4}with _bu_driver_lock:\n((?: {8}.*\n|\n)*)", body, re.M):
        chunk = stmt.group(1)
        for b in blocking:
            assert b not in chunk, (
                "the global registry lock is held across %s — that re-creates AC-289: "
                "one lane's slow browser call blocks every lane's" % b)


def test_the_blocking_select_is_still_the_45s_one():
    """DENOMINATOR. The tests above matter because the critical section is long. If
    the timeout were dropped to something trivial the contention would stop mattering
    and these tests would be guarding nothing — so assert the cost is still real."""
    sig = re.search(r"def _bu_driver_call\([^)]*timeout_s: int = (\d+)", SRC, re.S)
    assert sig, "the timeout parameter moved"
    assert int(sig.group(1)) >= 30, (
        "timeout_s default dropped to %s — if browser calls are now short, re-evaluate "
        "whether per-session locking is still worth its complexity" % sig.group(1))


def test_per_session_serialisation_is_PRESERVED():
    """The invariant a naive fix would destroy. One driver is one subprocess behind one
    pipe pair; two concurrent calls to the same session would interleave writes and
    reads and corrupt the protocol. Removing the lock 'fixes' contention and breaks
    correctness, so the per-session lock must still exist and still be acquired."""
    assert "_bu_session_locks" in SRC, "the per-session lock registry is gone"
    lf = _fn("_bu_lock_for")
    # RLock, and it must STAY an RLock (AC-290): _bu_driver_call holds this lock
    # and calls _bu_driver_stop, which must also take it so /api/browser/stop
    # cannot tear a driver down mid-call. With a plain Lock those two requirements
    # self-deadlock to the acquire timeout and return "busy with a previous call" —
    # this subsystem's own symptom, produced by the fix for it.
    assert "threading.RLock()" in lf, (
        "_bu_lock_for no longer creates an RLock — if it was 'simplified' to Lock, "
        "_bu_driver_stop's inner acquire self-deadlocks on the two paths where "
        "_bu_driver_call calls it (profile change, IO error)")
    body = _fn("_bu_driver_call")
    assert re.search(r"_lk\s*=\s*_bu_lock_for\(session\)", body), (
        "the per-session lock is looked up but not bound/acquired")
    assert ".acquire(" in body, "the per-session lock is never acquired"


def test_the_session_lock_is_released_on_every_path():
    """Six early `return`s live inside that critical section. `with` gave release for
    free; an explicit acquire (needed for the timeout) means an explicit release, and a
    leaked driver lock wedges that session's browser FOREVER — the failure this card is
    about, made permanent and per-session."""
    body = _fn("_bu_driver_call")
    assert re.search(r"\n    finally:\n(?: +#.*\n|\n)* +_lk\.release\(\)", body), (
        "the per-session lock is not released in a finally: — an early return or an "
        "exception leaks it and wedges that session's browser permanently")


def test_driver_stop_takes_the_lock_ITSELF_and_not_across_proc_wait():
    """I reintroduced AC-289 at 1/5 scale while fixing it: `_bu_driver_stop` blocks on
    proc.wait(timeout=8), and my first version called it from inside the registry lock
    because that is what the old code did. Splitting it — pop under the lock, teardown
    outside — makes the mistake unavailable rather than merely avoided."""
    stop = _fn("_bu_driver_stop")
    assert "with _bu_driver_lock:" in stop, (
        "_bu_driver_stop no longer takes the registry lock itself, so its callers must "
        "again remember to — which is how it got held across proc.wait()")
    lock_chunk = re.search(r"with _bu_driver_lock:\n((?: {8}.*\n|\n)*)", stop)
    assert lock_chunk, "could not read the locked section"
    assert "wait(" not in lock_chunk.group(1), (
        "proc.wait() is inside the registry lock — an 8s global stall, AC-289 again")
    # callers must not double-wrap it
    call = _fn("_bu_driver_call")
    assert not re.search(r"with _bu_driver_lock:\n\s+_bu_driver_stop\(", call), (
        "a caller wraps _bu_driver_stop in the registry lock again; it takes it itself, "
        "so this deadlocks or re-creates the stall")


# ───────────────────────── behaviour, not just source ────────────────────────

def test_two_sessions_do_not_serialise_on_each_other():
    """Exercises the LOCK DISCIPLINE with the real helper: two different sessions must
    hold their locks concurrently, one session must not.

    Uses the shipped _bu_lock_for rather than a reimplementation, so a change that
    makes it return one shared lock fails here even if the source checks pass.
    """
    import importlib.util
    import os
    import sys
    import tempfile
    home = Path(tempfile.mkdtemp()) / "h"
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location(
            "amux_bulock", Path(__file__).parent.parent / "amux-server.py")
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_bulock"] = mod
        spec.loader.exec_module(mod)
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev

    a, b = mod._bu_lock_for("lane-a"), mod._bu_lock_for("lane-b")
    assert a is not b, "two sessions got the SAME lock — this is AC-289 unchanged"
    assert a is mod._bu_lock_for("lane-a"), "a session gets a NEW lock each call, so it "\
                                            "serialises nothing at all"

    # lane-a holds; lane-b must still acquire immediately.
    a.acquire()
    try:
        t0 = time.monotonic()
        assert b.acquire(timeout=2), "lane-b could not acquire while lane-a held — "\
                                     "the locks are not independent"
        b.release()
        assert time.monotonic() - t0 < 1.0, "lane-b waited on lane-a's lock"

        # THE SAME SESSION MUST BLOCK A DIFFERENT THREAD. Checked from a second
        # thread on purpose: two concurrent HTTP requests are two threads, which is
        # the situation this lock exists for. The first version of this assertion
        # re-acquired from the SAME thread, which a plain Lock refused and an RLock
        # correctly allows — so it was testing non-reentrancy, an accident of the
        # old implementation, rather than mutual exclusion, the actual requirement.
        # It failed the moment RLock landed and it was the test that was wrong.
        got = []
        t = threading.Thread(target=lambda: got.append(a.acquire(timeout=0.3)))
        t.start(); t.join()
        assert got == [False], (
            "another THREAD acquired this session's lock while it was held — two "
            "calls would interleave on one stdin/stdout pipe and corrupt the protocol")

        # ...and the owning thread MAY re-enter, which is what lets _bu_driver_stop
        # take the lock whether it is called from inside a call or from the endpoint.
        assert a.acquire(timeout=0.2), (
            "the session lock is not reentrant, so _bu_driver_stop deadlocks when "
            "_bu_driver_call invokes it (AC-290)")
        a.release()
    finally:
        a.release()


# ───────────── teardown must not interleave with an in-flight call (AC-290) ──

def test_driver_stop_takes_the_SESSION_lock():
    """POST /api/browser/stop tore a driver down without the per-session lock, so it
    could write `close` into a pipe an in-flight call was mid-select() on — the exact
    interleaving the lock exists to prevent, from the one caller outside the invariant.

    Pre-existing rather than introduced by AC-289: the lock-free version predates it
    and the window is unchanged in width. Making the invariant explicit is what let it
    be noticed."""
    stop = _fn("_bu_driver_stop")
    assert "_bu_lock_for(session)" in stop, (
        "_bu_driver_stop no longer takes the session lock — /api/browser/stop can "
        "again tear down a driver underneath an in-flight call (AC-290)")


def test_the_OBVIOUS_fix_would_self_deadlock():
    """Pins WHY the session lock is an RLock, because the reasoning is invisible from
    the call sites and 'simplify RLock to Lock' is a tempting cleanup.

    _bu_driver_call holds the session lock and calls _bu_driver_stop on two paths (a
    profile change and an IO error). _bu_driver_stop must also take it, for the reason
    above. With a plain Lock those requirements are contradictory: the inner acquire
    blocks forever against its own thread. Demonstrated rather than asserted.
    """
    plain = threading.Lock()
    plain.acquire()
    assert not plain.acquire(timeout=0.2), (
        "threading.Lock became reentrant — if so this whole note is obsolete, delete "
        "it rather than the RLock")
    plain.release()
    r = threading.RLock()
    r.acquire()
    assert r.acquire(timeout=0.2), "RLock is not reentrant — the fix does not work"
    r.release(); r.release()


def test_teardown_is_separable_from_the_registry_pop():
    """The split that keeps AC-289 fixed while adding AC-290: the session lock is held
    across proc.wait(timeout=8) deliberately (that is the point — it blocks THIS
    session), but the GLOBAL registry lock must not be, or an 8s fleet-wide stall
    replaces the 45s one."""
    assert re.search(r"^def _bu_driver_teardown\(", SRC, re.M), (
        "the teardown is no longer a separate function, so the registry lock and the "
        "blocking proc.wait() are at risk of being fused again")
    td = _fn("_bu_driver_teardown")
    assert "_bu_driver_lock" not in td, "the global registry lock is held across proc.wait()"
    assert "_bu_lock_for" not in td, (
        "teardown acquires the session lock itself as well as its caller — harmless "
        "with RLock but it means the ownership contract has drifted")
    stop = _fn("_bu_driver_stop")
    lock_chunk = re.search(r"with _bu_driver_lock:\n((?: {12}.*\n|\n)*)", stop)
    assert lock_chunk and "wait(" not in lock_chunk.group(1), (
        "proc.wait() is back inside the global registry lock — AC-289 at 8s scale")


def test_stop_endpoint_has_no_TOCTOU_on_the_registry():
    """`if session in _bu_drivers:` then `_bu_drivers[session]` — any death path
    between the two (a concurrent call's IO error, a `closed` response, another stop)
    pops the entry and the subscript raises KeyError, surfacing as a 500 on a request
    that was about to succeed."""
    i = SRC.find('path == "/api/browser/stop"')
    assert i > 0, "the stop endpoint moved"
    body = SRC[i:i + 1600]
    assert not re.search(r"if session in _bu_drivers:\s*\n\s*_prof = _bu_drivers\[session\]", body), (
        "the stop endpoint reads _bu_drivers twice (membership then subscript); a "
        "concurrent teardown between them is a KeyError -> 500 (AC-290)")
    assert "_bu_drivers.get(session)" in body, (
        "the stop endpoint no longer reads the registry once into a local")
