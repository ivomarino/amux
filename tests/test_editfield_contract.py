"""Every editField() call site must pass a field the editor actually handles.

AMUX-2559 ("I cant add a worker to a group"): b009f6e's vocab pass renamed the
card menu's field ARGUMENT 'tags' -> 'groups' as if it were a display string.
editField and submitEdit both branch on 'tags', so the mismatch fell through
every branch: the Groups editor opened as a generic text box titled "Edit" and
Save fired NO api call at all — a silent no-op, the worst failure shape,
because the modal opens and closes convincingly.

This is the string-enum sibling of the dead-global class (13 casualties and
counting in test_client_storage_budget): a rename that touches call sites but
not the branches they select. The contract is checkable statically — extract
every field literal passed to editField and assert the dispatcher has a branch
(or an explicit passthrough) for it.
"""

import re
from pathlib import Path

SERVER_PATH = Path(__file__).parent.parent / "amux-server.py"


def test_every_editField_call_site_passes_a_handled_field():
    src = SERVER_PATH.read_text()

    # Fields the dispatcher has branches for: `field === 'x'` inside
    # editField/saveEdit, plus keys of the titles map (the generic-input
    # fallback is legitimate only for fields the SAVE path also handles, so
    # the save branches are the authority).
    handled = set(re.findall(r"field === '([a-z]+)'", src))
    handled |= set(re.findall(r"editState\.field === '([a-z]+)'", src))
    assert "tags" in handled, "extraction broke — 'tags' branch not found"

    # Call sites: editField('<session expr>','field',...). The first arg is a
    # template expression; the second is the literal we care about.
    sites = re.findall(r"editField\('[^']*',\s*'([a-z]+)'", src)
    assert len(sites) >= 8, "extraction broke — expected many editField call sites, got %d" % len(sites)

    unhandled = sorted({f for f in sites if f not in handled})
    assert not unhandled, (
        "editField call site(s) pass fields no dispatcher branch handles — "
        "the modal opens and Save silently no-ops (AMUX-2559 class): %s. "
        "Either add the branch or fix the call site's field name; the label "
        "is vocabulary, the field is the contract." % unhandled)


def test_the_specimen_field_would_have_been_caught():
    """Can-it-fail arm: the exact AMUX-2559 shape ('groups' at a call site,
    no 'groups' branch) must trip the check above."""
    handled = {"tags", "name", "desc"}
    sites = ["tags", "groups"]
    unhandled = [f for f in sites if f not in handled]
    assert unhandled == ["groups"]
