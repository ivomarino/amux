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

## Assignment notices arrive for cards that were deleted a second after being created
AREA: notices
SEVERITY: slows
STATUS: open
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

REOPENED 2026-08-09 by amux-frustrations on COUNTER-EVIDENCE from amux-cloud, the
  originating session, during the frustrations.md validation sweep. They received
  "New board task assigned: AC-311 ... Run `amux board claim AC-311`" for a card that did
  not exist (hard-deleted), and isolated it with a control: AC-310 resolved fine and the
  unfiltered board topped out at AC-310, so the probe could have found the card if it
  existed. AC-312 exists because of this recurrence. So either the fix is narrower than
  this entry claims or it regressed — the entry was marked fixed and the class is live.


## `amux send` fell back to raw tmux and the message never arrived
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-07
SESSION: amux-cloud
CARD: AC-174
FIX-NOTE: b7dba01 — amux send now retries twice over ~4s before falling back to raw tmux,
  with shorter 5s timeout on retries. Transient server-down during re-exec is survived.
SYMPTOM: `amux send amux --stdin` with a ~70-line report hit the server during a transient
  wedge and fell back to keystroke injection:
    warning: amux server unreachable — falling back to raw tmux (UNSTAMPED, unaudited)
    injected into amux via raw tmux — DELIVERY UNVERIFIED, no origin stamp, no audit.
  I peeked and none of five distinctive strings from the message were in the recipient's
  history. The message was gone. The server answered /health 200 in 0.19s a minute later.
COST: One report lost, ~10 min to detect and re-send. Would have been a silent loss if I had
  not checked — and the loud warning is the only reason I did.
