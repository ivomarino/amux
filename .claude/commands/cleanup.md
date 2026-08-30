---
description: Repo/branch/file hygiene sweep — stray artifacts, doc drift, stale branches, worktrees, upstream sync, backup sprawl. Verifies before purging.
allowed-tools: Bash, Read, Edit, Write
argument-hint: [dry-run|full] (default: full)
---

# /cleanup — amux file cleanup & consolidation routine

A repeatable hygiene sweep for this repo and this box's `~/.amux` state.
Built from a real cleanup session (2026-08-30) that found: a 37MB binary
accidentally committed, a security-sensitive file (`CLAUDE.local.md`)
pushed to a public fork, a live systemd script left untracked, a doc file
independently drifting in **three** places, 6 stale branches, 2 worktrees
for merged PRs, and 17 accumulated backup-file generations. None of that
was hypothetical — every category below is something that actually
happened here, not a theoretical checklist.

**Golden rule, stated explicitly because it was tested and mattered:**
verify before you purge. "Looks like a duplicate" and "confirmed
byte-for-byte subset with a diff/comm check" are different claims — only
act on the second. If a branch or file MIGHT hold unique value, flag it
and stop; don't delete on a hunch. (The user's own framing: "old stuff can
be purged but always double check.")

---

## 1. Tracked-file hygiene (repo)

```bash
# Find large tracked blobs that don't belong (compiled binaries, dumps)
git ls-files -z | xargs -0 du -b 2>/dev/null | sort -rn | head -20

# Cross-check: does anything live/running depend on a file that's
# UNTRACKED? (systemd ExecStart, cron, launchd plist — the audit trail
# must be real, ethos rule 6)
systemctl --user list-units --all 2>/dev/null | grep -oP '(?<=ExecStart=)\S+' 2>/dev/null
# or: systemctl --user cat <unit> | grep ExecStart
# then: git ls-files <that path> — empty means a fresh clone breaks
```

**Security check — do this every time, not just when something looks
wrong:** grep tracked files for names that match your own "never commit
this" conventions (`CLAUDE.local.md`, `*.env`, `*secrets*`, `*credentials*`,
anything your `.gitignore` comments call out as sensitive) and confirm
`.gitignore` actually covers them:

```bash
git ls-files | grep -iE 'local\.md$|\.env$|secret|credential|\.key$|\.pem$'
# for each hit, check: is it supposed to be here?
git check-ignore -v <path>   # empty = NOT ignored, even if it should be
```

If something sensitive is tracked and already pushed: `git rm --cached` +
add to `.gitignore` + commit stops it from being carried forward. That
does **not** scrub it from history on a remote it already reached — flag
that explicitly and let the human decide about a history rewrite
(rewriting a shared/in-review branch is disruptive; not a unilateral call).

## 2. Doc/skill drift

The same fact living in more than one file is checked, not assumed —
this codebase's own rule ("never a second copy of the same fact") applies
to prose as much as code:

```bash
# Any obviously-paired doc files (same basename, different dirs)?
find . -iname "amux.md" -o -iname "README.md" 2>/dev/null | grep -v node_modules
diff <(cat copy1) <(cat copy2)   # if near-identical, consolidate

# Is a user-level copy (~/.claude/commands/, ~/.claude/skills/) shadowing
# a project one for lanes whose CWD isn't this repo? Compare both:
diff ~/.claude/commands/<name>.md .claude/commands/<name>.md
```

**Fix at the root, not by re-syncing every time:** if two files are
supposed to always match, make one a symlink to the other so drift is
structurally impossible, not just corrected once. For a *repo* file with a
*user-level* fallback (different lanes, different CWDs), there's no
symlink across that boundary — sync the content once, and note in this
routine's own run that it needs a manual re-sync after future doc edits
(or better: script the sync and call it from here).

## 3. Branch hygiene

Never delete a branch on "looks old" alone — check real containment:

```bash
git fetch --prune origin   # do this per remote; multiple remotes in one
git fetch --prune fork     # `git fetch --prune r1 r2` silently fails —
git fetch --prune upstream # extra names are parsed as refspecs, not remotes

for b in $(git branch --format='%(refname:short)'); do
  if git merge-base --is-ancestor "$b" origin/main 2>/dev/null; then
    echo "$b: literal ancestor of origin/main — safe to delete"
  else
    ahead=$(git rev-list --count origin/main.."$b" 2>/dev/null)
    echo "$b: NOT an ancestor (ahead by $ahead) — check further before touching"
  fi
done
```

**A branch can be safely mergeable and still not show as an ancestor** —
squash-merge rewrites the commit, so `merge-base --is-ancestor` returns
false even though the PR is fully merged. For anything that check flags
as "not an ancestor," check the PR's actual state before concluding
anything:

```bash
gh pr list --repo <org>/<repo> --state merged --head <branch> --json number,state
# if MERGED, confirm the squash commit really landed (not just metadata):
git log origin/main --oneline --grep="<something distinctive from the branch>"
```

For a branch that's genuinely ahead with unique commits, don't assume
they're either "definitely valuable" or "definitely redundant" — check:

```bash
git log origin/main..<branch> --oneline   # what's actually unique
# for each unique-looking commit, grep for its subject/symbol elsewhere:
git log --oneline --all --grep="<distinctive phrase>" -i
grep -rn "<the function/route/endpoint it adds>" <where it'd land if merged>
```

If it's real, unshipped work — don't delete it. Flag it by name with what
it contains, so a human can route it (cherry-pick, new PR, or explicit
"actually drop this").

## 4. Worktree hygiene

```bash
git worktree list
# for each, check the PR state (same command as above) — remove worktrees
# for MERGED or CLOSED PRs, keep worktrees for OPEN ones:
git worktree remove <path>
```

## 5. Diverged-but-same-content branches (rare, but real)

Sometimes two refs have commits with identical author+timestamp+message
but different tree content — a genuine divergence from a concurrent
session, not simple staleness. **Never force-push or blindly pick a side.**
Preview a real merge first:

```bash
git merge --no-commit --no-ff <other-ref>
# then VERIFY it's lossless, don't just trust "no conflicts":
comm -23 <(git show <other-ref>:<file> | sort) <(sort <file>)
# empty output = other side was a strict subset, nothing lost
```

Only commit the merge (a real merge commit, never `--force`) once that
check comes back empty for every file that actually differed.

## 6. Push safety

If a push-guard or similar blocks on commits authored by a different
session/lane: that's a deliberate ownership check, not a bug. Don't
override it on your own judgment — surface exactly what's blocking and
who it's attributed to, and let the human decide. If they say to proceed
regardless of authorship, the guard's own error message names the escape
hatch (e.g. `AMUX_ALLOW_FOREIGN=1`) — use exactly that, not a broader
bypass.

## 7. Backup-file sprawl

Accumulation without discrimination (ethos rule 5) — multiple generations
of the same backup pattern (`file.bak-<date>`, `file.bak-<reason>`) piling
up with nothing ever pruning them:

```bash
# Keep the newest per basename, ARCHIVE (don't delete) the rest:
mkdir -p ~/.amux/backup-archive-$(date +%F)
for base in <list of base filenames with .bak-* variants>; do
  ls -1t ${base}.bak-* 2>/dev/null | tail -n +2 | xargs -I{} mv {} ~/.amux/backup-archive-$(date +%F)/
done
```

Archiving beats deleting for anything that might be someone's operational
checkpoint — the cost of keeping 17 small files is negligible; the cost of
deleting the one that mattered is not.

## 8. Report, don't sweep

End every run with a clear before/after: what was deleted (with the
verification that made it safe), what was flagged instead (with why it
wasn't safe to touch), and what's still open pending a human decision.
Ethos rule 8: report and recommend, don't silently decide for someone
else's data.

---

## Instructions

`$ARGUMENTS`: `dry-run` reports every finding above without deleting,
moving, or committing anything — use this for the first pass, or whenever
asked to "check" rather than "clean up". `full` (default) executes the
safe categories (1, 2 via symlink, 3–4 once verified, 6–7) and stops to
ask before anything in category 5 or anything flagged as ambiguous in 3.

Always finish with the category-8 report, even on a dry run.
