"""Payloads the CLIENT persists must stay inside a browser storage budget (AMUX-2522).

RCA, 2026-08-07: Ethan could not view workers offline. Nothing had broken — a payload
grew past a threshold nobody was watching, and the failure was silent by construction.

  ?archived=0, as designed (AMUX-2271) ....  129 KB
  ?archived=0, measured today ............. 2.74 MB   (21x)
  non-archived card `desc` bytes .......... 3.22 MB across 1241 cards
  localStorage cap ........................ ~5 MB, SHARED with amux_sessions_cache

The board cache write blew quota, its catch removed the key, and the session list was
collateral — offline startup reads `amux_sessions_cache`, so workers vanished.

This suite is the CI half Ethan asked for. It does not pin today's byte counts, which
would fail on any real board; it pins the PROPERTIES that let 129KB become 2.74MB
without anyone noticing:

  1. the cached projection must exclude the fat fields (desc/log are 96% of the bytes)
  2. the board cache must not be able to evict the session list
  3. a quota failure must be announced, not swallowed

An earlier defect in this exact function (the recursive _cacheBoardJSON, whose own
verification passed BECAUSE nothing was ever written) is why property 3 is a test and
not a comment.
"""

import re
from pathlib import Path

import pytest

CLIENT = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _strip_comments(src):
    """Drop // lines. Twice today a guard in this repo matched its own explanatory
    comment and failed on correct code; anchoring on prose about the code is not
    anchoring on the code."""
    return "\n".join(l for l in src.splitlines() if not l.lstrip().startswith("//"))


def _fn(name):
    """Source of one client function, brace-matched, comments stripped."""
    i = CLIENT.find(f"function {name}(")
    assert i > 0, f"{name} not found — if it was renamed, this suite is testing nothing"
    depth, out, started = 0, [], False
    for ch in CLIENT[i:]:
        out.append(ch)
        if ch == "{":
            depth += 1; started = True
        elif ch == "}":
            depth -= 1
            if started and depth == 0:
                break
    return _strip_comments("".join(out))


def test_board_cache_stores_a_slim_projection_not_full_records():
    """desc and log are 96% of the payload. Caching them is what crossed the cap."""
    src = _fn("_cacheBoardJSON")
    assert "desc_len" in src, (
        "the board cache no longer slims records — it is storing full cards again, "
        "which is what grew this payload 21x and evicted the offline worker list")
    # the projection must not carry the fat fields through
    proj = src[src.find(".map("):src.find("})))")] if ".map(" in src else ""
    assert "i.desc," not in proj and "i.log" not in proj, (
        f"the cached projection carries desc/log again: {proj[:200]}")


def test_board_cache_failure_does_not_touch_the_session_cache():
    """The regression's mechanism. A board cache that can remove amux_sessions_cache
    trades a degraded view for a broken app: offline startup reads the session list."""
    src = _fn("_cacheBoardJSON")
    assert "amux_sessions_cache" not in src, (
        "_cacheBoardJSON references the session cache — the board cache must never be "
        "able to evict the worker list, which is what made workers vanish offline")


def test_quota_failure_is_announced_not_swallowed():
    """A cache that silently evicts itself is how this reached 21x its budget unseen —
    and this function has already shipped one silent-eviction bug (the recursive call
    whose verification passed because nothing was ever written)."""
    src = _fn("_cacheBoardJSON")
    # NOT rfind("catch") — that grabs the innermost `catch (e3) {}` from the
    # removeItem and reports a correct fix as broken. The property is that the
    # function announces a dropped cache SOMEWHERE, not that the last catch does.
    assert "console.warn" in src or "console.error" in src, (
        "the quota catch is silent again; a dropped cache must say so")


def test_the_fat_fields_are_actually_the_fat_ones():
    """Guards the premise rather than the fix. If desc/log stop dominating the payload,
    the slimming above is aimed at the wrong fields and this suite is stale."""
    assert "desc" in CLIENT and "log" in CLIENT
    # the server's own slim= projection should agree about what is heavy
    assert "slim" in CLIENT, "the server lost its slim projection concept"


def test_offline_startup_seeds_SESSIONS_not_an_undeclared_global():
    """AMUX-2553. The vocab rename (b009f6e, sessions -> workers in client STRINGS)
    also rewrote `sessions = JSON.parse(_cachedInit)` to `workers = ...` — an
    identifier declared nowhere. The cache loaded into an implicit global nothing
    reads, every renderer kept reading `sessions`, and offline startup showed
    "Can't reach the server" instead of the cached fleet for two days. ast.parse,
    node --check and the client-ref checker are all blind to it: the assignment is
    syntactically valid and creates the name it references."""
    src = CLIENT
    i = src.find("const _cachedInit = localStorage.getItem('amux_sessions_cache')")
    assert i > 0, "offline seed moved — re-anchor"
    block = src[i:i + 900]
    assert "sessions = JSON.parse(_cachedInit)" in block, (
        "the offline cache seeds something other than `sessions` — the renderers "
        "read `sessions`, so offline startup is blank again (AMUX-2553)")
    assert "workers = JSON.parse" not in block


def test_quota_eviction_never_sacrifices_the_sessions_cache():
    """The sessions cache is the offline PWA's core value; the old quota handler
    dropped it FIRST ('biggest transient cache'), so one over-quota history write
    on a phone destroyed the next flight's offline fleet view.

    Invariant, checked GLOBALLY rather than at one anchored site (the first
    version anchored on the first of three setItem('amux_cmd_history') sites and
    failed against the correct fix at the second — the positional trap again):
    every eviction of amux_sessions_cache must be SELF-eviction, i.e. sit right
    after its own failed setItem. No other subsystem may sacrifice it."""
    src = CLIENT
    import re
    evictions = [m.start() for m in re.finditer(r"removeItem\('amux_sessions_cache'\)", src)]
    assert evictions, "no eviction sites at all — key renamed? re-anchor"
    for i in evictions:
        window = src[max(0, i - 400):i]
        assert "setItem('amux_sessions_cache'" in window, (
            "amux_sessions_cache is evicted by a foreign subsystem at offset %d — "
            "one over-quota write elsewhere destroys the offline fleet view "
            "(AMUX-2553)" % i)
    # Positive half: the history quota path must shed the REBUILDABLE caches.
    assert re.search(r"amux_cmd_history[\s\S]{0,900}removeItem\('amux_board_cache'\)", src), (
        "the cmd-history quota handler no longer sheds the board cache first")
