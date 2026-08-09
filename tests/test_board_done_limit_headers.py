"""AC-301: an explicit ?done_limit= must filter before capping, and must announce the cap.

AC-291 fixed the DEFAULT path: filter first, then cap what the caller asked for, and emit
X-Amux-Truncated so a short list is distinguishable from a complete one. Two other returns
served explicit done_limit values and did neither — they capped in SQL before filtering (wrong
denominator) and emitted only X-Amux-Done-Limit, which names the knob rather than the omission.

Worst possible path for that: a caller passes done_limit precisely WHEN they suspect truncation.
Measured before the fix, with the build bracketed so one server was measured:
    ?archived=1&session=amux-cloud&slim=1              -> 48 rows
    ?archived=1&session=amux-cloud&slim=1&done_limit=3 ->  1 row, X-Amux-Done-Limit only
One row is not "the 3 newest of mine"; it is what survives filtering a 3-row GLOBAL window.
After: 4 rows with Truncated=1, Terminal-Total=47, Terminal-Returned=3.

Source-shape tests, and the limit is stated rather than glossed: the handler is deep in a 75k-line
single file behind an HTTP layer, so these pin the SHAPE. The behavioural check that actually
verified this is on the card (three live requests, build bracketed before and after). Two defects
today lived in code that every source-shape test passed, so treat these as a regression tripwire.
"""
import re
from pathlib import Path

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()

FOUR_HEADERS = ("X-Amux-Done-Limit", "X-Amux-Truncated",
                "X-Amux-Terminal-Total", "X-Amux-Terminal-Returned")


def _board_get():
    """The GET /api/board handler body, up to the next route."""
    i = SRC.index('if method == "GET" and path == "/api/board":')
    rest = SRC[i:]
    nxt = re.search(r'\n            if method == "POST" and path == "/api/board"', rest)
    return rest[:nxt.start() if nxt else 30000]


def _explicit_returns(body):
    """Every `return self._json(...)` that sits under an `if done_limit != 100:` or
    `if done_limit == 0 or done_limit != 100:` guard — i.e. the explicit-parameter paths."""
    out = []
    for m in re.finditer(r"if done_limit (?:!= 100|== 0 or done_limit != 100):", body):
        # WINDOW SIZED FOR THE COMMENT, NOT THE CODE. First version used 2600 chars
        # and sliced to r+600; the fix's own comment block is ~30 lines, so the
        # `return` landed near the boundary and the header dict was cut off mid-way —
        # the test failed against code that was correct. Same family as every other
        # positional-probe miss today, self-inflicted by verbose comments. Slice from
        # the RETURN so the window does not depend on how much prose precedes it.
        seg = body[m.start():m.start() + 6000]
        r = seg.find("return self._json")
        if r != -1:
            out.append(seg[:r] + seg[r:r + 900])
    return out


def test_both_explicit_done_limit_paths_exist():
    """Two of them, and finding only one is how I fixed the wrong branch first: the projecting
    branch has an EARLY return that skipped the AC-291 remedy sitting eleven lines below it."""
    paths = _explicit_returns(_board_get())
    assert len(paths) >= 2, (
        "expected at least 2 explicit-done_limit return paths, found %d — if they were "
        "consolidated that is fine, but re-check that the survivor filters before capping "
        "and emits all four headers" % len(paths))


def test_every_explicit_path_announces_truncation():
    """THE DEFECT. X-Amux-Done-Limit names which limit was requested; X-Amux-Truncated says
    whether it BIT. A caller cannot tell a complete 12-row answer from a 12-row window onto
    2000 without the second one."""
    for i, seg in enumerate(_explicit_returns(_board_get())):
        for h in FOUR_HEADERS:
            assert h in seg, (
                "explicit done_limit path #%d does not emit %s — a list that can omit must "
                "announce it, and naming the knob is not announcing the omission (AC-301)"
                % (i, h))


def test_every_explicit_path_loads_unfiltered_then_caps():
    """The wrong-denominator half. _load_board(done_limit=N) truncates in SQL BEFORE
    _board_project filters, so the cap lands on the whole board and the filter runs on that
    window. Load with 0, filter, THEN _cap_terminal."""
    for i, seg in enumerate(_explicit_returns(_board_get())):
        assert "_load_board(done_limit=0)" in seg, (
            "path #%d still caps in SQL before filtering — that is AC-291's wrong-denominator "
            "bug reached through the explicit parameter" % i)
        assert "_cap_terminal(" in seg, (
            "path #%d does not cap the FILTERED set, so done_limit is now ignored entirely — "
            "the opposite over-correction" % i)
        assert seg.index("_load_board(done_limit=0)") < seg.index("_cap_terminal("), (
            "path #%d caps before it loads/filters; order is the whole fix" % i)


def test_done_limit_zero_is_not_capped():
    """done_limit=0 means "no cap" and must not be fed to _cap_terminal — capping at 0 would
    return zero terminal cards, turning the opt-OUT into the strictest possible limit."""
    for i, seg in enumerate(_explicit_returns(_board_get())):
        assert "if done_limit == 0:" in seg, (
            "path #%d has no done_limit==0 guard before _cap_terminal; 0 means uncapped, and "
            "_cap_terminal(x, 0) would drop every terminal card" % i)
