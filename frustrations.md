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

## `git add amux-server.py` on a shared checkout ships another session's uncommitted hunk under your message
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2443
SYMPTOM: I made an edit, ran the checks, and went to stage it — `git status --porcelain`
  came back EMPTY and `git diff amux-server.py` showed nothing, seconds after a probe had
  confirmed my change was in the working tree and not in HEAD. It had been committed by
  someone else: `24a294b` "fix(task-guard): a lane whose whole queue is blocked is not
  delinquent (AC-240)" by amux-cloud, 79 insertions, of which ~30 were my unrelated
  advance-sweep change (AMUX-2442). Their `git add amux-server.py` takes the whole file,
  not their hunks.
COST: No lost work and the combined commit is green (224 tests), so the cost is entirely
  in the trail: `git log -S` on the advance sweep lands on a commit message about
  task-guard, and the two changes — both touching the idle/nudge path — were never tested
  independently of each other. Also ~10 minutes reading git state that looked like the
  "lost edit" failure from earlier in this session before the real cause was clear. The
  mirror case is what makes it structural rather than a one-off: I had used
  `git apply --cached` earlier the same day specifically to avoid doing this to
  amux-cloud's in-flight AC-233 work in this same file, so the discipline is real, it is
  just not enforced anywhere and one session forgetting it is enough.
