#!/usr/bin/env python3
"""Daily cloud.amux.io health check + deterministic AUTOFIX.

Checks cloud is online and every customer environment works, AUTO-REPAIRS the failure
modes that are deterministic and safe, and ESCALATES everything else. Every action
leaves a trace (stdout + a host JSONL ledger + a board escalation when it cannot fix).

WHY THIS EXISTS (the 2026-08-16..18 outage). cloud.amux.io was 502 for days. Two root
causes, both fixed BY HAND: (1) the host root disk was 100% full; (2) a failed deploy
ran out of disk mid-write and TRUNCATED /etc/amux/gateway.env (missing
CLERK_PUBLISHABLE_KEY), so gateway.py crash-looped (KeyError) and nginx returned 502.
Nothing detected either automatically. This script encodes the exact hand-repairs so the
next occurrence self-heals within a day, or escalates loudly, instead of staying dark.

SAFE deterministic repairs (each leaves a trace):
  - gateway crash-looping AND gateway.env missing critical keys -> restore gateway.env
    from the newest good backup (merged with current), restart gateway.
  - gateway down AND disk full of LOGS -> truncate container json-logs + journald, restart.
  - gateway crash-looping for another reason -> restart gateway once.
ESCALATE, never auto-act (ethos rule 8 — customer data is the owner's):
  - disk full of DATA (volumes), not logs -> cannot delete; alert + board.
  - any repair that did not restore service -> alert + board.

USAGE
  python3 cloud/cloud_autofix.py           # check + autofix + report
  python3 cloud/cloud_autofix.py --no-fix  # check + report only (dry, no repairs)
  python3 cloud/cloud_autofix.py --json     # machine-readable summary
Exit 0 = healthy (or repaired); 1 = still broken / escalated.
"""
import json
import os
import subprocess
import sys
import time

CLOUD = "https://cloud.amux.io"
HOST = os.environ.get("AMUX_CLOUD_HOST", "34.121.177.76")
SSH_KEY = os.path.expanduser("~/.ssh/amux_cloud")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRITICAL_ENV_KEYS = ["CLERK_PUBLISHABLE_KEY", "CLERK_SECRET_KEY", "COOKIE_SECRET"]

TRACE = []


def trace(action, detail, ok=None):
    row = {"ts": int(time.time()), "action": action, "detail": detail, "ok": ok}
    TRACE.append(row)
    print("  [autofix] %s: %s%s" % (action, detail, "" if ok is None else (" -> %s" % ("ok" if ok else "FAILED"))),
          file=sys.stderr)


def ssh(script, timeout=90):
    """Run a python3 script on the cloud host via stdin. Returns stdout or ''."""
    try:
        r = subprocess.run(
            ["ssh", "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=8",
             "-i", SSH_KEY, "root@%s" % HOST, "python3 -"],
            input=script, capture_output=True, text=True, timeout=timeout)
        return r.stdout.strip()
    except Exception as e:
        return "SSH_ERROR: %s" % str(e)[:80]


def probe_cloud():
    """HTTP status at CLOUD/. 302 = healthy (auth redirect); 5xx/000 = down."""
    try:
        r = subprocess.run(["curl", "-sk", "-o", "/dev/null", "-w", "%{http_code}",
                            "--max-time", "12", CLOUD + "/"], capture_output=True, text=True, timeout=20)
        return int(r.stdout.strip() or 0)
    except Exception:
        return 0


# Host-side diagnostic: returns one JSON line the local logic acts on.
_DIAG = r'''
import json, os, subprocess
def run(*a):
    try: return subprocess.run(a, capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception: return ""
out = {}
out["gateway_active"] = run("systemctl","is-active","amux-gateway")
out["gateway_nrestarts"] = run("systemctl","show","amux-gateway","-p","NRestarts","--value")
df = run("df","-B1M","--output=pcent,avail","/").splitlines()
if len(df) >= 2:
    pcent, avail = df[-1].split()
    out["disk_pct"] = int(pcent.strip().rstrip("%")); out["disk_free_mb"] = int(avail)
env = "/etc/amux/gateway.env"
present = set()
try:
    for line in open(env):
        if "=" in line and not line.startswith("#"): present.add(line.split("=",1)[0])
except Exception: pass
out["env_missing"] = [k for k in ["CLERK_PUBLISHABLE_KEY","CLERK_SECRET_KEY","COOKIE_SECRET","CONTAINER_SCHEME"] if k not in present]
# logs vs data: container json-logs + journald size (MB) that we CAN safely reclaim
logs = 0
try:
    import glob
    for f in glob.glob("/var/lib/docker/containers/*/*-json.log"):
        try: logs += os.path.getsize(f)
        except Exception: pass
except Exception: pass
out["reclaimable_log_mb"] = logs // (1024*1024)
# newest good gateway.env backup that carries the critical keys
best = None
try:
    import glob
    for b in sorted(glob.glob("/etc/amux/gateway.env.*"), key=lambda p: os.path.getmtime(p), reverse=True):
        ks = set()
        for line in open(b):
            if "=" in line: ks.add(line.split("=",1)[0])
        if {"CLERK_PUBLISHABLE_KEY","CLERK_SECRET_KEY","COOKIE_SECRET"} <= ks:
            best = b; break
except Exception: pass
out["env_backup"] = best
print(json.dumps(out))
'''


