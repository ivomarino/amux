#!/usr/bin/env python3
"""
amux cloud workspace seeder + verifier.

Provisions a trial workspace and populates it from a declarative PLAN: context
docs, sessions, amux schedules, board items. Optionally runs each session once
and verifies its output, so a prospect environment can be built and proven
before anyone is invited into it.

Two intended modes, both driven by the same plan format:

  automatic      an agent researches a company domain, emits a plan JSON, and
                 this applies it. Research is agentic; applying is deterministic.
                 See --emit-template for the shape to fill in.

  semi-automatic a human describes the sessions and supplies files; the plan is
                 hand-authored (or edited from a template) and applied here.

Usage:
  python3 cloud/seed.py --plan plans/capital-express.json            # provision + seed
  python3 cloud/seed.py --plan p.json --org org_abc123               # seed existing org
  python3 cloud/seed.py --plan p.json --run --verify                 # + run once + check
  python3 cloud/seed.py --emit-template > plans/new.json
  python3 cloud/seed.py --plan p.json --teardown                     # remove the org

Env:
  CLERK_SECRET_KEY   (only needed for --teardown of Clerk users)
  COOKIE_SECRET      gateway HMAC secret; doubles as the X-E2E-Secret admin tier
  E2E_GATEWAY        default https://cloud.amux.io
  ADMIN_USER_ID      Clerk user id to act as (defaults to the amux owner)
"""
import argparse, hashlib, hmac, json, os, ssl, sys, time, urllib.error, urllib.parse, urllib.request, uuid

GATEWAY = os.environ.get("E2E_GATEWAY", "https://cloud.amux.io")
COOKIE_SECRET = os.environ.get("COOKIE_SECRET", "")
ADMIN_USER_ID = os.environ.get("ADMIN_USER_ID", "user_3AP4n5hreSZdTsJbhIt22Xv6LDh")

_ctx = ssl.create_default_context()
_ctx.check_hostname = False
_ctx.verify_mode = ssl.CERT_NONE

PASS = FAIL = 0
WARN = []


def ok(m):
    global PASS
    PASS += 1
    print(f"  \033[32m✓\033[0m {m}", flush=True)


def bad(m):
    global FAIL
    FAIL += 1
    print(f"  \033[31m✗\033[0m {m}", flush=True)


def warn(m):
    WARN.append(m)
    print(f"  \033[33m⚠\033[0m {m}", flush=True)


def log(m):
    print(f"  {m}", flush=True)


def step(m):
    print(f"\n\033[1m→ {m}\033[0m", flush=True)


# ── gateway calls (god mode via X-E2E-Secret) ─────────────────────────────────

def _cookie(org=None):
    p = f"{ADMIN_USER_ID}|{int(time.time())}"
    sig = hmac.new(COOKIE_SECRET.encode(), p.encode(), hashlib.sha256).hexdigest()
    c = f"amux_session={p}|{sig}"
    return c + (f"; amux_org={org}" if org else "")


def gw(method, path, body=None, org=None, raw=None, ctype="application/json", timeout=90):
    url = f"{GATEWAY}{path}"
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    headers = {"Cookie": _cookie(org), "X-E2E-Secret": COOKIE_SECRET,
               "Accept": "application/json", "Content-Type": ctype,
               "User-Agent": "amux-seed/1.0"}
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        r = urllib.request.urlopen(req, timeout=timeout, context=_ctx)
        return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except urllib.error.URLError as e:
        return 0, str(e)


def gw_json(method, path, **kw):
    code, body = gw(method, path, **kw)
    try:
        return code, json.loads(body)
    except Exception:
        return code, body


def wait_ready(org, max_wait=300):
    """Block until the org's container answers. Cold boot is ~30s."""
    start = time.time()
    while time.time() - start < max_wait:
        code, _ = gw("GET", "/api/sessions", org=org)
        if code == 200:
            return int(time.time() - start)
        if code == 402:
            return -1
        time.sleep(6)
    return None


# ── plan steps ────────────────────────────────────────────────────────────────

def provision(plan):
    o = plan.get("org", {})
    body = {"email": o["email"], "trial_days": o.get("trial_days", 7),
            "budget_usd": o.get("budget_usd", 5),
            "name": o.get("name") or f"{o['email']} (trial)",
            "notify": bool(o.get("notify", False))}
    code, d = gw_json("POST", "/api/gateway/admin/provision", body=body)
    if code != 201:
        bad(f"provision failed: {code} {str(d)[:200]}")
        return None
    ok(f"provisioned {d['org_id']} for {d['email']} "
       f"(trial {o.get('trial_days',7)}d, ${d['budget_usd']}, auth={d.get('claude_auth')})")
    if d.get("claude_auth") == "none":
        warn("workspace has NO Claude auth — set ANTHROPIC_API_KEY on the gateway "
             "or supply api_key in the plan, or sessions will not produce output")
    if not d.get("clerk_invitation_sent") and body["notify"]:
        warn(f"invite email not sent: {d.get('clerk_detail','')[:120]}")
    return d