FIX: amux ALREADY KNOWS the answer — the co-edit notice ("Commit <sha> by session <X>
  touched files you also edited recently") is generated from data the server holds. It
  just fires AFTER the commit, which is the one moment it cannot help. Move the same
  check earlier: `scripts/git-hooks/pre-commit` asks the amux API which other sessions
  have edited the staged paths recently, and warns (not blocks) when you are staging a
  whole file that someone else is live in, naming them and pointing at the
  `git apply --cached` recipe already in CLAUDE.md. No new primitive — filesystem plus
  messages, surfaced at the moment of the decision instead of after it.
NOTE: This is the THIRD `AREA: attribution` entry filed on 2026-08-06, after AC-227
  (passenger check reads an upstream cherry-pick as foreign forever) and AC-230 (co-edit
  notice named the reporting session, not the author). All three are shared-checkout
  commit provenance, and all three are downstream of one fact: N sessions share one
  working tree and git has no concept of which session owns a hunk. Per this file's own
  thesis, three entries in one AREA is the argument that the thing needs designing rather
  than patching — that design is worth doing before a fourth.

## The staged-guard was never tracked, and the installed pre-commit was months stale
AREA: attribution
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2444
SUPERSEDES: the FIX field of the entry above (AMUX-2443), which proposed building a
  pre-commit cross-session warning. That warning already existed and had been running the
  whole time — I found out by committing and watching it fire at me. The entry's SYMPTOM
  and COST stand; its FIX was written from a wrong premise and this entry replaces it.
SYMPTOM: Three defects stacked, each hiding the next.
  1. `amux-staged-guard` existed ONLY in `.git/hooks/` — which git does not track. It was
     absent from `scripts/git-hooks/`, so a fresh clone runs install-hooks.sh, gets a
     pre-commit whose guard call finds nothing beside it, and proceeds.
  2. The tracked pre-commit's guard call was `if [ -x "$g" ]; then ...; fi` — no else. So
     that clone's cross-session protection is off SILENTLY. The installed copy had a loud
     "staged-guard MISSING ... protection is OFF" warning; the tracked copy had regressed it.
  3. `.git/hooks/pre-commit` differed from `scripts/git-hooks/pre-commit`, and the drift was
     security-relevant: the AC-239 secret patterns added to the tracked hook earlier TODAY
     (`sk_(test|live)_`, R2/AWS secrets, CLERK_SECRET_KEY, Slack, GitLab) were `installed=0`
     for all of them. install-hooks.sh had not been re-run.
COST: My commit 8e102eb printed "Security scan passed" from a scanner that could not match
  a Clerk key, a Slack token, or a GitLab PAT — i.e. the exact class of credential AC-239
  was filed for, where four real ones including live R2 keys sat committed in a public repo
  since 2026-03-11. The fix for that leak was written, tested, committed, and inactive on
  the machine where commits are actually made. CI had it; the first line of defence did not,
  and it said PASSED. That is the "check that cannot fail" shape at its worst, because the
  green came from the gate itself.
  Defect 1 is also a textbook one-level-down repeat: the tracked hook's own comment explains
  that the guard CALL was made unconditional because it had previously been hand-added to
  the installed copy only — the author fixed the caller and left the callee untracked, so
  the identical failure survived directly underneath the comment describing it.
FIX: card AMUX-2444, landed with this entry — `amux-staged-guard` is now tracked in
  `scripts/git-hooks/`; install-hooks.sh
  installs BOTH files and then `cmp`s each against its source, failing loudly on drift
  rather than printing a success line; the tracked hook's else branch announces a missing
  guard. Verified by running the conditional with the guard absent (warning fires) and
  present (silent, exits 0), and by confirming all four AC-239 patterns are now
  `installed=1`. The remaining gap is AMUX-2443: on a single-file project amux-server.py is
  always "shared", which the guard only NOTEs, so the sweep that started all this is still
  possible — that one is open on purpose.

## A generated hook file said nothing about being generated, so it got committed and froze
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2444
SUPERSEDES: the entry above of the same card id, whose SYMPTOM and FIX were built on a
  wrong premise. That entry says `amux-staged-guard` "existed ONLY in .git/hooks/ — which
  git does not track" and treats that as an oversight to be corrected by tracking it. It is
  not an oversight. Only the third defect in that entry — the stale installed pre-commit
  with the AC-239 patterns inactive — was real, and that part stands.
SYMPTOM: `.git/hooks/amux-staged-guard` is written per work_dir by
  `_install_amux_precommit_guard()` from `_AMUX_GUARD_BODY` in amux-server.py, and the
  server injects its own shim into whatever pre-commit exists. The file carries no banner
  saying any of that. Reading it gives you a plain, sensible, untracked script and no
  evidence at all that it is output.
COST: I concluded it had gone untracked by mistake, committed a copy into
  scripts/git-hooks/, and pointed install-hooks.sh at it. That made the installer a SECOND
  PRODUCER of a generated file, frozen at the moment I copied it — so running it overwrote
  the live guard and reverted amux-cloud's AC-241 improvement, which had shipped about an
  hour earlier. It would have reverted every future one too. It also caused the guard to run
  twice (server shim + my call), and I misread that double-print as a peer hand-editing the
  installed copy and told them so — a wrong accusation on top of a wrong fix.
  The tell I did not have: the sibling installer for the OTHER generated guard already warns
  "edit amux-server.py, not the installed copy" — but it warns at INSTALL time, into the
  server log, which is not where someone reading the file is looking. Rule 4's second layer:
  the evidence existed and was not where the reader was.
FIX: 8443cd9 — `_AMUX_GUARD_BODY` now opens with "GENERATED FILE — DO NOT EDIT, AND DO NOT
  COMMIT IT TO A REPO", naming the source symbol, the writer function, that local edits are
  silently replaced, and why it is untracked on purpose. Revert of the bad fix in fe86e63;
  install-hooks.sh now installs only the pre-commit it owns and REPORTS whether the server's
  guard is present rather than producing one. Verified on a real restart: server re-injected
  its shim, invocations back to 1, installed guard carries AC-241's text, AC-239 patterns
  still active.
NOTE: this is the 5th AREA-clustered entry today on shared-checkout provenance (4 under
  attribution, this one under instruments because the defect is the missing banner). All are
  downstream of the same fact — N sessions share one working tree and git has no concept of
  which session owns a hunk. Every fix so far, mine included, is a better WARNING about a
  condition git cannot represent. "All fixed" should not be read as solved.

## A wrong field name on /api/sessions is indistinguishable from the data not existing
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2447
SYMPTOM: gtm-videos needed each lane's working directory to test whether a cwd change had
  orphaned a memory index. They read `/api/sessions` and got None, and reasonably concluded
  the API does not expose cwd — recording it as a limit they could not work around. The field
  is there; it is called `dir`. There is no `cwd` key at all, so `.get("cwd")` returns None,
  identical to what a present-but-empty field would return.
COST: A confirmable hypothesis was left unconfirmed and written up as untestable, and the
  slug — which encodes the cwd and was sitting in front of both of us — went unused as
  evidence for hours. I then repeated the same shape from the other side: I asserted the
  overcounting detector was "the pushed tip" that "should not sit long" without reading the
  ref. It was never pushed. Both of us reasoned confidently about state neither had measured.
FIX: `GET /api/sessions/contract` now exists (line 62434), derived from a live payload rather
  than hand-listed. Fields, descriptions, `undocumented`/`documented_but_absent` arrays make
  staleness visible. Scoped to the caller's tag isolation. No alias — the class is closed for
  every consumer, not just `cwd`.
NOTE: the general shape is worth naming because it recurs — a probe that cannot express
  "you asked the wrong question" returns something indistinguishable from an answer. Same
  family as the empty grep that reads as a measurement, and as `git status` after a commit,
  which cannot distinguish "nothing of theirs was there" from "I swept all of it". In each
  case the failing check produces a green, plausible result and nothing prompts a recheck.

## An unimplemented gateway admin route answers 503, not 404, and wakes a container doing it
AREA: cloud
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud-demos
CARD: AC-235
SYMPTOM: `DELETE /api/gateway/admin/orgs/<id>` does not exist — org teardown is
  `DELETE /api/gateway/orgs/<id>`, with no `admin` segment. But the gateway has no
  catch-all for `/api/gateway/admin/*`, so the request fell past every admin handler
  into the container proxy, which called `_ensure_container_starting` and answered
  `{"error":"starting"} 503`. Five DELETEs, five identical 503s.
COST: Two full rounds of misdiagnosis pointed at the wrong subsystem. The host had
  genuinely been sick for hours (container thundering herd, AC-231), so a 503 was
  exactly the shape of the failure I was already fighting, and I read it as "the herd
  is still saturating the box" — while GET on the same admin API was returning 200 in
  4.1s and the box was idle at 0 running containers. I reset the instance and rewrote
  the boot fix twice before noticing the contradiction between a stable GET and a
  failing DELETE against the same service. The route had never existed at any point.
FIX: `b96510b` — unmatched `/api/gateway/admin/*` now returns 404 naming the method,
  the path, all 11 real admin routes, and a hint pointing at the correct teardown
  route. Control-plane paths are never proxied to an org container.
NOTE: the sharp edge is that 503 is HEALTH-SHAPED. A 404 says "you asked for something
  that isn't there" and is self-correcting in one round; a 503 says "this service is
  unwell", which is unfalsifiable from the client and, when the service HAS been unwell,
  corroborates the wrong theory instead of contradicting it. An error that mimics the
  outage you are already investigating is worse than a silent failure — it does not just
  fail to inform, it actively confirms. Same family as the ethos's loud-wrong probe: the
  answer arrives, looks plausible, and nothing prompts a recheck. When adding a route
  namespace, add its catch-all in the same commit; the fallthrough target is whatever
  happens to sit below, and here that was a side-effecting container start.

## The gate's own "wrong type?" hint recommends two types the server rejects
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-249
SYMPTOM: A blocked transition prints `cli_wrong_type: amux board type <ID>
  <chore|task|doc|research|ops|decision|investigation>`, and `amux board --help` prints the
  same set. Two of those seven are not valid types. `amux board type AC-248 decision`
  returns `{"error":"unknown type 'decision'","valid_types":["code","escalation","blocker",
  "investigation","ops","research","chore","doc","tripwire","watch"]}`. `task` is not in the
  valid list either. Conversely `escalation`, `blocker`, `tripwire` and `watch` are valid but
  never suggested, so the hint is wrong in both directions at once.
COST: Retyping a card is the sanctioned escape from a gate that does not fit — rule 3's
  "fix the type, not the truth". I followed the hint verbatim, it failed, and the failure was
  silent in a way that mattered: `amux board type` errored while the `amux board done` in the
  same breath had ALREADY written its `--outcome` (outcome writes before the gate check, by
  design). So the card sat typed `code`, gated on "Implemented and merged", with its outcome
  text already committed — and the obvious retry would have appended the whole outcome a
  second time. I checked the card before re-running and avoided it, but the safe move is not
  discoverable from the error.
FIX: Closed in two passes on two surfaces. b1c3e93 (amux-cloud, AC-249): derived
  `cli_wrong_type` from `_ITEM_TYPES`, published `valid_types` on the 409, excluded the
  card's own type, and pointed both CLIs at the server line. 32b9d14 (amux, AMUX-2479):
  added `valid_types` to the fields payload and made the CLI usage line render from the
  server too. The gate hint path (what an agent hits when blocked) was b1c3e93; the usage
  line path was 32b9d14. Both now derive from the authoritative list.
  Validated by amux-cloud.
NOTE: this is the ethos rule-7 shape where the SANCTIONED INSTRUCTION is the theatre — same
  family as `amux board claim` (AMUX-2140), which did not exist, fell through to help text and
  exited 0. That one was worse because it reported success; this one fails loudly, which is
  why it cost minutes rather than a wrong belief. Both come from the same source: text telling
  an agent what to run, never exercised against the thing that runs it.

## `amux board needsyou` adopted a --flag as the question and reported success
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2455
SYMPTOM: `amux board needsyou <id> --stdin < file` printed `AMUX-2454 → tagged needs:you`,
  exit 0. The verb takes a free-prose ask as `$*`, so the unrecognised flag became the ask:
  it discarded the piped file entirely and wrote the literal line `NEEDS-YOU: --stdin` onto
  the card. Every status verb in this CLI takes `--stdin`/flags, so reaching for it here is
  the obvious move, and nothing said otherwise.
COST: Two cards (AMUX-2446, AMUX-2454) carried a garbage ask, and on AMUX-2454 it destroyed
  the herdr measurement that was the entire point of the card — the deliverable was gone
  and the card looked written-to. I did it TWICE before noticing, and only noticed because
  a fingerprint check I ran for an unrelated reason showed the desc at 18 bytes. Reporting
  success while recording garbage is worse than refusing: the ledger looks populated and a
  reviewer believes it.
FIX: In `amux`, needsyou now refuses an ask beginning with `--`, naming the two correct
  forms (prose inline, or `amux board progress <id> "$(cat file)"` for long text). Exercised
  both directions: `--stdin` refuses with usage; a real prose ask still records. Both
  polluted cards cleaned, with the measurement verified intact afterwards.
NOTE: this is the sibling of AC-222, where the same verb printed a status arrow for a write
  that changes no status. Two entries now on `needsyou` specifically, both about the command
  reporting something other than what it did — which is the argument for auditing its
  output contract rather than patching the next symptom.

## The decompose nudge told me to patch three cards I had already closed
AREA: notices
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-252
SYMPTOM: "[amux] 3 of your prompts are captured on the board but not yet decomposed into
  real cards: AC-243, AC-244, AC-246 ... PATCH THESE IDS SPECIFICALLY." All three were
  already `done` with their own outcomes, and AC-244's two children (AC-247, AC-248) had
  been created and closed. Timestamps settle it: emitted 14:53:07 when all three genuinely
  WERE todo; closed 15:03:17, 15:03:32, 15:19:41; delivered ~26min after the last close.
  True when written, false when read. The predicate was never wrong — both the server
  fastpath and the client badge filter status='todo' correctly.
COST: Low in minutes, high in what it nearly caused. The instruction is imperative and
  specific — PATCH THESE IDS — so complying literally means writing a fresh outcome onto
  three cards that already carry their own. That is exactly the misattribution the
  message's own last line warns about ("each carries its own, or the ledger records work
  against the wrong unit and a reviewer believes it"). A worker trusting the nudge over
  the board corrupts the ledger the nudge exists to protect. I checked the cards first and
  found them closed, but nothing in the message suggests checking.
FIX: `c32cf8a` — the nudge now passes guard="decompose:<ids>" and `_steer_guard_stale`
  rechecks the NAMED ids at delivery, dropping the message only when none is still a live
  todo (a partial decomposition still gets chased). The guard framework already existed for
  this and has since AMUX-1737; this caller simply never opted in.
NOTE: the general shape is a nudge asserting a fact with a shorter shelf life than the
  queue's delivery latency. Delivering at the turn boundary is the RIGHT grain (the
  no-global-pub-sub decision in ethos.md), which means the fix is never faster delivery but
  revalidation at the moment of speaking. Worth auditing every other _steer_enqueue caller
  that states a fact rather than asks a question — that is what AC-252 is for. Also worth
  recording: my first verification reported the control as stale, and the CONTROL was wrong,
  not the code — I selected it with status='todo' and no `deleted IS NULL`, so I picked a
  deleted card. The same missing-predicate mistake in the probe that the guard fixes in the
  product, one layer down, which is the nesting ethos rule 1 describes.

