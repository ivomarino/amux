#!/usr/bin/env python3
"""CI check: every _-prefixed function CALLED in the dashboard client is DEFINED.

This class shipped three real bugs in one night (2026-08-06): switchView
dispatching to six deleted notes functions, _gridRestoreLayout throwing on the
first saved pane, wsLoadProfile wiping panes then throwing. All were invisible
to `node --check` (parse-only) and to ast.parse (blind to the client), and all
were one grep from being caught. This is that grep, as a check that can fail.

Scope is deliberately OUR namespace (leading underscore) — generic globals
would false-positive on CDN libraries and browser APIs, and a flaky red here
would recreate the exact disease it exists to cure.

Self-tests run FIRST, every time: a planted undefined call must be caught
(the check can fail) and a known-defined name must resolve (the probe can
find things). A sweep that cannot demonstrate both proves nothing (ethos #7).
"""
import re
import sys

SRC = 'amux-server.py'

# Names matched by the _-prefix convention that are legitimately defined
# elsewhere (element ids used as globals, library injections). Keep SHORT and
# commented — every entry here is a hole in the check.
ALLOW = {
    '_',            # lodash-style placeholder in a couple of arrow params
}


def client_js(src):
    return "\n".join(re.findall(r'<script>\s*\n(.*?)</script>', src, re.DOTALL))


def strip_comments(js):
    js = re.sub(r'/\*.*?\*/', '', js, flags=re.DOTALL)
    js = re.sub(r'^\s*//.*$', '', js, flags=re.M)
    return js


def defined_names(js):
    # `function NAME` ANYWHERE — declarations AND named function expressions
    # ((function _x(){})(), setTimeout(function _retry(){...})). The first cut
    # required leading whitespace, so named expressions were invisible as
    # definitions while their own definition line MATCHED the call regex —
    # three false positives on a clean tree.
    d = set(re.findall(r'function\s+([A-Za-z_$][\w$]*)', js))
    d |= set(re.findall(r'(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=', js))
    d |= set(re.findall(r'window\.([A-Za-z_$][\w$]*)\s*=', js))
    d |= set(re.findall(r'([A-Za-z_$][\w$]*)\s*:\s*(?:async\s*)?function', js))
    return d


def called_names(js):
    return set(re.findall(r'([A-Za-z_$][\w$]*)\s*\(', js))


def undefined_underscore_calls(js):
    js = strip_comments(js)
    d, c = defined_names(js), called_names(js)
    return sorted(n for n in c - d if n.startswith('_') and n not in ALLOW)


def main():
    # ── self-tests: the check must be able to fail, and to pass ──
    planted = "function _realThing() {} _realThing(); _plantedGhost();"
    got = undefined_underscore_calls(planted)
    assert got == ['_plantedGhost'], f"self-test FAILED: check cannot catch a planted ghost ({got})"
    clean = "function _fine() {} _fine();"
    assert undefined_underscore_calls(clean) == [], "self-test FAILED: false positive on clean input"

    src = open(SRC).read()
    js = client_js(src)
    assert len(js) > 500_000, f"client extraction broke ({len(js)} bytes) — the check would pass vacuously"
    bad = undefined_underscore_calls(js)
    if bad:
        print(f"FAIL: {len(bad)} _-prefixed name(s) are CALLED in the client but DEFINED nowhere:")
        for n in bad:
            print("   ", n)
        print("(a deleted function with surviving callers — the switchView/notes class)")
        sys.exit(1)
    print(f"OK: client refs resolve ({len(js)} bytes scanned, self-tests passed)")


if __name__ == '__main__':
    main()


# ── Bare-identifier check (AMUX board regression, 2026-08-06) ────────────────
# The vocabulary rename (tags -> groups) renamed a DECLARATION and left its use
# behind: `const groups = item.tags || []` followed by `tags.forEach(...)`,
# which threw ReferenceError and broke every board render.
#
# Nothing caught it. `node --check` proves a block PARSES, not that its names
# RESOLVE. The refs check above resolves `_`-prefixed FUNCTION calls only. And
# the rename's own invariant — "every changed line differs only by the
# vocabulary words" — passes by construction for an identifier rename, because
# that is exactly what the diff looks like. Three green checks, one live crash.
#
# This closes the specific class: a bare vocabulary identifier used as a value
# with no `const/let/var/function` binding of that name anywhere in the client.
# Deliberately narrow — a full scope analysis needs a real JS parser, and a
# check that reports nothing is worth more than one that cannot be trusted.
_VOCAB_IDENTS = ("tags", "groups", "sessions", "workers")


def bare_vocab_uses(js):
    """Uses of a vocabulary word as a bare identifier (not obj.tags, not a key)."""
    out = set()
    for name in _VOCAB_IDENTS:
        # METHOD CALL ONLY: name.method( — nothing looser.
        # My first cut matched `name` followed by a dot, which fired on PROSE
        # inside string literals ("No limited workers.</div>", "Search
        # workers...") and reported 6 phantom orphans. A checker that cries wolf
        # is worse than none, so this requires a real call shape: the word, a
        # dot, an identifier, and an open paren.
        for m in re.finditer(r"(?<![.\w$'\"])" + name + r"\.[A-Za-z_$][\w$]*\s*\(", js):
            out.add(name)
    return out


def declared_vocab(js):
    d = set()
    for name in _VOCAB_IDENTS:
        if re.search(r"\b(?:const|let|var|function)\s+" + name + r"\b", js) or \
           re.search(r"\b" + name + r"\s*=>", js) or \
           re.search(r"function\s*\([^)]*\b" + name + r"\b", js):
            d.add(name)
    return d


def check_bare_vocab(js):
    used, declared = bare_vocab_uses(js), declared_vocab(js)
    missing = sorted(used - declared)
    # Self-test both directions: a planted orphan must be caught, and a properly
    # declared one must not be. A checker that only ever passes is theatre.
    assert "tags" in bare_vocab_uses("const groups = x.tags || []; tags.forEach(f);"), \
        "self-test: planted orphan not detected"
    assert "tags" in declared_vocab("const tags = x.tags || []; tags.forEach(f);"), \
        "self-test: real declaration not seen"
    return missing


if __name__ == "__main__":
    _src = open(SRC, encoding="utf-8").read() if "SRC" in dir() else open(
        "/Users/ethan/Dev/amux/amux-server.py", encoding="utf-8").read()
    _js = client_js(_src)
    _missing = check_bare_vocab(strip_comments(_js))
    if _missing:
        print("FAIL: vocabulary identifier used but never declared: " + ", ".join(_missing))
        raise SystemExit(1)
    print("OK: no orphaned vocabulary identifiers (%d checked)" % len(_VOCAB_IDENTS))
