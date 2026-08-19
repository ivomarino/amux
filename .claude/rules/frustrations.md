# Log amux-level friction to `frustrations.md`

You run *inside* amux. When amux itself gets in your way — a command that lies, a
notice that misattributes, a gate you cannot satisfy honestly, a probe that cannot
express the answer, a nudge that fires forever — **append an entry to
`frustrations.md` at the repo root.**

**"The repo root" means the checkout that can actually PUSH.** On a machine with
more than one clone this is not a pedantic distinction, it is the whole value of
the file: on 2026-08-17 four entries were appended to a checkout that was ~1000
commits behind with unpushed local commits and an hourly sync job that had failed
80+ runs. That copy held 25 entries; the real one held 124. The appends
SUCCEEDED — no error, nothing to notice — and reached nobody. The argument this
file exists to make is that one frustration is a complaint and a cluster is an
argument; a cluster only forms in the file everyone reads.

So before appending, confirm the checkout is not stranded:

```bash
git rev-list --count origin/main..HEAD   # unpushed commits here
git rev-list --count HEAD..origin/main   # how far behind
```

If BOTH are non-zero the checkout has diverged, cannot fast-forward, and nothing
will carry your entry upstream — append to a clone that is current instead. The
SessionStart freshness hook now says this out loud when it applies, because a
rule that only asks you to remember is the kind ethos rule 6 warns about.

And if this file itself has already diverged both ways — your local appends AND
origin-only entries a peer landed (AMUX-3367, seen live on the Mixpeek
FRUSTRATIONS.md) — do NOT reach for either single-arm git remedy: `git add` +
commit REVERTS the peer's entries, `git checkout origin/main -- <file>` DELETES
yours, and the direction test cannot separate them because BOTH are true at once.
UNION-MERGE: `git checkout origin/main -- <file>` to take origin's version, then
RE-APPEND your entries on top and commit. The idle commit-nudge now prints this
directive by name when a dirty append-only file is in the set, but the operation
is yours to run.

This is not a diary. It is the input to deciding what to fix next, so it has to be
greppable and it has to be honest about cost.

## When to log

Log it when amux cost you something you would not have paid with a better harness:

- a command reported success and did nothing, or reported the wrong thing
- an instrument could not express the failure you were looking at
- a gate could not be satisfied truthfully, so the honest move was to stop
- a notice/nudge sent you at the wrong card, the wrong session, or fired forever
- you had to leave the sanctioned path (raw curl, manual edit) to get work done
- two components disagreed about the same fact

Do **not** log: your own mistakes with no amux involvement, one-off environment
noise, or anything you fixed in the same breath with no cost to anyone. A frustration
is friction the NEXT session will also hit.

## How to log it

Append at the bottom. Never rewrite someone else's entry — add a new one that
supersedes it and say so. One entry per distinct friction; if it has two causes it is
two entries.

Use the field block exactly as written in `frustrations.md`'s own header — the fields
are fixed so `grep '^STATUS: open'` and `grep '^AREA: cli'` work. If you invent a
field, nobody's grep finds it.

**Link the card.** A frustration without a `CARD:` is a complaint; with one it is a
work item someone can pick up. If there is no card yet, file one.

**Record the COST in what it actually cost** — minutes, a wrong conclusion shipped, a
push blocked, a card closed that should not have been. "Annoying" is not a cost.

## Then act on it

Logging is not the fix. If the friction is cheap to fix and it is yours to fix, fix
it and set `STATUS: fixed` with the sha. If it belongs to another session's
subsystem, file the card and route it to them. The file exists so the pattern across
entries becomes visible — three entries with `AREA: attribution` is an argument that
one thing needs rebuilding, which no single entry makes on its own.
