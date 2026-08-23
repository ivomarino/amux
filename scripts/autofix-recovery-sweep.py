#!/usr/bin/env python3
"""AMUX-3464: recovery sweep for pre-AF-137 auto-filed reports.

Those cards were filed with session=NULL, invisible to auto-pickup (AF-137),
and their subjects may have long since recovered. This sweep re-runs each
report class's own check against the LIVE instruments and:

  - RETIRES recovered reports: attributed PATCH to discarded with the
    re-check evidence appended. The server's discard transition clears the
    autofix idem row (the re-arm hook, board.rs), so recurrence files fresh
    — retirement never suppresses future signal. REFUSE to run if the
    running server predates that hook.
  - ROUTES still-live reports to AMUX_AUTOFIX_SESSION's lane, capped per
    run (the migration-event caution from AF-137: never discharge the
    backlog into one queue at once).
  - LEAVES anything it cannot honestly decide (no traffic on a slow-endpoint
    report is quiet, not recovered — "stopped happening is not fixed").

board.autofix_cards_are_dispatchable is the progress meter: green when the
backlog is dispositioned. Doctrine note: this is a WORKER's judgment pass
with evidence recorded per card, not a detector auto-closing on green — the
distinction autofix.rs's own tests pin.

Usage: python3 scripts/autofix-recovery-sweep.py [--dry-run] [--route-cap N]
"""
import json
import os
import re
import ssl
import subprocess
import sys
import urllib.request

ROUTE_CAP = 10
SESSION = os.environ.get("AMUX_SESSION", "amux")
ROUTE_TO = "amux"

# The commit that added the discard->re-arm hook. Retiring without it live
# would suppress recurrence of every retired signature, silently.
REARM_MARKER_PATH = "crates/amux-server/src/api/board.rs"
REARM_MARKER = "detector RE-ARMED"


def base_url():
    out = subprocess.run(["amux", "url"], capture_output=True, text=True)
    u = out.stdout.strip()
    return u if u else os.environ.get("AMUX_URL", "https://localhost:8824")


CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE
BASE = base_url().rstrip("/")


def api(method, path, body=None):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json", "X-Amux-Session": SESSION},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=15, context=CTX) as r:
        return json.loads(r.read().decode())


def rearm_hook_is_live():
    """The running binary must contain the discard re-arm. /health's commit
    plus a git containment check answers it without guessing from time."""
    health = api("GET", "/health")
    commit = (health.get("commit") or "").replace("-dirty", "")
    if not commit or commit == "unknown":
        return False, "health carries no commit"
    src = subprocess.run(
        ["git", "show", f"{commit}:{REARM_MARKER_PATH}"], capture_output=True, text=True
    )
    if src.returncode != 0:
        return False, f"cannot read board.rs at {commit}"
    ok = REARM_MARKER in src.stdout
    return ok, f"running commit {commit} {'has' if ok else 'PREDATES'} the re-arm hook"


def open_unowned_reports():
    # ?full=1 is REQUIRED, not decoration (AMUX-3496 regression, found
    # 2026-08-23): the default board list went slim, so `desc` stopped
    # shipping — and this function identifies auto-filed cards by a string
    # INSIDE desc. With .get("desc") returning None for every row, the
    # filter skipped everything and the sweep reported "0 to do" while 76
    # unowned reports sat on the board. A no-op that prints success is
    # worse than a crash.
    rows = api("GET", "/api/board?full=1")
    if rows and not any("desc" in r for r in rows):
        # The instrument, not the board: refuse rather than report a clean
        # sweep computed from fields that are not there (ethos rule 7).
        raise SystemExit(
            "REFUSING: /api/board?full=1 returned rows with no `desc` field — "
            "this sweep classifies on desc, so a filtered result here would be "
            "a false all-clear. Check the list payload shape before re-running."
        )
    out = []
    for r in rows:
        if r.get("session"):
            continue
        if r.get("status") in ("done", "verified", "discarded"):
            continue
        if r.get("archived"):
            continue
        if "Filed automatically by amux" not in (r.get("desc") or ""):
            continue
        out.append(r)
    return out


