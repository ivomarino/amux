#!/usr/bin/env python3
"""Test for MR-43: amux-staged-guard derives a session identity from the tmux
pane name when $AMUX_SESSION is empty, instead of silently no-opping and
leaving the lane's edits absent from the cross-session record.

Run: python3 ~/.amux/hooks/test_amux_staged_guard.py   (exit 0 = all pass)
"""
import importlib.machinery
import os
import subprocess
import sys

HOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "amux-staged-guard")
mod = importlib.machinery.SourceFileLoader("_asg_test", HOOK).load_module()


def _fake_run(stdout):
    def run(args, **kw):
        class R:
            pass
        r = R()
        r.stdout = stdout
        return r
    return run


def main():
    failures = []
    real_run = subprocess.run

    # CONTROL FIRST: an amux-prefixed pane name DOES resolve, so a matcher that
    # silently always returns "" cannot hide behind an all-negative suite.
    subprocess.run = _fake_run("amux-mixpeek-research\n")
    got = mod._derive_session_from_tmux()
    if got != "mixpeek-research":
        failures.append(
            f"control: 'amux-mixpeek-research' should derive 'mixpeek-research', got {got!r}")

    # A human's own tmux session (no amux- prefix) must never be claimed as a
    # lane — the whole point of scoping the fallback to the prefix.
    subprocess.run = _fake_run("main\n")
    got = mod._derive_session_from_tmux()
    if got != "":
        failures.append(f"a bare tmux session name must not resolve to a session: got {got!r}")

    # Outside tmux entirely (or tmux missing from PATH): fail closed to "",
    # never raise — this runs inside a git hook, which must not crash a commit.
    def _raise(*a, **kw):
        raise FileNotFoundError("no tmux")
    subprocess.run = _raise
    try:
        got = mod._derive_session_from_tmux()
        raised = None
    except Exception as e:
        got, raised = None, e
    if raised is not None:
        failures.append(f"must not raise when tmux is unavailable: {raised!r}")
    elif got != "":
        failures.append(f"tmux unavailable should derive '', got {got!r}")

    subprocess.run = real_run

    # GUARD_VERSION must have moved off the pre-fix baseline, or every already-
    # installed copy on this machine reads as current and never re-syncs (the
    # file's own header: "the installed-copy inventory greps" on this number).
    if mod.GUARD_VERSION <= 8:
        failures.append(f"GUARD_VERSION is {mod.GUARD_VERSION} — bump it, every install checks this")

    # AF-190: the server emits `split_risk`; this asserts a HOOK actually prints
    # it. A field nobody renders reaches nobody, and nothing about reading the
    # server code would say so — that gap is ethos rule 1, and it is why the
    # renderer was pulled out of main() into a function a test can call.
    out = []
    mod._render_split_risk({
        "split_risk": [{
            "owner": "amux",
            "staged": ["crates/amux-server/src/api/board.rs"],
            "left_dirty": ["/repo/crates/amux-server/src/db/board_store.rs"],
            "why": "a symbol added on one side may be missing from the other",
        }]
    }, out.append)
    txt = "".join(out)
    for needle, what in [
        ("amux", "the peer whose work is being split"),
        ("board.rs", "the staged file"),
        ("board_store.rs", "the file left behind — naming it is the whole point"),
        ("NOT committed", "that the second half is not in this commit"),
    ]:
        if needle not in txt:
            failures.append(f"split_risk render omits {what} ({needle!r}): {txt!r}")

    # CONTROL: silent when there is nothing to say. A warning that prints on
    # every commit is one nobody reads, which is exactly how the insertion-count
    # line this replaces came to be ignored.
    for empty in ({}, {"split_risk": []}, {"split_risk": None}):
        out = []
        mod._render_split_risk(empty, out.append)
        if out:
            failures.append(f"split_risk must print NOTHING for {empty!r}, got {out!r}")

    # AF-365: the BLOCKED remedy must offer the non-destructive exit FIRST.
    #
    # On a shared index `git restore --staged <their path>` mutates state that
    # belongs to the other lane: their file is staged because THEY staged it, and
    # unstaging is an edit to someone else's in-flight work made by a party who
    # cannot see what they intended. `git commit <your paths>` ignores the index
    # for everything it does not name, so both lanes commit whole in either order.
    #
    # HONEST ABOUT WHAT THIS PROVES. It reads the SHIPPED hook file rather than
    # executing the branch, because that text is emitted inline in main() and
    # reaching it needs a full multi-session git fixture. So this pins that the
    # advice EXISTS and is ORDERED, not that the branch runs. That is weaker than
    # the cells above and is worth saying rather than leaving the reader to assume
    # parity. It still cannot pass against a paraphrase: it reads the artifact that
    # ships, so deleting the advice reddens it.
    hook_src = open(HOOK).read()
    blocked_at = hook_src.find("COMMIT BLOCKED")
    if blocked_at < 0:
        failures.append("cannot find the COMMIT BLOCKED section in the shipped hook")
    else:
        tail = hook_src[blocked_at:]
        pathspec_at = tail.find("COMMIT ONLY YOUR OWN PATHS")
        # Anchor on strings that appear only in the EMITTED advice, never in a
        # comment. The first version of this cell searched for "git restore
        # --staged" and matched the explanatory comment above the code, which
        # made the ordering assertion measure prose instead of output.
        restore_at = tail.find("Or unstage theirs")
        if pathspec_at < 0:
            failures.append("the blocked remedy no longer offers a pathspec commit")
        elif restore_at < 0:
            failures.append("the blocked remedy no longer offers the unstage exit")
        elif pathspec_at > restore_at:
            failures.append(
                "the DESTRUCTIVE remedy is listed before the non-destructive one; "
                "`git restore --staged` edits the peer's staged work and should not "
                "be the first thing a blocked lane reaches for")
        # And the reason must travel with it. An unexplained ordering gets
        # 'tidied' back by the next person who thinks restore reads better first.
        # No arbitrary window: search from the pathspec advice to the end of the
        # unstage line. A fixed byte bound silently stops covering the text it
        # was chosen to cover as soon as anyone adds a comment above it, which
        # is what a [:4000] bound did on the first run of this cell.
        if "EDITS THE SHARED INDEX" not in tail[pathspec_at:restore_at + 400]:
            failures.append(
                "the restore remedy no longer says it edits the peer's index; "
                "without the reason, the ordering above is arbitrary and reversible")

    if failures:
        print(f"FAIL {len(failures)}:")
        for f in failures:
            print(" -", f)
        return 1
    print("ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
