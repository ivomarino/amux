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
    home path is resolved at import time.

    AMUX_HOME is RESTORED after the import. It only has to be set while the module
    body runs — CC_HOME is bound there and the module keeps it — but leaving it set
    leaks a tmp path into every module imported later in the session. That is not
    hypothetical: it broke tests/test_gate_contract.py, which imports the server
    fresh and got a home directory pytest had already deleted. A test that fails
    only depending on which file ran before it is worse than no test, and this one
    pointed at an innocent module.
    """
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_nudge", None)
        spec = importlib.util.spec_from_file_location(
            "amux_server_nudge", Path(__file__).parent.parent / "amux-server.py")
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_nudge"] = mod
        spec.loader.exec_module(mod)
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


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


@pytest.fixture(scope="module")
def srv_mod(tmp_path_factory):
    """Module only — these assert on predicates, not on DB state."""
    return _load(tmp_path_factory.mktemp("pairing"))


# ───────────── tier 3: the pairing tripwire (backend, AMUX-2478) ─────────────
#
# All three of tonight's causes were one half of a pair widened while the other
# was left behind. Tiers 1 and 2 (extract the predicate; generate one side from
# the other) are applied in the code. This is tier 3: enumerate the enforcement
# predicate's DOMAIN and assert every member has a consumer behaviour, so the
# pairing has a standing known-positive instead of a comment asking to be
# remembered. It fails when someone widens one half.

def test_every_signoff_target_has_a_router_branch(srv_mod):
    """For each target status requiring reviewer sign-off, some card status must
    route to the reviewer. A target nobody can be routed for is an author stuck
    at a 409 while the reviewer is never told — cause 3 exactly."""
    routed = {srv_mod._advance_target(s)
              for s in srv_mod._ADVANCE_NEXT
              if srv_mod._reviewer_acts_next(s)}
    missing = set(srv_mod._REVIEWER_SIGNOFF_TARGETS) - routed
    assert not missing, (
        f"sign-off is enforced for {sorted(missing)} but no card status routes to the "
        f"reviewer for it — widen _ADVANCE_NEXT or the router, or narrow enforcement")
    print(f"checked {len(srv_mod._REVIEWER_SIGNOFF_TARGETS)} sign-off targets, "
          f"{len(routed)} routed")


def test_advance_ladder_has_no_dead_rungs(srv_mod):
    """Every status the nudge SELECTS must have a next target, or the card is quoted
    a gate it already satisfied (cause 2). The selection set is the domain here."""
    selected = ("doing", "review", "done")
    dead = [s for s in selected if not srv_mod._advance_target(s)]
    assert not dead, f"nudge selects {dead} but the ladder gives them no target"
    for s in selected:
        assert srv_mod._advance_target(s) != s, f"{s} advances to itself"
    print(f"checked {len(selected)} selected statuses, all advance forward")


def test_reviewer_acts_next_is_derived_not_hardcoded(srv_mod):
    """The whole point of tier 1: narrowing enforcement must narrow routing with it,
    with no second edit. If this fails, someone re-hardcoded the routing set."""
    orig = srv_mod._REVIEWER_SIGNOFF_TARGETS
    try:
        srv_mod._REVIEWER_SIGNOFF_TARGETS = ("verified",)   # narrow enforcement only
        assert srv_mod._reviewer_acts_next("done") is True, "done->verified still gated"
        assert srv_mod._reviewer_acts_next("review") is False, (
            "review->done no longer requires sign-off, but the router still claims it "
            "does — the routing set is not derived from the enforcement set")
    finally:
        srv_mod._REVIEWER_SIGNOFF_TARGETS = orig


# ───── reviewer engagement by MESSAGE, not just board write (AMUX-2479) ──────

def _msg(mod, origin, text, ts_ms):
    mod.get_db().execute(
        "INSERT INTO cmd_history (text,type,session,ts,origin) VALUES (?,'session','someone',?,?)",
        (text, ts_ms, origin))
    mod.get_db().commit()


def test_message_from_reviewer_counts_as_engagement(rig):
    mod, _ = rig
    _msg(mod, "radio-canada", "looked at BACKE-3182, ship it", 1_700_000_000_000)
    assert mod._reviewer_msg_engagement("BACKE-3182", "radio-canada") == 1_700_000_000_000


def test_a_BLOCK_counts_too_engagement_not_approval(rig):
    """backend's constraint. Their MF-500 round-1 was a refusal; a sentiment check
    would call it not-an-ack and re-nudge an actively engaged reviewer."""
    mod, _ = rig
    _msg(mod, "peer", "MF-500 NOT acked — three of these must move back", 1_700_000_000_000)
    assert mod._reviewer_msg_engagement("MF-500", "peer") == 1_700_000_000_000


def test_origin_spelling_variants_still_match(rig):
    """Real origins seen in one night: hyphenated, space-separated, and suffixed with
    '[manual:ip:...]'. Equality would miss two of the three."""
    mod, _ = rig
    _msg(mod, "mixpeek frustrations [manual:ip:100.66.26.84]", "MF-501 done", 1_700_000_000_001)
    assert mod._reviewer_msg_engagement("MF-501", "mixpeek-frustrations") == 1_700_000_000_001


def test_a_DIFFERENT_session_does_not_count(rig):
    """The counter-case: prefix matching must not let a similarly-named lane silence
    a nudge. 'amux-frustrations' is not 'mixpeek-frustrations'."""
    mod, _ = rig
    _msg(mod, "amux-frustrations", "MF-502 looks fine to me", 1_700_000_000_002)
    assert mod._reviewer_msg_engagement("MF-502", "mixpeek-frustrations") == 0


def test_word_boundary_MF_500_does_not_match_MF_5001(rig):
    mod, _ = rig
    _msg(mod, "peer", "MF-5001 is unrelated", 1_700_000_000_003)
    assert mod._reviewer_msg_engagement("MF-500", "peer") == 0


def test_router_stays_QUIET_when_reviewer_engaged_by_message(rig):
    """End to end: the whole point. Reviewer answered out-of-band, so no re-nudge."""
    mod, sent = rig
    _card(mod, "P-9", "done", "code", desc="x")
    db = mod.get_db()
    db.execute("UPDATE issues SET reviewer='peer' WHERE id='P-9'")
    db.commit()
    _msg(mod, "peer", "P-9 reviewed, holding on one point", int(time.time() * 1000))

    mod._advance_last.pop("probe", None)
    sent.clear()
    mod._advance_open_card("probe")
    assert not sent, f"re-nudged a reviewer who had already answered by message: {sent}"


def test_router_STILL_FIRES_when_reviewer_has_not_engaged(rig):
    """The paired positive. Without it, the test above is satisfied by a check that
    silences everything."""
    mod, sent = rig
    _card(mod, "P-10", "done", "code", desc="x")
    db = mod.get_db()
    db.execute("UPDATE issues SET reviewer='peer' WHERE id='P-10'")
    db.commit()
    _msg(mod, "peer", "talking about some OTHER card entirely", int(time.time() * 1000))

    mod._advance_last.pop("probe", None)
    sent.clear()
    mod._advance_open_card("probe")
    assert sent and sent[-1][0] == "peer", (
        "reviewer was never asked despite no engagement — the silence check is "
        "matching everything, which is worse than not having it")
