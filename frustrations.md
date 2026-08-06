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
STATUS: fixed
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
FIX: b4ea1d0 — the pickup query now excludes cards carrying the tag. Chose that over
  converting the tag to a status: the status route reclassifies ~142 cards in one
  migration and surfaces them all in Needs-you at once, while the exclusion exempts
  exactly 2 currently-dispatchable cards. Measured before shipping, not after. The
  tag/status split itself is still open — this closed the dispatch hole, not the
  representation question.

## The passenger check compares SHAs, so an already-upstream cherry-pick reads foreign forever
AREA: attribution
SEVERITY: slows
STATUS: fixed
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
FIX: CLAUDE.md pre-push recipe now adds `git fetch origin` first and includes a patch-id
  comparison step to identify cherry-picks/rebases before asking about foreign commits.
  Validated by amux-cloud.

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
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-228
SYMPTOM: `?field=enabled&limit=500` returns all 459 rows, 570 KB, fields
  `['command','created','deleted','done_action']`. The filter did nothing and the response
  looks like a successful filtered result. `?id=` filters correctly, so filtering exists.
COST: A caller scoping to one field and counting rows gets a confident wrong answer with
  no tell — the same failure class the endpoint was built to fix. Also 87% of the payload
  is `command` diffs while `enabled` is 1%, on a mobile-first PWA.
FIX: Already fixed in amux-server.py lines 64680-64710: `?field=` is now honoured as a
  WHERE clause filter, unknown params are rejected with 400, and large old/new values are
  truncated in list view (unless `?full=1` or `?id=` is provided).
  Validated by amux-cloud.

## The browser driver drops to `backend: cli` mid-session and every eval returns null
AREA: browser
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-233
SYMPTOM: After a few `/api/browser/action` evals the backend silently changes from
  `driver` to `cli`, `location.href` becomes `about:blank`, and results come back empty.
  Happened three times in one visual-review pass; amux hit it twice the same night.
COST: A UI review that needs more than ~4 steps cannot be completed. Two of the three
  questions I was asked to answer about the Scope tab went unanswered — not because the
  feature was fine, but because the rig died mid-pass.
FIX: `_bu_eval` now checks if a driver existed and died (`session in _bu_drivers` but
  `_bu_active_driver` returns None) and returns an explicit error with `backend:
  "dead-driver"` instead of silently falling back to CLI. The error tells the caller to
  restart with POST /api/browser/start. Code correct by inspection (amux-cloud validated
  the logic) but the loud path has not been exercised — reproducing it requires a driver
  that existed and died, and the only live driver belongs to another session.
  STATUS NOTE: fixed-unverified — awaiting next natural driver death to confirm.

## A reviewer who BLOCKS a card is re-nudged forever
AREA: notices
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-234
SYMPTOM: Blocking a card is a completed review action, but the card stays in `review`, so
  the sweep re-selects it every cycle. I was nudged three times for one card I had already
  reviewed and blocked, with the analysis sitting on it.
COST: The re-nag becomes the loudest signal in the room while the actual defect sits
  untouched, and only the author can clear it. Trains reviewers to ignore review nudges,
  which is the one class that should never be ignored.
FIX: e20a112 — the advance loop now checks interaction_log for deliberate reviewer writes
  (patch/status_update/gate_force). If the reviewer's most recent deliberate write is newer
  than any other party's, the nudge is suppressed ("ball is with the author"). Fail-open
  on errors so a broken check never silences real review requests. AC-234 reviewed and
  closed by amux-frustrations.
  Validated by amux-cloud.

## The `verified` gate requires e2e, and every e2e path runs against the cloud host
AREA: gates
SEVERITY: blocks
STATUS: fixed
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
FIX: Gate text changed from "CI/CD green (incl. e2e)" to "CI/CD green (if e2e infra is
  unavailable, note why -- that is not a failure)". A session can now honestly satisfy the
  gate when e2e cannot run by noting the reason, rather than being blocked.
  Validated by amux-cloud.

## Board issues do not auto-progress during idle periods
AREA: board
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2442
SYMPTOM: When a session is idle, board issues that could advance (e.g., `todo` → `doing`,
  through workflow stages) do not auto-progress. Session page came back up after being down,
  suggesting idle time required manual intervention or restart to resume progression.
COST: Idle periods become stalled time; planned workflows pause. If there is a designed
  progression strategy for unattended cards, it does not execute.
FIX: amux-frustrations initially closed as by-design (AF-2), checking only that the
  mechanism existed. amux validated and found the CALL SITE gate was edge-triggered
  (prev != "idle"), so a lane already parked idle was never re-evaluated. The level-
  triggered sweep (_pickup_level_sweep) carried only the pickup half, not the advance
  half. Fixed by amux in AMUX-2442: the sweep now calls _advance_open_card before
  pickup, matching the edge's two-call sequence. Verified in prod — 7 stalled lanes
  were nudged on first run.
  Validated by amux.

## Auto-deploy only fires on `amux board done`, not on session idle/stop
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-homepage
CARD: AH-70
SYMPTOM: Committed the AEO graph-agents page and ran `amux board done AH-67`. The
  PostToolUse Bash hook matched and pushed — but by then the session had already gone
  idle for 30+ minutes with the commit sitting unpushed. The page was live on GitHub
  only after manual CI re-run. The hook only matches Bash commands containing
  "amux board done" or "api/board.*status.*done", so any idle period between the last
  commit and the done-call leaves changes stranded.
COST: The AEO page was committed and ready but unreachable for 30+ minutes.
  A second CI run was needed (the first had already timed out before the push happened).
  The user checked the URL twice and reported "still not live". ~45 min of unnecessary
  delay on a page deployment that should have been instant.
FIX: Already fixed. `.claude/settings.json` now has a Stop hook calling
  `auto-deploy.sh --on-stop`, which pushes unconditionally on session end/idle (bypassing
  the board-done trigger check). The PostToolUse Bash hook remains for immediate push on
  board-done.
  Awaiting validation by amux-homepage (idle).

## A review PATCH using `desc` silently DELETED the author's entire card content
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-236
SYMPTOM: amux-gtm reviewed AC-216 and AC-231 with a PATCH carrying `desc`, which replaces.
  Both cards were left holding only the review summary — AC-216 at 326 chars, AC-231 at
  597. Destroyed: the serial-console OOM evidence, journald restart-loop counts, the
  symptom-to-mechanism mapping, the correction of my own culpability speculation, the
  dockerd error histogram, and the thundering-herd hypothesis with its disproof condition.
  `desc_append` exists and is not what a reviewer reaches for.
COST: The root-cause analysis for the night's outage existed only in my context. Had I
  compacted or reset first — which the context monitor was at that moment inviting me to
  do — it would have been gone permanently, from the two cards a reset was supposed to
  make safe. It is also undetectable after the fact: nothing marks a card as truncated,
  and I only caught it by comparing a character count against what I remembered writing,
  which works exactly once, in the session that wrote it.
FIX: Already fixed in amux-server.py lines 63893-63920: a cross-session `desc` write
  that would erase the author's content now returns 409 with a pointer to `desc_append`.
  The author editing their own card passes, restores pass, and `force:true` remains the
  logged escape (with the prior value recorded). AC-236 already marked done on the board.
  Validated by amux-cloud.
