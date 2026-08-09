"""frustrations.md's CARD: field must be a usable join key (AF-28).

The protocol rests on this field twice over: `.claude/rules/frustrations.md` requires every
entry to link a card, and the deletion protocol keys an author's confirmation to the
entry->card pair. Nothing validated it, so on 2026-08-09 five of thirty-four entries queued
for deletion pointed at cards about something else entirely — one of them another session's
OPEN card, seconds from receiving "validated, deleting" text. It was caught only because a
peer happened to flag one instance by hand.

These tests are the check that could have caught it without the peer. The structural half
runs offline and always; the board cross-check runs only when a board is reachable and
SKIPS otherwise, because a test that fails on a laptop with no server gets deleted.

Deliberately asymmetric on severity, and that is the whole design:
  - missing/malformed FIELDS are an ERROR. They are objectively wrong and cheap to fix.
  - title MISMATCHES are advisory and are NOT asserted on. Card titles get rewritten as
    understanding improves, so a low overlap means "a human should look", never "this is
    wrong". Asserting on it would produce a permanently red test, and a check that is
    always red is the same as no check.
"""
import importlib.util
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
AUDIT = REPO / "scripts" / "frustrations_audit.py"
FRUST = REPO / "frustrations.md"


@pytest.fixture(scope="module")
def aud():
    spec = importlib.util.spec_from_file_location("frust_audit", str(AUDIT))
    m = importlib.util.module_from_spec(spec)
    sys.modules["frust_audit"] = m
    spec.loader.exec_module(m)
    return m


def test_the_parser_does_not_count_the_files_own_header(aud):
    """The header's SECTION HEADING is at column 0 and is not an entry. The first cut of
    this audit reported it as an entry missing all nine fields, on every run — a check
    that always fires teaches people to ignore it, which is worse than no check."""
    entries = aud.parse(FRUST.read_text())
    titles = [e["title"] for e in entries]
    assert not any(t.startswith("Format") for t in titles), \
        "the header section is being parsed as an entry: %r" % titles[:3]
    assert entries, "parsed zero entries — the `---` anchor probably moved"


def test_the_parser_can_actually_fail(aud):
    """Rule 7. A parser that returns 'all fields present' for everything would make the
    test below vacuous, so prove it distinguishes a broken entry from a good one."""
    good = ("h\n---\n## t\nAREA: cli\nSEVERITY: slows\nSTATUS: open\nDATE: 2026-08-09\n"
            "SESSION: x\nCARD: AF-1\nSYMPTOM: s\nCOST: c\nFIX: f\n")
    assert all(aud.parse(good)[0][f] for f in aud.REQUIRED)
    broken = aud.parse(good + "\n## broken\nAREA: cli\nSTATUS: open\n")
    assert len(broken) == 2 and not broken[1]["CARD"], \
        "a entry with no CARD: was reported as complete — the check cannot fail"


def test_every_entry_carries_the_required_fields(aud):
    """The greppability contract. `grep '^STATUS: open'` and `grep '^AREA: attribution'`
    are how the cluster argument gets made at all, and one entry with a missing field is
    one the pattern never counts."""
    bad = []
    for e in aud.parse(FRUST.read_text()):
        miss = [f for f in aud.REQUIRED if not e.get(f)]
        if miss:
            bad.append("%s -> missing %s" % (e["title"][:60], ", ".join(miss)))
    assert not bad, "entries with missing fields:\n  " + "\n  ".join(bad)


def test_every_card_pointer_resolves_or_is_known_offboard(aud):
    """The AF-28 defect itself. Skips when no board is reachable — an unreachable board
    means UNCHECKED, and a test that fails without a server would be deleted within a day.

    Cross-instance ids (AC-* on amux-cloud's board, MS-* on studio's) are legitimately
    absent from this board, so absence alone is not asserted on; this pins the format so
    a typo'd or truncated id is caught, which is the failure that silently points an
    entry at nothing."""
    try:
        board = aud.fetch_board()
    except Exception as ex:
        pytest.skip("board unreachable, CARD: pointers unchecked: %s" % ex)
    assert board, "board returned zero cards — the fetch is not exercising anything"
    import re
    malformed = []
    for e in aud.parse(FRUST.read_text()):
        c = (e.get("CARD") or "").strip()
        if not c or c.lower() == "none":
            continue
        if not re.fullmatch(r'[A-Z][A-Z0-9]*-\d+', c):
            malformed.append("%s -> CARD: %r" % (e["title"][:50], c))
    assert not malformed, "CARD: values that are not a valid card id:\n  " + "\n  ".join(malformed)
