# Fleet friction themes: the living ledger

Recurring friction across **amux and Mixpeek**, at the level of the CLASS rather
than the incident. Updated daily by the sweep in `SCHED-399`
(`scripts/friction_themes.py` computes the signals, the session names the theme).

A theme belongs here when the same KIND of thing keeps costing time even though
the specific bug differs each time: "we always forget to add permissions to the
right place", "we always forget the integration test". Its fix is almost always a
sentence in a prompt, a hook, or a gate rather than a code change.

## Why this file exists instead of a dated report per day

The 2026-08-29 review (`fleet-friction-review-2026-08-29.md`) was a one-shot over
1,000 messages. Run daily, that shape produces 365 dated documents a year, each
re-deriving the same ten themes, and nobody reads the second one. Ethos rule 5:
if it becomes a log, it needed to split, not append.

So this is ONE file, and a daily run does exactly three things to it:

1. **Increments** a theme that recurred (`LAST_SEEN`, `OCCURRENCES`, new evidence).
2. **Adds** a theme only when the signals show a class with no home here yet.
3. **Retires** a theme whose signals have gone quiet for 14 days, with the
   evidence that they went quiet.

A day that changes nothing here is a normal day. Padding it is worse than
skipping it.

**A SECOND RUN ON THE SAME DAY DOES NOT RE-INCREMENT.** Check `LAST_SEEN` before
touching a theme: if it already reads today's date, that theme was counted this
cycle and a second pass must not count it again. Run the scan, compare, say so.

ADDING is still allowed, and the distinction is the whole point. The ban is on
counting ONE observation twice, not on recording something the earlier pass did
not see. A class that first became visible during the second pass has been
observed once and belongs in the file once. Written this way because the first
draft of this rule said "the three operations do not apply", which would have
suppressed a real finding to protect a count, and the very next thing this sweep
did was find one.

This is not hypothetical and it is the one way this file can corrupt itself.
SCHED-399 fired at 11:00 on 2026-08-31 and the sweep was invoked twice, at 11:01
and 11:11. The second scan returned an IDENTICAL active set with identical `n` for
every signal; only `considered` moved, by 1 to 9 rows, as the rolling window slid
nine minutes. Re-incrementing there would have written OCCURRENCES: 3 for a class
seen once, and OCCURRENCES is the number the whole file exists to make
trustworthy. A ledger that inflates its own counts is worth less than no ledger,
and nothing else here would have noticed.

The discriminator is cheap and it is already in the file: `LAST_SEEN` is the guard,
so use it rather than memory of whether you ran today.

## Format: fixed fields so this greps

```
## <the theme, stated as what keeps happening>
SCOPE: <amux|mixpeek|both>          # both = it belongs in the GLOBAL prompt
STATUS: <open|absorbed|retired>
FIRST_SEEN: <YYYY-MM-DD>
LAST_SEEN: <YYYY-MM-DD>
OCCURRENCES: <n>                    # daily runs that saw this theme active
SIGNALS: <comma-separated keys from friction_themes.py>
FIX_SITE: <the exact file or mechanism that would absorb this>
CARDS: <ids, or none>
EVIDENCE: <the measurement, with its number>
```

`SCOPE: both` is the field that answers Ethan's actual question. A theme with
evidence in both codebases belongs in `~/.claude/CLAUDE.md`, where every lane
reads it. One-repo themes belong in that repo's own file. The scan computes
which, per theme, so it is not a judgement call made from memory.

`STATUS: absorbed` means the prose or mechanism shipped. It is NOT the same as
the friction being gone, and the next run should keep watching the signals: nine
of the ten themes below existed as correct prose BEFORE they were measured, and
the prose lost. A theme goes `retired` only when its signals stay quiet.

Greps that should keep working:

```bash
grep '^SCOPE: both' docs/friction-themes.md        # candidates for the global prompt
grep '^STATUS: open' docs/friction-themes.md       # what is still live
grep -B2 -A8 '^## ' docs/friction-themes.md        # whole themes
```

---

## Seed: the ten themes measured 2026-08-29

Full evidence for each is in `fleet-friction-review-2026-08-29.md`. They are
recorded here at one paragraph so the daily run has a baseline to increment
rather than re-deriving them every morning. `OCCURRENCES: 1` is that one review;
scope was assessed on amux evidence and is re-measured across both repos daily.

