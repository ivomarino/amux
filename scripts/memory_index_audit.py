#!/usr/bin/env python3
"""Audit Claude Code per-project memory dirs: is every memory reachable, and is the state diagnosable?

Written by gtm-videos for amux (board GV-644 / AMUX-2446). Backstop for a class of bug where a
memory file still EXISTS and is still CORRECT but is pointed at by neither MEMORY.md nor
memory-archive.md, so nothing loads it and nothing explains why. Nothing is deleted; it becomes
unreachable, and unreachable is what a reader experiences as deleted.

Two assertions, both with their denominator printed, and note they are scoped DIFFERENTLY:

  A. Every pointer in this dir's index resolves FROM THIS DIR. Scoped locally on purpose: a
     pointer the reader cannot open is unopenable regardless of what exists elsewhere. This is
     the assertion that found the severe bug (a lane whose index carried 138 pointers and could
     open 1, its files sitting one directory up).
  B. Every memory file is referenced by SOME index, anywhere. Scoped globally, and it was
     scoped locally and WRONG when first written. Files in a cwd-derived pool are shared by many
     lanes while each lane's index is written to its own projects/<slug>/memory, so "absent from
     this dir's index" conflated genuinely-unreferenced with referenced-fine-from-another-project.
     On the mixpeek pool that inflated the orphan count 151 -> 267. Files indexed only elsewhere
     are reported as a note, not a violation: paired with DANGLING in that other dir, they are the
     fingerprint of the split-index bug.
  B also catches the inverse — a file in BOTH index and archive, simultaneously live and retired,
  which a presence-only check cannot see.

Without B, "absent from the index" is ambiguous between "retired on purpose" and "silently lost",
and a state you cannot interrogate is one nobody fixes.

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


def all_claims(dirs):
    """filename -> {projects whose index/archive points at it}.

    Needed because assertion B was WRONG when first written (found 2026-08-06 while chasing
    AMUX-2446, after amux surfaced the shared-pool layout). It asked "is this file in THIS
    dir's index?", which assumes a directory's files belong to that directory's index. They
    do not: many lanes share one cwd-derived pool for FILES while each lane's INDEX is written
    to its own projects/<slug>/memory. So "absent from this index" conflated two states —
    genuinely unreferenced, and referenced perfectly well from another project. On the mixpeek
    pool that inflated the orphan count from 151 to 267. Scope the question to every index, and
    report indexed-elsewhere separately, because that is the fingerprint of the cause-2 bug
    (files here, index over there, pointers dangling from where the index lives).
    """
    claims = {}
    for d in dirs:
        proj = os.path.basename(os.path.dirname(d))
        for t in set(PTR.findall(read(os.path.join(d, INDEX)))) | set(PTR.findall(read(os.path.join(d, ARCHIVE)))):
            claims.setdefault(os.path.basename(t), set()).add(proj)
    return claims


def audit(d, claims=None):
    idx, arch = read(os.path.join(d, INDEX)), read(os.path.join(d, ARCHIVE))
    if not idx:
        print(f"{d}\n  no {INDEX} — not a memory dir, skipping")
        return None
    files = sorted(f for f in os.listdir(d) if f.endswith(".md") and is_memory(d, f))
    proj = os.path.basename(os.path.dirname(d))

    # A: pointers resolve FROM WHERE THE INDEX LIVES. Correctly scoped to this dir — a pointer
    # the reader cannot open is unopenable no matter what exists elsewhere. This is the
    # assertion that found the severe bug; keep it local.
    ptrs = PTR.findall(idx)
    # A pointer that does not resolve LOCALLY but does resolve at a path the index
    # itself publishes is not the bug this assertion exists to catch. amux now
    # writes a resolution block at generation time (AMUX-2446):
    #     > **Where these memories live.** ...
    #     >   - `/abs/path/to/memory/` (137 entries)
    # so the reader is told where to open them. Counting those as DANGLING would
    # leave the detector permanently red on a directory that is navigable, and a
    # check that stays red after the fix gets switched off — the same way an
    # unexplained failure class in a CI audit gets waved through.
    #
    # Reported as HINTED, separately from both clean and broken, because the
    # underlying split is still real and still worth seeing: it is a working
    # workaround, not an absence of the condition.
    hinted_dirs = re.findall(r"^>\s+-\s+`([^`]+)`", idx, re.M)
    unresolved = [p for p in ptrs if not os.path.exists(os.path.join(d, p))]
    hinted = [p for p in unresolved
              if any(os.path.exists(os.path.join(h, p)) for h in hinted_dirs)]
    dangling = [p for p in unresolved if p not in hinted]

    # B: is each file referenced anywhere at all? Match on the pointer target, never on a
    # substring of the file — a filename can appear in prose and would read as indexed.
    idx_targets, arch_targets = set(ptrs), set(PTR.findall(arch))
    both = [f for f in files if f in idx_targets and f in arch_targets]
    unref = [f for f in files if f not in idx_targets and f not in arch_targets]
    if claims is None:
        claims = {}
    elsewhere = {f: sorted(claims.get(f, set()) - {proj}) for f in unref}
    orphaned = [f for f in unref if not elsewhere[f]]
    remote = [f for f in unref if elsewhere[f]]

    bad = len(dangling) + len(both) + len(orphaned)
    print(f"{d}")
    print(f"  A. index pointers resolving here:   {len(ptrs) - len(unresolved)}/{len(ptrs)}"
          + (f"  (+{len(hinted)} openable via the index's own resolution block)" if hinted else ""))
    print(f"  B. files referenced by some index:  {len(files) - len(orphaned)}/{len(files)}")
    for f in dangling:
        print(f"     DANGLING  {f}  (this index points at a file it cannot open)")
    for f in both:
        print(f"     IN BOTH   {f}  (simultaneously live and retired)")
    for f in orphaned:
        print(f"     ORPHANED  {f}  (no index anywhere references it)")
    if remote:
        print(f"     note: {len(remote)} file(s) here are indexed only by other project(s), "
              f"e.g. {remote[0]} <- {', '.join(elsewhere[remote[0]][:2])}")
        print(f"           not counted as violations; combined with DANGLING there, that pair is "
              f"the shared-pool/split-index signature")
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
        if audit(t, all_claims([t])) != 0:
            print("  FAIL: cried wolf on a clean directory"); ok = False

        # seeded: c.md exists but nothing points at it -> must fire
        open(os.path.join(t, "c.md"), "w").write("---\nname: c\n---\nbody\n")
        print("[self-test] seeded orphan, expect >=1:")
        if (audit(t, all_claims([t])) or 0) < 1:
            print("  FAIL: missed a seeded orphan — the detector is inert"); ok = False

        # seeded: b.md in BOTH -> must fire
        open(os.path.join(t, INDEX), "a").write("- [B](b.md) — hook\n")
        print("[self-test] seeded in-both, expect >=2:")
        if (audit(t, all_claims([t])) or 0) < 2:
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
    # Assertion B is scoped to EVERY index, so it always needs the global claim map, even when
    # auditing one dir with --dir. Building it from only the audited dirs would reintroduce the
    # exact mis-scoping this map exists to fix.
    claims = all_claims(sorted(glob.glob(os.path.expanduser("~/.claude/projects/*/memory"))))
    total, audited = 0, 0
    for d in dirs:
        r = audit(d, claims)
        if r is not None:
            total += r; audited += 1
    print(f"\n{audited} memory dir(s) audited, {total} violation(s) total")
    sys.exit(1 if total else 0)


if __name__ == "__main__":
    main()
