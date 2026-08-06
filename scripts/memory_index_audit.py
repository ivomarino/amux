#!/usr/bin/env python3
"""Audit Claude Code per-project memory dirs: is every memory reachable, and is the state diagnosable?

Written by gtm-videos for amux (board GV-644 / AMUX-2446). Backstop for a class of bug where a
memory file still EXISTS and is still CORRECT but is pointed at by neither MEMORY.md nor
memory-archive.md, so nothing loads it and nothing explains why. Nothing is deleted; it becomes
unreachable, and unreachable is what a reader experiences as deleted.

Two assertions, both with their denominator printed:

  A. Every index pointer resolves to a file.
  B. Every memory file appears in EXACTLY ONE of MEMORY.md or memory-archive.md.

B is the one that matters. Without it, "absent from the index" is ambiguous between "retired on
purpose" and "silently lost", and a state you cannot interrogate is one nobody fixes. It also
catches the inverse (a file listed in BOTH, simultaneously live and retired) which a
presence-only check cannot see.

Why the denominators are printed and not just the failures: "0 violations" and "scoped to nothing"
produce identical output. `checked 472, 0 bad` self-refutes at 472=0 where a bare `0 bad` does not.

INFRASTRUCTURE EXCLUSION is load-bearing, not politeness. amux's own memory dir contains
MEMORY.preamble-backup.md and amux.md, the latter opening "CLAUDE-TAG-MEM-MARKER: shared notes for
amux-tagged lanes". Neither is a memory. Without the exclusion this reports a defect on a clean
directory, which is how a good check gets switched off by the second person who runs it.

Usage:
  memory_index_audit.py                 # audit every project memory dir
  memory_index_audit.py --dir <path>    # audit one
  memory_index_audit.py --self-test     # prove the detector can both fire AND stay quiet
Exit 0 clean, 1 violations found, 2 self-test failed.
"""
import argparse
import glob
import os
import re
import sys
import tempfile

PTR = re.compile(r"^- \[[^\]]+\]\(([^)]+)\)", re.M)
INDEX, ARCHIVE = "MEMORY.md", "memory-archive.md"
SKIP_NAMES = {INDEX, ARCHIVE, "MEMORY.preamble-backup.md"}
SKIP_HEADER = "CLAUDE-TAG-MEM-MARKER"


def read(p):
    try:
        with open(p, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def is_memory(d, f):
    """A .md in the dir that is an actual memory, not index/archive/infrastructure."""
    if f in SKIP_NAMES:
        return False
    # header sniff, not whole-file: the marker is declared at the top by convention
    return SKIP_HEADER not in read(os.path.join(d, f))[:400]


def audit(d):
    idx, arch = read(os.path.join(d, INDEX)), read(os.path.join(d, ARCHIVE))
    if not idx:
        print(f"{d}\n  no {INDEX} — not a memory dir, skipping")
        return None
    files = sorted(f for f in os.listdir(d) if f.endswith(".md") and is_memory(d, f))

    # A: pointers resolve. The archive is a legitimate pointer target from the index.
    ptrs = PTR.findall(idx)
    dangling = [p for p in ptrs if not os.path.exists(os.path.join(d, p))]

    # B: exactly one home. Match on the pointer target, never on a substring of the whole
    # file — a filename can appear inside prose, which would read as indexed when it is not.
    idx_targets = set(ptrs)
    arch_targets = set(PTR.findall(arch))
    both = [f for f in files if f in idx_targets and f in arch_targets]
    neither = [f for f in files if f not in idx_targets and f not in arch_targets]

    bad = len(dangling) + len(both) + len(neither)
    print(f"{d}")
    print(f"  A. index pointers resolving:        {len(ptrs) - len(dangling)}/{len(ptrs)}")
    print(f"  B. memories with exactly one home:  {len(files) - len(both) - len(neither)}/{len(files)}")
    for f in dangling:
        print(f"     DANGLING  {f}  (index points at a file that does not exist)")
    for f in both:
        print(f"     IN BOTH   {f}  (simultaneously live and retired)")
    for f in neither:
        print(f"     ORPHANED  {f}  (unreachable: nothing points at it)")
    print(f"  -> {'clean' if not bad else str(bad) + ' violation(s)'}")
    return bad


def self_test():
    """Both directions. A detector that only ever passes is indistinguishable from one that
    cannot fail, so prove it goes RED on a seeded violation and GREEN on a clean dir."""
    ok = True
    with tempfile.TemporaryDirectory() as t:
        # clean: one memory, indexed; one retired, archived; plus infra that must be ignored
        open(os.path.join(t, "a.md"), "w").write("---\nname: a\n---\nbody\n")
        open(os.path.join(t, "b.md"), "w").write("---\nname: b\n---\nbody\n")
        open(os.path.join(t, "amux.md"), "w").write(f"{SKIP_HEADER}: not a memory\n")
        open(os.path.join(t, INDEX), "w").write(f"- [A](a.md) — hook\n- [Archived]({ARCHIVE}) — hook\n")
        open(os.path.join(t, ARCHIVE), "w").write("- [B](b.md) — hook\n")
        print("[self-test] clean dir, expect 0:")
        if audit(t) != 0:
            print("  FAIL: cried wolf on a clean directory"); ok = False

        # seeded: c.md exists but nothing points at it -> must fire
        open(os.path.join(t, "c.md"), "w").write("---\nname: c\n---\nbody\n")
        print("[self-test] seeded orphan, expect >=1:")
        if (audit(t) or 0) < 1:
            print("  FAIL: missed a seeded orphan — the detector is inert"); ok = False

        # seeded: b.md in BOTH -> must fire
        open(os.path.join(t, INDEX), "a").write("- [B](b.md) — hook\n")
        print("[self-test] seeded in-both, expect >=2:")
        if (audit(t) or 0) < 2:
            print("  FAIL: missed a file listed live AND retired"); ok = False
    print(f"[self-test] {'PASS' if ok else 'FAIL'}")
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", action="append", help="memory dir (repeatable); default = all projects")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        sys.exit(0 if self_test() else 2)
    dirs = a.dir or sorted(glob.glob(os.path.expanduser("~/.claude/projects/*/memory")))
    if not dirs:
        print("no memory dirs found"); sys.exit(0)
    total, audited = 0, 0
    for d in dirs:
        r = audit(d)
        if r is not None:
            total += r; audited += 1
    print(f"\n{audited} memory dir(s) audited, {total} violation(s) total")
    sys.exit(1 if total else 0)


if __name__ == "__main__":
    main()
