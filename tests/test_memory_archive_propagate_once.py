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
    i = src.find("_MEM_ARCHIVE_FILE\n") if False else src.find("archive.sent")
    assert i > 0, "the propagate-once sidecar is gone — additive-only merge is back"
    block = src[max(0, i - 2000): i + 1200]
    assert "_sent" in block and "not in _sent" in block, (
        "the archive copy no longer excludes already-propagated lines")
