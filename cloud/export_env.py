#!/usr/bin/env python3
"""Export a LIVE cloud env to an EnvSpec YAML (amux env_config / AMUX-2977 shape).

The other half of Ethan's "save the env as YAML, rapidly redeploy for similar
verticals": seed.py + /api/env/apply WRITE an env; this READS a running one back
into the SAME schema, so you can capture a good env — produced docs and all — as
a reusable vertical template instead of hand-authoring it.

Output shape is identical field-for-field to what /api/env/apply consumes
(EnvSpec: groups[], workers[], schedules[], columns[], files[]), so an exported
YAML round-trips: export org A -> edit the org specifics -> apply to org B.

Usage:
    COOKIE_SECRET=... ADMIN_USER_ID=... \
      python3 cloud/export_env.py --org org_8e89a846b6f5be7d > cloud/verticals/foo.yaml
    # add --files-dir /root/rothco/docs to capture the seeded docs' content

Env: same as seed.py (COOKIE_SECRET, ADMIN_USER_ID, E2E_GATEWAY). Reuses seed.py's
gateway client so there is ONE authenticated path, not a second one.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import seed  # noqa: E402 — reuse gw()/gw_json()/_cookie() rather than re-implement auth

try:
    import yaml
except ImportError:
    sys.exit("pyyaml required: pip3 install pyyaml")


def _list(v, *keys):
    """A gateway list response may be a bare array or {key: [...]}. Normalize."""
    if isinstance(v, list):
        return v
    if isinstance(v, dict):
        for k in keys:
            if isinstance(v.get(k), list):
                return v[k]
    return []


def export(org, files_dir=None):
    # ---- workers (WorkerSpec: name, dir, groups[<-tags>, desc, model, provider]) ----
    _, sess_raw = seed.gw_json("GET", "/api/sessions", org=org)
    sessions = _list(sess_raw, "sessions")
    workers = []
    all_groups = {}
    for s in sessions:
        name = s.get("name")
        if not name or name == "hello-world":  # scaffold is never part of a template
            continue
        tags = s.get("tags") or s.get("tag") or []
        if isinstance(tags, str):
            tags = [t.strip() for t in tags.split(",") if t.strip()]
        # The CONFIGURED model, not the runtime one. `active_model` is what the
        # last turn ran on and can be a status marker like `<synthetic>` (a capped
        # worker that never really ran) — writing that into a template would
        # produce an un-applyable worker. Prefer the configured `model`, and any
        # non-family value (`<synthetic>`, empty) falls back to the sonnet default.
        model = s.get("model") or s.get("active_model") or ""
        model = str(model)
        if "sonnet" in model:
            model = "sonnet"
        elif "haiku" in model:
            model = "haiku"
        elif "opus" in model:
            model = "opus"
        else:
            model = "sonnet"  # <synthetic>, empty, or unknown -> the demo default
        workers.append({
            "name": name,
            "dir": s.get("dir") or s.get("cwd") or "",
            "groups": list(tags),
            "desc": s.get("desc") or "",
            "model": model,
            "provider": s.get("provider") or "claude",
        })
        for t in tags:
            all_groups.setdefault(t, {"name": t, "department": "", "goal": ""})

    # ---- schedules (worker, title, expr, enabled, command) ----
    _, sched_raw = seed.gw_json("GET", "/api/schedules", org=org)
    schedules = []
    for sc in _list(sched_raw, "schedules"):
        schedules.append({
            "worker": sc.get("session") or sc.get("worker") or "",
            "title": sc.get("title") or "",
            "expr": sc.get("schedule_expr") or sc.get("expr") or "",
            "enabled": bool(sc.get("enabled", 0)),
            "command": sc.get("command") or "",
        })

    # ---- board columns ----
    _, board_raw = seed.gw_json("GET", "/api/board", org=org)
    cols = []
    seen = set()
    for it in _list(board_raw, "items"):
        c = it.get("column") or it.get("col")
        if c and c not in seen:
            seen.add(c)
            cols.append(c)

    # ---- files (docs): {path, content}. Read content off the container path. ----
    files = []
    if files_dir:
        _, ls = seed.gw_json("GET", f"/api/files?path={files_dir}", org=org)
        for entry in _list(ls, "files", "entries"):
            p = entry.get("path") or (files_dir.rstrip("/") + "/" + entry.get("name", ""))
            if entry.get("type") == "dir" or entry.get("is_dir"):
                continue
            # /api/file returns {"content": "<raw>", ...}, NOT the bare bytes.
            # Storing the whole JSON envelope would write literal `{"content":...}`
            # into the redeployed doc — extract the field.
            code, resp = seed.gw_json("GET", f"/api/file?path={p}", org=org)
            if code == 200 and isinstance(resp, dict) and "content" in resp:
                files.append({"path": p, "content": resp["content"]})
            elif code == 200 and isinstance(resp, str):
                files.append({"path": p, "content": resp})

    spec = {
        "_comment": f"Exported from live env {org} by export_env.py — EnvSpec (AMUX-2977). "
                    f"Edit org specifics and POST to /api/env/apply to redeploy for a similar vertical.",
        "groups": list(all_groups.values()),
        "workers": workers,
        "columns": cols,
        "schedules": schedules,
        "files": files,
    }
    return spec


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--org", required=True, help="org id to export (amux-user-<org>)")
    ap.add_argument("--files-dir", help="container dir to capture as files[] (e.g. /root/rothco/docs)")
    a = ap.parse_args()
    if not seed.COOKIE_SECRET:
        sys.exit("COOKIE_SECRET is required (same as seed.py)")
    spec = export(a.org, a.files_dir)
    sys.stdout.write(yaml.safe_dump(spec, sort_keys=False, width=100, allow_unicode=True))
    sys.stderr.write(
        f"# exported {len(spec['workers'])} workers, {len(spec['groups'])} groups, "
        f"{len(spec['columns'])} columns, {len(spec['schedules'])} schedules, "
        f"{len(spec['files'])} files\n")


if __name__ == "__main__":
    main()