## The board accumulates, and no status makes it discriminate
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-31
OCCURRENCES: 2
SIGNALS: board-resting:*, rule-restatement:backlog-growth
FIX_SITE: crates/amux-server/src/api/board*, plus a runtime job
CARDS: AF-317
EVIDENCE: 1,978 open cards; `todo` median age 28.8d with 88% over a week. No
status has a TTL, a WIP limit or a forced disposition. Re-measured 2026-08-30:
Mixpeek `needsyou` is 265 cards and grew +73 in 7 days, so the accumulation is
live and is currently faster on the Mixpeek side.

## `needsyou` is the cheap escape hatch, so the real asks are buried
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-01
OCCURRENCES: 3
LAST_SEEN_NOTE: re-measured 2026-09-01, and it is GROWING
SIGNALS: board-resting:*:needsyou
FIX_SITE: board status gate; `needsyou` requires a typed `--ask`
CARDS: AF-318
EVIDENCE: 445 cards in `needsyou`, median 15d, and 51% match no ask-shape at all.
Their titles are plain engineering work. The twenty that genuinely need Ethan
are indistinguishable inside them.
Re-measured 2026-08-31: the scanner read Mixpeek `needsyou` at 300 against a 215
baseline seven days ago. CORRECTED same day by mixpeek-frustrations, who counted
the full board: 490 needsyou total, of which 120 are ARCHIVED and 370 live, and 77
of the live ones already carry a typed ask (58 decision, 11 credential, 5 access,
2 external, 1 judgment). So the typed-ask SCHEMA has reached that board and only
the GATE has not, which is a cheaper fix than "ship AF-318 there" implies.
The 120 archived is the number worth raising: an archived `needsyou` card asks
nobody and appears in no view, so it is not waiting, it is gone, while still
inflating every count of the queue. That is the shape amux already warns about for
a NOTE appended to an archived card (the write succeeds and reaches nobody);
archiving a card that is ASKING is the same silence one level up, and nothing
warns. That is the same shape as the
`measured` contract in the theme below: enforced on one side, watched on the other.
One datapoint from this lane on the other side of it: four cards in this lane's own
`needsyou` (AF-155, AF-206, AF-286 and a fourth filed before finding them) were ONE
question asked four times. Consolidating them to a single card is the per-lane
version of what the gate does globally.

Re-measured 2026-09-01: Mixpeek `needsyou` is 313 cards, median age 17.0 days,
70% over a week old, oldest 60.9 days, and +93 against the 220 open seven days
ago. It is not resting, it is GROWING at roughly 13 cards a day. The typed-ask
schema reached that board (see above) and the gate still has not, so the queue
keeps taking cards that ask nobody anything.

## Nudging is the dominant channel and the loop has no negative feedback
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-31
OCCURRENCES: 1
SIGNALS: nudge-no-movement, rule-restatement:idle-stall
FIX_SITE: `idle_backlog_drain_cooldown_s()` and the board_drive job
CARDS: AF-319
EVIDENCE: 496 of 1,000 messages were `board_drive` nudges: 160k tokens in 34h
and 84% of one lane's entire inbox, with the queue unmoved. Cadence scales UP
with backlog size, so the biggest queues sit at the floor permanently.
Re-measured 2026-08-31: `rule-restatement:idle-stall` fired 10 times in one day
against a 1.23/day trailing baseline, EIGHT TIMES the baseline and the largest
excursion of any signal this pass. Spans both repos (2 amux / 6 mixpeek / 2
other). The prose already exists in three places (amux CLAUDE.md, the frustrations
rule, mixpeek CLAUDE.md) and Ethan is still writing "keep going until theyre all
verified at scale you dont need me" and "continue todo and everything until
they're all verified". Prose in three files losing to the mechanism eight times in
a day is the argument that AF-319 is a mechanism fix, not a wording fix.

