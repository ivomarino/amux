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
LAST_SEEN: 2026-08-30
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
LAST_SEEN: 2026-08-29
OCCURRENCES: 1
SIGNALS: board-resting:*:needsyou
FIX_SITE: board status gate; `needsyou` requires a typed `--ask`
CARDS: AF-318
EVIDENCE: 445 cards in `needsyou`, median 15d, and 51% match no ask-shape at all.
Their titles are plain engineering work. The twenty that genuinely need Ethan
are indistinguishable inside them.

## Nudging is the dominant channel and the loop has no negative feedback
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-29
OCCURRENCES: 1
SIGNALS: nudge-no-movement, rule-restatement:idle-stall
FIX_SITE: `idle_backlog_drain_cooldown_s()` and the board_drive job
CARDS: AF-319
EVIDENCE: 496 of 1,000 messages were `board_drive` nudges: 160k tokens in 34h
and 84% of one lane's entire inbox, with the queue unmoved. Cadence scales UP
with backlog size, so the biggest queues sit at the floor permanently.

## Verification is something Ethan has to demand, every single time
SCOPE: both
STATUS: absorbed
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-30
OCCURRENCES: 2
SIGNALS: rule-restatement:verification, rule-restatement:evidence
FIX_SITE: VERIFY.md per repo + evidence required on `done` (shipped for amux)
CARDS: AF-321
EVIDENCE: he re-dictated what verification means 7 times in 34 hours. The gate
now refuses `done` without evidence in amux. Mixpeek has no equivalent gate, so
watch whether the restatements migrate there. The class was still active on
2026-08-30 with 4 hits, all Mixpeek-side.

## Access and credential gaps surface mid-task, never before
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-29
OCCURRENCES: 1
SIGNALS: rule-restatement:permissions, ledger-cluster:auth-secrets
FIX_SITE: a preflight that names required credentials before a lane starts
CARDS: none
EVIDENCE: this is the class Ethan named by hand in MSG-35488 ("we always forget
to add permissions to the right place"). Credential-shaped cards are 13% of
`needsyou`, and every one of them is a task that had already started.

## Instruments that lie: the single largest cluster in either ledger
SCOPE: both
STATUS: absorbed
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-30
OCCURRENCES: 2
SIGNALS: ledger-cluster:instruments, rule-restatement:instrument-lies
FIX_SITE: the `measured`/`n_considered` contract + `tests/diagnostic_contract.rs`
CARDS: AF-320
EVIDENCE: 41 of 83 amux ledger entries are an instrument that could not express
its own failure. Re-measured 2026-08-30 across both repos: 99 open entries in
this class, 19 amux / 80 Mixpeek. The contract is enforced for new amux
diagnostic routes only, which is why this stays open on the Mixpeek side.

## One checkout, N lanes, and git has one index
SCOPE: amux
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-08-30
OCCURRENCES: 2
SIGNALS: ledger-cluster:attribution
FIX_SITE: per-lane git worktree
CARDS: AF-316
EVIDENCE: 9 separate attribution entries across the ledger are one fact. A
shared index means a peer's `git add` ships your in-flight work under their
name. 29 open entries now carry this shape across both ledgers.

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