def diagnose():
    raw = ssh(_DIAG)
    try:
        return json.loads(raw.splitlines()[-1])
    except Exception:
        return {"error": raw[:120]}


def fix_gateway_env(backup):
    """Restore gateway.env from a known-good backup merged with current (atomic)."""
    script = r'''
import os, tempfile
BAK = %r
def load(p):
    d = {}
    try:
        for line in open(p):
            line = line.rstrip("\n")
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1); d[k] = v
    except FileNotFoundError: pass
    return d
bak = load(BAK); cur = load("/etc/amux/gateway.env")
if not {"CLERK_PUBLISHABLE_KEY","CLERK_SECRET_KEY","COOKIE_SECRET"} <= set(bak):
    print("ABORT: backup missing critical keys"); raise SystemExit(1)
merged = dict(bak); merged.update(cur); merged.setdefault("CONTAINER_SCHEME", "https")
fd, tmp = tempfile.mkstemp(dir="/etc/amux")
with os.fdopen(fd, "w") as f:
    for k, v in merged.items(): f.write("%%s=%%s\n" %% (k, v))
os.chmod(tmp, 0o600); os.replace(tmp, "/etc/amux/gateway.env")
print("restored %%d keys" %% len(merged))
''' % backup
    out = ssh(script)
    ok = "restored" in out
    trace("restore_gateway_env", "from %s: %s" % (os.path.basename(backup or "?"), out[:60]), ok)
    return ok


def fix_logs():
    out = ssh(r'''
import subprocess, glob, os
n = 0
for f in glob.glob("/var/lib/docker/containers/*/*-json.log"):
    try:
        if os.path.getsize(f) > 20*1024*1024:
            open(f, "w").close(); n += 1
    except Exception: pass
subprocess.run(["journalctl", "--vacuum-size=100M"], capture_output=True)
print("truncated %d logs" % n)
''')
    trace("truncate_logs", out[:60], "truncated" in out)
    return "truncated" in out


def restart_gateway():
    out = ssh("import subprocess; subprocess.run(['systemctl','restart','amux-gateway']); "
              "import time; time.sleep(5); "
              "print(subprocess.run(['systemctl','is-active','amux-gateway'],capture_output=True,text=True).stdout.strip())")
    ok = out.strip().endswith("active")
    trace("restart_gateway", "state=%s" % out.strip()[:20], ok)
    return ok


def escalate(summary, detail):
    trace("escalate", summary, None)
    # board card (attributed) so the fleet sees it even if paging is down
    try:
        base = subprocess.run(["amux", "url"], capture_output=True, text=True, timeout=10).stdout.strip()
        subprocess.run(["curl", "-sk", "-X", "POST", "-H", "Content-Type: application/json",
                        "-H", "X-Amux-Session:%s" % os.environ.get("AMUX_SESSION", "cloud-autofix"),
                        "-d", json.dumps({"title": "cloud-autofix: %s" % summary, "desc": detail,
                                          "status": "needsyou", "session": "amux-cloud"}),
                        "%s/api/board" % base], capture_output=True, text=True, timeout=15)
    except Exception:
        pass
    # fire-alarm (email channel is repaired; push/sms are owner setup)
    try:
        subprocess.run(["amux", "alert", "cloud-autofix could not self-heal: %s. %s" % (summary, detail),
                        "Cloud health autofix escalation"], capture_output=True, text=True, timeout=20)
    except Exception:
        pass


def check_envs():
    """Run the per-environment/persona suite for a green/red matrix (read-only)."""
    try:
        r = subprocess.run([sys.executable, os.path.join(REPO, "cloud/tests/e2e_personas.py"), "--json"],
                           capture_output=True, text=True, timeout=600, cwd=REPO)
        return json.loads(r.stdout.strip().splitlines()[-1]) if r.stdout.strip() else {"error": "no output"}
    except Exception as e:
        return {"error": str(e)[:100]}