def upload_docs(org, docs):
    """Write context files into the workspace before anyone signs in."""
    for doc in docs:
        path = doc["path"]
        directory, _, fname = path.rpartition("/")
        directory = directory or "/root"
        content = doc.get("content", "")
        if "content_file" in doc:
            content = open(doc["content_file"]).read()
        gw("POST", "/api/fs/mkdir", body={"path": directory}, org=org)
        b = "----amux" + uuid.uuid4().hex
        payload = b"".join([
            f"--{b}\r\nContent-Disposition: form-data; name=\"dir\"\r\n\r\n{directory}\r\n".encode(),
            (f"--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\n"
             f"Content-Type: text/plain\r\n\r\n").encode() + content.encode() + b"\r\n",
            f"--{b}--\r\n".encode(),
        ])
        code, resp = gw("POST", "/api/fs/upload", org=org, raw=payload,
                        ctype=f"multipart/form-data; boundary={b}")
        if code == 200:
            ok(f"doc {path} ({len(content)} bytes)")
        else:
            bad(f"doc {path} failed: {code} {resp[:140]}")


def create_sessions(org, sessions):
    made = []
    for s in sessions:
        body = {"name": s["name"], "desc": s.get("desc", ""),
                "provider": s.get("provider", "claude")}
        if s.get("dir"):
            body["dir"] = s["dir"]
        code, resp = gw("POST", "/api/sessions", body=body, org=org, timeout=120)
        if code in (200, 201):
            ok(f"session '{s['name']}' ({body['provider']})")
            made.append(s)
        elif code == 409:
            warn(f"session '{s['name']}' already existed")
            made.append(s)
        else:
            bad(f"session '{s['name']}' failed: {code} {resp[:140]}")
    return made


def create_schedules(org, sessions):
    """Attach amux schedules. Kept disabled until explicitly enabled so a seeded
    demo cannot start burning budget on its own."""
    out = []
    for s in sessions:
        for sch in s.get("schedules", []):
            body = {"title": sch["title"], "session": s["name"],
                    "command": sch["command"],
                    "schedule_expr": sch.get("expr", "daily at 9am"),
                    "enabled": 1 if sch.get("enabled") else 0}
            code, d = gw_json("POST", "/api/schedules", body=body, org=org)
            if code in (200, 201) and isinstance(d, dict) and d.get("id"):
                out.append((d["id"], s["name"], sch["title"]))
                ok(f"schedule {d['id']} '{sch['title']}' → {s['name']} "
                   f"[{body['schedule_expr']}]{'' if body['enabled'] else ' (disabled)'}")
            else:
                bad(f"schedule '{sch['title']}' failed: {code} {str(d)[:140]}")
    return out


def create_board(org, items):
    for it in items:
        code, _ = gw("POST", "/api/board",
                     body={"title": it["title"], "desc": it.get("desc", ""),
                           "status": it.get("status", "todo")}, org=org)
        (ok if code in (200, 201) else bad)(f"board item '{it['title'][:48]}'")


def run_once(org, sessions, schedules):
    """Kick each session once: prefer its schedule (proves the schedule path),
    else send the seed prompt directly."""
    fired = []
    sched_by_session = {}
    for sid, sname, title in schedules:
        sched_by_session.setdefault(sname, (sid, title))
    for s in sessions:
        name = s["name"]
        if name in sched_by_session:
            sid, title = sched_by_session[name]
            code, _ = gw("POST", f"/api/schedules/{sid}/run", org=org, timeout=120)
            (ok if code in (200, 202) else bad)(f"ran schedule '{title}' for {name} ({code})")
            fired.append(name)
        elif s.get("prompt"):
            code, _ = gw("POST", f"/api/sessions/{name}/send",
                         body={"text": s["prompt"], "record_history": True},
                         org=org, timeout=120)
            (ok if code in (200, 202) else bad)(f"sent seed prompt to {name} ({code})")
            fired.append(name)
        else:
            warn(f"{name}: nothing to run (no schedule, no prompt)")
    return fired


