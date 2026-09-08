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
LAST_SEEN: 2026-09-08
OCCURRENCES: 5
SIGNALS: board-resting:*, rule-restatement:backlog-growth
FIX_SITE: crates/amux-server/src/api/board*, plus a runtime job
CARDS: AF-317
EVIDENCE: 1,978 open cards; `todo` median age 28.8d with 88% over a week. No
status has a TTL, a WIP limit or a forced disposition. Re-measured 2026-08-30:
Mixpeek `needsyou` is 265 cards and grew +73 in 7 days, so the accumulation is
live and is currently faster on the Mixpeek side.

Re-measured 2026-09-04: `board-resting:mixpeek:blocked` is 39 cards, median age
32.8 days, 85% over a week, +6 against 33 seven days ago. Still growing, and the
median age is now the highest of any resting queue in this file.

Re-measured 2026-09-06: `ledger-cluster:board-gates` n=3 new and spanning both
repos (1 amux / 2 mixpeek), 96 open in total. The sharpest new specimen is
structural rather than volumetric: the `todo` WIP limit is 20, and one lane holds
123 — the limit is enforced against NEW entrants and was never applied to the
standing population, so it blocks legitimate routing while doing nothing about
the backlog it exists to prevent. Hit live this pass: routing AF-111 to its owner
was refused with `todo_wip_limit_reached ... amux already holds 121 todo card(s)`,
and the honest move was `backlog` with a trigger.

Re-measured 2026-09-08: `board-resting:mixpeek:blocked` is 40 cards, median age
36.8d, 90% over a week, oldest 56.0d, +4 against 36 seven days ago, still
`growing: true`. The median age keeps climbing (32.8d on 09-04, 34.9d on 09-06,
36.8d today) and is again the oldest resting queue in this file, so the queue is
not draining and turning over, it is aging in place.

## `needsyou` is the cheap escape hatch, so the real asks are buried
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-08
OCCURRENCES: 7
LAST_SEEN_NOTE: re-measured 2026-09-05 in BOTH repos for the first time; still GROWING
SIGNALS: board-resting:*:needsyou, cross-lane-repeat
FIX_SITE: board status gate (`needsyou` requires a typed `--ask`) AND an owner-side
producer — nothing on either board tells the human he is the blocker
CARDS: AF-318, AF-510
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

Re-measured 2026-09-04: Mixpeek `needsyou` is 306 cards, median 16.1 days, 66%
over a week, oldest 64.5 days, +105 against 201 seven days ago. Growth is ~15/day
against ~13/day on 2026-09-01, so the gate still has not reached that board.

Re-measured 2026-09-05, and this is the first pass that measured BOTH SIDES, which
changes what the theme is about. Mixpeek `needsyou` is 323 cards, median 16.1 days,
66% over a week, oldest 65.5 days, +109 against 214 seven days ago. amux is 506
cards, median 5.6 days, p90 20.5, 179 of them over a week old. Roughly 829 cards
across the fleet name a human as the blocker.

THE SECOND FIX SITE, found by the 10am sweep the same day and not visible from the
card counts alone: nothing on EITHER board tells that human. `board_drive.rs:4017`
sends the only `needsyou` reminder to `target: session.to_string()` — the LANE that
filed the ask — while its own text says "waiting on the HUMAN, not the lane". The
comment at `board_drive.rs:4763` says AF-465's remaining split "waits on confirming
that producer"; confirmed this pass that no such producer exists (180 schedules, 13
mention "digest", none carries needs:you; no push/email emitter; AC-413 says the
same). So the queue has two independent leaks — a gate that lets untyped asks in,
and no channel that lets real ones out — and the file previously only tracked the
first. AF-510 carries the second.

What made this visible: Ethan sent "whats the status?" five times in three days to
primis and tubescience. primis had 8 of 9 non-terminal cards in `needsyou` aged
40-120h; tubescience 42 non-terminal, mostly blocked/needsyou at 70-259h. The
answer to his question was "it is waiting on you", and asking a lane was the only
way to find that out.

Re-measured 2026-09-06: mixpeek `needsyou` is 349 cards, median age 16.2d, 72%
older than 7 days, oldest 66.6d — and **+98 against the 251 open 7 days ago**, so
it is growing by ~14 cards a day. `blocked` moved the same direction: 43 cards,
median 34.9d, +7. Both signals carry `growing: true`.

On the amux side the same shape is visible from inside: 36 of this lane's 42
non-terminal cards are `needsyou`, and one of them (AF-510) is literally "506
needs:you cards name Ethan as the blocker and nothing tells him". A queue whose
own backlog contains the card describing the queue is the accumulation this theme
names.

