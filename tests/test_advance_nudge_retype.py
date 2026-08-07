"""The idle/advance nudge must offer RE-TYPE, and must not quote a gate the card passed.

AMUX-2478, found by mixpeek-frustrations. Four finished cards — a doc move, two
negative-result investigations, an archive move — sat terminal-at-done getting this
nudge every turn. Typed `code`, they faced gates (CI green / deployed / confirmed in
prod) with nothing to bind to for a markdown move or a negative result. They refused
every exit the menu offered, correctly: false verified, fabricated trigger, false
discard.

The state they needed already existed at gate layer 2 — investigation/doc/research/
chore carry done="Outcome recorded in the item" and verified="Outcome confirmed to
still hold" — but the menu never said so. Ethos rule 3: when a gate does not fit, fix
the TYPE, not the truth. A menu listing only dishonest exits is what teaches a capable
model to pick one, and `code` is the DEFAULT type, so this is the common case (1,143 of
1,215 open cards were typed `code`).

Second defect, found while fixing the first and shipped by me: `gate_next` was
`"review" if status == "doing" else "done"`. When `done` was added to this nudge's
selection (f88fbc3) the ternary was not updated, so a card already AT `done` was told
to satisfy the gate for `done` — the status it was already in. The one class of card
the nudge was extended to reach got the one gate it had already passed.

Runs the REAL `_advance_open_card` against a throwaway AMUX_HOME with send_text
captured, so what is asserted is the string a session actually receives — not a
paraphrase of it. Simulating what the function is believed to emit cannot catch it
emitting something else (ethos rule 7).
"""

import importlib.util
import os
import sys
import time
from pathlib import Path

import pytest


