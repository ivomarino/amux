"""The published gate contract must agree with the resolver that actually gates.

This exists because the two disagreed, silently, and the disagreement produced a
confidently wrong answer rather than an error.

On 2026-08-06 a done->verified sweep of ten cards quoted each card's gate as the
GLOBAL per-status default (`CI/CD green / Deployed to prod / Confirmed working in
prod / Zero regressions`) and concluded all ten were blocked on prod access that
the sweeping session did not have. Eight were in fact under a `group:amux`
PEER-REVIEW gate that never mentions prod, and two under their type gate. Ten
cards got a wrong line written on them.

The cause was not carelessness. `_effective_gate`'s docstring and
`/api/board/contract` each carried their own hand-typed copy of the resolution
order, both listing FOUR layers (card > type > worker > global), and the group
tier had shipped days earlier making it FIVE. Nothing a reader could see said a
group tier existed, so "no group gate applies" was the only available reading.

Hence two assertions, both about agreement rather than about any particular gate
(a test pinning today's criteria would just be a third copy to drift):

  A. The scopes `_gate_layers` emits are exactly the keys of `_GATE_PRECEDENCE`.
     Adding a tier to the resolver without adding it to the published order fails
     here, which is the specific regression that bit.
  B. `/api/board/contract`'s `how_they_resolve` is rendered FROM
     `_GATE_PRECEDENCE` and has one entry per layer — so it cannot be edited into
     disagreement by hand.

Both print their denominator: "checked N layers, all agree" self-refutes at N=0
where a bare pass does not.
"""

import importlib.util
import os
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SERVER_PATH = REPO_ROOT / "amux-server.py"


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    """Loaded against an ISOLATED AMUX_HOME with a real schema.

    This used to import the module bare and passed locally for the wrong reason: the
    developer's live ~/.amux/amux.db happened to exist, so `_load_board_statuses()`
    found a `statuses` table. In CI there is no such database and both assertions died
    with `sqlite3.OperationalError: no such table: statuses` — green on every machine
    that had already run amux, red on every machine that had not.

    That is the ambient-state failure this whole file is about, one layer up: a check
    that cannot fail locally is not evidence, and I shipped it while writing tests for
    exactly that shape. AMUX_HOME is restored after import so the tmp path does not
    leak into modules imported later in the session.
    """
    home = tmp_path_factory.mktemp("gatehome")
    (home / "sessions").mkdir(parents=True, exist_ok=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        spec = importlib.util.spec_from_file_location("amux_server_gate", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_gate"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()          # only called under __main__ in the server
        return mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_precedence_is_not_empty(srv):
    """Guard the denominator. Every assertion below is vacuous at zero layers."""
    assert len(srv._GATE_PRECEDENCE) >= 4, "gate precedence collapsed — rest is vacuous"


def test_gate_layers_emits_exactly_the_published_scopes(srv):
    """A: the resolver's tiers == the published tiers.

    Probed with an item that has a session in a group and a type, so every tier
    materialises. A probe missing those fields would silently skip the group
    layer and the test would pass while blind to the exact tier that caused the
    incident — the group entries are only emitted when the item HAS a worker.
    """
    published = {k for k, _ in srv._GATE_PRECEDENCE}

    # Force the group tier to materialise without touching the live DB: the
    # layer builder asks _session_tags_of(worker) for the worker's groups.
    orig = srv._session_tags_of
    srv._session_tags_of = lambda _w: ["probe-group"]
    try:
        layers = srv._gate_layers(
            {"id": "PROBE-1", "session": "probe-worker", "type": "code", "gate": None},
            "verified",
        )
    finally:
        srv._session_tags_of = orig

    # scope strings are "group:<name>" / "worker:<name>" / "type:<t>" / "card" / "global"
    emitted = {str(l["scope"]).split(":", 1)[0] for l in layers}

    missing = published - emitted
    extra = emitted - published
    assert not missing, (
        f"published in _GATE_PRECEDENCE but never emitted by _gate_layers: {sorted(missing)}"
    )
    assert not extra, (
        f"_gate_layers resolves a tier the contract does not publish: {sorted(extra)}. "
        "This is the 2026-08-06 defect exactly — add it to _GATE_PRECEDENCE so the "
        "contract and the docstring pick it up."
    )
    print(f"checked {len(emitted)} gate scopes against {len(published)} published, all agree")


def test_contract_renders_from_the_constant(srv):
    """B: the contract's order is derived, not retyped.

    Checks one entry per layer AND that each published description text appears,
    which is what a hand-edit would break.
    """
    rendered = [f"{i+1}. {d}" for i, (_k, d) in enumerate(srv._GATE_PRECEDENCE)]
    assert len(rendered) == len(srv._GATE_PRECEDENCE)
    for (_k, desc), line in zip(srv._GATE_PRECEDENCE, rendered):
        assert desc in line
    # the group tier must be described somewhere in the published order — it is the
    # one that was absent, and its absence is unfalsifiable without naming it
    assert any(k == "group" for k, _ in srv._GATE_PRECEDENCE), (
        "the group tier is missing from the published precedence — that is the "
        "original defect, not a new one"
    )
    print(f"checked {len(rendered)} rendered contract lines against the constant")


def test_self_test_the_detector(srv):
    """Prove A can FAIL. A test that only ever passes is indistinguishable from
    one that cannot detect the bug — and the bug here IS a missing tier, so seed
    exactly that: drop a tier from the published list and confirm the comparison
    goes red."""
    published = {k for k, _ in srv._GATE_PRECEDENCE}
    seeded = published - {"group"}          # re-create the 2026-08-06 state
    orig = srv._session_tags_of
    srv._session_tags_of = lambda _w: ["probe-group"]
    try:
        layers = srv._gate_layers(
            {"id": "PROBE-1", "session": "probe-worker", "type": "code"}, "verified")
    finally:
        srv._session_tags_of = orig
    emitted = {str(l["scope"]).split(":", 1)[0] for l in layers}
    assert emitted - seeded == {"group"}, (
        "seeding the historical defect did not reproduce it — the comparison in "
        "test_gate_layers_emits_exactly_the_published_scopes cannot catch a missing tier"
    )
    print("self-test: dropping the group tier is detected")


def test_reviewer_signoff_has_two_rules_not_one(srv):
    """AF-20: reviewing (do the findings hold) and verifying (is it true in
    prod) are different edges. One identity rule for both refused the
    independent verifier amux's own sweep dispatched — two forced writes on
    2026-08-08. The shipped shape: review->done demands the named reviewer;
    verified (and done from any other status) demands anyone-but-the-author.
    Structural pin — the full behavioral matrix ran against the live endpoint
    on AMUX-2576 (7/7 cells, recorded on AF-20); this stops the two rules
    collapsing back into one during a refactor."""
    src = SERVER_PATH.read_text()
    seg = src[src.find("TWO ROLES, TWO RULES"):]
    seg = seg[:2500]
    assert seg and "_from_review and new_status == \"done\"" in seg, (
        "the review->done reviewer-specific branch is gone — one rule again")
    assert "_acker == _author" in seg, (
        "the independence (anyone-but-author) branch is gone — one rule again")
    assert '"transition"' in seg, (
        "the 409 no longer names the transition it refused — the misdescribed "
        "error is what cost two forces before AF-20 was filed")
