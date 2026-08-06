# amux frustrations

Friction that **amux itself** caused a session working inside it. Appended to as we
hit things; read when deciding what to fix next.

The rule for when and how to log is in
[`.claude/rules/frustrations.md`](.claude/rules/frustrations.md). The short version:
log friction the NEXT session will also hit, link a card, and record the cost in what
it actually cost.

## Format — fixed fields so this greps

Append at the bottom. One entry per distinct friction. Never rewrite an existing
entry; add a new one that supersedes it and say so.

The template below is INDENTED two spaces on purpose: at column 0 it would match the
same greps as real entries, and the header would count itself as a frustration. An
instrument that measures itself is the bug this file exists to record.

```
  ## <one-line title, the symptom not the theory>
  AREA: <cli|board|attribution|notices|instruments|gates|browser|cloud|scheduler>
  SEVERITY: <blocks|slows|annoys>
  STATUS: <open|fixed>
  DATE: <YYYY-MM-DD>
  SESSION: <who hit it>
  CARD: <ID, or `none` only if genuinely unfilable>
  SYMPTOM: <what you actually saw — the output, the exit code, the wrong value>
  COST: <what it cost: minutes, a wrong conclusion, a blocked push, a false close>
  FIX: <what would fix it, or the sha if STATUS is fixed>
```

Greps that should keep working:

```bash
grep '^STATUS: open' frustrations.md          # what is still live
grep '^AREA: attribution' frustrations.md     # cluster by subsystem
grep '^SEVERITY: blocks' frustrations.md      # what stops work outright
grep -B1 -A8 '^## ' frustrations.md           # whole entries
```

**Why fixed fields:** three entries sharing an `AREA` is an argument that one thing
needs rebuilding. No single entry makes that argument, and free-form prose cannot be
counted.

---

## `amux board needsyou` printed a status arrow for a write that changes no status
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-222
SYMPTOM: `amux board needsyou AC-221 "..."` printed `AC-221 → todo`, exit 0. The card's
  status was still `todo` afterwards. The verb is a TAG write by design and never sets
  status, but it borrowed the status-transition printer, which renders the card's
  CURRENT status behind an arrow.
COST: I read it as a failed write, filed a bug against a working command, and then
  routed around it with a hand-rolled PATCH — losing the audited path for a write that
  had actually succeeded twice.
FIX: d18ec81 — `_board_outcome` takes a label for non-transition writes; the tag path
  now prints `→ tagged needs:you`. amux found a third site (`amux board type`) the same
  way; my sweep had missed it because my grep pattern included `; return $?`.

## The `needs:you` tag does not exempt a card from auto-pickup
AREA: board
SEVERITY: blocks
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-223
SYMPTOM: A card tagged `needs:you` — the sanctioned way to say "blocked on a human" —
  was claimed by auto-pickup ~40 min later and handed back as ordinary work.
  `_pickup_next_board_task` filters on `owner_type` and `archived` and never references
  `needs:you` or `issue_tags`. Two representations exist (tag vs `status=needsyou`) and
  only the minority one actually stops dispatch.
COST: A worker can be handed a card whose owner never made the decision. Hit twice in
  one session, by two different lanes. Also cost me two cards silently sitting in `todo`
  while I believed they were parked.
FIX: Either pickup excludes the tag, or `needsyou` sets status too. The second
  reclassifies ~100 cards in one migration — ask what the first run after the fix emits.

## The passenger check compares SHAs, so an already-upstream cherry-pick reads foreign forever
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-227
SYMPTOM: CLAUDE.md's pre-push recipe lists `origin/main..main` and says to ask the author
  about any foreign commit. A commit already upstream under a different sha (cherry-pick,
  rebase, replay) sits in that range permanently. Confirmed: `acdbfdf` and `9ebc42c` share
  patch-id `dff284cf093aecaa`.