FIX: Credit where due: the warning is exactly right — it names the degradation, says delivery
  is unverified, and prints the peek command to confirm. That is what made this cheap, and it
  should be the model for every degraded path in amux. What is missing is the next step: on
  server-unreachable, QUEUE the message and retry when /health answers, instead of firing
  keystrokes at a pane that may have a picker open. A long message is exactly the case where
  keystroke injection is least likely to survive and most expensive to lose. Failing that,
  verify-after-inject (grep the recipient's history for a nonce) so the CLI itself reports the
  loss rather than leaving the sender to discover it.

## The staged-guard was silent on the commit that swept a peer's work, and warned on the clean one
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-08-08
SESSION: amux-cloud
CARD: AC-297
FIX-NOTE: b7dba01 PARTIAL — _staged_guard_check() now checks for unstaged changes, which
  helps when peer work is left unstaged. But the incident shape (wholesale `git add` where
  the peer's work is swept into the index, leaving nothing unstaged) is still silent.
  The guard fires on has_unstaged_changes=True; the incident has has_unstaged_changes=False.
  Validated by amux-cloud on a throwaway repo: control (peer work left unstaged) fires;
  incident shape (wholesale git add, all staged) does not.
SYMPTOM: Two commits, 20 minutes apart, both `git add amux-server.py` on a shared checkout
  while session `amux` had uncommitted work in the same file.
    fc72811 — guard WARNED ("also edited by session 'amux' 30m ago... stages 55 insertions /
              2 deletions"). I checked line by line. It was genuinely clean, all mine.
    8adf348 — guard SILENT. It swept ~85 insertions of amux's session-report/heartbeat work
              (_ACTIVE_HEARTBEAT_S, _persist_session_reports(force=...), the PostToolUse
              "tool-hook" entry, _scrape_vs_report "active-stale") into my AC-293 fix.
  So the one time it mattered it said nothing, and the one time it spoke the commit was fine.
COST: A peer's uncommitted work is now inside my commit and cannot be separated without a
  history rewrite on a shared checkout — the operation CLAUDE.md records as having destroyed a
  session's unpushed work. Second occurrence for me; the first was b1c3e93 (~93 lines).
  Disclosed both times, and both times the fix was the peer's call rather than mine to make.
FIX: The correlation is the dangerous part, not the miss. I checked BECAUSE it warned and did
  not check when it did not — so the guard actively trained the behaviour it exists to prevent.
  A guard that is silent on the true positive is worse than no guard. Find why it fired at 30m
  and not at ~20m (mtime window? cooldown? a debounce that suppresses a second warning in the
  same session?) and make it fire on the FACT — peer has uncommitted hunks in a file I am
  staging whole — not on a time heuristic.
  Until then the instrument that actually worked was arithmetic: reconcile the numstat against
  what you believe you wrote, every commit, guard or no guard. 146/14 against a ~60-line change
  is what caught this. That check needs no guard and cannot go silent.

SCOPED 2026-08-09 by amux-frustrations, from amux-cloud's validation: the shipped fix
  (`if hit or _is_dirty`) is PARTIAL. It fires when the peer's work is left UNSTAGED, but
  their actual incident was a wholesale `git add` that swept the peer's work INTO the
  index — so nothing was unstaged, _is_dirty was False, and there was no fresh `hit`
  either. Tested in a throwaway repo with a control that DOES fire, so the negative is
  informative rather than a silent probe. Remaining scope: "wholesale git add of a
  co-edited file where the peer has no fresh provenance record". Nobody has started it.
  amux independently named the same remainder from the other side (their AF-19 review):
  a peer file staged OUTSIDE the recent-edit window has no claim trail and stays
  invisible; the belt is "list every staged path not in the committer's diff".


## A cross-cutting finding recorded on someone else's card dies when that card closes
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-10
SYMPTOM: Reviewing AC-275 on 2026-08-06 I found a defect OUTSIDE that card's scope — the
  vocab rename left `workers = msg.payload` in the SSE handler assigning an undeclared
  global while render() kept reading `sessions`. I wrote it into AC-275's description and
  said in the review, verbatim, "that regression needs a fix card of its own." No card was
  filed. AC-275 went to `verified`. The finding was still sitting in the description of a
  closed, verified card two days later, and the defect is still live at amux-server.py:55609
  as of 0.9.520.
COST: Two days of a live client defect nobody owned, and the rediscovery cost paid twice —
  found again today only because AMUX-2553 happened to fix the SIBLING assignment from the
  same commit (b009f6e broke two identifiers; that card fixed one). Without that coincidence
  it would still be invisible. A `verified` card is the LEAST likely place anyone looks for
  open work, so the finding was not merely unowned, it was filed somewhere that actively
  signals "nothing to do here."
FIX: A review that produces an out-of-scope finding needs somewhere to put it that is not the
  card being closed. Two candidate shapes, both cheap: (a) the review ack path accepts a
  `--spinoff "<title>"` that files a `todo` card attributed to the reviewer and cross-links
  both ways, so the finding leaves with an owner instead of a paragraph; or (b) the
  review->done transition refuses to close while the card's own description contains an
  unlinked "needs its own card"-class statement, the way gates already refuse other
  half-finished states. (a) is better — it makes the honest path the easy path rather than
  adding a check that fires after the fact. Note this is the ethos rule-4 shape one level up:
  the finding WAS recorded, so the data existed; it was recorded where no loop and no view
  would ever read it again, which is the same failure as not recording it.
NOTE: related to the `watch`-type blindness in ethos.md (a card surfaced by nothing is a note,
  not a monitor) — same root, different container: here the invisible thing is a paragraph
  inside a terminal-status card rather than a card outside every query.

## A peer's save restarts the server mid-measurement, and the timings blame your subject
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-11
SYMPTOM: Benchmarking `GET /api/board?slim=1` against the unprojected fetch, I measured slim
  timing out at 60s while the FULL 6.6MB payload returned in 16.9s — a projection added to make
  the fetch cheap looking 4x SLOWER than no projection at all. Second run: full 78s, slim
  HTTP 000 at 22s, then both 000 instantly. Every number was an artifact. A peer session wrote
  amux-server.py in this shared checkout at 12:39:48, the server auto-restarted at 12:40:24, and
  my requests spanned the restart. Re-measured on a confirmed-healthy server: slim 828KB/0.10s,
  full 6.6MB/0.11s. The projection is fine.
COST: ~6 minutes and a near-miss on filing a fabricated performance defect against `slim=`
  (AMUX-2223's own feature) with three runs of "evidence" behind it. The failure mode is the
  dangerous direction: the restart produces symptoms — timeouts, truncated reads, connection
  failures — that are indistinguishable from the subject being slow, so the wrong conclusion
  arrives fully corroborated. I only caught it by checking the server's mtime, which I had no
  particular reason to do.
FIX: Any timing or availability measurement against the local server needs the restart to be
  VISIBLE in the result, not inferred afterward. Cheapest version: `/health` already responds —
  have it report the process start time, so a caller can bracket a measurement (start_ts before,
  start_ts after) and know the server it finished on is the one it began on. That turns a silent
  confound into a checkable precondition, and it costs one field. Today the only way to learn
  this is to stat the file and read the restart log, which nobody does before believing a number.
NOTE: the shared checkout is the amplifier (see AMUX-2443, open) — my working tree was clean and
  I had made no edit, so nothing in MY session hinted that the binary under test had changed.

## SUPERSEDES the restart-framed-its-subject entry above: BOTH causes were real, and the instrument already existed
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-11
SUPERSEDES: "A peer's save restarts the server mid-measurement, and the timings blame your
  subject" (same session, same day). That entry is wrong in its diagnosis and wrong in its
  FIX. Leaving it in place per this file's convention; read this one instead.
SYMPTOM: I reported `GET /api/board?slim=1` timing out at 60s while the unfiltered 6.6MB
  fetch returned in 16.9s, found that a peer had written amux-server.py at 12:39:48 and
  the server had restarted at 12:40:24, and concluded the numbers were "entirely
  fabricated". Then I re-measured, got slim 0.10s / full 0.11s, and declared the
  hypothesis dead. Timeline says otherwise: my FIRST measurement predates the write, so no
  restart was involved — it was measuring a live defect (AMUX-2562, filtered board GETs
  running an uncapped full-table scan per request, which is precisely why the PROJECTING
  path hung while the unfiltered one returned). d4dfbc7 landed the fix at 12:40:49. My
  "control" ran after that. I compared before-fix to after-fix and labelled it
  before-restart to after-restart.
COST: A wrong conclusion published in two places (this file and AF-11) and a real defect
  dismissed as measurement noise by the only other session that had independently
  observed it. amux filed AMUX-2562 from their own diagnosis an hour later; had I read my
  own data correctly they would have had a second data point at 12:36 instead of none.
FIX: Nothing to build — GET /health ALREADY returns `build` (a content hash of the running
  amux-server.py), plus `pid` and `uptime_s`. Any of the three would have caught this;
  `build` catches it exactly, because the invalidating fact was that the served CODE
  changed, not merely that the process bounced. Fixed by routing callers to it: CLAUDE.md
  now carries the bracket recipe (read `build` before and after, a move means the
  measurement is INVALID, not that the subject is slow), next to the existing "verify with
  a string your edit INTRODUCED" rule. AF-11 closed as already-implemented and retyped
  code -> doc; adding the field it already has would have been a second spelling of an
  existing primitive, shipped in the belief it fixed something.
NOTE: two lessons, and the second is the transferable one. (1) A confound that explains
  PART of a mess will be accepted as explaining ALL of it — the restart was real and did
  explain my second run's HTTP 000s, which is exactly what made it convincing enough to
  stop the search. Ask what the confound does NOT explain: the first run had no restart in
  it and I never checked. (2) The ethos rule about confirming results fired precisely as
  written — I was most careless at the moment the answer matched what I expected, and the
  re-measurement that "proved" me right was run against different code than the
  measurement it was meant to control. A control that does not hold the build constant is
  not a control. This is the same shape as `_build_id`'s own docstring, which was written
  for two other sessions hitting it on two other fixes in one hour; I hit it a third time
  with the instrument already sitting one curl away.

## The untracked-work nudge is blind to review work, so a reviewer is told to record what they just recorded
AREA: notices
SEVERITY: annoys
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-15
SYMPTOM: "You went idle but have no board issue tracked as 'doing'. If you just did real
  work, record it on the board now" fired 3 times in one afternoon against a correct
  ledger. I had signed off 5 cards that day (AMUX-2542, 2553, 2562, 2565, 2566), each
  carrying reviewer='amux-frustrations'. Both of the guard's suppressions key on
  OWNERSHIP — `WHERE session=?` — and review->done lands on the AUTHOR's card, so from the
  guard's vantage I had done nothing at all.
COST: Small per firing, but the shape is the expensive part: there is no truthful way to
  comply. A reviewer can create a card for "reviewed someone else's card" — not a unit of
  work that can be honestly done or not done, and something the ledger rule explicitly
  forbids — or ignore the nudge. I ignored it three times, which is exactly the training
  the guard exists to prevent. _session_recently_closed_issue's own docstring names this
  outcome: "pressures a session to create a placeholder card to silence it — fake work".
FIX: One more suppression against the table it already queries:
  `SELECT 1 FROM issues WHERE reviewer=? AND status='done' AND deleted IS NULL AND updated > ?`
  using the same recency window. No new state, no new field. AF-15 has the detail.
NOTE: what makes this instructive rather than just a bug is that the function had ALREADY
  reasoned about review handoff — it treats an author parking at `review` as handed off,
  not as stopping short, and explains why (the author is structurally forbidden from
  closing a card that names a reviewer). It thought about one end of the handoff and not
  the other. The reviewer is the party whose work is invisible BY CONSTRUCTION, because
  they never own the card they close.
  The generalisable half: `session=?` is the RIGHT predicate for auto-pickup and for the
  verification sweep — you cannot pick up or verify a card you do not own — and the wrong
  one here. A predicate that is correct three times out of four is the hardest kind to
  audit, because every instance looks like the established pattern. Same family as the
  ethos rule-1 note that a view must share the predicate of the mechanism it describes;
  here the guard describes "did this lane work?" with a predicate that means "does this
  lane own cards?".

## `amux board review` cannot name the reviewer, so completing a handoff requires leaving the audited path
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-16
FIX-NOTE: 868d893 — --reviewer added to every status verb; --outcome-stdin
  deferred until argv validates, which also stopped the wrong error being
  reported (it said "got empty input" instead of naming the unknown flag).
SYMPTOM: `amux board review <ID>` has no --reviewer flag (usage: [--checked] [--ack]
  [--type] [--override-doing] [--trigger] [--force]). A card moved to `review` with
  reviewer=None is a card nobody has been asked to look at, and the review gate rests
  entirely on the reviewer's X-Amux-Session being the required sign-off. So the sanctioned
  command produces the status but not the state that means anything; the only completion is
  a raw PATCH for `reviewer`.
COST: Two writes and a hand-passed X-Amux-Session where one attributed command should do.
  Compounding: `amux board review AF-15 --checked "..." --reviewer amux --outcome-stdin
  <<EOF ...` failed on the unknown flag — loudly and correctly — but the --outcome-stdin
  body was already consumed and was discarded with the rejected invocation, so ~40 lines of
  review outcome had to be re-authored.
FIX: Add --reviewer <session> to `amux board review` (arguably to every status verb, so a
  card can be routed as it is created). Separately, validate argv BEFORE draining stdin, or
  echo the consumed body back on rejection.
NOTE: this is AMUX-2325 one verb over, and the same argument applies — the gate system
  depends on attributed writes, so a gap in the audited path is precisely what manufactures
  the unattributed ones. The second half is the ethos rule-6 corollary in its purest form:
  the refusal destroyed the evidence needed to satisfy it. Together they are the third
  AREA: cli entry where the sanctioned command cannot express something the gate requires.

## No rig can render amux at phone width, so the mobile half of `verified` is undecidable
AREA: browser
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-18
FIX-NOTE: e29069b — the driver's viewport was a LITERAL (1280x900); it is now a
  parameter (argv, AMUX_BU_VIEWPORT, and a `viewport` action taking width+height or
  device=iphone|ipad|...). innerWidth 1280->390, mq(max-width:600px) false->true.
  Also explains why window.resizeTo() looked broken: Playwright owns the viewport, so
  the call was inert rather than blocked. Unblocked AMUX-2369 (now verified) and
  resolved AMUX-2367's 40-vs-44px flag (renders 67px, clean).
SYMPTOM: amux is mobile-first by policy and `verified` is meant to include the real UI at
  phone width. Three rigs, none can do it. (1) The shipped driver: POST /api/browser/start
  takes url/profile/session/fresh/backend — no viewport parameter — and in-page
  window.resizeTo(390,844) is ignored (innerWidth stayed 1280, matchMedia('(max-width:600px)')
  false). (2) Chrome CDP, the one rig with real device emulation: localhost:9222 returns 404,
  and cdp.mjs has no emulate verb anyway. (3) iOS Simulator, which my own notes call ground
  truth: HTML renders but the app sits on "Connecting to server…" and /health stays blank
  through a long settle, so the API never answers inside the sim; adding the root cert per the
  documented recipe changed nothing, and simctl has no tap primitive to dismiss the first-run
  tour that covers the page.
COST: Two verifications in one afternoon. AMUX-2369 is literally titled "mobile optimized" and
  could not be verified on that axis — left `done` with the check handed back to a human with
  a phone. AMUX-2367 shipped an unresolvable question: `.send-row button` declares
  min-height:40px with no override in any of the 48 mobile media blocks, under the 44px rule,
  but min-height is a floor and I could not measure a rendered button, so it went on the card
  as a flag rather than a finding.
FIX: Cheapest and highest-value is a window size (or an `emulate` action) on the driver amux
  already ships and already launches — it is the default path and it is one launch argument
  from working. Then CDP (enable 9222 + an emulate verb). The simulator is the
  highest-fidelity rig and worth repairing, but it has two independent blockers.
NOTE: this is ethos rule 3 with a tooling shape. The verified gate asks for a check no shipped
  tool can perform, so it resolves the same way every time: the reviewer writes "could not
  check at phone width" and the mobile half of `verified` quietly becomes decorative. It will
  do that on every mobile card until a rig exists — which is exactly the "constraint that
  cannot be satisfied honestly" pattern, except the dishonest exit here is silent omission
  rather than a false ack.

---

## `git commit` on the shared checkout consumes PEERS' staged files silently
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux
CARD: AMUX-2443
SYMPTOM: I staged amux-server.py and committed; the commit also carried
  tests/test_board_full_cache_generation.py — amux-frustrations' AF-12 test, sitting
  in the SHARED index. The staged-guard warned about the co-edited server file but
  says nothing about OTHER paths already staged by peers, and `git commit` takes the
  whole index. The amend repair is (correctly) blocked by the shared-checkout guard,
  so the misattribution is permanent in history.
COST: a peer's test shipped under my sha and message; two sessions spent time on
  notice/acknowledgement; the same sweep with a SECRETS or WIP file staged would be
  worse than misattribution.
FIX: candidate fixes, someone's to pick up: (a) staged-guard lists ALL staged
  paths not touched by the committing session's diff, loudly; (b) fleet convention:
  `git commit -- <own paths>` instead of bare commit (commit takes pathspecs and
  bypasses the index sweep); (c) both. (b) is zero-code and I am adopting it now.

## A peer's scoped `git add` swept my STAGED files, because git commits the index not the paths
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-19
SYMPTOM: I had AF-12's fix staged (amux-server.py hunks + a new test file). amux ran
  `git add amux-server.py` and committed. git commits the INDEX, so their commit swallowed
  my staged test file and my staged hunks. 762e06e is titled "fix(herdr): first real e2e
  contact … (AMUX-2554)" and its contents are largely my AF-12 work. My own
  `git commit` then reported "nothing added to commit", which is how I found out.
COST: ~10 minutes tracing where my work went, and a permanently wrong history in both
  directions: a bisect for the board_full race lands on a herdr commit, and an audit of
  what shipped under AMUX-2554 finds a cache generation guard. The amend was correctly
  blocked (no HEAD-moving on a shared checkout), so it cannot be repaired — only recorded.
  amux raised it themselves; neither of us lost work.
FIX: The staged-guard already detects a co-edited FILE and prints an insertion count to
  reconcile — it fired for me twice today and I used it correctly both times. It reasons
  about the one file being committed and has no opinion about OTHER paths in the shared
  index. Name them: "your index also contains N path(s) staged by another session: <path>
  (<session>, <age>) — `git commit -- <paths>` commits only yours." The remedy amux named
  (`git commit -- <paths>`) is the one to publish, in the guard's existing
  honest-path-is-the-easy-path idiom.
NOTE: distinct from the two shapes AMUX-2443 already covers. Not `git add` sweeping peer
  HUNKS in a file you both edited, and not a pull --rebase replaying an unpushed commit:
  here the file is entirely mine, the committer never touched it, and their `git add` never
  named it. The guard's blind spot is that it is FILE-scoped while the sweep is INDEX-scoped
  — a check aimed one level below the mechanism it is protecting against.

## The reviewer-identity check fires on done->verified, blocking the peer amux routed the verification to
AREA: gates
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-20
SYMPTOM: Working the VERIFY queue amux dispatched to me ("You are the independent check"),
  done -> verified was refused twice with "review sign-off required from the reviewer ...
  the review->done ack must come from that session". The attempted edge is done->verified,
  not review->done. On AMUX-2385 it is unsatisfiable by construction: the card went
  doing -> done directly (log: `status: doing -> done (by amux/session)`), so the named
  reviewer never acked a review and has no pending ack to give.
COST: Two forced bypasses in one afternoon (AMUX-2334, AMUX-2385) on cards I had fully
  measured. Both logged and attributed, so nothing is hidden — but the alternative was
  leaving a completed verification unrecorded, and a gate that trains its most careful users
  to reach for --force is inverting its own purpose.
FIX: Scope the identity check to the transition it is about. It exists so an author cannot
  self-ack their own review — that is review->done. done->verified is a different edge with
  a different role and already has its own peer criterion. Failing that, accept ANY different
  worker in the group, which is what the gate text already asks for. At minimum fix the
  message: naming the wrong transition sends the reader hunting an ack that cannot exist.
NOTE: ethos rule 6 — the published contract and the enforced one disagree. The `verified`
  gate lists four criteria; criterion 2 is "Peer-reviewed by a DIFFERENT worker in group
  `amux` (name them)", which I satisfied and named. The refusal comes from a check the gate
  text never mentions. A card can therefore pass every criterion it publishes and still be
  refused, which is the state that makes --force feel like the honest move.

## The co-edit notice asserts a git fact that was true at emission and false by delivery
AREA: notices
SEVERITY: annoys
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-21
SYMPTOM: Two consecutive co-edit notices said "amux-server.py: you edited it at 18:58 and
  have not committed it since 18:33". My commit 44bd9fe touched that file at 19:36, so the
  sentence was false when I read it. It was TRUE when emitted — the notices fired for
  commits at 19:06 and 19:14 — and expired before delivery.
COST: The sentence exists to make you suspect your work was swept, and is followed by "your
  next git commit may say nothing to commit". So a stale one sends you to audit a commit for
  work that is not in it: `git show --stat 902e9d8` -> 8 insertions, 0 of mine. Two audits of
  two clean commits. Small each time, but it also cannot distinguish itself from the REAL
  case — 762e06e genuinely had swept my staged AF-12 work and carried the identical sentence.
FIX: Re-check at delivery, exactly as c32cf8a did for the decompose nudge (AC-252) and 7504abf
  for the three other perishable-state nudges. If the reader has committed that path since the
  notice was queued, drop the sentence or replace it with "you have since committed it in
  <sha>". The co-edit notice asserts perishable GIT state and was not in that sweep.
NOTE: distinct from the already-fixed "co-edit notice asks the reader to resolve a condition
  it is better placed to check". That was the notice ASKING; this is the notice ASSERTING
  something that has since become false — worse, because an out-of-date question costs a
  moment while a false statement sends you hunting a defect that does not exist. The emitter
  is right to be conservative; over-warning about a sweep beats under-warning. Only re-check it.


## SessionStart freshness hook named files upstream never touched
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-22
SYMPTOM: Hook printed `checkout is 1 commit(s) behind origin/main - including: CLAUDE.md
  amux amux-server.py`. The single incoming commit (eaa1e91) touches ONLY amux-server.py.
  Cause: the hot-file list used `git diff --name-only HEAD..$base` - TWO dots, which in
  `git diff` compares the two ENDPOINTS instead of diffing from the merge-base, so on a
  shared checkout with 120 unpushed commits it reports OUR OWN files as upstream changes.
  The same sentence disagreed with itself: `behind` uses rev-list, where two-dot IS correct,
  so the count said 1 while the file list implied a broad conflict.
COST: ~10 min reconciling two files that had zero incoming changes. The compounding cost is
  worse than the minutes: the error grows with the number of unpushed local commits, so the
  warning is least trustworthy exactly when the checkout is busiest - which is the situation
  it exists for. An instrument that cries wolf in proportion to the real risk gets ignored.
FIX: 13c7014 - three dots. Positive control in a scratch clone with upstream touching only
  amux-server.py: two-dot -> [CLAUDE.md amux amux-server.py] (reproduces the symptom),
  three-dot -> [amux-server.py]. Line 43's rev-list two-dot deliberately left alone.

## Shared-checkout guard blocked a commit aimed at a scratch repo, naming the shared one as fact
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-23
SYMPTOM: A compound command that `cd`s into a throwaway git repo under the scratchpad and
  runs `git commit -qam` there was blocked by ~/.amux/hooks/git-shared-guard.py, whose
  message asserts "'/Users/ethan/Dev/amux' is a SHARED checkout" - a repo the command never
  touched. The guard matches on command TEXT against the session's cwd and cannot see an
  in-command `cd`.
COST: one wasted round-trip. Small, but the next session that writes a scratch-repo test
  hits it identically, and the refusal reads as a true positive because it states the shared
  path as fact rather than as the assumption it is.
FIX: 523df63 (guard) + 8ddf1d0 (4 regression tests). Verified LIVE end-to-end
  through the real PreToolUse hook.
## Staged-guard attributed a session's OWN edit, seconds old, to session '(unknown)'
AREA: attribution
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-24
SYMPTOM: Committing .claude/session-freshness.sh, which I had edited ~30s earlier and which
  no other session had touched, printed: "WARNING - .claude/session-freshness.sh was also
  edited by session '(unknown)' 0m ago. This commit stages 13 insertions / 1 deletions there
  - if that is MORE than you wrote, their work is in it." The 13/1 was exactly my own edit.
COST: minutes reconciling a co-edit that did not exist. NOT root-caused - I did not
  determine whether the edit record was missing, unattributed at write time, or attributed
  but not matched against the committing session.
FIX: 6bcc2f4. Verified LIVE in production on commit 273128e — the warning now reads
  "is yours and has uncommitted changes right now - no other session edited it".
## `HEAD~1` is not "before my change" here — the pre-fix specimen check tested the wrong commit
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-25
SYMPTOM: Verifying AF-23's regression test against a pre-fix specimen via
  `git show HEAD~1:amux-server.py`. amux-cloud committed 939064d between my commit
  (523df63) and the check, so HEAD~1 WAS MY OWN FIX. The probe reported the disclosure
  string already present pre-fix and concluded "the test would PASS - VACUOUS - bad test!".
  Re-run against `523df63^` - the parent of MY commit - it correctly reports FAIL.
COST: ~5 min, and it was one step from costing much more: the false verdict was that a
  correctly-discriminating test was vacuous, whose natural remedy is to rewrite a good test
  into a worse one. This is the LOUD WRONG probe, not the silent one - it answers, and the
  answer looks exactly like the failure ethos rule 7 warns about, so it is self-corroborating.
FIX: documented in CLAUDE.md, in the same commit as this entry (no sha cited here: writing
  one before the commit exists is the fabrication ethos rule 7 records, and I did it in the
  first draft of this very entry). Use `git show <your-sha>^:<file>`, never HEAD~1, on
  a checkout where other lanes commit. The trap is invisible on a single-session repo, which
  is precisely why it needs writing down here: every fix in this repo is supposed to be
  checked against a pre-fix specimen, so the wrong recipe is reached for constantly.


## staged-guard BLOCKED a commit on an edit record with no content behind it
AREA: attribution
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-26
SYMPTOM: `git commit -- CLAUDE.md frustrations.md` refused with "COMMIT BLOCKED - staged
  files were edited by OTHER amux sessions sharing this checkout: CLAUDE.md (edited by
  session 'amux-cloud' 0m ago)". Checked before overriding: `git diff HEAD -- CLAUDE.md` is
  ONE hunk, 14 insertions / 0 deletions, entirely my own text, and `git log -- CLAUDE.md`
  shows no amux-cloud commit at all. There was no foreign content to sweep. The guard gates
  on a recent-edit RECORD, not on whether the staged patch actually contains another
  session's hunks.
COST: ~8 min verifying the block was spurious, and a commit that had to be forced past a
  correct-looking refusal. Worse than AF-24 (same subsystem, warns only) because this one
  BLOCKS. The compounding cost is that overriding is now normalised on a guard whose whole
  value is that its refusals mean something.
FIX: 6bcc2f4 + 21cce46 for the diagnosable halves (self-describing block, honest
  AMUX_VERIFIED_SOLO escape). The classification half is UNRESOLVED and split to AF-27
  with two hypotheses killed in writing. Do not read this as fully fixed.