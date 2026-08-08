"""A lingering limit banner must not re-park a lane for an extra 24h.

The incident (mixpeek-autopilot, 2026-08-08, from server.log):

  12:57:57  auto-selected option 1, reset_at=1786292700
  14:08:36  auto-selected option 1, reset_at=1786222200   (today 16:50)
  14:37:04  auto-selected option 1, reset_at=1786222200
  14:38:10  auto-selected option 1, reset_at=1786222200   (61s after the last)
  15:13:41  auto-selected option 1, reset_at=1786222200
  16:50:03  auto-selected option 1, reset_at=1786308600   (TOMORROW 16:50)

Three seconds after the flag became due, the scan re-parsed the banner still
sitting on the idle pane; "resets 4:50pm" with 4:50pm just passed resolves to
tomorrow, and the overwrite beat the auto-resume tick that keys on
reset_at <= now. The lane — idle since 12:23 — was silently parked another day.
The five presses of "1" across the afternoon are the same amnesia: flags and
cooldowns lived only in memory, and this process re-execs on every save of
amux-server.py.

These tests cover the two mechanisms shipped against that log:
  _correct_stale_banner_reset — recognises the roll-to-tomorrow shape and
    undoes it using amux's own memory (flag/tombstone) or, absent memory
    (post-restart), the lane's last deliberate send.
  _persist_rate_limit_state / _hydrate_rate_limit_state — the memory itself,
    surviving the restart.
"""

import importlib.util
import json
import os
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"

# The specimen, verbatim from the log.
RESET_TODAY = 1786222200      # 2026-08-08 16:50 local
RESET_TOMORROW = 1786308600   # RESET_TODAY + 86400, what 16:50:03 stored
NOW_AT_INCIDENT = 1786222203  # 16:50:03


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_rl", None)
        spec = importlib.util.spec_from_file_location("amux_server_rl", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_rl"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        yield mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def test_the_incident_specimen_is_corrected(srv):
    """The 16:50:03 re-parse, with the 14:08 flag still armed, must come back
    as TODAY's reset (already passed) — not tomorrow's."""
    actions = {"rate_limit_reset_at": RESET_TODAY}
    out = srv._correct_stale_banner_reset(
        "autopilot", RESET_TOMORROW, NOW_AT_INCIDENT, actions)
    assert out == RESET_TODAY, (
        "the stale banner re-parse still rolls the due flag a day forward — "
        "auto-resume loses the race it lost on 2026-08-08: %r" % out)


def test_the_expiry_tombstone_also_counts_as_memory(srv):
    """After the expiry sweep pops the flag it leaves rate_limit_expired_at;
    a later re-parse (banner still on screen at reset+11min) must match it."""
    actions = {"rate_limit_expired_at": float(RESET_TODAY)}
    out = srv._correct_stale_banner_reset(
        "autopilot", RESET_TOMORROW, RESET_TODAY + 660, actions)
    assert out == RESET_TODAY


def test_no_memory_falls_back_to_last_send(srv, monkeypatch):
    """Post-restart there is no flag. A lane whose last deliberate input
    predates the rolled-back time cannot have hit a NEW limit after it."""
    monkeypatch.setattr(srv, "_load_meta",
                        lambda name: {"last_send": RESET_TODAY - 4 * 3600})
    out = srv._correct_stale_banner_reset(
        "autopilot", RESET_TOMORROW, NOW_AT_INCIDENT, None)
    assert out == RESET_TODAY


def test_fresh_input_after_the_rolled_back_time_means_genuine(srv, monkeypatch):
    """The counter-case that stops the correction being a blanket rollback: a
    lane that received input AFTER today's occurrence and then hit a cap has a
    banner that really does mean tomorrow. Correcting it would resume straight
    into a live limit."""
    monkeypatch.setattr(srv, "_load_meta",
                        lambda name: {"last_send": RESET_TODAY + 300})
    out = srv._correct_stale_banner_reset(
        "autopilot", RESET_TOMORROW, RESET_TODAY + 900, None)
    assert out == RESET_TOMORROW


def test_no_memory_and_no_last_send_leaves_the_parse_alone(srv, monkeypatch):
    """With nothing to discriminate on, do not guess — a wrong rollback sends
    'continue' into a capped session."""
    monkeypatch.setattr(srv, "_load_meta", lambda name: {})
    out = srv._correct_stale_banner_reset(
        "autopilot", RESET_TOMORROW, NOW_AT_INCIDENT, None)
    assert out == RESET_TOMORROW


def test_a_weekly_reset_days_out_never_takes_this_path(srv, monkeypatch):
    """Weekly banners parse via month-day formats to timestamps days ahead;
    their tod_prev is still in the FUTURE, which must exempt them even when
    last_send is ancient (the first draft of this guard got that wrong)."""
    monkeypatch.setattr(srv, "_load_meta", lambda name: {"last_send": 1})
    now = NOW_AT_INCIDENT
    weekly = int(now + 3 * 86400 + 7200)
    out = srv._correct_stale_banner_reset("autopilot", weekly, now, None)
    assert out == weekly


def test_a_same_day_future_reset_is_untouched(srv, monkeypatch):
    """'resets 6pm' parsed at 5pm lands ~1h out — nowhere near the 20-24h roll
    signature. Even with an idle lane and an old flag, leave it alone."""
    monkeypatch.setattr(srv, "_load_meta", lambda name: {"last_send": 1})
    now = NOW_AT_INCIDENT
    soon = int(now + 3600)
    out = srv._correct_stale_banner_reset(
        "autopilot", soon, now, {"rate_limit_reset_at": soon - 86400})
    assert out == soon


def test_none_passes_through(srv):
    assert srv._correct_stale_banner_reset("x", None, time.time(), {}) is None


def test_rate_limit_state_survives_a_restart(srv):
    """The amnesia is the enabler: every save of amux-server.py re-execs the
    process, and flags/cooldowns held only in memory reset with it. Round-trip
    the persistence and check the hydrate filter."""
    now = time.time()
    srv._session_auto_actions.clear()
    srv._session_auto_actions["parked"] = {
        "rate_limit_reset_at": now + 3600,
        "rate_limit_menu_pressed_ts": now - 60,
        "rate_limit_weekly": False,
        "unrelated_key": "must-not-persist",
    }
    srv._session_auto_actions["ancient"] = {
        # Reset passed 3 days ago — hydrating it would resurrect the stale
        # badge AMUX-2566 removed.
        "rate_limit_reset_at": now - 3 * 86400,
    }
    srv._persist_rate_limit_state(force=True)
    srv._session_auto_actions.clear()
    srv._hydrate_rate_limit_state()
    got = srv._session_auto_actions
    assert "parked" in got, "persisted state did not hydrate"
    assert got["parked"]["rate_limit_reset_at"] == pytest.approx(now + 3600)
    assert got["parked"]["rate_limit_menu_pressed_ts"] == pytest.approx(now - 60)
    assert "unrelated_key" not in got["parked"], (
        "persistence swept non-rate-limit keys into prefs")
    assert "ancient" not in got, (
        "a reset 3 days past hydrated back — stale badges resurrect across restarts")


def test_press_cooldown_key_is_in_the_persist_set(srv):
    """The five-press afternoon needs the cooldown to SURVIVE restarts — a
    persisted flag with an in-memory cooldown re-presses after every save."""
    assert "rate_limit_menu_pressed_ts" in srv._RATE_LIMIT_PERSIST_KEYS