## The co-edit notice asks the reader to resolve a condition it is better placed to check
AREA: notices
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2456
SYMPTOM: "Commit <sha> by session X touched files you also edited recently... If you had
  UNCOMMITTED changes there, they are in that commit now" — then tells the reader to go
  check. It never checks. Same hypothetical AC-241 removed from the PRE-commit guard, which
  now prints the staged line count so the committer can compare against what they wrote.
COST: Six notices to me in one afternoon (24a294b, 9450b38, 90ac2e2, b96510b, c32cf8a,
  7504abf), six verification round-trips, zero true positives FROM THE NOTICE. The one real
  sweep today — 36 lines of mine inside 24a294b — I found independently by reading git
  state; the notice arrived after. It fires on FILE overlap, and in a single-file repo that
  is every peer commit, so the base rate is ~100% and the informative rate is ~0.
  The round-trips are not the real cost. A notice that is almost always a false alarm trains
  the reader to skim it, and the one time it is real is the time it gets skimmed.
FIX: Fixed in commit-report handler (line 71663). The notice now compares the recipient's
  first-hand Edit/Write tool_use evidence against their own last commit (via Amux-Session
  trailer). Edit-before-commit = suppressed (nothing outstanding). Edit-after-commit = alarm.
  Mtime-inferred edits can fire the notice but never produce the assertion (authorship not
  established). Three rounds of refinement (false positives from mtime, timestamp from wrong
  map, --grep substring match). The routine case is now silent, making the notice's arrival
  itself the signal.