def _load(home):
    """Fresh module bound to an isolated AMUX_HOME. Re-imported per test because the
    home path is resolved at import time."""
    os.environ["AMUX_HOME"] = str(home)
    sys.modules.pop("amux_server_nudge", None)
    spec = importlib.util.spec_from_file_location(
        "amux_server_nudge", Path(__file__).parent.parent / "amux-server.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["amux_server_nudge"] = mod
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture
def rig(tmp_path):
    home = tmp_path / "amuxhome"
    (home / "sessions").mkdir(parents=True)
    # The function opens CC_SESSIONS/<name>.env on its FIRST line; without the file
    # it throws, the broad except swallows it to slog, and every assertion here
    # would pass or fail for reasons unrelated to the nudge text.
    (home / "sessions" / "probe.env").write_text("CC_DIR=/tmp\n")
    mod = _load(home)
    mod._init_db()          # only called under __main__ in the server
    sent = []
    mod.send_text = lambda name, text, **kw: (sent.append((name, text)), (True, ""))[1]
    mod.is_running = lambda n: True
    mod._status_applies = lambda st, sess: (True, "")
    mod._session_tags_of = lambda s: []
    return mod, sent


def _card(mod, cid, status, ctype, desc="", session="probe"):
    db = mod.get_db()
    now = int(time.time())
    # owner_type='agent' and archived=0 are REQUIRED by the selection query; a card
    # missing either is silently invisible to the nudge, and a test seeded without
    # them passes its "no message" assertions for entirely the wrong reason.
    db.execute(
        "INSERT INTO issues (id,title,desc,status,session,type,created,updated,notified,"
        "                    owner_type,archived) "
        "VALUES (?,?,?,?,?,?,?,?,1,'agent',0)",
        (cid, f"title for {cid}", desc, status, session, ctype, now, now))
    db.commit()
    return cid


def _nudge(mod, sent, session="probe"):
    """Drive the real function; return the message text a session would receive."""
    mod._advance_last.pop(session, None)
    sent.clear()
    mod._advance_open_card(session)
    return sent[-1][1] if sent else ""


# ───────────────────────── the missing honest exit ───────────────────────────

def test_code_card_with_no_evidence_is_told_it_looks_mistyped(rig):
    mod, sent = rig
    _card(mod, "P-1", "done", "code", desc="moved a markdown file into docs/")
    msg = _nudge(mod, sent)

    assert msg, "no nudge was sent — the rest of this test proves nothing"
    assert "LOOKS MIS-TYPED" in msg, msg
    assert "amux board type P-1" in msg
    assert "Outcome recorded in the item" in msg
    assert "Fix the type, not the truth" in msg


def test_code_card_WITH_commit_evidence_still_offers_retype_but_softer(rig):
    """Evidence means it is probably genuinely code, so do not accuse it — but the
    honest exit must still be on the menu, because evidence in the desc does not
    prove the card's WORK was code."""
    mod, sent = rig
    _card(mod, "P-2", "done", "code", desc="landed in commit deadbeef1234, PR #4412")
    msg = _nudge(mod, sent)

    assert "amux board type P-2" in msg, msg
    assert "LOOKS MIS-TYPED" not in msg


def test_noncode_card_is_not_told_to_retype(rig):
    """The counter-case. A check that fires on every card cannot discriminate, and an
    investigation already HAS an honest gate — telling it to retype would be noise."""
    mod, sent = rig
    _card(mod, "P-3", "done", "investigation", desc="result was negative")
    msg = _nudge(mod, sent)

    assert msg, "no nudge sent — cannot conclude anything from an absent string"
    assert "amux board type P-3" not in msg
    assert "MIS-TYPED" not in msg
    # and it must still be a real nudge, not an empty one
    assert "P-3" in msg


# ──────────────────── the gate_next defect this fix uncovered ────────────────

def test_done_card_is_gated_on_verified_not_on_done(rig):
    """A card at `done` was quoted the gate for `done` — the status it already holds."""
    mod, sent = rig
    _card(mod, "P-4", "done", "code", desc="x")
    msg = _nudge(mod, sent)

    assert "The gate for 'verified' is:" in msg, msg
    assert "The gate for 'done' is:" not in msg, (
        "a card already at done was told to satisfy done's gate — f88fbc3's ternary bug")


def test_doing_and_review_targets_are_unchanged(rig):
    """Guard against fixing `done` by breaking the two statuses that were correct."""
    mod, sent = rig
    _card(mod, "P-5", "doing", "code", desc="x")
    assert "The gate for 'review' is:" in _nudge(mod, sent), "doing -> review regressed"

    db = mod.get_db()
    db.execute("UPDATE issues SET status='review' WHERE id='P-5'")
    db.commit()
    assert "The gate for 'done' is:" in _nudge(mod, sent), "review -> done regressed"


def test_done_card_not_nudged_when_verified_does_not_apply(rig):
    """If `verified` is not a status this lane can reach, `done` IS terminal — nudging
    can only re-fire forever with no honest exit, which is the reported symptom."""
    mod, sent = rig
    mod._status_applies = lambda st, sess: (False, "not enabled for this lane")
    _card(mod, "P-6", "done", "code", desc="x")
    assert _nudge(mod, sent) == "", "terminal-at-done card was nudged with nowhere to go"


# ─────────────── third cause: done + unacked reviewer (MF-495 canary) ────────

def test_done_card_with_unacked_reviewer_routes_to_the_REVIEWER(rig):
    """AMUX-2478 third cause, predicted from mixpeek-frustrations' MF-495.

    The sign-off REQUIREMENT covers `new_status in ("done","verified")` — widened
    from review-only so an author could not skip review via doing->verified
    (AMUX-2217). The ROUTING that tells the reviewer they owe an ack still tested
    `status == "review"`. A card at done with an unacked reviewer therefore nudged
    its AUTHOR to reach verified, every attempt 409'd on "review sign-off required",
    and the reviewer was never told. No coherent action exists for either party.
    """
    mod, sent = rig
    _card(mod, "P-7", "done", "investigation", desc="negative result recorded")
    db = mod.get_db()
    db.execute("UPDATE issues SET reviewer='peer' WHERE id='P-7'")
    db.commit()

    mod._advance_last.pop("probe", None)
    sent.clear()
    mod._advance_open_card("probe")

    assert sent, "nothing was sent — the card fell through both edges entirely"
    target, msg = sent[-1]
    assert target == "peer", (
        f"nudge went to {target!r}, not the reviewer — the author cannot close a "
        f"reviewer-gated card, so this is an unactionable loop")
    assert "ack done->verified" in msg, msg
    assert "review->done" not in msg, "told the reviewer to ack a move the card already made"


def test_review_card_routing_still_says_review_to_done(rig):
    """Guard the case that already worked, so widening the predicate cannot break it."""
    mod, sent = rig
    _card(mod, "P-8", "review", "code", desc="x")
    db = mod.get_db()
    db.execute("UPDATE issues SET reviewer='peer' WHERE id='P-8'")
    db.commit()

    mod._advance_last.pop("probe", None)
    sent.clear()
    mod._advance_open_card("probe")

    assert sent and sent[-1][0] == "peer"
    assert "ack review->done" in sent[-1][1]