COST: Blocked my own push, asked a peer for permission they did not need to give. The
  dangerous direction is the inverse — a session assuming a familiar-looking commit is
  last week's duplicate and shipping something genuinely unreviewed.
FIX: `git fetch` first (the recipe never says to), then compare `git patch-id --stable`
  against upstream before asking anyone.

## The co-edit sweep notice named the reporting session, not the commit's author
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-230
SYMPTOM: "Commit 1b743ec by session 'amux-homepage' touched files you also edited" — that
  commit's own `Amux-Session` trailer reads `amux-cloud`. The notice interpolated the
  session that POSTED commit-report, which on a shared checkout is routinely not the author.
COST: The same message ends "Do not report a sha you did not create". Following it
  literally means disowning your own work; the mirror case is claiming someone else's — on
  the one subject this fleet has spent the most effort getting right.
FIX: 6ecc3cb — read the trailer for the sha it was already fetching with `git show`; fall
  back to the reporter only for untrailered commits, and say so.

## `/api/schedules/audit` silently ignores `?field=` and returns everything
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-228
SYMPTOM: `?field=enabled&limit=500` returns all 459 rows, 570 KB, fields
  `['command','created','deleted','done_action']`. The filter did nothing and the response
  looks like a successful filtered result. `?id=` filters correctly, so filtering exists.
COST: A caller scoping to one field and counting rows gets a confident wrong answer with
  no tell — the same failure class the endpoint was built to fix. Also 87% of the payload
  is `command` diffs while `enabled` is 1%, on a mobile-first PWA.
FIX: Honour `?field=`, or reject unknown params with 400. Silently ignoring is the one
  option that manufactures false confidence. Truncate large old/new values in list view.

## The browser driver drops to `backend: cli` mid-session and every eval returns null
AREA: browser
SEVERITY: blocks
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-233
SYMPTOM: After a few `/api/browser/action` evals the backend silently changes from
  `driver` to `cli`, `location.href` becomes `about:blank`, and results come back empty.
  Happened three times in one visual-review pass; amux hit it twice the same night.
COST: A UI review that needs more than ~4 steps cannot be completed. Two of the three
  questions I was asked to answer about the Scope tab went unanswered — not because the
  feature was fine, but because the rig died mid-pass.
FIX: Unknown. At minimum the fallback should be LOUD — silently answering from a
  different backend with an empty page is worse than erroring, because the caller reads
  the emptiness as a finding about the page.

## A reviewer who BLOCKS a card is re-nudged forever
AREA: notices
SEVERITY: annoys
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-234
SYMPTOM: Blocking a card is a completed review action, but the card stays in `review`, so
  the sweep re-selects it every cycle. I was nudged three times for one card I had already
  reviewed and blocked, with the analysis sitting on it.
COST: The re-nag becomes the loudest signal in the room while the actual defect sits
  untouched, and only the author can clear it. Trains reviewers to ignore review nudges,
  which is the one class that should never be ignored.
FIX: A state (or a reviewer-responded timestamp) meaning "reviewed, ball is with the
  author", and suppress while it holds. Related: a stale nudge also fired for a card
  closed 64s earlier — re-check status at SEND time, not selection time.

## The `verified` gate requires e2e, and every e2e path runs against the cloud host
AREA: gates
SEVERITY: blocks
STATUS: open
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-216
SYMPTOM: The verified gate's first item is "CI/CD green (incl. e2e)". `checks.yml` is
  syntax + pytest and contains no e2e; the e2e workflows (`cloud-exec-check.yml`,
  `cloud-browser-verify.yml`) SSH into the cloud host. While that host is down, no card on
  the board can reach `verified`.
COST: Ten `done` cards were unverifiable at once, for a reason none of them had anything
  to do with. A verification sweep asked me to move them and the honest answer for every
  single one was the same sentence.
FIX: Not obvious and not necessarily a bug — but the gate should be able to distinguish
  "e2e failed" from "e2e could not run", and a fleet-wide sweep should say so once rather
  than per-card.