NOTE: third entry on this one notice, after AC-230 (named the reporting session, not the
  author) and AC-241 (the pre-commit sibling's hypothetical). Three fixes on one message is
  the file's own threshold for designing rather than patching.

## `amux board --outcome` had no stdin path, so an outcome quoting a command was shell-eaten
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-255
SYMPTOM: `amux board review AMUX-2456 --outcome "... resolved it with \`--grep=Amux-Session:
  <session>\`, which is a SUBSTRING search ..."`. The shell evaluated the backticks before
  amux saw them. What landed on the card was "resolved the last commit with , which is a
  SUBSTRING search" — the command gone, the sentence dangling. bash printed its own syntax
  error on a separate line while `AMUX-2456 → review` printed success, so the transition
  read as clean.
COST: A review handoff went out to a peer naming a bug class, with the exact command that
  CAUSED the bug deleted from the explanation — the one detail the reviewer needs. Caught
  only because I re-read the card afterwards; nothing in the output said the text had been
  altered. Cheap to repair here, but the same shape silently truncates any outcome that
  quotes a command, which is most of the good ones.
FIX: `72c2470` — `--outcome-stdin` and `--outcome-file`, mirroring what `send` has had since
  AMUX-1888 and what `board add` already had. Verified on a scratch card: backticks,
  `$(date)` and `$HOME` all land verbatim.
NOTE: the rule against inline double-quoted text is already written down in CLAUDE.md, for
  `amux send`, with an incident behind it. I violated it on a DIFFERENT verb, and the reason
  is worth recording because it is not carelessness: the sanctioned escape existed for the
  two commands whose whole payload is prose, and not for the one field that most often
  quotes shell — a session recording what it ran and what sha came out. When the safe path
  is missing exactly where the dangerous input is most likely, "remember the rule" is not
  the fix; the missing flag is. Same shape as AMUX-2325, where the gate's only honest exit
  was unwalkable from the audited path. Also the same habit-transfer failure ethos rule 7
  names: I had used --stdin correctly for `amux send` twice in this session and did not
  carry it to the adjacent verb.

## Editing amux-server.py silently disables the guard that protects amux-server.py commits
AREA: git
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-06
SESSION: amux-cloud
CARD: AC-261
SYMPTOM: `_has_cotenants` in the generated staged-guard hook does `except Exception:
  return False`. When the amux server is unreachable the co-tenant check is skipped and
  nothing is printed — output identical to "checked, you have no co-tenants". The server
  re-execs on every save of `amux-server.py`, so editing that file is itself what makes the
  server briefly unreachable. The file where a co-edit sweep is most likely on this shared
  checkout is the one whose editing turns off the check for it.
COST: `b1c3e93` swept ~93 lines of another session's uncommitted work (their
  `_inherited_instruction_files` and memory-inherited handler) into my commit under my
  message and my `Amux-Session` trailer, with no warning shown. Not recoverable by rewrite:
  `git reset --soft HEAD~1` is the right tool and the shared-checkout guard correctly
  refuses it, because moving shared HEAD decapitates other sessions' commits. So the peer
  gets a disclosure and a misattributed commit instead of a clean history.
