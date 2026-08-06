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
