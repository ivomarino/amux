#!/usr/bin/env python3
# observed-edits POST half (AF-123). After every Bash command: report files
# under the command's cwd whose mtime moved since the PRE marker, as OBSERVED
# edit records the staged-guard merges at firsthand rank. See the PRE half's
# header for why this exists (the firsthand=0 lane bias) and why observation
# beats parsing (heredocs and extensionless paths are invisible to a regex,
# never to an mtime).
#
# TRACKED SOURCE: scripts/claude-hooks/observed-edits-post.py. Installed to
# ~/.amux/hooks/ and wired in ~/.claude/settings.json (PostToolUse, matcher
# "Bash"). Fail-open always; hard wall-clock budget so a huge tree cannot
# slow every command. Writes one line per report to
# ~/.amux/hooks/state/observed-edits.log — the verify-by-what-it-WROTE marker
# (AMUX-2538's lesson: a hook that looks wired and never ran is invisible
# without one).
import json
import os
import ssl
import sys
import time
import urllib.request

PRUNE = {".git", "node_modules", "target", ".venv", "__pycache__", ".next", "dist"}
MAX_PATHS = 80
FIND_BUDGET_S = 1.5


def main():
    session = (os.environ.get("AMUX_SESSION") or "").strip()
    if not session:
        return
    home = os.environ.get("AMUX_HOME") or os.path.expanduser("~/.amux")
    marker = os.path.join(home, "hooks", "state", f"observed-{session}.t0")
    try:
        t0 = os.stat(marker).st_mtime
    except OSError:
        return
    # Stale marker (no PRE fired, or > 30 min old command): report nothing
    # rather than attributing a peer's writes from a dead window.
    if time.time() - t0 > 1800:
        return
    try:
        d = json.load(sys.stdin)
    except Exception:
        return
    cwd = (d.get("cwd") or "").strip()
    if not cwd or not os.path.isdir(cwd):
        return

    deadline = time.monotonic() + FIND_BUDGET_S
    hits = []
    for root, dirs, files in os.walk(cwd):
        if time.monotonic() > deadline or len(hits) >= MAX_PATHS:
            break
        dirs[:] = [x for x in dirs if x not in PRUNE and not x.startswith(".cache")]
        for f in files:
            p = os.path.join(root, f)
            try:
                if os.stat(p).st_mtime >= t0:
                    hits.append(p)
                    if len(hits) >= MAX_PATHS:
                        break
            except OSError:
                continue
    if not hits:
        return

    url = os.environ.get("AMUX_URL") or "https://localhost:8824"
    try:
        with open(os.path.join(home, "endpoint.json")) as fh:
            ep = json.load(fh)
        stale = list(ep.get("retired_ports") or [])
        if ep.get("legacy_port"):
            stale.append(ep["legacy_port"])
        from urllib.parse import urlsplit
        sp = urlsplit(url)
        if sp.hostname in ("localhost", "127.0.0.1", "::1") and sp.port in stale:
            url = (ep.get("canonical_url") or url).rstrip("/")
    except Exception:
        pass
    try:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        req = urllib.request.Request(
            url.rstrip("/") + "/api/git/observed-edits",
            data=json.dumps({"paths": hits}).encode(),
            headers={"Content-Type": "application/json", "X-Amux-Session": session},
            method="POST")
        urllib.request.urlopen(req, timeout=2, context=ctx).read()
        outcome = "sent"
    except Exception as e:
        outcome = f"send-failed:{e.__class__.__name__}"
    try:
        with open(os.path.join(home, "hooks", "state", "observed-edits.log"), "a") as fh:
            fh.write(f"{int(time.time())} {session} n={len(hits)} {outcome}\n")
    except Exception:
        pass


try:
    main()
except Exception:
    pass
sys.exit(0)