## Verification is something Ethan has to demand, every single time
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-01
OCCURRENCES: 3
SIGNALS: rule-restatement:verification, rule-restatement:evidence
FIX_SITE: NOT prose. A push: nothing routes a `done` card to a verifier, so the
author has to remember to ask. See AF-393.
CARDS: AF-321, AF-393
EVIDENCE: he re-dictated what verification means 7 times in 34 hours. The gate
now refuses `done` without evidence in amux. Mixpeek has no equivalent gate, so
watch whether the restatements migrate there. The class was still active on
2026-08-30 with 4 hits, all Mixpeek-side.
STATUS MOVED BACK TO open ON 2026-09-01, and that is the finding rather than a
bookkeeping change. The theme was marked `absorbed` on 2026-08-30 because the
prose and the amux evidence gate shipped. The next measurement is
`rule-restatement:verification` n=16 against a 3.46/day baseline: FOUR AND A HALF
TIMES it, the highest reading this file has recorded for any class, and it went up
AFTER absorption. 10 of 16 Mixpeek, 3 amux, 3 other, so it is not one repo's gap.
The file's own warning about `absorbed` ("nine of the ten themes existed as correct
prose BEFORE they were measured, and the prose lost") is now measured on itself.
WHAT THE EVIDENCE SAYS THE MECHANISM IS. Ethan names it himself in MSG-37657:
"figure out why this wasn't automatically verified as part of the board gate". The
gate is not the missing piece and adding prose is not either. Measured across the
fleet the same day: 2260 cards are `verified`, so the gate is satisfiable at scale,
and tubescience runs 95% verified across 619 of them. The distribution is bimodal,
not low: amux-cloud 75%, amux-gtm 64%, amux-homepage 39%, amux-frustrations 16%,
amux 3%, all under the SAME group gate. The lanes with huge `done` piles are the
ones that never ASK a peer, and nothing in the system asks for them. Verification
is pull-only; every hit above is Ethan doing the routing by hand.
Confirmed from the inside on 2026-09-01: this lane sent two verification requests
carrying the reproduce command, the expected output and a mutation to run. Six
cards cleared in one round trip, by amux-cloud and amux-homepage, both of whom had
spare capacity the whole time. The ask was the only missing step.