Re-measured 2026-09-08: amux `needsyou` is 66 cards, median age 14.8d, 70% over
a week, oldest 39.7d, and **+20 against the 46 open 7 days ago**, carrying
`growing: true`. This is a board-state signal, so neither of the two counting
defects AF-585 fixed can touch it.

## Nudging is the dominant channel and the loop has no negative feedback
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-08
OCCURRENCES: 4
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

Re-measured 2026-09-04: `rule-restatement:idle-stall` n=4 against a 1.23/day
baseline, 3.3x, down from 8x on 2026-08-31. `nudge-no-movement` fired at n=1 and
its single lane is `ledger-selftest-target`, a test fixture with 14 machine
messages and 0 human ones, so that signal contributed no real specimen this pass
and is reported rather than counted.

Re-measured 2026-09-06: `nudge-no-movement` n=3, and this time the specimens are
REAL lanes, not the test fixture that made 09-04 uncountable — mvs-research (23
machine messages, 0 human, 0 cards closed), mixpeek-studio (20/0/0),
general-canvas-apps (14/0/0). Fleet totals for the day: 1,274 machine messages
against 15 human ones, a ratio of 85:1.

This pass found the mechanism underneath rather than restating the prose, and it
is worse than "the cadence is too high": 123 of the fleet's 209 LIVE todo cards
(58%) belong to `amux`, which is an isolated lane, and `board_drive` builds its
lane list as `all_lane_names().filter(|l| !session_is_isolated(l))`. So the
largest queue in the fleet is addressed to a lane the dispatcher structurally
skips — the nudge loop cannot move it no matter how often it fires, and nothing
anywhere said so. The existing check could not see it either: its predicate is
`COALESCE(session,'')=''` and an isolated lane HAS a session (ethos rule 1, a
view must share the predicate of the mechanism it describes). Shipped as a
MECHANISM, not a sentence: `board.todo_is_reachable_by_dispatch`, 7d409d0a,
AF-535. What surfaced it was a human typing "this workers board is evident if
the board system still not working".

Re-measured 2026-09-08, and the measurement itself had to be repaired first
(AF-585, d308ecfd). `rule-restatement:idle-stall` reads n=83 over 55 lanes with
a 31% top-lane share, which is the most theme-shaped concentration a signal can
have. It is a FAN-OUT: 65% of that evidence arrived in minutes where one message
reached five or more lanes, and the widest single minute carried an identical
message to 41 lanes (09-07 18:09, "start all non-archived workers"), with 28 in
another and bare "continue" reaching 24 and 19. `rule-restatement:duplicate-work`
is the same minutes, n=65 over 54 lanes, 83% fan-out.

So the 83 is not 55 lanes independently hitting a friction, and this theme is
counted on the fan-out itself rather than on lane breadth. Broadcasting to 41
lanes at once IS the theme: nudging is the channel, and when the loop does not
move the queue the correction available to a human is to send the same sentence
to everybody. `nudge-no-movement` did not fire this pass.

## Verification is something Ethan has to demand, every single time
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-04
OCCURRENCES: 4
SIGNALS: rule-restatement:verification, rule-restatement:evidence
FIX_SITE: NOT prose, and NOT a new mechanism. See AF-393: the push exists, the
naming is enforced where a group asks for it, and the open question is whether the
FLEET DEFAULT gate should ask for a peer at all.
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

CORRECTED SAME DAY, 2026-09-01, and the correction is the more useful finding.
This theme's first version said "nothing routes a done card to a verifier". Wrong:
`reviewer` is a real field with a real push behind it, `reviewer_notify` in
board.rs and a reviewer nudge in board_drive.rs firing on `status == review AND
reviewer == you`. Measured across the whole board:

  done      2483 cards,  225 with a reviewer named   ( 9%)
  verified  2271 cards,  233 with a reviewer named   (10%)
  review      32 cards,    4 with a reviewer named   (12%)

The mechanism is built, wired and unused. 2038 of 2271 cards reached `verified`
without ever naming a reviewer, and only 32 cards are in `review` at all.
So the defect is one layer over: the verified gate's own criterion is "Peer-reviewed
by a DIFFERENT worker (NAME THEM)", and 90% name them only in free-text evidence.
The board cannot answer "who verified this?" as data, which makes every
verification ratio in this file uncheckable, including the ones above. That is
ethos rule 4 inside the gate that exists to enforce evidence: the answer is
required, supplied, and stored where nothing can read it.