def verify(org, sessions, settle=90):
    """Check each session actually produced output, and that any declared
    expectations appear in it. Reads `history`, not `output` — a full-screen
    prompt clears the viewport and would read as 'nothing happened'."""
    log(f"letting agents work for {settle}s…")
    time.sleep(settle)
    for s in sessions:
        name = s["name"]
        code, d = gw_json("GET", f"/api/sessions/{name}/peek?lines=400", org=org, timeout=120)
        if code != 200 or not isinstance(d, dict):
            bad(f"{name}: peek failed ({code})")
            continue
        text = d.get("history") or d.get("output") or ""
        if len(text.strip()) < 40:
            bad(f"{name}: no meaningful output ({len(text)} chars) — "
                f"likely no Claude auth in this workspace")
            continue
        ok(f"{name}: produced {len(text)} chars of output")
        for needle in s.get("expect", []):
            if needle.lower() in text.lower():
                ok(f"{name}: found expected '{needle}'")
            else:
                bad(f"{name}: expected '{needle}' not in output")


def spend_report(org):
    code, d = gw_json("GET", "/api/gateway/admin/orgs")
    if code != 200 or not isinstance(d, dict):
        return
    for o in d.get("orgs", []):
        if o["id"] == org:
            log(f"spend ${o.get('spend_usd') or 0:.4f} of ${o.get('budget_usd')} budget; "
                f"trial ends {time.strftime('%Y-%m-%d', time.gmtime(o['trial_ends_at']))}")
            return


TEMPLATE = {
    "org": {"email": "prospect@example.com", "name": "Example Co (trial)",
            "trial_days": 7, "budget_usd": 5, "notify": False},
    "docs": [
        {"path": "/root/demo/context.md",
         "content": "# Context\n\nDomain facts the agents should rely on.\n"}
    ],
    "board": [{"title": "Demo: review agent output", "status": "todo"}],
    "sessions": [
        {"name": "use-case-1", "provider": "claude", "dir": "/root/demo",
         "desc": "One-line description of the use case",
         "prompt": "Read /root/demo/context.md and produce the first deliverable.",
         "expect": ["a string that must appear in the output"],
         "schedules": [{"title": "Daily run", "expr": "daily at 9am",
                        "command": "Re-run the analysis and post a summary.",
                        "enabled": False}]}
    ],
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--plan")
    ap.add_argument("--org", help="seed an existing org instead of provisioning")
    ap.add_argument("--run", action="store_true", help="run each session/schedule once")
    ap.add_argument("--verify", action="store_true", help="check outputs after running")
    ap.add_argument("--settle", type=int, default=90, help="seconds to let agents work")
    ap.add_argument("--teardown", action="store_true", help="delete the org afterwards")
    ap.add_argument("--emit-template", action="store_true")
    a = ap.parse_args()

    if a.emit_template:
        print(json.dumps(TEMPLATE, indent=2))
        return 0
    if not a.plan:
        ap.error("--plan is required (or --emit-template)")
    if not COOKIE_SECRET:
        print("FATAL: COOKIE_SECRET not set")
        return 1

    plan = json.load(open(a.plan))
    print(f"═══ amux workspace seed: {os.path.basename(a.plan)} ═══")
    print(f"    gateway: {GATEWAY}")

    org = a.org
    if not org:
        step("Provision workspace")
        d = provision(plan)
        if not d:
            return 1
        org = d["org_id"]
        print(f"\n    org_id:     {org}")
        print(f"    invite_url: {d['invite_url']}")

    step("Boot container")
    el = wait_ready(org)
    if el is None:
        bad("container never became ready")
        return 1
    if el == -1:
        bad("workspace is gated (budget or trial expired)")
        return 1
    ok(f"container ready in {el}s")

    if plan.get("docs"):
        step("Upload context docs")
        upload_docs(org, plan["docs"])
    if plan.get("board"):
        step("Seed board")
        create_board(org, plan["board"])

    step("Create sessions")
    sessions = create_sessions(org, plan.get("sessions", []))

    step("Create schedules")
    schedules = create_schedules(org, sessions)

    if a.run:
        step("Run once")
        run_once(org, sessions, schedules)
    if a.verify:
        step("Verify outputs")
        verify(org, sessions, settle=a.settle)

    step("Spend / trial state")
    spend_report(org)

    if a.teardown:
        step("Teardown")
        code, d = gw_json("DELETE", f"/api/gateway/admin/cleanup/{org}", timeout=120)
        (ok if code == 200 else bad)(f"cleanup {org} ({code})")

    print("\n" + "═" * 52)
    print(f"  PASS: {PASS}  FAIL: {FAIL}  WARN: {len(WARN)}")
    for w in WARN:
        print(f"  ⚠ {w}")
    print(f"  ORG: {org}")
    print(f"  RESULT: {'PASSED' if FAIL == 0 else 'FAILED'}")
    return 0 if FAIL == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
