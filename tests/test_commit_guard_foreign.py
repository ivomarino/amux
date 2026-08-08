"""The idle dirty-tree nudge must not tell a session to commit a PEER's work (AC-300).

Filed by amux-cloud after they received: "You went idle with 1 uncommitted change(s)
under your working directory: amux-server.py — Commit completed work now... Don't leave
the working tree dirty." The change was not theirs; it was session `amux` mid-iteration
on hook matchers. They declined.

Followed literally, that instruction is "commit your peer's unfinished code", and it is
the mechanism behind two real sweeps the same day (b1c3e93 ~93 lines, 8adf348 ~85 lines,
both disclosed). The third time, the recipient recognised it and stopped — which is the
only reason this is a card rather than a fourth incident. A guard that works only when
its recipient distrusts it is not a guard.

The classifier already had the answer. `_staged_guard_check` returns `foreign` (a peer
edited it, this session did not) alongside `shared` (both did). The nudge consumed
`shared` and ignored `foreign` entirely, so a file that was 100% someone else's work was
named with no warning at all — the remedy existed one key over and nothing reached it.
"""
import re
from pathlib import Path

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _guard_block():
    i = SRC.find("You went idle with {n} uncommitted change(s)")
    assert i > 0, "the dirty-tree nudge moved — re-anchor this test"
    # Read BACKWARDS from the message to the classifier call: the attribution logic
    # runs before the text is built. Bounding the start deliberately, not just the end.
    j = SRC.rfind("_staged_guard_check(", 0, i)
    assert j > 0
    return SRC[j:i]


def test_nudge_consumes_the_FOREIGN_category():
    """The regression. `foreign` means a peer edited it and this session did not."""
    b = _guard_block()
    assert '"foreign"' in b or "'foreign'" in b, (
        "the nudge ignores the classifier's `foreign` list again, so a file that is "
        "entirely a peer's work is named in 'commit your uncommitted changes' with no "
        "warning (AC-300)")


def test_nudge_is_SUPPRESSED_when_nothing_is_this_session_s():
    """If every dirty file belongs to someone else there is no honest action for this
    session to take, and the only available one is destructive. Warning is not enough
    there — the nudge should not fire at all."""
    b = _guard_block()
    assert re.search(r"len\(_fg\)\s*==\s*len\(files\)", b), (
        "the all-foreign case no longer short-circuits; the session is still being "
        "asked to commit a tree containing none of its own work")
    # Bound the END semantically, at the sibling branch, rather than by a byte count.
    # The first version used a 400-char window and failed on a long explanatory
    # comment inside the branch — a test red against correct code, which is the
    # positional-probe trap amux-cloud named the same night ("bound the END boundary
    # as deliberately as the start").
    start = b.find("len(_fg) == len(files)")
    end = b.find("if _fg:", start)
    branch = b[start:end if end > start else len(b)]
    assert "return False" in branch, (
        "the all-foreign branch does not actually suppress the nudge")


def test_the_co_edited_warning_still_exists():
    """The counter-case. `shared` (both sessions touched it) must still WARN rather than
    suppress — on a single-file repo suppressing it would silence the guard permanently,
    which is the opposite over-correction and is called out in the code it guards."""
    b = _guard_block()
    assert "CO-EDITED" in b, "the co-edited warning was removed while adding the foreign one"
    assert '"shared"' in b or "'shared'" in b


def test_foreign_warning_says_DO_NOT_COMMIT():
    """A warning that only describes the situation leaves the recipient to infer the
    action. The two sweeps happened because the instruction was unambiguous and wrong;
    the correction has to be equally unambiguous."""
    b = _guard_block()
    m = re.search(r"NOT YOURS[^\"]*", b)
    assert m, "the foreign branch does not label the files as not-yours"
    assert "Do not commit" in b, "the foreign warning does not tell the reader not to commit"
