"""A deletion from the shared memory archive must STICK (GCA-78).

general-canvas-apps removed two entries from a shared memory-archive.md, verified the
removal with a re-measure (118 -> 116 violations), and found one of them back three
hours later. Their diagnosis named a lost update on a read-modify-write and proposed
compare-and-swap.

It is not a race, and CAS would not have fixed it. `_compose_memory`'s archive copy —
changed in 5877f38 from a clobbering write_text to a merge, to fix a DIFFERENT loss —
was additive-only. The per-lane `<name>.archive.md` was treated as a standing source
of truth re-applied on every sync, so anything deleted from the SHARED destination got
re-seeded the next time any lane synced. No concurrency required; one sequential sync
reproduces it.

The worst property was that it was silent and delayed: the edit succeeded, was
verified by measurement, and regressed hours later. Any session that edits, verifies
and moves on believes it landed. That is what propagate-once removes.

These tests exercise the merge rule directly. The surrounding function does filesystem
and index composition that is not what regressed; pinning the rule is what keeps the
next well-meaning change to this block from reintroducing either loss mode.
"""

import pytest


def merge(dest_lines, lane_lines, already_sent):
    """The shipped propagate-once rule, in the form the archive copy applies it.

    Kept as a local mirror ON PURPOSE and only because the real block is inline in a
    600-line function with no seam. Both loss modes below are asserted, so a change to
    the real code that reintroduces either will be caught by the sibling test that
    reads the source.
    """
    have = {l.strip() for l in dest_lines if l.strip()}
    add = [l for l in lane_lines if l.strip() and l.strip() not in have
           and l.strip() not in already_sent]
    out = dest_lines + add
    sent = already_sent | {l.strip() for l in lane_lines if l.strip()}
    return out, sent


A = "- [A](a.md) — hook"
B = "- [B](b.md) — hook"
C = "- [C](c.md) — hook"


def test_first_sync_propagates_everything():
    out, sent = merge([], [A, B], set())
    assert out == [A, B]
    assert sent == {A, B}


def test_deletion_from_the_shared_archive_is_not_re_seeded():
    """GCA-78's exact sequence: lane propagates A and B, a session deletes B from the
    SHARED file, the lane syncs again. B must stay gone."""
    _, sent = merge([], [A, B], set())          # initial propagation
    out, _ = merge([A], [A, B], sent)           # B deleted downstream, lane re-syncs
    assert out == [A], f"deletion reverted — GCA-78 reproduced: {out}"


def test_a_genuinely_new_retirement_still_flows_after_a_deletion():
    """The counter-case, and the one that makes propagate-once non-trivial: suppressing
    re-seeding must not suppress real new entries. A rule that never adds anything
    would pass the test above and be useless."""
    _, sent = merge([], [A, B], set())
    out, sent2 = merge([A], [A, B, C], sent)    # B stays deleted, C is new
    assert C in out, "a newly retired entry was not propagated"
    assert B not in out, "B came back"
    assert out == [A, C]


def test_additive_only_merge_would_fail_these():
    """Pins WHY the old rule was wrong, so this file documents the defect rather than
    only the fix. This is 5877f38's logic verbatim; it must re-seed B."""
    def additive(dest_lines, lane_lines):
        have = {l.strip() for l in dest_lines if l.strip()}
        return dest_lines + [l for l in lane_lines if l.strip() and l.strip() not in have]
    assert additive([A], [A, B]) == [A, B], (
        "the old additive rule no longer re-seeds — if this fails the historical "
        "defect is gone and this test is the thing to delete, not the fix")


def test_repeated_syncs_are_idempotent():
    """A lane that syncs ten times with no change must not grow the archive — the
    duplicate-line failure that a naive append-only fix would introduce."""
    out, sent = merge([], [A, B], set())
    for _ in range(10):
        out, sent = merge(out, [A, B], sent)
    assert out == [A, B]