CORRECTED TWICE ON 2026-09-01, and the second correction retires the number in the
first. The "90% of verified cards name no reviewer" figure above compared across
gates that ask different things. The "name them" criterion exists only in the
group:amux gate; the fleet default has no peer criterion at all. Read from three of
today's unnamed cards: TG-3341 and MS-1266 resolve to ["Outcome confirmed to still
hold"], SP-650 to [CI green, deployed to prod, confirmed working in prod, zero
regressions]. None asks for a second party, so those lanes are not evading
anything.
So 90% measured compliance with a criterion 90% of those cards were never subject
to: the instrument comparing across populations that are not comparable, committed
by this sweep, in the file that exists to catch that class.
WHAT SURVIVES, narrowly: the group:amux gate is enforced and works (amux-cloud's
first verify today was REFUSED for a missing reviewer and all eight of today's
verifications carry the field as data). And amux itself is inside group:amux at 3%
verified over 351 done, so that backlog is a real compliance gap rather than a gate
mismatch. The theme stays open on that, and on the 4.6x restatement rate, not on
the 90%.

Re-measured 2026-09-04: `rule-restatement:verification` n=7 against a 3.77/day
baseline, 1.9x, and 6 of the 7 are amux-side. That is DOWN from the 4.6x reading
on 2026-09-01 that moved this theme back to open, and the per-repo split has
inverted: 10 of 16 were Mixpeek then, 1 of 7 now. Not called absorbed on one
reading in the right direction; a 1.9x excursion is still an excursion, and this
file has already recorded this theme going UP after being marked absorbed.

## Access and credential gaps surface mid-task, never before
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-04
OCCURRENCES: 3
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

Re-measured 2026-09-04: `rule-restatement:permissions` n=4 against a 1.08/day
baseline, 3.7x, and ALL FOUR are the same lane (tubescience) on the same day.
Scope reads MIXPEEK this pass rather than both. The specimens are the shape this
theme names, every one mid-task: "why do I need this permission" (18:20), and
"you should have GitHub access, access to the app ... and supabase credentials
all of which are in this folder" (11:29), which is the preflight arriving as a
message seven hours before the complaint.

## Instruments that lie: the single largest cluster in either ledger
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-08-29
LAST_SEEN: 2026-09-08
OCCURRENCES: 6
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

Re-measured 2026-09-04: `ledger-cluster:instruments` n=9 (2 amux / 7 mixpeek),
163 open in total, 25 amux / 138 mixpeek. THE NUMBER MOVED BECAUSE THE
INSTRUMENT DID, and the next run must not read it as a surge. 4216a537 widened
`GREEN_BUT_EMPTY`, the cross-cutting membership added on 2026-09-01, after
measuring that it matched the WORDING of the three specimens it was written from
rather than the class: it caught 32 of 1,206 open entries while 63 more describe
the identical shape in other words. Standing open therefore goes 127 -> 163 and
the 1-day signal 5 -> 9 on an unchanged corpus. Surgical, re-verified: every
other cluster is byte-identical across the two scans.
WHAT THE 63 ACTUALLY ARE, and it is the finding rather than the regex: a call
that answers with a success-shaped status over work it did not do. A silent
200-empty, `objects/batch` dropping any blob it cannot fetch, `count()` ignoring
its `filters` and returning the namespace total, `is_active: false` not taking an
app offline, `post_filters` typed and documented and never applied. 92 of the 93
memberships are Mixpeek, scattered over eight subsystem clusters with none
holding more than 38%, which is why no single cluster ever made the argument.
THE FIX SITE MOVES WITH IT. This theme's FIX_SITE is the amux `measured` /
`n_considered` contract, an INTERNAL DIAGNOSTICS mechanism, and it cannot absorb
"the batches endpoint reports COMPLETED while a leg runs". The Mixpeek half needs
a response contract on the PRODUCT API, which is subsystem work owned by those
lanes and not this sweep's to write. Carded and routed rather than written as
prose here.

Re-measured 2026-09-06: `ledger-cluster:instruments` n=2 new, 166 open in total
(25 amux / 141 mixpeek). Three fresh specimens from this lane alone in one
session, all the same shape — an output that reads as a clean result when the
measurement did not run:
- `scripts/mutate.sh` reported `command exited 0` for a mutation whose build never
  completed, because a pipeline's status is its last element's and the documented
  usage pipes cargo through grep. After a mutation, "exited 0" reads as THE CHECK
  CANNOT FAIL. Fixed 5285562c (AF-532); the headline symptom is still open.