FIX: `5865401` — the skip now announces itself, naming what was not checked and what to run
  instead. Fail-open preserved deliberately: blocking every lane when the server is down is
  worse than missing a warning. The change is visibility, not behaviour.
NOTE: the AC-241 numstat was already on screen when I did this. It printed "114 insertions"
  against an edit I knew was ~20 lines and I committed anyway. That is the argument for why
  the fix here is a SKIP NOTICE rather than a better number: a figure the reader has learned
  to skim does not become informative by being correct. What was missing was not a more
  accurate measurement but a statement that the measurement had not been taken. Same family
  as the ethos rule-4 point that a skip leaving no trace is indistinguishable from a scan
  that found nothing — and this is the third entry today whose root is that a signal could
  not distinguish two sessions or two states (see AC-256).

## /api/board/contract published 4 gate layers while the resolver enforced 5
AREA: gates
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: AMUX-2477
SYMPTOM: `GET /api/board/contract` → `gates.how_they_resolve` listed four layers
  (card > type > worker > global). `_effective_gate` resolves five — the `group:` tier
  shipped days earlier and neither the contract nor `_effective_gate`'s own docstring
  picked it up. Separately, `GET /api/board/gates` (the plural, and the natural guess
  for "show me the gate rules") fell through to `/api/board/<id>` and answered
  `{"error":"item not found"}` — a reader asking about gate RULES told their CARD does
  not exist. A comment in the contract handler shows another session already burned a
  guess on that same path.