## Access and credential gaps surface mid-task, never before
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-01
OCCURRENCES: 2
SIGNALS: rule-restatement:permissions, ledger-cluster:auth-secrets
FIX_SITE: a preflight that names required credentials before a lane starts
CARDS: AF-372
EVIDENCE: this is the class Ethan named by hand in MSG-35488 ("we always forget
to add permissions to the right place"). Credential-shaped cards are 13% of
`needsyou`, and every one of them is a task that had already started.
Re-measured 2026-08-31: `rule-restatement:permissions` fired 5 times in one day
against a 0.77/day baseline, SIX AND A HALF TIMES it, spanning both repos (1 amux
/ 2 mixpeek / 2 other). The prose already exists in amux CLAUDE.md and the global
CLAUDE.md. Specimens are mid-task every time, which is the whole shape: "use amux
connector for gmail access and granola" (hoichoi, 11:00) arrives when the work is
already underway, not before it. Carded as AF-372 rather than written as more
prose, because two files already say it and the signal is at 6.5x anyway.

Re-measured 2026-09-01: `rule-restatement:permissions` fired 4 times against a
0.85/day baseline, 4.7x, spanning both repos (2 amux / 1 mixpeek / 1 other). Every
specimen is mid-task again, and two of the four are the SAME session asking twice
in 54 minutes (MSG-37628 07:34, MSG-37678 08:28), the second one asking for the
list to be written into a README so a human can enable them up front. That is the
preflight this theme has been asking for, requested by Ethan in his own words.

## Instruments that lie: the single largest cluster in either ledger
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-01
OCCURRENCES: 3
SIGNALS: ledger-cluster:instruments, rule-restatement:instrument-lies
FIX_SITE: the `measured`/`n_considered` contract + `tests/diagnostic_contract.rs`
CARDS: AF-320, AF-394
EVIDENCE: 41 of 83 amux ledger entries are an instrument that could not express
its own failure. Re-measured 2026-08-30 across both repos: 99 open entries in
this class, 19 amux / 80 Mixpeek. The contract is enforced for new amux
diagnostic routes only, which is why this stays open on the Mixpeek side.
2026-09-01: THIS THEME'S OWN SIGNAL WAS UNDERCOUNTING IT, which is the class
happening to the instrument that measures the class. `ledger-cluster:instruments`
read QUIET on the 11:05 scan while three fresh instances of the shape sat in the
same output under `engine` and `api-contract`.
Cause, and it corrects the card that filed it (AF-394): AREA_CANON is
first-match-wins over a title that leads with its subsystem, but REORDERING would
have fixed nothing. 16 open entries across both ledgers describe a success report
contradicted by an empty result, and only 3 of them contain any word from the
instruments arm at all. The failure was vocabulary, not ordering, and those 16
were scattered over seven clusters.
Fixed by an ADDITIONAL cross-cutting membership rather than a reorder or a full
multi-label canon, both of which were measured before being rejected: reordering
steals entries from the subsystem clusters and moves their trailing baselines, and
letting every AREA_CANON arm apply independently gives 2.08 labels per entry, with
`doc` matching every entry that says "documents" (8 -> 156). The shipped change
moves exactly one cluster: instruments 103 -> 117, 17 of 18 others untouched, 1145
labels over 1131 entries. The signal now fires at n=4 in a 1-day window.
STATUS MOVED BACK TO open: this was `absorbed` on the strength of the amux
diagnostic-route contract, and the Mixpeek side is where the class actually lives
(80 of 99 last time, and 4 of 4 new entries today are Mixpeek).

## A fix ships, its tests pass, and it does nothing in production
SCOPE: amux
STATUS: open
FIRST_SEEN: 2026-08-30
LAST_SEEN: 2026-08-30
OCCURRENCES: 1
SIGNALS: ledger-cluster:instruments, rule-restatement:verification
FIX_SITE: the seam between a tested pure function and its untested call site
NOTE: this block was finished by a shell write on top of Edit-tool content, which is the
mixed-edit shape AF-342 is about, and it was staged to verify the fix against the live
server rather than only against tests.
CARDS: AF-342
EVIDENCE: AF-342 shipped with four passing cells over a correct pure decision and a
one-line derivation inside an async handler that nobody could test. The derivation read
a field any mtime satisfies, so the fix was inert on every path in the fleet. Every
instrument said pass: unit tests, mutation cells, and the deployed payload carrying the
new key. Only a live call against the running server showed the arm never firing.
Re-introducing the exact production bug by mutation then passed all 44 tests, which is
the measurement that names the gap: the tests pinned the decision and not the input to
it. The general shape is that extraction for testability stops at the function boundary,
and the bug moves one line up into the argument.

## One checkout, N lanes, and git has one index
SCOPE: both
STATUS: absorbed
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-31
OCCURRENCES: 3
SIGNALS: ledger-cluster:attribution, ledger-cluster:board-gates
FIX_SITE: per-lane git worktree (AF-336); harness rule in ~/.claude/CLAUDE.md
CARDS: AF-316, AF-336, AF-356, AF-365, AF-368
EVIDENCE: 9 separate attribution entries across the ledger are one fact. A
shared index means a peer's `git add` ships your in-flight work under their
name. 29 open entries now carry this shape across both ledgers.
SCOPE PROMOTED amux -> both on 2026-08-31, which is this sweep's main output.
`ledger-cluster:attribution` stands at 33 open, 12 amux and 21 MIXPEEK, and
`ledger-cluster:board-gates` at 83 open, 81 of them Mixpeek. The Mixpeek half is
the same class arriving through different tooling. CITATIONS CORRECTED 2026-08-31
by mixpeek-frustrations, who read `.githooks/pre-push` rather than the cards and
found one of my three instances stale:

  BR-81 tree-guards      LIVE, and the sharpest instance. Reads the worktree in
                         BOTH discovery (`cd server && grep -rl ... tests/unit/`,
                         line 1215) and execution (`pytest $_guards`, line 1225).
                         27 guards found, 23 walking the filesystem with no
                         git-tracking check. They measured it single-variable in a
                         tempdir: 2273 files walked / 5 of 5 PASS, versus 2274 with
                         one plausible peer WIP file added / 1 FAIL. A blocking
                         gate, and the pusher sees a pytest tail with nothing
                         saying the file is not theirs.
  AUTOD-121 studio tsc   FIXED by MC-1496, and my citation was stale. The gate now
                         materialises (`git archive "$push_head" studio | tar -x`,
                         line 427), verified here. It also fixed a half I could not
                         have seen from the card: the ratchet used to compare
                         against whatever tsc-baseline.json was on disk, so a
                         peer's in-flight baseline decided your push.
  AUTOD-116 graft-push   CLAIM correct, CITATION does not resolve: that id is an
                         unrelated Autodesk email card. Held on their memory, not
                         on a card, and recorded that way rather than dropped.

THE DISTINCTION THAT WORDING MISSED, and it changed the global rule. In Mixpeek
the SELECTION is not the leak: 20 call sites select from the pushed range
(`git diff --name-only "${base}...${push_head}"`) and none select from
`git status`. The leak is EXECUTION, the tool running against bytes on disk after
selecting correctly. So `git status` answers "is a peer working" and NOT "is that
what reddened me", because a gate only fires when YOUR range touches its paths.
Both questions are now in the global rule with the command each one takes.

Nobody carried this between repos; both arrived at it independently, which is what
makes it harness-level rather than one repo's quirk.
ABSORBED into `~/.claude/CLAUDE.md` as "Shared checkouts: a red build is not
evidence it is yours", carrying two commands rather than a principle: check
`git status --porcelain` before believing a red is yours, and treat an
mtime-derived owner as not-evidence. Both were measured the same day: a lane
diagnosed a filesystem race on a peer's uncommitted file, and two lanes each held
a record naming the other for a third lane's work. STATUS is `absorbed`, not
`retired`: the prose shipped, the signals stay watched, and the mechanism fix
(AF-336) is still open.

## Workers ask for authority they already have
SCOPE: both
STATUS: absorbed
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-29
OCCURRENCES: 1
SIGNALS: rule-restatement:autonomy
FIX_SITE: the standing-authority section of `~/.claude/CLAUDE.md`
CARDS: AF-322
EVIDENCE: Ethan granted authority by hand to five lanes in one 34-hour window,
plus eleven bare "continue" messages. The boundary list is now written down;
whether it reaches lanes is what the signal measures.

## Ethan is the fleet's status poller
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-30
OCCURRENCES: 2
SIGNALS: cross-lane-repeat, rule-restatement:staleness
FIX_SITE: a lane status a human can read without asking the lane
CARDS: none
EVIDENCE: the same instruction reaching two or more lanes in a day is the
measurable form. Four instances on 2026-08-30, one of them spanning both repos.

## The auto-builder restarts the server under in-flight work, and every caller fails differently
SCOPE: amux
STATUS: open
FIRST_SEEN: 2026-08-31
LAST_SEEN: 2026-08-31
OCCURRENCES: 1
SIGNALS: ledger-cluster:instruments, ledger-cluster:cli
FIX_SITE: the seam between the builder's restart and any caller holding a request
CARDS: AF-362, AF-371
EVIDENCE: three instances in ONE day, each failing a different way, which is why
none of them looked like a class until they were put side by side. The builder
rebuilds and swaps the binary on EVERY commit, so the window is not rare: it opens
several times an hour on a day when lanes are committing.
(1) `frustrations-archive.py`'s card-carry got curl exit 7 on two entries and
REPORTED IT honestly, leaving the entry archived and the card without its symptom.
Half-completed, visibly. AF-362.
(2) An mdai run was captured by the offline outbox and handed a synthetic 202, so
the panel reported a COMPLETED RUN that never left the browser and the op sat in
the outbox as `Syncing 0/1`. Reported as success, which is the worst of the three.
Ethan hit this one twice and reported it as two separate bugs. AF-371.
(3) `amux board add` returned rc=7 and printed NOTHING at all. Silent. Caught only
because the caller checked the exit code of a command it expected to print an id.
The three failure modes are honest-partial, false-success, and silent, from one
cause. Any fix aimed at one of them leaves the other two, which is the argument for
the theme rather than three cards.
NOT SCOPE: both. The auto-builder is amux-only; Mixpeek has no equivalent, and no
Mixpeek ledger entry carries this shape.

## The message bus cannot say whether a message landed
SCOPE: amux
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-29
OCCURRENCES: 1
SIGNALS: ledger-cluster:messaging
FIX_SITE: delivery accounting in the send path
CARDS: none
EVIDENCE: `output` is a viewport and `history` is the record; reading the wrong
one has already produced a "message was swallowed" incident that was false.
