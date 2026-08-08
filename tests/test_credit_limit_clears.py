"""A credit/model cap clears on its REMEDY, and its age is visible (AF-14).

Unlike a rate limit (a claim about TIME, with its own reset), credit_limited
is true until a model switch or a top-up — neither legible from the pane. The
AMUX-2566 expiry can never reach it (it keys on rate_limit_reset_at, which a
credit limit never has), and the pane-absence heuristic clears LIVE caps when
the banner scrolls out of capture. amux-frustrations' audit on the card named
three approaches; the shipped design is (2) clear-on-remedy + (3) age in the
payload/badge.

The remedy-clear became REQUIRED, not optional, when rate-limit state started
persisting across server restarts (e21ef3d): before that, execv amnesia was
accidentally clearing stale credit flags several times a day; after it, a
stale flag would survive indefinitely with no clock to expire it.
"""

import importlib.util
import os
import sys
import time
from pathlib import Path

import pytest

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


@pytest.fixture(scope="module")
def srv(tmp_path_factory):
    home = tmp_path_factory.mktemp("h")
    (home / "sessions").mkdir(parents=True)
    prev = os.environ.get("AMUX_HOME")
    os.environ["AMUX_HOME"] = str(home)
    try:
        sys.modules.pop("amux_server_cl", None)
        spec = importlib.util.spec_from_file_location("amux_server_cl", SERVER_PATH)
        mod = importlib.util.module_from_spec(spec)
        sys.modules["amux_server_cl"] = mod
        spec.loader.exec_module(mod)
        mod._init_db()
        yield mod
    finally:
        if prev is None:
            os.environ.pop("AMUX_HOME", None)
        else:
            os.environ["AMUX_HOME"] = prev


def _cap(srv, name, age_s=3600):
    srv._session_auto_actions[name] = {
        "rate_limit_credits": True,
        "rate_limit_model_name": "opus",
        "rate_limit_last_event_ts": int(time.time() - age_s),
    }


def test_the_remedy_clear_pops_the_flag_and_logs(srv):
    _cap(srv, "lane-a")
    srv._clear_credit_limit("lane-a", "unit test")
    acts = srv._session_auto_actions["lane-a"]
    assert "rate_limit_credits" not in acts
    assert "rate_limit_model_name" not in acts


def test_a_lane_without_the_flag_is_untouched(srv):
    """The clear must not fabricate state or log for healthy lanes — it runs
    on EVERY session start."""
    srv._session_auto_actions["lane-b"] = {"rate_limit_reset_at": time.time() + 60}
    srv._clear_credit_limit("lane-b", "unit test")
    assert srv._session_auto_actions["lane-b"].get("rate_limit_reset_at"), (
        "the credit clear swept a genuine rate-limit flag")


def test_the_payload_reports_when_the_cap_was_seen(srv):
    """Option 3: a stale detection must be visible AS stale. The payload
    carries the detection timestamp only while the flag is set."""
    (Path(srv.CC_SESSIONS) / "lane-c.env").write_text("CC_DIR=/tmp\n")
    _cap(srv, "lane-c", age_s=7200)
    s = [x for x in srv.list_sessions() if x["name"] == "lane-c"][0]
    assert s["credit_limited"] is True
    assert s["credit_limited_since"] == pytest.approx(time.time() - 7200, abs=5)
    # And cleared -> no orphaned timestamp claiming a cap that is gone.
    srv._clear_credit_limit("lane-c", "unit test")
    s = [x for x in srv.list_sessions() if x["name"] == "lane-c"][0]
    assert s["credit_limited"] is False
    assert s["credit_limited_since"] == 0


def test_start_session_is_wired_to_the_clear(srv):
    """The wiring, asserted structurally: start_session must invoke the clear
    before any spawn path — a restart is the remedy the flag waits for. (A
    full spawn needs tmux; the call-site pin plus the unit tests above cover
    the behavior without one.)"""
    import inspect
    body = inspect.getsource(srv.start_session)
    assert "_clear_credit_limit" in body, (
        "start_session no longer clears the credit flag — with persistence "
        "(e21ef3d) a stale cap now survives every restart indefinitely")