- `google_sa::sa_config()` read the real `~/.amux` rather than the home it was
  handed, so one test was green in CI and red on every developer box, and neither
  result said which environment it had measured. Fixed 5ce96bee (AF-529).
- `GET /api/board?all=1` includes ARCHIVED rows, so a count over it reported 183
  stranded cards where 3 were live. Caught before filing, only because a second
  measurement disagreed. Same family as AF-460.

Re-measured 2026-09-08: `ledger-cluster:instruments` gained 4 open entries in
one day spanning both repos (2 amux / 2 mixpeek) against 170 open in total, the
largest standing cluster in either ledger for the sixth pass running.

The sharpest specimen this pass is FIRST-PARTY: this sweep's own instrument.
`concentration()` in `scripts/friction_themes.py` exists so that `n` cannot pass
one incident off as a fleet theme, and it was 0 for 11 — it could not fire at
all. Its incident test required `len(days) == 1` while the window is a rolling
24h that always covers two calendar dates, so the same five-message, four-minute,
one-lane incident read True at 12:00 and False at 00:02. Its own unit test missed
it for three days by clustering synthetic timestamps at `now`, which lands on one
date ~99.7% of the day while production never does. A second blind spot had no
detector at all: one message fanned out to many lanes reads as MAXIMUM breadth.
Both fixed in d308ecfd with mutation-checked tests; after the fix 5 of 11 signals
flag, including the top four by n. AF-585.

This is the theme's own shape applied to the tool that measures the theme, and
it is the reason today's two loudest signals did not increment as breadth.

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
LAST_SEEN: 2026-09-04
OCCURRENCES: 3
SIGNALS: cross-lane-repeat, rule-restatement:staleness
FIX_SITE: a lane status a human can read without asking the lane
CARDS: none
EVIDENCE: the same instruction reaching two or more lanes in a day is the
measurable form. Four instances on 2026-08-30, one of them spanning both repos.
Re-measured 2026-09-04, AND THE INSTRUMENT WAS ARGUING THE THEME IN THE HARNESS'S
OWN VOICE. `cross-lane-repeat` read n=14 against 97 messages considered. 4 of the
14 were the `[amux: @-mentions above are amux workers...]` footer that amux
APPENDS to a message: MSG-40511 and MSG-42067 share 194 identical characters and
nothing else, one asking to add public datasets to a table and the other asking
for MVS throughput metrics, and the signal called them the same instruction sent
to two lanes. Fixed in e1ad6c4e; `instruction_of` now cuts at the first `[amux: `.
CORRECTED READING: n=10, not 14. That is the number this theme is incremented on.
This signal is this theme's ONLY evidence, and its claim is that Ethan has to
repeat himself, so 29% of the evidence being the harness's own text is a defect
in the argument rather than in a count. Recorded here because the next run's
comparison is against 10.
WHAT SURVIVES AT 10 IS STILL THE THEME. The sharpest specimen is his own words:
"I want you to send this command to every active non archived worker: 'review
your backlog and todo see whats still relevant, discard what is not'"
(MSG-40168), with MSG-40166 sending it by hand to mvs-infra a minute earlier. He
is fanning an instruction out to the fleet by hand because nothing else will.
Four more at 15:55-15:58 are four separately-typed deployment instructions to
amux, amux-codex, amux-cloud and amux-testing-e2e about ONE blocked checkout,
which is the same shape one layer up: one condition, four hand-written messages.

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

