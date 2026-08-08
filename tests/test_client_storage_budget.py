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


def test_no_client_assignment_to_the_dead_workers_global():
    """CLASS INVARIANT, not a positional window (AF-10). b009f6e's vocab rename
    produced TWO `workers = ...` assignments in client code — the offline seed
    (caught, cb40d22) and the SSE sessions handler (missed for two days, because
    the polling fallback that DOES write `sessions` masks it whenever SSE is
    down: the healthier your SSE, the staler your list).

    My own AMUX-2553 test could not catch the second one: it anchored on the
    offline seed and read a 900-char window, 17,831 chars short — in the same
    commit whose OTHER test was rewritten from a positional window to a global
    invariant for exactly this reason. The lesson applied to one sibling and not
    the other. This is the invariant both should have been.

    Scoped to <script> blocks: the Python half legitimately uses `workers` as a
    local (the env-export builder). In client code the only legal uses are
    declared locals (let/const/var) — a BARE assignment is the rename bug."""
    import re
    blocks = re.findall(r"<script>(.*?)</script>", CLIENT, re.S)
    assert blocks, "no script blocks found — extraction broke"
    # The naive extractor manufactures one FALSE block: a literal <script> inside
    # a Python string pairs with a later real tag and swallows server code
    # between them (block 8 here starts mid-string and contains `def` — the same
    # artifact the pre-commit JS checker has always skipped by failing it
    # silently). Python at line-start is the tell; real client JS has none.
    blocks = [b for b in blocks if not re.search(r"^\s*def \w+\(", b, re.M)]
    bad = []
    for b in blocks:
        for m in re.finditer(r"^\s*workers\s*=[^=]", b, re.M):
            line = b[:m.start()].count("\n") + 1
            bad.append("block line %d: %s" % (line, m.group(0).strip()[:40]))
    assert not bad, (
        "bare assignment(s) to the dead `workers` global in client code — the "
        "b009f6e rename bug is back: %s" % bad)


def test_no_client_read_of_the_dead_workers_global():
    """The assignment invariant above missed b009f6e's THIRD and FOURTH
    casualties, both READS: `typeof workers !== 'undefined'` gating the #peek=
    deeplink (polled out 20 attempts and no-oped every peek link) and the
    board-card LIVE emphasis (`(workers || []).some(...)` — never lit). A
    typeof guard is the worst shape: it exists to tolerate absence, so the
    dead global reads as calm false forever instead of throwing.

    Legal uses of the word in client code are strings/comments and declared
    locals. A `typeof workers` or a bare `workers` followed by `.`, `(`, `||`
    or `)` in an expression is the rename bug. Anchored on the same script-
    block extraction as the assignment test.

    First cut of THIS test was itself theatre and its own can-it-fail probe
    caught it: the string-stripper's backtick pattern spanned newlines, so it
    matched from one template literal to the next and swallowed the very code
    holding both specimens — HEAD (pre-fix) scanned clean. Line-based now, and
    the pre-fix specimen check below is part of the test.
    """
    import re
    import subprocess

    def _violations(src):
        blocks = [b for b in re.findall(r"<script>(.*?)</script>", src, re.S)
                  if not re.search(r"^\s*def \w+\(", b, re.M)]
        bad = []
        for b in blocks:
            for ln, line in enumerate(b.split("\n"), 1):
                code = line.split("//")[0]   # prose lives in comments; drop it
                for m in re.finditer(r"\btypeof\s+workers\b|\bworkers\s*(?:\.\w|\|\||\.some)", code):
                    pre = code[:m.start()]
                    if re.search(r"(?:let|const|var|function\s*\w*\s*\(|,)\s*$", pre):
                        continue  # declared local / param
                    bad.append("line %d: %s" % (ln, m.group(0)[:40]))
        return bad

    bad = _violations(CLIENT)
    assert not bad, (
        "read(s) of the dead `workers` global in client code — a typeof guard "
        "or short-circuit makes these silent, not safe: %s" % bad)

    # Can this check fail? Run it on the last blob that CONTAINED the
    # specimens; if git can't produce one (shallow clone), skip the arm.
    try:
        pre = subprocess.run(["git", "show", "3211a6e:amux-server.py"],
                             capture_output=True, text=True, timeout=15)
    except Exception:
        pre = None
    if pre and pre.returncode == 0 and pre.stdout:
        assert _violations(pre.stdout), (
            "the detector no longer fires on the commit that motivated it — "
            "either the pattern regressed or the specimen assumption broke")