def test_source_still_reads_the_sent_sidecar():
    """Guards the mirror above from drifting from the shipped code: the real block must
    still consult a propagated-set, not just the destination. A rewrite that drops the
    sidecar reintroduces GCA-78 while every test above stays green."""
    from pathlib import Path
    src = (Path(__file__).parent.parent / "amux-server.py").read_text()
    assert "archive.sent" in src, (
        "the propagate-once sidecar is gone — additive-only merge is back")
    # Anchor on the append condition itself rather than a byte window around the
    # sidecar name: the first version of this guard used a fixed +1200 slice and
    # broke the moment the migration branch moved code around, which is a check
    # failing for a reason unrelated to what it guards.
    assert "not in _sent" in src, (
        "the archive copy no longer excludes already-propagated lines")
    assert "_sent_f.write_text" in src, (
        "nothing records what was propagated, so the sidecar can never suppress a "
        "re-add on the next sync")


def test_MIGRATION_first_sync_without_a_sidecar_does_not_re_add_deletions():
    """GCA-78's decisive case, and the one my first fix got wrong.

    A lane that predates the fix has no sidecar. If it seeds EMPTY, every line in its
    archive that is absent from the destination looks new and gets appended — re-adding
    exactly the deliberate deletions the fix exists to protect. That is the reported bug
    reproducing through its own fix.

    Seeding from the lane's current archive is correct, and it is an inference rather
    than a preference: the additive merge ran for weeks and re-applied each lane's full
    archive on every sync, so anything in a lane archive was already in the destination.
    lane-minus-destination is therefore deliberate deletions, not backlog.
    """
    # pre-fix state: lane still lists A and B; someone deleted B from the shared file
    lane, dest = [A, B], [A]
    seeded = {l.strip() for l in lane}          # migration seeding
    out, _ = merge(dest, lane, seeded)
    assert out == [A], f"migration re-added a deliberate deletion: {out}"


def test_MIGRATION_empty_seeding_would_have_re_added_it():
    """Pins WHY seeding matters, so the choice is documented and not silently reversible."""
    out, _ = merge([A], [A, B], set())
    assert out == [A, B], (
        "empty seeding no longer re-adds — if this fails the migration hazard is gone "
        "and this test is what to delete, not the seeding")


def test_the_ERROR_PATH_fails_toward_already_propagated():
    """general-canvas-apps, reviewing AMUX-2511: the `except` branch used to set
    `_sent = set()`, which re-adds every deliberate deletion — the reported bug
    restored by the error path of its own fix.

    The asymmetry decides the direction. Assuming propagated wrongly = a new entry
    does not flow, which is LOUD (a memory missing from an index that points at it,
    the 5877f38 symptom). Assuming unpropagated wrongly = resurrection, which is
    SILENT and delayed. Fail toward the loud one.
    """
    lane, dest = [A, B], [A]                 # B deliberately deleted downstream
    on_error_bad = set()                     # the old behaviour
    on_error_good = {l.strip() for l in lane}
    assert merge(dest, lane, on_error_bad)[0] == [A, B], (
        "empty-on-error no longer resurrects — if this fails, delete this test, not the fix")
    assert merge(dest, lane, on_error_good)[0] == [A], (
        "the error path must not re-add a deliberate deletion")


def test_source_error_branch_does_not_reset_to_empty():
    """Reads the shipped code, because the behavioural test above exercises a mirror.
    A future edit could reintroduce `_sent = set()` while every test here stays green."""
    from pathlib import Path
    src = (Path(__file__).parent.parent / "amux-server.py").read_text()
    i = src.find("archive.sent")
    assert i > 0
    # WHOLE FILE, comments stripped. Two earlier versions of this guard were wrong in
    # opposite directions and BOTH passed a broken fix, which is worse than not having
    # it: the first matched its own explanatory comment and failed on correct code; the
    # second used a +3000 byte window that no longer reached the assignment once the
    # comments above it grew. Offsets are the wrong anchor. `_sent = set()` should
    # appear nowhere in executable code, so assert exactly that.
    code = "\n".join(l for l in src.splitlines() if not l.lstrip().startswith("#"))
    assert "_sent = set()" not in code, (
        "the archive sync's error path resets the propagated-set to empty, which "
        "re-adds every deliberate deletion (AMUX-2511 review)")