## A capability ships and is nameable nowhere, so the person who asked for it keeps asking
SCOPE: both
STATUS: absorbed
FIRST_SEEN: 2026-09-03
LAST_SEEN: 2026-09-08
OCCURRENCES: 2
SIGNALS: rule-restatement:backlog-growth
FIX_SITE: `~/.claude/CLAUDE.md` board section, held by
`board_drive::tests::the_global_prompt_names_the_real_backlog_dispatch_key`
CARDS: AF-449
EVIDENCE: `rule-restatement:backlog-growth` fired at n=23 against a 0.54/day
baseline (43x) across BOTH repos — 15 amux, 8 mixpeek — with the scanner
reporting `prose_exists: false` and `already_written_in: []`.
The messages are not complaints about the pile. They ask for a switch:
"Toggle this worker so that it automatically drains the backlog. There should be
an environment variable in the scope" (MSG-40011, primis) and "there should be a
flag" (MSG-40009, tubescience). That switch already existed:
`AMUX_DISPATCH_BACKLOG_WHEN_IDLE`, shipped by AMUX-4055 in 16eaeae0 at 22:00 on
2026-09-02, default off, scoped worker > group > global.
It was named in no prompt file, no doc, and no nudge text — checked, not assumed:
grep across ~/.claude/CLAUDE.md, amux CLAUDE.md, docs/ and .claude/rules/ returned
nothing, and the idle nudge that fires on exactly this condition tells a lane
"board-drive only dispatches `todo` ... you have to pull from it" without naming
the flag that changes it. Eleven hours between shipping and the first of 23 asks.
DELIBERATELY NOT COUNTED under "The board accumulates" (SIGNALS lists this same
key). That theme is about a pile with no TTL and no forced disposition; today's
messages are about a control that exists and cannot be named. Incrementing it
from this evidence would inflate a count with observations that do not support
it, which is the one way this file corrupts itself.
WHY IT IS `absorbed` AND NOT `open`: the paragraph shipped to the global prompt
this pass. That is not the same as the friction being gone — nine of the ten seed
themes were correct prose that lost to the mechanism — so the prose is held by a
test that reads the CONSTANT rather than a copy of its text, and reports UNMEASURED
rather than passing when the global prompt is absent (cloud image). Rename the key
and the test fails; drop the paragraph and the test fails. Watch
`rule-restatement:backlog-growth` next run: if it stays at 20+ with the prose
live, the prose lost and the fix site is the nudge text, not the prompt.

Checked 2026-09-08, because the paragraph above names the exact test: with the
prose live, `rule-restatement:backlog-growth` is n=15, down from the 23 that
prompted it, against a 1.62/day baseline. Below the 20+ that would have said the
prose lost, and still ~9x baseline, so this is not yet a clean read either way.
Concentration is the reason to withhold a verdict rather than the number: 3
lanes, 67% of it one lane (amux-testing-e2e), so most of the remaining 15 is one
lane's overnight run and not fleet-wide demand for a switch that now has a name.
Watch it again on a day that lane is quiet.

## A gate asks for something the card cannot truthfully provide, so the honest move is `--force`
SCOPE: both
STATUS: open
FIRST_SEEN: 2026-09-08
LAST_SEEN: 2026-09-08
OCCURRENCES: 1
SIGNALS: ledger-cluster:board-gates
FIX_SITE: the gate precedence walk in `crates/amux-server/src/db/board_store.rs`
(`GateSource`), which resolves card > worker > group > column > type_default and
lets only the LAST tier derive from the item type
CARDS: AF-570, AF-586
EVIDENCE: `ledger-cluster:board-gates` gained 5 open entries in one day spanning
both repos (1 amux / 4 mixpeek) against 100 open in total (5 amux / 95 mixpeek).

Two independent specimens in two days, at two different tiers of the same walk:

- AF-570 (2026-09-07): the `group:amux` verified gate demands "functionality
  change is live and exercised" from card types that produce no functionality
  change.
- AF-586 (2026-09-08), reported live by the `tubescience` lane during this
  sweep: its WORKER-scope done gate is `["Implemented and merged", "Tests / lint
  pass", "Peer reviewed"]` and applies to every card type. Confirmed by reading
  `GET /api/board/session-gates`. Four forced closes in one day (TUBES-2483,
  TUBES-2484, TUBES-2493 research; TUBES-2503 relaying a verbal authorization),
  because acking "merged/tested/peer-reviewed" for a source-read or an
  authorization relay would be false.

This is ethos rule 3 as a mechanism: for a research or ops card on such a lane
there is NO truthful path through the gate, so the logged bypass is the honest
move and the gate teaches every lane that `--force` is routine. The refusal is
not silent about it either — it says "retyping will NOT change it" — so the
operator is correctly told that the one lever that looks like it should work
does not.

WHY THIS IS A MECHANISM CARD AND NOT A PARAGRAPH: `@additive` already shipped
for AF-570, and it solves the ADJACENT problem (a scoped gate that should add to
the type default rather than replace it). Applied here it makes things worse: it
would keep the code criteria AND add the type's, so the research card still
cannot pass. What is missing is type-AWARENESS in the scoped tiers, or letting an
explicit retype outrank a scoped gate. No sentence in any prompt file can absorb
this; a lane that reads the rule and agrees with it still cannot close the card.

NOT COUNTED as a `rule-restatement` signal: nothing here came from Ethan
restating anything. It came from the ledger cluster and from a peer lane
reporting its own blocked closes, which is the source this theme should be read
from.