def classify(card):
    t = card.get("title") or ""
    m = re.match(r"invariant ([a-z0-9_.]+) failing", t)
    if m:
        return ("invariant", m.group(1))
    if re.search(r"took [\d.]+s", t) or re.search(r"p\d+ .* norm", t):
        m2 = re.search(r"((?:GET|POST|PATCH|DELETE) )?(/api/[^ ]+)", t)
        return ("slow", m2.group(2) if m2 else None)
    return ("other", None)


def main():
    dry = "--dry-run" in sys.argv
    cap = ROUTE_CAP
    if "--route-cap" in sys.argv:
        cap = int(sys.argv[sys.argv.index("--route-cap") + 1])

    ok, why = rearm_hook_is_live()
    print(f"re-arm precondition: {why}")
    if not ok:
        print("REFUSING to retire anything — retirement without the re-arm hook "
              "suppresses recurrence of every retired signature. Wait for adoption.")
        return 1

    inv_state = {
        r["invariant_id"]: r.get("status")
        for r in api("GET", "/api/debug/invariants").get("latest_per_invariant", [])
    }
    stats = api("GET", "/api/logs/stats?since_h=24")
    fam_p95 = {f["family"]: (f.get("p95_ms") or 0, f.get("count") or 0)
               for f in stats.get("families", [])}

    cards = open_unowned_reports()
    print(f"open unowned auto-filed reports: {len(cards)}")
    retired, routed, left = [], [], []
    for c in sorted(cards, key=lambda x: x.get("created") or 0, reverse=True):
        kind, key = classify(c)
        cid = c["id"]
        if kind == "invariant":
            cur = inv_state.get(key)
            if cur == "pass":
                ev = (f"RETIRED by the AMUX-3464 recovery sweep: invariant {key} "
                      f"currently PASSES on the live server (/api/debug/invariants). "
                      f"No fix is claimed; the detector is re-armed by this discard, "
                      f"so recurrence files a fresh card that now reaches a lane.")
                retired.append((cid, ev))
            elif cur == "fail":
                routed.append((cid, f"invariant {key} still failing live"))
            else:
                left.append((cid, f"invariant {key} not in latest_per_invariant — undecidable"))
        elif kind == "slow" and key:
            fam = "/".join(key.split("/")[:3])
            p95, n = fam_p95.get(fam, (None, 0))
            if p95 is not None and n >= 50 and p95 < 5000:
                ev = (f"RETIRED by the AMUX-3464 recovery sweep: family {fam} p95 is "
                      f"{p95}ms over {n} requests in the last 24h (threshold was 10s-class). "
                      f"No fix claimed; detector re-armed by this discard.")
                retired.append((cid, ev))
            elif p95 is None or n < 50:
                left.append((cid, f"family {fam}: insufficient traffic ({n}) — quiet is not recovered"))
            else:
                routed.append((cid, f"family {fam} p95 {p95}ms still slow"))
        else:
            left.append((cid, "class needs a hand check"))

    print(f"retire: {len(retired)}  route-live: {len(routed)} (cap {cap})  leave: {len(left)}")
    if dry:
        for cid, ev in retired[:8]:
            print("  would retire", cid, "—", ev[:90])
        for cid, why_ in routed[:8]:
            print("  would route ", cid, "—", why_)
        return 0

    for cid, ev in retired:
        api("PATCH", f"/api/board/{cid}",
            {"desc_append": "\n\n" + ev, "status": "discarded", "gate_ack": True})
    for cid, why_ in routed[:cap]:
        api("PATCH", f"/api/board/{cid}",
            {"session": ROUTE_TO,
             "desc_append": f"\n\nROUTED by the AMUX-3464 sweep: {why_} — live signal, now dispatchable."})
    for cid, why_ in left:
        print("  left:", cid, "—", why_)
    print(f"done: retired {len(retired)}, routed {min(len(routed), cap)}, "
          f"deferred-routing {max(0, len(routed) - cap)}, left {len(left)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