def check_orphans():
    """Running amux-user containers with NO gateway.db org/user row. The deploy is
    DIRECTORY-driven (deploy-cloud.yml loops /var/amux/users/*/), so a workspace dir
    left behind by an incomplete deletion gets its container RESURRECTED even after
    the org is gone from the DB — 6 came back on 2026-08-18 (AC-373). Report-only:
    a DB-less container can also be a brief mid-provision race, so surfacing it in
    the trace ledger (which a sweep reads) is the fix, not an unattended delete."""
    # The host has NO sqlite3 CLI — query the gateway DB via the python sqlite3
    # module. A `sqlite3 …` CLI version of this returned '' for every id (command
    # not found), which flags EVERY container as an orphan — the exact ethos-rule-7
    # instrument that reports a confident wrong answer (caught 2026-08-18).
    out = ssh(r'''
import json, subprocess, sqlite3
def run(*a):
    try: return subprocess.run(a, capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception: return ""
names = [n for n in run("docker","ps","--format","{{.Names}}").splitlines() if n.startswith("amux-user-")]
orphans = []
try:
    c = sqlite3.connect("/var/amux/gateway.db")
    for n in names:
        oid = n[len("amux-user-"):]
        row = c.execute("SELECT 1 FROM orgs WHERE id=? UNION SELECT 1 FROM users WHERE id=? LIMIT 1", (oid, oid)).fetchone()
        if not row:
            orphans.append(oid)
    print(json.dumps({"running": len(names), "orphans": orphans}))
except Exception as e:
    print(json.dumps({"running": len(names), "error": str(e)[:80]}))
''', timeout=45)
    try:
        return json.loads(out)
    except Exception:
        return {"error": (out or "")[:100]}


def main():
    no_fix = "--no-fix" in sys.argv
    as_json = "--json" in sys.argv
    result = {"trace": TRACE, "healthy": False}

    status = probe_cloud()
    result["cloud_status"] = status
    trace("probe", "cloud.amux.io -> %d" % status, status in (200, 301, 302, 401, 403))

    if status in (200, 301, 302, 401, 403):
        # Cloud is serving. Verify the environments too.
        result["healthy"] = True
        result["envs"] = check_envs()
        trace("check_envs", "reachable=%s failed=%s" % (result["envs"].get("cloud_reachable"),
                                                        result["envs"].get("failed")), None)
        # Orphaned (DB-less) running containers — a deploy resurrection self-announces here.
        result["orphans"] = check_orphans()
        _orph = result["orphans"].get("orphans") or []
        trace("orphans", "running=%s orphaned=%s%s" % (
            result["orphans"].get("running"), len(_orph),
            (" -> " + ",".join(_orph)) if _orph else ""), not _orph)
    else:
        # Cloud is DOWN. Diagnose and apply deterministic repairs.
        d = diagnose()
        result["diagnosis"] = d
        trace("diagnose", "gw=%s restarts=%s disk=%s%% env_missing=%s log_mb=%s"
              % (d.get("gateway_active"), d.get("gateway_nrestarts"), d.get("disk_pct"),
                 d.get("env_missing"), d.get("reclaimable_log_mb")), None)
        if no_fix:
            trace("no_fix", "dry run — skipping repairs", None)
        else:
            fixed_something = False
            # Repair 1: truncated gateway.env (the incident's real blocker).
            if d.get("env_missing") and d.get("env_backup"):
                fixed_something |= fix_gateway_env(d["env_backup"])
            # Repair 2: disk full of reclaimable LOGS.
            if (d.get("disk_pct", 0) >= 95) and d.get("reclaimable_log_mb", 0) >= 300:
                fixed_something |= fix_logs()
            # Repair 3: bring the gateway up (covers crash-loop + post-repair).
            restart_gateway()
            # Re-probe.
            time.sleep(3)
            status2 = probe_cloud()
            result["cloud_status_after"] = status2
            trace("reprobe", "cloud.amux.io -> %d" % status2, status2 in (301, 302, 200, 401, 403))
            result["healthy"] = status2 in (200, 301, 302, 401, 403)
            # Escalate what could not be fixed.
            if not result["healthy"]:
                if d.get("disk_pct", 0) >= 95 and d.get("reclaimable_log_mb", 0) < 300:
                    escalate("disk full of DATA, cannot auto-delete (ethos rule 8)",
                             "disk %s%%, %sMB free; reclaimable logs only %sMB. Needs a resize or an owner-authorised reap."
                             % (d.get("disk_pct"), d.get("disk_free_mb"), d.get("reclaimable_log_mb")))
                else:
                    escalate("cloud still 502 after deterministic repairs",
                             "diagnosis=%s; repairs did not restore service." % json.dumps(d)[:300])

    # Persist the trace ledger on the host (best-effort — disk may be full).
    ssh("import json; open('/var/log/cloud-autofix.jsonl','a').write(%r+chr(10))"
        % json.dumps({"ts": int(time.time()), "healthy": result["healthy"], "trace": TRACE}), timeout=20)

    if as_json:
        print(json.dumps(result, indent=2))
    else:
        print("\ncloud-autofix: %s (status %s%s)" % (
            "HEALTHY" if result["healthy"] else "UNHEALTHY / ESCALATED",
            result.get("cloud_status"),
            "->%s" % result["cloud_status_after"] if "cloud_status_after" in result else ""))
    sys.exit(0 if result["healthy"] else 1)


if __name__ == "__main__":
    main()
