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

# AF-124: the walk observes EVERY moved mtime under cwd, including a peer's
# concurrent write — and a pure READ of one file must never claim another
# file's write at firsthand rank (amux-frustrations' live control: POST fired
# for `cat mine_untouched.rs` and claimed a peer's peer_file.rs). So the walk
# is gated on the COMMAND the way the inferred path already is: a command
# whose every segment is read-only reports nothing. This is a conservative
# PYTHON PORT of is_pure_read_command (git_guard.rs is canonical); drift is
# asymmetric by design — a verb the port misses stays reported (today's
# behavior), never silently unreported.
READ_ONLY_VERBS = {
    "ls", "cat", "head", "tail", "less", "more", "grep", "egrep", "fgrep",
    "rg", "ag", "wc", "stat", "file", "find", "cmp", "diff", "sort", "uniq",
    "cut", "column", "od", "xxd", "hexdump", "tree", "du", "basename",
    "dirname", "realpath", "readlink", "sha256sum", "md5sum", "nl", "tac",
    "pwd", "echo", "printf", "cd", "which", "type", "env", "sleep",
}
GIT_READ_SUBCMDS = {
    "show", "log", "diff", "status", "blame", "grep", "cat-file", "shortlog",
    "describe", "rev-parse", "rev-list", "ls-files", "ls-tree", "reflog",
    "whatchanged", "annotate", "name-rev", "show-ref", "for-each-ref",
}


def has_output_redirection(cmd):
    i, n = 0, len(cmd)
    while i < n:
        if cmd[i] == ">":
            j = i + 1
            if j < n and cmd[j] == ">":
                j += 1
            while j < n and cmd[j] in " \t":
                j += 1
            if j < n and cmd[j] != "&":
                return True
        i += 1
    return False


def is_pure_read_command(cmd):
    if has_output_redirection(cmd):
        return False
    saw = False
    for seg in __import__("re").split(r"[|;&\n()`]", cmd):
        seg = seg.strip()
        if not seg or seg.startswith("#"):
            # AF-126: a comment segment writes nothing and must not force the
            # command non-read (same fix as the rust canonical).
            continue
        saw = True
        tok = seg.split()[0] if seg.split() else ""
        verb = os.path.basename(tok)
        if verb == "git":
            rest = iter(seg.split()[1:])
            sub = None
            for t in rest:
                if t in ("-C", "-c"):
                    next(rest, None)
                    continue
                if t.startswith("-"):
                    continue
                sub = t
                break
            if sub in GIT_READ_SUBCMDS:
                continue
            return False
        if verb not in READ_ONLY_VERBS:
            return False
    return saw


def log_line(home, session, text):
    try:
        with open(os.path.join(home, "hooks", "state", "observed-edits.log"), "a") as fh:
            fh.write(f"{int(time.time())} {session} {text}\n")
    except Exception:
        pass


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
    # AF-124: a pure-read command claims nothing, whatever moved meanwhile.
    cmd = ((d.get("tool_input") or {}).get("command") or "")
    if cmd and is_pure_read_command(cmd):
        log_line(home, session, "n=0 pure-read")
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
        # AF-124's fourth case: "ran and found nothing" must be
        # distinguishable from "never ran" (AMUX-2538) — the quiet path logs.
        log_line(home, session, "n=0 no-moved-mtimes")
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
    log_line(home, session, f"n={len(hits)} {outcome}")


try:
    main()
except Exception:
    pass
sys.exit(0)