COST: a done→verified sweep of ten cards (AMUX-2310..2332) quoted the GLOBAL per-status
  default as each card's gate and wrote ten wrong lines onto real cards, then wrote a
  long "evidence for the gate decision" note on AMUX-2466 built entirely on the wrong
  premise. Eight of the ten were under a `group:amux` PEER-REVIEW gate that never
  mentions prod, so the sweep's central claim — "blocked because cloud.amux.io returns
  401 and I cannot exercise prod" — was answering a question nobody asked. The other two
  were under their type gate, so the opposite claim ("unsatisfiable by construction for
  research/investigation") was also false. Worse than the wrong lines: the note
  recommended a per-type gate change to Ethan when the peer-review gate he had asked for
  was ALREADY LIVE for group amux, which would have put a decision in front of him that
  had already been made.
FIX: `ba802de` — `_GATE_PRECEDENCE` is the order once; the contract renders from it and
  the docstring points at it. Contract leads with AUTHORITATIVE (`GET /api/board/gate?
  item=&status=`) and publishes `active_overrides` (the group/worker gates actually SET,
  not just the fact the tiers exist). `/api/board/gates` answers instead of misdirecting.
  `tests/test_gate_contract.py` pins resolver scopes == published keys and self-tests by
  seeding the missing group tier.
NOTE: the reusable shape is not "docs go stale". It is that naming a TIER without its
  CONTENTS reads as confirmation the tier is empty — the contract said a per-session
  override layer existed, I checked nothing was set for my worker, and concluded no
  override applied. That is the ethos rule-1 corollary (a view must share the predicate
  of the mechanism it describes) in its under-filtering direction: the view was not
  wrong about what exists, it was silent about what is SET, and silence read as zero.
  Same family as the three entries above whose root is a signal that could not
  distinguish two states — this is the fourth in AREA gates/instruments this week, which
  is the argument that the "publish a rule, hand-type it elsewhere" pattern needs to go,
  not just this instance of it.

## `amux board` could not set `reviewer`, the field that gates review -> done
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-06
SESSION: amux
CARD: BACKE-3183
SYMPTOM: `amux board review BACKE-3183 --reviewer backend` -> `error: amux board: unknown
  flag '--reviewer'`. The usage line lists `--checked --ack --type --override-doing
  --trigger` and there is no reviewer verb or flag anywhere in the CLI (`grep -n reviewer
  amux` returned one hit, in an unrelated comment). The server accepts `reviewer` on PATCH
  and has all along, so nothing looks broken from the API side.
COST: minutes here, but the shape is the cost. review->done is BLOCKED on a named
  reviewer's sign-off — I hit that refusal on AMUX-2311 earlier tonight ("review sign-off
  required from the reviewer"). So the only way to route a card for review was a raw
  `curl -X PATCH`, which carries no X-Amux-Session. The review gate exists precisely to
  make cross-session sign-off ATTRIBUTABLE, and the missing verb was manufacturing the
  unattributed writes it depends on.
FIX: `c2d57ed` — `amux board reviewer <ISSUE-ID> <session>` (pass `none` to clear), same
  shape as the `type` verb. Verified by reading the field back rather than trusting the
  success line: `reviewer = 'backend'`.
NOTE: this is the SECOND instance of AMUX-2325's exact defect, and the `type` verb's own
  comment — four lines above where I added this one — documents the first. One instance
  is a missed verb; two says the rule should be inverted: every gate-relevant FIELD needs
  an attributed setter by default, and the audit should be "which fields can a gate block
  on, and does each have an `amux board` verb?" rather than waiting for a session to trip
  over the next one. `depends_on` and `owner_type` are both gate-relevant and both
  currently unsettable from the CLI — same trap, unsprung.

## `amux board type` advertised two types the server rejects, and omitted one it accepts
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-07
SESSION: amux
CARD: AMUX-2479
SYMPTOM: `amux board type AMUX-2479 decision` -> `{"error": "unknown type 'decision'"}`. I picked
  `decision` from the CLI's OWN usage line, which read `types: code task chore doc research ops
  investigation decision escalation blocker watch`. The server's `_ITEM_TYPES` is `code escalation
  blocker investigation ops research chore doc tripwire watch` — so the CLI advertised `task` and
  `decision` (both rejected) and omitted `tripwire` (accepted). Two hand-maintained copies of one
  list, drifted.
COST: small in minutes, but it is the worst shape of help: confidently wrong, and it points you at
  your CARD rather than at the tool. The server's error is good (it returns `valid_types`), so the
  recovery was fast — without that it would have read as the card being in a bad state.
FIX: `32b9d14` — the server publishes `fields.valid_types` machine-readably (the existing prose
  `"One of [...]"` was not parseable), and the CLI renders its usage line from that with a fallback
  when the server is unreachable. Verified live: usage now prints `tripwire`, and greps clean for
  `task`/`decision`.
NOTE: fourth instance tonight of ONE fact maintained in two places and drifting — after the gate
  contract vs the resolver (AMUX-2477), the nudge's selection vs its target-status ternary, and the
  reviewer sign-off requirement vs its routing (both AMUX-2478). backend named the remedy hierarchy
  that covers all four: extract the predicate when both sites share a codebase; GENERATE one side
  from the other when they are different artifacts (this entry is that tier — a bash CLI and a
  Python server cannot share a constant, so the list is served); tripwire the pairing when neither
  works. The generalisable line is theirs: every "keep these in sync" comment is a defect report
  about the code's shape, filed in advance.

## Pre-push "is this commit mine?" checks use %an, which every session shares
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-07
SESSION: amux
CARD: AMUX-2480
SYMPTOM: `git log --format='%an' origin/main..main | sort -u` returns exactly `Ethan
  Steininger` — for every commit, from every session, always. Every lane on this machine
  commits under one git identity, so any author-based check has ONE possible value and
  cannot distinguish own work from a peer's. amux-cloud hit the identical thing the same
  day with `git log --oneline --author="$(git config user.name)"`, which is worse in the
  same way: an `--author` filter READS as narrowing, and here it is a no-op that returns
  a confident, targeted-looking wrong answer. The real discriminator is the
  `Amux-Session` trailer, which amux already stamps on every commit.
COST: I reported "all commits unpushed are yours, none foreign" to Ethan repeatedly
  across a long session. It was false: 4 of 26 belonged to amux-cloud, three of them
  touching `amux-server.py`, which is the path that fires deploy-cloud.yml and
  cloud-image.yml. Had he said "push" on the strength of that, a peer's unreviewed work
  would have deployed to cloud.amux.io at a moment they did not choose — the exact
  incident the Deploy section of CLAUDE.md exists to prevent, walked into by following
  its instruction to check with an instrument that cannot answer. amux-cloud only caught
  their instance because a COUNT looked implausible, not because the check complained.
  (They consented when asked, so nothing shipped wrongly; the check is what failed, not
  the outcome.)
FIX: Two changes. (1) The pre-push guard already read the trailer (line 20926) but silently
  passed untrailered commits — now collects them under "(no Amux-Session trailer)" and blocks
  on them the same way it blocks on foreign-session commits. (2) CLAUDE.md's deploy recipe
  changed from `git log --oneline` (which shows only %an — identical for every session) to
  `git log --format="%h [%(trailers:key=...)] %s"` with a comment explaining why %an is
  meaningless on a shared checkout and that `[]` = untrailered = treat as foreign.
NOTE: second entry today whose root is an instrument that returns a plausible answer
  while discriminating nothing (see the action-name trap on AMUX-2479: 6,814 `patch` rows
  vs 137 `status_update`, so keying on the action name would have matched ~everything).
  Both are the ethos rule-7 shape — "a filter that silently matches EVERYTHING is the same
  defect as one that matches nothing, except it returns a confident wrong answer instead
  of silence" — and both were caught by a number looking wrong, not by the check failing.
  Two independent sessions, same root, same day: that is the argument this belongs in the
  guard rather than in anyone's discipline.

## Column delete and modal close share the ✕ affordance — a dismiss loop hit a destructive confirm
AREA: board
SEVERITY: annoys
STATUS: open
DATE: 2026-08-07
SESSION: amux
CARD: AMUX-2491
SYMPTOM: driving the cloud board in a browser, I dismissed onboarding modals with a blanket
  `document.querySelectorAll("button")` loop clicking anything whose text was `✕` or `Skip`.
  It reported `{"dismissed":26}`. One of those 26 was a board COLUMN's delete control, and the
  next screenshot showed `Delete "Summary" column? Items will move to To Do.` with Delete and
  Cancel. I cancelled; all 10 Wexus columns and all 9 issues verified intact afterwards via the
  API, so nothing was lost.
COST: a near-miss, not a loss — but the loss would have been a customer demo env's custom
  workflow column (`summary`) plus the silent relocation of its cards to To Do, discovered by
  nobody until a prospect opened the board. The blast radius was three envs' worth of
  hand-built demo state and the only thing between it and a live confirm dialog was that I
  screenshotted before clicking again.
FIX: two halves, and the first is mine — never write a blanket click-everything-matching loop
  against a live UI; enumerate and check what each target IS before clicking. The amux half is
  real too and is why this is logged rather than swallowed: a DESTRUCTIVE control (delete this
  column, cards get moved) is rendered with the same `✕` glyph as DISMISSIVE controls (close
  this modal). Nothing in the affordance distinguishes "close this thing" from "destroy this
  thing and relocate its contents", so any agent — or any human clicking fast — can hit it.
  Column delete should be behind a distinct affordance (an overflow menu, or a trash glyph),
  not the same character as every close button on the page.
NOTE: same family as the night's other entries, one layer out. Every one of those was an
  instrument that could not distinguish two states; this is a CONTROL that does not distinguish
  two intents. The lesson generalises the same way: if two things that must never be confused
  look identical at the point of use, something will eventually confuse them, and being careful
  is not a mechanism.

## Assignment notices arrive for cards that were deleted a second after being created
AREA: notices
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-07
SESSION: amux-cloud
CARD: AC-284
SYMPTOM: "New board task assigned: AC-284 — [scratch] foreign-owned archive guard probe —
  delete me. Run `amux board claim AC-284` to take it." The card had already been deleted.
  `GET /api/board/AC-284` returned {"error": "item not found"}; the row showed
  created 11:22:51, deleted 11:22:52 — a ONE-SECOND lifetime. AC-285 repeated it within
  the hour. Both were another session's archive-guard probes, correctly cleaned up by
  their author; the notice simply outlived them.
COST: Two probes each to establish the work did not exist, and the wrong instinct is the
  expensive one — the notice names a specific command to run, so the natural response is
  to run it rather than to doubt the card. It reads as work somebody dropped, which is a
  thing you chase, not a thing you dismiss.
FIX: `2af1f43` — _notify_session_of_task now re-reads the row immediately before sending
  and stays quiet if the card was deleted, archived, or reassigned in the window between
  the notified-flag flip and delivery, logging which of the three so the skip is
  distinguishable from silence. Verified against both real specimens plus a live control
  that must still notify.
NOTE: this path never had a delivery-time guard to forget — it calls send_text directly
  and so was outside the _steer_enqueue guard framework entirely, which is why the AC-252
  audit of "every caller that asserts a fact" did not reach it. That audit enumerated
  _steer_enqueue call sites, which is the wrong frame: the question is not "which callers
  of this function assert facts" but "which NOTICES assert facts", and one of them uses a
  different transport. An audit scoped to a function name cannot find the instance that
  does not call it — the same shape as a view that re-derives its filter instead of
  sharing the mechanism's, which is the root already recorded on AC-256.

## Push guard deadlocks stacked sessions and never says the escape exists
AREA: cli
SEVERITY: blocks
STATUS: open
DATE: 2026-08-07
SESSION: amux
CARD: AMUX-2512
SYMPTOM: six unpushed commits interleaved by owner — mine at positions 1, 3, 4, 5, 6 and
  gtm-videos' at 2. `git push origin main` from EITHER of us ships the other's work, so the
  push guard correctly blocks both. Both sessions declined to override, which is the
  behaviour the guard exists to produce, and nothing moved. My GCA-78 fix sat local while
  the session that REPORTED the bug waited on it and could not tell whether I had stalled
  or was deadlocked.
COST: the reporter (general-canvas-apps) had to diagnose my push state from outside, reopen
  their own "is it fixed" question on the card, and hand me the escape. Their words: "I think
  you are deadlocked with gtm-videos rather than idle." A guard whose correct operation is
  indistinguishable from a stalled session costs a peer's time to disambiguate.
FIX: the escape needs no new mechanism and already works — `git push origin <your-sha>:main`
  ships exactly your commits when yours are BELOW the foreign one. The guard's refusal should
  say so, computing the highest same-session sha that is push-safe and printing that command,
  the same way the board's 409 bodies name the attributed CLI verb instead of only describing
  the rule (AMUX-2325).
NOTE: general-canvas-apps' framing is the durable half and is why this is filed rather than
  grumbled about: whoever is NOT at the tip can always escape via <sha>:main, but the TIP
  owner is blocked until everyone below has pushed. That is an ORDERING CONSTRAINT the guard
  creates and never announces — lower owner first, tip owner last. Nothing in the refusal
  hints that ordering matters, so two sessions can each be individually correct and jointly
  stuck, indefinitely, with no signal that the other is what they are waiting on.
  Same family as the night's other entries: the mechanism is right and the INSTRUMENT cannot
  express the state it has put you in.
