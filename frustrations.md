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


## Dashboard's usage-limit discriminator says 'worker'; the live endpoint says 'session'
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-09
SESSION: rust-rebuild (provider adapters, RR-0043)
CARD: AMUX-2581
SYMPTOM: Porting the Claude usage probe to Rust, I took the 5h-window discriminator
  from the only in-repo consumer, loadUsage() in amux-server.py (`l.kind === 'worker'`).
  The live /api/oauth/usage endpoint returns `kind: "session"` for that window — the JS
  check never matches anymore, so the dashboard labels the 5h bar with the raw kind
  string, and the stale discriminator nearly shipped into the new Rust mapper verbatim.
COST: ~10 min re-probing the live endpoint; one step from encoding a never-matching
  filter into the Rust adapter (an ethos-7 silent probe: it would have "worked" because
  the top-level five_hour shape still mapped, masking the dead limits[] branch).
FIX: loadUsage() should accept both "session" and "worker" (the Rust mapper now does);
  better, both consumers should assert the discriminator against a recorded live
  fixture so endpoint drift fails a test instead of silently unlabeling a bar.

---
## Journal `entity_type` stored serde's object form — every tag filter matched nothing
AREA: journal
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2593
SYMPTOM: `_amux_state_events.entity_type` held the adjacently-tagged serde object
  (`{"kind":"task"}`) instead of the bare tag (`task`), so every
  `entity_type = '<tag>'` SQL filter silently matched zero rows: redistribute dedupe,
  `/api/metrics/fleet` `last()`, and the circuit breaker's `window_stats` (its
  `completed` was permanently 0 — a breaker that can never see success). Classic
  ethos-7 silent probe: the queries ran, returned empty, and empty looked like
  "no events yet". Found only because RR-0111a's replay verifier needed the tag
  as a join key and its round-trip test could actually fail.
COST: The circuit breaker judged fleet health on a numerator of zero for as long
  as the journal existed; fleet metrics under-reported; nothing errored. Two
  sessions (this one and amux-rust) then fixed the SAME mismatch in opposite
  directions concurrently — writer-to-bare vs readers-to-object — which is the
  drift the single-codebase rule exists to prevent; resolved toward bare tags
  with both-format-tolerant readers since existing DBs hold old rows.
FIX: Writer stores bare tags; `parse_entity_type` (db/mod.rs) + `entity_tag`
  (db/replay.rs) + IN-clauses at the two query sites accept both forms. The
  durable lesson: when a column is written by serde serialization, pin its
  STORED form with a test that queries it back by literal value — serialization
  defaults are an implementation detail until SQL depends on them.

---
## Rust origin answered unknown /api/* with the SPA shell (200 text/html) — nested proxy fallbacks silently shadowed
AREA: instruments
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (updict/browser parity lane)
CARD: AMUX-2594
SYMPTOM: On 8824, GET /api/groups, /api/browser/state, /api/fs?path=/tmp and every
  other unrouted API path returned 200 text/html — the static catch-all
  (`/{*path}`) serving index.html. Two distinct downstream failures: (1) the SPA
  group picker silently emptied ("adding a group didn't work": r.json() threw on
  HTML, .catch swallowed it), and (2) an auth probe reported "/api/fs returns 200
  with NO token", reading the shell as an unauthenticated file listing. Worse: a
  nested router `.fallback(py_proxy)` compiled, passed its unit tests, and was
  STILL shadowed in the full composition, because unit tests nest the router
  without the competing static catch-all.
COST: One wrong security conclusion shipped upstream (phantom unauthenticated fs
  endpoint); the SPA groups feature broken on the Rust origin; a browser-verb
  proxy that looked done on unit-test evidence and was inert live.
FIX: static_files.rs returns Python's JSON 404 for unknown /api/*; proxied
  namespaces use EXPLICIT `/` + `/{*rest}` routes (py_proxy::passthrough_routes),
  never nested `.fallback()`; tests/proxy_composition.rs pins the property at the
  FULL-router level with a dead python port. Durable lesson: a nested fallback is
  not a route — test routing properties on the composed app, not the nest.

---
## Rust and Python servers each minted their own auth token — same claimed file, different names
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (updict/browser parity lane)
CARD: AMUX-2594
SYMPTOM: auth.rs's docstring said the token file is "shared with the Python
  server", but Rust read `~/.amux/auth-token` (dash) while Python owns
  `~/.amux/auth_token` (underscore). The Rust server minted its own token on
  first boot, so every client holding the real token (the SPA served by Python,
  curl recipes, probes) got 401 from 8824 — which presented as a confusing
  "auth asymmetry" between origins and burned probe time on the wrong theory
  (that the UI-guard sha was an accepted token form; it never was — Python's
  localhost bypass was producing the 200s).
COST: Cross-origin 401s for every valid-token caller; a probe narrative built
  on two wrong mechanisms before the filename diff was spotted.
FIX: config.rs reads Python's `auth_token`; AMUX_AUTH_TOKEN env parity
  ("none" disables); require_bearer is a port of Python's _check_auth
  (localhost bypass, public paths, `_token=` only, JSON 401). The stale
  `auth-token` file is left on disk but nothing reads it.

---
## Board card archive via PATCH 500s on the Rust origin — and the harness's fire-and-forget cleanup hid it for three runs
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (rust parity-gaps lane)
CARD: AMUX-2586
SYMPTOM: `PATCH /api/board/<id> {"archived":1}` — the exact write the SPA's
  card-archive and the parity harness's cleanup use — returned 500 on the Rust
  origin (the field was absent from PATCH_WRITABLE; Python has accepted it
  since AMUX-2492). The harness's cleanup `.catch(() => {})`ed the response,
  so each run left an unarchived PARITY- card on the LIVE shared board with
  no error anywhere: PH-2/PH-3/PH-5 accumulated across days and read as
  stray unowned todos to every board consumer.
COST: 3 stray cards polluting the live board's Unowned view across multiple
  days; the silent-cleanup shape meant nobody knew which side had failed
  (python vs rust) until the response was actually read.
FIX: board.rs PATCH now ports Python's archived semantics byte-for-byte
  (truthy set 1/true/yes/on, cross-lane guard requiring authorized_by,
  un-archive never gated; test patch_archived_round_trip_with_cross_lane_guard);
  the harness cleanup checks the PATCH status and prints CLEANUP FAILED with
  the card id. Strays archived by hand via the Python API. Lesson repeated:
  a cleanup that cannot report failure is a generator of exactly the debris
  it exists to remove.

---
## Group-config PATCH: COALESCE arms are dead code — explicit JSON null 500s on both origins
AREA: board
SEVERITY: wrong-conclusion
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2597
SYMPTOM: /api/groups/<n>/config PATCH looks like it preserves absent keys via
  COALESCE upsert arms, but SQL NULL trips the column's NOT NULL before conflict
  resolution ever runs — so an explicit JSON null 500s on BOTH servers and the
  COALESCE arms can never fire. Also PATCH resets absent keys (send the full
  object). Found while porting to Rust; verified against Python's exact schema+
  SQL; an earlier "null preserves" reading was a killed hypothesis, recorded.
COST: A client sending a partial config update silently wipes the other keys; a
  null 500s with no useful message. Ported faithfully to Rust (bug-compatible)
  so the fix must land on both or the boundary drifts.
FIX: Decide the intended semantics (partial-merge vs full-replace), implement on
  both servers, and add a null-body regression test each side.

---
## /api/fs/search error text promises AMUX_SEARCH_RG that Python never reads
AREA: files
SEVERITY: annoy
STATUS: fixed
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2597
SYMPTOM: Python's search error text tells the user to set AMUX_SEARCH_RG
  (amux-server.py:21056) but no code reads it — a documented knob that does not
  exist (ethos rule 6 shape: the claim without the implementation).
COST: Anyone following the error's advice sets an env var that changes nothing.
FIX: The Rust port implements AMUX_SEARCH_RG for real rather than inheriting the
  false claim (crates/amux-server/src/api/fs.rs). Python side still carries the
  dead promise — fix or delete the text there.

---
## Rust SSE "ping" goes out as an SSE COMMENT — EventSource clients can never see it
AREA: sse
SEVERITY: slows
STATUS: fixed (5899463 — KeepAlive::new().event(...data(ping_payload())), sse.rs:120-122)
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2611
SYMPTOM: Wire capture of GET /api/events on the Rust server (build 51f43c50d66e88b9)
  shows the keep-alive as `: {"type":"ping","v":"0.9.529"}` — a comment frame, not a
  `data:` event. sse.rs's own comment says "The keep-alive IS the ping contract: 10s
  cadence, real data event", but axum 0.8.9's KeepAlive::text(t) is literally
  `self.event(Event::default().comment(t))` (axum src/response/sse.rs:552). The SPA
  consumes pings via EventSource.onmessage (app.js:21378), which fires only on data
  events, so `msg.type === 'ping'` (app.js:21462) is unreachable against the Rust
  server: the version self-reload-on-deploy path is dead, and pings never feed
  `_lastDataTime`, so an idle fleet (>18s without real events) trips the client's
  zombie detector and forces reconnect loops. The `v` value itself is correct
  (matches served app.js APP_VER).
COST: A 12s data-line probe reported 0 pings and initially read as "no pings sent";
  discriminating comment-vs-data took a raw curl capture. In prod: deploy adoption
  by open windows silently does not happen on the Rust server, and quiet fleets
  reconnect every ~18s. Secondary: axum keep-alives fire only after `interval` of
  SILENCE (unlike Python's unconditional 10s ping), so even after the data-event
  fix, a busy stream can starve version-adoption pings.
FIX: crates/amux-server/src/api/sse.rs — replace `.text(ping_payload())` with
  `.event(Event::default().data(ping_payload()))` (KeepAlive::event exists in axum
  0.8.9); consider an unconditional 10s ping task for Python parity. Regression
  test: read the raw stream and assert the ping arrives as a `data:` frame.

---
## Browser profile DELETE can rmtree a real Chrome profile (python, live)
AREA: browser
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2602
SYMPTOM: DELETE /api/browser/profile/<name> (amux-server.py:74351) resolves via
  _bu_profile_dir, which for some names lands inside the user's REAL Chrome
  user-data-dir — and then rmtree's it. An API meant to manage amux-owned
  automation profiles can delete a human's actual browser profile.
COST: Data-loss class on the live server; found only because the Rust port had
  to decide what the guard SHOULD be (native port refuses non-amux-owned dirs).
FIX: Python needs the same containment guard while it lives; the Rust deviation
  is documented in docs/rust-migration/server-boundary.md.

---
## Two /api/logs handlers in amux-server.py; the second is unreachable dead code
AREA: api
SEVERITY: misleads
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2607
SYMPTOM: amux-server.py declares GET /api/logs twice: :67673 (category/session/
  limit -> {"events","count"}) and :71933 (type/since/filter/lines ->
  {"events","raw","raw_total_lines"}). Dispatch is sequential first-match, so
  the :71933 block can never run — two handlers in the same file claim the same
  route with DIFFERENT param and response contracts, and only reading the
  dispatch order reveals which one is real.
COST: The AMUX-2605 rust port was pointed at BOTH line numbers as the contract
  to preserve; porting the dead one would have shipped an /api/logs whose shape
  the SPA (app.js:16520) never consumes. Discriminating cost a live-fixture
  capture against 8822 that reading the source alone could not settle.
FIX: Delete the :71933 block or fold its useful params (since) into the live
  handler. The rust origin ports the LIVE :67673 shape (api/request_log.rs),
  verified against the running python server.

---
## Resume drops --name, so a session's pane title shows the CONVERSATION's old name, not the worker's
AREA: attribution
SEVERITY: misleads
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2612
SYMPTOM: This worker is `amux` ($AMUX_SESSION=amux, tmux session amux-amux, log
  ~/.amux/logs/amux.log). Its tmux PANE TITLE reads `amux-rust`. Root cause is in
  the launcher: session_flag is EITHER `--resume <uuid>` OR `--name <name>`, never
  both (amux-server.py:24258-24291; the rust port carries the same seam,
  session_verbs.rs:2480). Claude Code writes the terminal title from ITS OWN
  session name, which on a --resume path is the name baked in when the conversation
  was created. Confirmed, not inferred: ~/.claude/sessions/53855.json and 66447.json
  both map sessionId 1dd2cd21-c4a7-46b9-9b97-51fccbe721a2 -> name "amux-rust", while
  amux serves the same worker as `amux`. A model swap resumes by uuid, so EVERY
  model swap silently re-asserts the stale name.
COST: The model-swap continuity handoff tells the incoming model "read
  ~/.amux/logs/amux.log, it contains THIS session's terminal history" — and the
  banner inside it reads `amux-rust`. I spent a round trip establishing which of
  the two names was mine before I could trust any of the log as my own context.
  The failure mode this sets up is worse than the confusion: a session that
  believes it is a different lane will attribute its work, its commits and its
  board writes to that lane. Same class as AMUX-1768 (relay misattribution), except
  here the wrong name is displayed by amux's own instruments rather than typed by
  an agent.
FIX: Pass BOTH on resume — `--resume <uuid> --name <worker>` — so the displayed
  name always tracks the WORKER, which is the only identity amux stamps writes with.
  If Claude Code rejects the combination, have amux set the pane title itself
  (tmux select-pane -T "$name") after launch rather than leaving the harness's stale
  name on screen. Fix in the rust launcher first; the python one is being retired.
  Cheap detector while it is open: `amux whoami` already contrasts live worker
  identity against inherited env — extend it to compare against the pane title, so
  the disagreement is reported instead of discovered.

## Auto-builder script hardcoded the developer's checkout path — any other clone would rebuild the wrong repo
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (AMUX-2608 install-script lane)
CARD: AMUX-2608
SYMPTOM: scripts/rust-auto-build.sh (run by com.amux.server-rs-builder every 60s)
  read `REPO="/Users/ethan/Dev/amux"` and `INSTALL="$HOME/.local/bin/amux-server-rs"`
  as literals. Found while making ./install.sh write the builder plist for a fresh
  clone: the plist would point at the cloned script, and the script would then
  silently build ETHAN'S path, not the clone — or fail on a machine where that path
  does not exist while exiting 0-shaped from launchd's point of view (log-only).
COST: None realized here (caught during install.sh work, before any user hit it),
  but the failure it sets up is the install.sh e2e passing while the auto-upgrade
  seam serves a different repo's commits — a wrong-build deploy that nothing labels.
FIX: fixed in the AMUX-2608 change: REPO now derives from the script's own location
  (`$(dirname "${BASH_SOURCE[0]}")/..`) with AMUX_REPO/AMUX_RS_INSTALL/
  AMUX_RS_BUILD_STAMP/AMUX_RS_BUILD_LOG env overrides for the temp-prefix e2e.
  Behavior on this machine is unchanged (script lives at the same path it named).


## Idle nudge told me to commit 11 files I never touched, while the staged-guard said I owned none
AREA: notices
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-38
SYMPTOM: The idle dirty-tree nudge listed 11 files as mine to "commit completed work now",
  excluding only 2 as not-mine. I had touched none of the 11 - they are amux-rust's in-flight
  rust migration (crates/amux-server/src/api/*.rs, tests, install.sh, scripts/rust-auto-build.sh).
  The staged-guard, queried on the same dirty list at the same moment, disagreed completely:
  `POST /api/git/staged-guard` returned foreign=4 (owner=amux), unclaimed=18, shared/mine=0.
  My own work was already committed; git status showed nothing of mine.
COST: none, because I checked before committing - but only because I had spent the day on this
  exact defect class from the other side. Following the instruction literally sweeps a peer's
  whole in-flight rust migration into a commit under my name, which is the AMUX-2554 incident
  the fleet has already paid for twice. The instruction IS the hazard.
FIX: have the nudge resolve ownership through the same call the staged-guard uses instead of
  deriving "yours" from dirty-tree membership. Two components answering the same question
  differently is the duplicated-precedence bug AMUX-2330 already fixed once for gates: one
  answer, one owner. Note the nudge is not blind - it correctly excluded 2 files - so it has
  SOME signal and is wrong in one direction only, which is the more dangerous shape.

RESOLVED 2026-08-09 by the python retirement, NOT by a fix — recorded because 'fixed' and 'the code is gone' are different things. The nudge, including its NOT-YOURS exclusion, lived only in amux-server.py (792ce1f^:amux-server.py, exclusion at line 20190); that file is deleted from HEAD and nothing in crates/ implements it. AF-38 discarded.
  The finding survives as AMUX-2638: when the nudge is ported it must resolve ownership through the staged-guard path, not from dirty-tree membership — that substitution IS the bug and a fresh port reintroduces it by default, because `git status` is the obvious source.
  Also note the capability is simply GONE meanwhile: nothing tells any session about uncommitted work, on a shared checkout with ~7 lanes and 82 dirty files.


## Two entries validated and deleted today have already recurred
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-38
SYMPTOM: Under the new protocol (fix -> originating session validates -> delete the entry), 35
  entries were deleted on 2026-08-09. Within hours, two of that day's validated-fixed classes
  recurred: AC-284 (assignment notices for deleted cards - amux-cloud produced counter-evidence
  DURING the sweep, so it was caught and reopened before deletion) and AC-300 (the idle nudge
  telling a session to commit a peer's in-flight work - deleted as confirmed-fixed, then hit me
  hours later, now AF-38).
COST: no work lost yet. The cost is diagnostic: when AC-300's class recurred, the entry
  describing it was gone, so recognising it as a RECURRENCE rather than a novel bug depended on
  me happening to remember deleting it that morning. The next session will not have that.
FIX: not a request to reverse the protocol - deletion is Ethan's call and he made it, and git
  history at e35bf7d preserves the text. The cheap mitigation is already half-built: the sweep
  appends each deleted entry's COST line to its card before deleting (30 done). Extending that
  to carry the SYMPTOM line too would make a recurrence recognisable from the card alone, which
  is where someone hitting it again would actually look. amux-cloud argued the general form of
  this before the deletions and was told, correctly, that the call was made; this entry is the
  evidence they asked for rather than a re-litigation.

## A 405 on an unrouted path — the GET-only SPA catch-all makes "no such route" wear "wrong method"
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (AMUX-2610 build lane)
CARD: AMUX-2610
SYMPTOM: Any non-GET request to a path the rust router does not mount answers 405 (with
  Allow: GET) from the SPA catch-all (`/{*path}` is registered GET-only in static_files.rs),
  while a GET to the same unknown /api path answers 404. So POST /api/lookup logged a 405 —
  a status that says "path exists, method wrong" about a path that does not exist at all.
  Verified live on 8824 and reproduced against the router in tests.
COST: fed directly into the incident that motivated AMUX-2610: a model diagnosing a 405 had
  to grep mod.rs + module routers to discover whether the path was even routed — the
  expensive token spend Ethan flagged. The status code alone cannot discriminate the three
  honest cells (wrong method / unknown path / route landed after the rows).
FIX: not changed at the router (the catch-all's GET-only shape is load-bearing for the SPA
  shell); fixed at the instrument instead, same commit as this entry: /api/logs/analyze
  computes a per-405-group verdict that names the cell explicitly, including "no route
  exists at this path — the 405 is the GET-only SPA catch-all answering a non-GET", and
  /api/debug/routes serves the ROUTE_TABLE so "is X routed" is a GET, not a grep.

## Every direct prompt to a rust worker ran TWICE — the ledger card re-dispatched its own prompt
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (AMUX-2613 build lane)
CARD: AMUX-2613
SYMPTOM: The full-stack pomegranate E2E's claude transcript carried the same prompt twice:
  once raw ("Remember the word: pomegranate…"), then again wrapped in an ExecuteTask
  assignment ("Task tsk_…: Remember the word: pomegranate **Prompt:** …"). The pump
  delivered the message, capture_prompt_card minted its no-silent-work ledger card as
  `todo` — and an owned `todo` card is Runnable to the planner (deliberately: the L3
  380-invisible-todos lesson), so the next tick assigned the card and redelivered the
  same prompt as new work. Two turns, two token spends, two "OK"s, per prompt.
COST: one failed E2E verdict (turn-count wait tripped early on the phantom turn) plus the
  double token spend this would have silently charged EVERY direct prompt to every
  rust-orchestrated worker; invisible unless you read a provider transcript, since each
  delivery looks legitimate alone — the board showed one card and the queue two commands.
FIX: same commit as this entry: the ledger card mints as `doing` (in flight, owner
  attached), which disposition() reads as Assigned — never re-dispatched; an un-moved
  card after the turn lands in the stall detector's designed drift cell. Regression:
  captured_ledger_card_is_not_redispatched_by_the_planner (runtime.rs) fails on the
  pre-fix mint.

## The rust request log recorded a ~15-second restart choreography as a 76ms request
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: amux-rust (lifecycle-fix subagent)
CARD: AR-111
SYMPTOM: Forensics on the amux start incident: `_amux_request_log` shows
  `PATCH /api/sessions/amux/config` at ts 19:10:35 with latency 76.26ms — but the SAME
  request wrote its "Captured before model swap" log marker at 19:10:20 and the env
  header at 19:10:35.42, i.e. the handler ran a synchronous ~15s stop/relaunch
  choreography that the request log renders as a sub-100ms call. Whatever the
  middleware stamps (completion-time ts + an inner-layer latency, or a batched flush
  clock), a long-running request is indistinguishable from a fast one.
COST: ~30 minutes of incident reconstruction chasing a phantom second actor, because
  the timeline read as "capture at :20 cannot belong to a 76ms request at :35" — the
  instrument manufactured a contradiction that had to be disproved with three other
  artifacts (env header, session log markers, session_events).
FIX: request-log middleware should stamp arrival ts and wall-clock latency around the
  WHOLE handler future; a restart choreography should be a visibly long row.

## `amux serve --help` documented two flags that the same command now refuses
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: amux (python-removal doc sweep subagent)
CARD: AMUX-2621 (sibling umbrella for python-era doc drift; no dedicated card — this
  session was scoped to file edits and did not create one)
SYMPTOM: `cmd_serve` was retargeted from the deleted `amux-server.py` to the Rust
  binary, which takes NO argv, so it gained `die "amux serve takes only an optional
  port now"`. Its `--help` block was left untouched and still advertised
  `--bind host[,host,...]` and `--no-tls`, with four worked examples using them
  (`amux serve 8822 --bind 127.0.0.1`). Every one of those examples now exits 1.
  The top-level `amux --help` carried the same dead `[--bind ...]` spelling plus the
  pre-cutover default port (8822, which is now the LEGACY compat port, not the default).
COST: no incident yet — caught during the doc sweep. The trap it was set to spring:
  the sanctioned instruction and the failure are the same action (ethos rule 7's
  sharpest variant, AMUX-2140), so a session following `--help` literally gets an
  error and no way to tell "I typed it wrong" from "the help is stale".
FIX: help rewritten to the Rust reality — positional port only, mapped onto
  AMUX_RS_PORT, AMUX_RS_LEGACY_PORT documented, and an explicit note that `--bind`
  and `--no-tls` are GONE (Python-server flags; the binary always binds 0.0.0.0 and
  always serves HTTPS) so the removal reads as deliberate rather than as a bug.
  Generalisable: when a verb is retargeted to a new backend, its `--help` is part of
  the verb — retarget both or the command starts lying.

## e2e auth tests flip green->red mid-session: the server under test is rebuilt from a shared checkout that moves between runs
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: no-silent-actions agent (subagent; no $AMUX_SESSION in env)
CARD: none (agent has no session identity to attribute a card; parent should file — see report)
SYMPTOM: three consecutive runs of `npx playwright test --config e2e/playwright.config.ts`
on the same working tree: run 1 = 83 passed / 0 failed; run 2 = 12 failed; run 3 =
5 failed, all in phase0 auth ("protected API rejects a bad bearer token" expected
401, got 200) + settings_missing_endpoint_probe. Nothing in the diff between runs
was mine — the config's webServer runs `cargo run -p amux-server`, so every run
rebuilds whatever the concurrent lane has landed in crates/ since the last one.
The 401->200 flip itself looks like a REAL auth regression landing upstream while
I was testing the SPA layer.
COST: ~15 minutes ruling out my own SPA-only changes as the cause of server-side
auth failures; and a possible live auth regression (bad bearer accepted with 200)
observed but not attributable to a commit from here (NEVER-run-git constraint).
FIX: same instrument the CLAUDE.md /health-build bracket prescribes, applied to e2e:
have playwright.config.ts record the server build hash (GET /health .build) into the
run report so a mid-session flip names "the binary moved" instead of reading as
flaky tests; separately, someone with git access should bisect the 401->200 auth
behavior on current crates/amux-server HEAD.

## Peek showed 9% of each line — a `white-space: pre` on #peek-body killed wrapping
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: peek-render agent (subagent; no $AMUX_SESSION in env)
CARD: none (agent has no session identity to attribute a card; parent should file — see report)
SYMPTOM: peek stopped wrapping. Measured on #peek-body against a 220-col lane:
scrollWidth 4196px vs clientWidth 1416px at 1440px desktop (2780px of every line
unreachable without horizontal panning), and 4196 vs 366 at 390px phone — about 9%
of each line visible on the platform amux optimises for first. Long lines were cut
at the right edge mid-sentence with no wrap and no visible affordance to scroll.
COST: peek unusable for prose on a phone for the ~1h the build was live; and a
misdiagnosis shipped with it — the complaint that motivated the change ("a diff
wrapped into a ~710px column with two thirds of a 2000px view empty") was read as a
CSS wrapping bug when it was the pane width. The filing session's own lane was at 94
columns; 94ch x 7.49px = 704px, i.e. the "~710px column" was measuring the tmux pane,
not the stylesheet. The CSS change could not have fixed it and cost prose wrapping.
FIX: fixed — removed the #peek-body override so .overlay-body's pre-wrap/break-word
applies again (python's behaviour, byte-identical). Peek never needed a global `pre`:
wrapBoxBlocks() already gives each box-drawing run its own `.peek-box`
(white-space:pre; overflow-x:auto) so tables/diffs keep alignment in their own
scroller, and _fitRules() replaces full-pane rules with a fitted element. The
container `pre` defeated both. A comment at the site records the measurement so the
override is not re-added a third time.

## Opening peek permanently narrows the worker's tmux pane — observing changes the observed
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: peek-render agent (subagent; no $AMUX_SESSION in env)
CARD: AR-110
SYMPTOM: peek POSTs /resize to fit the pane to the viewer, and tmux pins
`window-size manual`, so the width persists after the viewer leaves. Verified live:
amux-test-claude was 220x50, one peek at a 390px viewport left it at 50x50 and it
stayed there. Across the fleet at scan time: mixpeek-autopilot 50 cols, amux 102,
amux-frustrations 94, amux-rust 94 — all real lanes emitting at a fraction of their
spawn width (220) for every later reader, because someone once peeked from a phone.
The floor is Math.max(50, ...) client-side and .clamp(50, 300) server-side, so 50 is
reachable and sticky.
COST: one wrong root-cause and a shipped CSS change that had to be reverted (see the
entry above) — the narrow pane presents exactly as "the renderer is wasting the
viewport", and nothing in peek shows the pane's column count, so the reader cannot
tell a narrow pane from a narrow render. Ongoing: any lane left narrow emits
hard-wrapped output to every future viewer and to its own transcript.
FIX: AR-110. Two parts worth separating — (1) do not let a transient viewer set a
persistent property of someone else's worker (restore on peek close, or scope the
resize to the read rather than the session); (2) surface the pane geometry in peek,
so "why is this 50 columns wide" is answerable from the instrument instead of from
`tmux list-sessions`.

## The subagent switcher is wired end-to-end and reaches 0 of 50 sessions
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-09
SESSION: peek-render agent (subagent; no $AMUX_SESSION in env)
CARD: none (agent has no session identity to attribute a card; parent should file — see report)
SYMPTOM: #peek-agent-nav (the ⌂/▲/▼ strip), agentNav(), the clickable .peek-agent-row
rows and the rust `agent-nav` verb are all present and byte-identical to the python
original — nothing was lost in the SPA extraction. The strip is gated on a VISIBLE
panel row (`⏺ main`/`◯ main`/`● main`/`○ main`) in the last 8 non-empty pane lines.
Running that predicate verbatim over every running session: 0 of 50 match, so the
strip is display:none everywhere, always. 46 of 50 DO show Claude's `← 2 agents`
status hint, but pressing ← (verified on an idle test session) opens the background
CONVERSATION manager — "Your conversation moved to the background · 4 awaiting input
· 0 working · 0 completed" with conversation rows — not a subagent panel with a
`main` row. Probe validated both ways first: a synthetic panel returns true, prose
returns false, so the zero is a real absence and not a broken matcher.
COST: a feature that looks complete in code review, in three layers plus a backend
verb, and that no user has ever been able to reach. Ethos rule 1 in its exact shape:
capability that exists but is received by nobody by default.
FIX: needs a live specimen of the current Claude Code agents panel to re-derive the
gate against — the `⏺ main` shape it looks for is either gone or only reachable from
a state nothing in the fleet enters. Do NOT widen the gate to the `← N agents` hint
without that: the existing comment warns that with rows hidden the nav keys open the
background-shells manager, and that is exactly what pressing ← did here. Separately,
what all 46 lanes actually have is background CONVERSATIONS, and amux exposes no
switcher for those at all — that is the reachable version of the same affordance.

## Every session log on the fleet stopped growing while tmux reported piping ON
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2628)
CARD: AMUX-2628
SYMPTOM: `~/.amux/logs/*.log` frozen at 19:30 for 50 sessions, still frozen at
21:02. Every individual signal read healthy: `#{pane_pipe}`=1, the writer process
alive, its fd 1 on the same inode as the file on disk, the file present and
non-empty. The pipe writer was `python3 -c "... for line in sys.stdin.buffer: ..."`,
whose `readline()` blocks until a LINE FEED — and a full-screen Claude TUI redraws
in place with CARRIAGE RETURNS (measured on the real amux-frustrations.log: 106,081
CR bytes against 2,506 LF, 42:1). So megabytes accumulated inside the reader and
nothing was ever written. Two independent second defects rode along: `pipe-pane -o`
TOGGLES an already-piped pane OFF (tmux 3.6a: arm -> 1, arm again with -o -> 0), so
starting an already-piped session silently disabled its logging and 20 of 50 fleet
sessions were sitting unpiped; and `capture_log_tail_for_reload` detached the pipe
and never re-armed it, so any provider/model/effort swap ended logging permanently.
COST: 90+ minutes of fleet-wide terminal history lost outright for the lanes that
were parked, and 9.3 MB recovered from the stuck reader buffers only because the
re-arm made the old writers hit EOF and flush. Two sessions' logs (amux-rust,
amux-frustrations) were each holding ~3 MB of unwritten output. Nothing anywhere
reported the outage — this was noticed by a human reading a log, an hour in.
FIX: fixed. Writer rewritten to chunked `read1` + `select`, treating CR as a
terminator (`python3 -u` does NOT help — it unbuffers stdout, the block is on the
READ side); `-o` dropped from both arm sites; reload capture re-arms. The reason
nobody saw it is now its own fix: `GET /api/debug/logs` correlates pipe state, log
mtime, pane activity and writer liveness per session and computes the verdict
("stale: piping on but no write in Ns while the pane was active"), with `log_verdict`
extracted as a pure function and unit-tested against this incident's own numbers so
the alarm is demonstrably able to fire.

## A hand-written `ps | grep` probe matched its own command line and invented 3 phantom failures
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-09
SESSION: (agent, AMUX-2628)
SYMPTOM: verifying the fleet re-arm, `ps -eo pid,command | grep 'sh -c python3' |
grep -c 'for line in sys.stdin.buffer'` reported 3 sessions still on the OLD writer.
There were zero. The bash tool's own process carries the search string in its argv,
so the probe matched itself three times, and the follow-up that tried to name the 3
sessions printed 60 lines of shell fragments as if they were session names — which
is the only reason it was caught. Re-run as a child-of-tmux-server filter: 52 new
writers, 0 old.
CARD: AMUX-2628
COST: ~10 minutes and one nearly-reported false conclusion ("3 sessions did not get
the fix") in the final report. The ethos file already names this exact trap ("a probe
that matches itself in a ps listing"), which is the point: the rule was written down
and it still did not fire at the moment of use, because the answer looked plausible.
FIX: the durable version is not "remember to exclude your own pid" — it is that
fleet-wide process questions should be asked of the server, not of `ps` by hand.
`/api/debug/logs` now answers "which sessions have a live writer" as structured data
(`writer_pid`, `writer_age_s`) computed in one place, so the next session does not
hand-roll the grep at all.

## A hot model switch that HAD landed was reported as failed, because the pane sat on an unanswered confirmation dialog
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2617)
SYMPTOM: the new hot-model-switch path (`/model <id>` delivered to the live agent
instead of a restart) reported `mode:"restart"`, `hot_error:"no acknowledgement in
the pane within 5s"` — twice — on switches that had ACTUALLY WORKED. Claude Code
guards a mid-conversation model change behind a selector ("Switch model? … 1. Yes,
switch to Haiku 4.5 / 2. No, go back") that appears in no `--help` output and never
on a fresh pane, so it only exists once a session has a real conversation. amux typed
the command, the dialog opened, nobody answered it, and the verifier timed out on an
ack that could not render yet. The restart it fell back to then answered the dialog
with a stray keystroke and RESUMED — which is the only reason the ack showed up at
all, replayed into the new pane.
CARD: AMUX-2617
COST: ~40 minutes, and the natural next move from the symptom alone is to widen the
timeout, which would never have helped. The variant cost it a second time: the fix
anchored on the dialog's TITLE, so `/effort` (titled "Change effort level?", same
body) kept falling back while `/model` worked, and the two failures looked nothing
alike.
FIX: fixed in the same change. Two parts, and the second is the durable one:
(1) `config_switch_confirm_key` answers the dialog, anchored on the BODY line both
variants share and picking the option by its "Yes" TEXT rather than by position, so
a reordering cannot turn a confirm into a cancel; (2) the fallback now logs the pane
tail plus `echo_seen`/`ack_seen` before restarting. A fallback that leaves no trace
is indistinguishable from a switch that never happened — the pane tail is what named
the dialog within one run, both times, and it is what makes the next unexplained
timeout decidable from the log alone.

## The API answered 200 {"ok":true,"message":"sent"} for a message the model never received
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2629)
CARD: AMUX-2629
SYMPTOM: `POST /api/sessions/amux-rust/send` at 20:55:25 answered `200 {"ok":true,
"message":"sent"}` in 1050ms. The text was typed into the pane and the Enter did not
register, so it sat in Claude Code's composer for 10m50s — the conversation JSONL
receives it at 21:06:15, only because a human pressed a bare Enter. Nine steering
messages were queued behind it. Every instrument agreed with the lie: `session_events`
holds one `message.sent` row at 20:55:26 and nothing after it, `steering_history` had
already dequeued the previous delivery as delivered, and the pane looked idle. The one
artifact that discriminates is Claude Code's own `queue-operation: enqueue` record —
it writes one for every mid-turn Enter it ACCEPTS (10 of them in that same transcript)
and there is none at 20:55. Nothing amux stores could have told anyone that.
COST: an hour of a lane sitting idle with the owner's instruction on screen and nine
commands queued behind it; then the owner's time to notice and press Enter himself.
Worse than the hour: the diagnosis was IMPOSSIBLE from amux's own data — every
recorded fact was consistent with successful delivery, so the natural conclusion from
the ledger alone ("it was delivered, the lane ignored it") is wrong and blames the
model.
FIX: fixed on AMUX-2629 (`verify_submitted` + `send_outcome` in api/session_verbs.rs).
"sent" is now read back from Claude Code's artifacts — the composer contents and the
conversation JSONL — never inferred from the `send-keys` exit code, and the response
carries `submitted` / `submission` / `retried` so the four outcomes a single `ok` bit
used to cover are distinguishable. THE UNDERLYING DEFECT IS NOT FIXED AND CANNOT BE
FIXED HERE: keystroke delivery into a TUI is best-effort by construction. Twenty
attempts across four pane states failed to reproduce the dropped Enter, which is the
point — it is intermittent, so it can only be detected and retried, not timed away.
The real fix is protocol delivery, where submission is an ACK (opencode::structured).

## Ghost-rescue can only rescue the messages that happen to carry a timestamp prefix
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: (agent, AMUX-2629)
CARD: AMUX-2629
SYMPTOM: the ported `[ghost-rescue]` sweep decides a stuck message is amux's — and so
safe to submit — only when the composer text starts with the dashboard's `[H:MM AM]`
stamp (py:9160, the only sound discriminator: anything else risks submitting a
half-written human thought). A read-only scan of the live fleet found 13 lanes holding
composer text with no matching user message in their transcript — `backend` "continue
with the queue", `ethan-dev` "push it", `mvs-infra` "Run the MVS prod health loop per
the runbook", and ten more — and ZERO of the 13 carry the stamp. The dashboard applies
the prefix inconsistently (`cmd_history` for amux-rust alone has both prefixed and
unprefixed human sends in the same hour), and agent-to-agent and nudge messages never
carry it.
COST: not yet counted in minutes, but it is 13 messages the fleet is currently sitting
on, and a fallback that covers 0% of the live population reads as protection that is
not there. Deliberately not widened: guessing "this looks like amux" would eventually
submit a person's unfinished sentence, which is worse than the stall.
FIX: two honest options, both upstream of the sweep. (1) Make the stamp universal — if
every amux-originated message carried a machine-readable origin marker, the guard would
be exact instead of a heuristic. (2) Better: deliver over the structured protocol, where
there is no composer to get stuck in and nothing to sweep for; the sweep's exit condition
is written into its module docs for that reason.

## `amux board` help executed a command out of its own help text
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-frustrations
CARD: AF-38
SYMPTOM: `amux board discard AF-38` (a verb that does not exist) printed the unknown-verb
  error AND `/Users/ethan/.local/bin/amux: line 1726: review: command not found`. The help
  body is emitted with an UNQUOTED `cat <<EOF`, so backticks in it are command substitution.
  Line 1760 had a literal `review` in backticks while lines 1753/1757 correctly escape
  theirs, so bash ran `review`, printed the error, and spliced its empty stdout — silently
  deleting the words from the rendered help ("handed to. " then nothing).
COST: two minutes and a wrong first impression that the CLI was broken. The real cost is
  latent: any backticked text anyone adds to that help block gets EXECUTED on every
  `amux board` with no verb. This is the same class the help text itself warns about two
  lines above, for `--outcome`.
FIX: escaped the backticks to match the neighbouring lines (and restored the text the
  substitution had been eating). Verified: stderr is now empty and the line renders in full.

## Usage meter said "no token" while the token was fine and Anthropic was rate-limiting
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: rust-usage
CARD: RU-1 (follow-on SPA label gap); this entry's own fix is in api/usage.rs
SYMPTOM: Settings showed "Claude subscription usage unavailable on this host (no
  token, expired token, or probe failed)" — one string for four different causes.
  The keychain credential was present and unexpired the whole time; the real cause
  was HTTP 429 from api.anthropic.com, intermittent on a host running ~95 Claude
  Code processes against one account. Two servers on this box disagreed 20s apart:
  one served real limits, the other served the same "unavailable" sentence.
COST: The meter read as a broken install for as long as it was dark, and the one
  message could not distinguish "log in again" (user action) from "wait, it clears
  itself" (no action). The endpoint's own #[ignore]'d live test was GREEN throughout,
  because it only iterated `usage.windows` — zero windows iterates zero times, so a
  totally failed probe asserted nothing (ethos rule 7, the vacuous-check shape).
FIX: Fixed. The probe is now discriminating (provider/claude.rs `UsageProbe`:
  NoToken / Expired / Http(code) / Transport / BadShape) and api/usage.rs turns each
  into its own reason plus a stable `cause` tag, with the HTTP status included. The
  live test now asserts the discriminator (a host WITH a credential must never report
  NoToken) instead of iterating a possibly-empty vec. Because the 429 is intermittent,
  a good reading is also kept and re-served for AMUX_USAGE_STALE_S (default 600s)
  marked `stale: true` with the live failure in `stale_reason`, so the meter stops
  flickering dark.

## A peer's `install` shipped my uncommitted, unverified WIP straight to the live server
AREA: cli
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: board-drive (AMUX-2637)
CARD: AMUX-2637
SYMPTOM: I created `crates/amux-server/src/runtime_jobs/board_drive.rs` and wired it
  into `lib.rs` at ~22:0x, having run NO tests yet. At 22:07 another session rebuilt
  and installed `~/.local/bin/amux-server-rs` from this shared checkout; `strings` on
  the live binary shows `runtime_jobs/board_drive.rs`, and `/api/debug/board-drive` —
  an endpoint I had written minutes earlier — answered on :8822. Within 3 minutes the
  live loop had claimed AF-38 and AR-112 and routed two review nudges on the real
  fleet. I never installed anything.
COST: Unverified code reached production and mutated the live board. It happened to be
  correct (AF-38/AF-34/AF-33/RH-96 all moved, WIP-1 held), but two defects I found
  MINUTES LATER by testing shipped with it: a lane was told "you went idle holding
  BDQ-1" one tick after being handed BDQ-1, and a review route re-fired every 60s until
  the 24h per-card budget was spent in three minutes. The live build still carries both.
  The `git push` guard in CLAUDE.md ("check what you are shipping that is not yours")
  covers the git dimension only; the BUILD dimension has no guard at all, and it is
  strictly worse — a push ships committed work, an install ships whatever is in the
  working tree, including a file that has never been compiled by its author.
FIX: The install path should refuse, or at minimum announce, a build made from a dirty
  tree containing files no commit references. Cheapest honest version: have the
  installer stamp `git status --porcelain` + the untracked file list into the binary
  and surface it at `/health` as `built_from_dirty_tree: [...]`, so "is this build
  someone's WIP?" is answerable from the instrument everyone already reads instead of
  from `strings`. Related to the shared-checkout push rule, same root: on a shared
  checkout, one session's routine action ships another session's in-flight work.

## The board-drive trace reported `eligible_todos: 0` for lanes with cards waiting
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: board-drive (AMUX-2637)
CARD: AMUX-2637
SYMPTOM: `/api/debug/board-drive` — the surface built specifically so a skipped lane is
  distinguishable from a dead loop — showed `bdq-assign  skipped  not-running  elig=0`
  while BDQ-1 sat dispatchable in that lane's queue. The counts were only filled in on
  the code paths that got PAST the liveness and turn-boundary gates, so every lane
  stopped by a gate reported its backlog as zero.
COST: Caught during my own verification, before anyone else read it — but it is the
  exact ethos rule 4 failure inside the instrument written to prevent it. The reader's
  question is "how much work is this lane sitting on, and why did it get none", and
  half the answer was a confident zero. A wrong number is worse than a missing one.
FIX: Fixed in board_drive.rs `drive_lane`: the backlog counts are computed BEFORE any
  gate and attached to every trace row, whatever stopped the lane. General form: when a
  trace has both a "why" and a "how much", the "how much" must not be computed on the
  happy path only.

## SUPERSEDES the "13 lanes holding stuck text" entry above — they were empty; the reader was wrong
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2629)
CARD: AMUX-2629
SYMPTOM: I reported 13 live lanes "holding composer text with no matching user message in their
transcript" and flagged that the ghost-rescue prefix guard would rescue none of them. Both halves
were built on a false reading. All 13 composers were EMPTY. What they held was Claude Code's DIM
suggestion — `\x1b[2m` — and `_pending_input` (py:25349, ported faithfully) strips ANSI before
reading the ❯ line, which makes a suggestion and real typed input the same string. Two other
sessions then spent time on it: one pressed Enter, C-m and Escape+Enter on those lanes and reported
that none worked (correct — there was nothing to submit), and reasoned toward a
background-conversation-manager theory; another read the "← 2 agents" marker as the common cause. It
is on every lane, including a brand-new claude in an empty directory that accepted Enter 20/20 times.
COST: three sessions' time chasing an artifact, one wrong hypothesis published, and the next step
queued up was submitting 13 stale instructions into live lanes — which would have been the real
damage. My own entry above is what made it look corroborated.
FIX: `composer_state()` in api/session_verbs.rs — the dim attribute decides, and callers must pass
the RAW `capture-pane -e` output. `pending_input` is DELETED rather than fixed: a function that can
be called with a stripped frame re-creates the bug silently, so there is now exactly one way to read
the composer and it cannot be handed the wrong input. The lesson generalises past this bug: when a
probe's output is the same for two states, the fault is the probe, and "strip the ANSI first" throws
away the only bit that distinguished them.

## A lane froze its own steering queue for four hours by writing the words "esc to interrupt"
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2629)
CARD: AMUX-2629
SYMPTOM: amux-rust held 10 steering messages for up to 229 minutes while every gate passed — env
file present, tmux alive, self-report `idle` 198s old, composer empty, status bar `⏵⏵ bypass
permissions on (shift+tab to cycle) · ← 2 agents`. The refusal came from send_text's active-signal
re-check, which is `"esc to interrupt" in tmux_capture(name, 12)` (py:25650) — an UNSCOPED substring
match over the whole pane. Lines 26-27 of that pane were the lane's own prose about a status-detection
fix: `Workers with "bypass permissions on" + "esc to interrupt" on the status bar were misdetected as
IDLE`. So the lane most likely to write that string is the lane that works on the scraper, and it
blocked itself. Compounding it, the tick took ONE row per lane oldest-first and moved to the next
LANE on refusal, so one undeliverable row froze all ten.
COST: four hours of a lane not receiving the owner's instructions, while the owner asked twice why
workers were not moving. Finding it needed a hand-written DB read plus a pane capture, because the
tick logged only successes — a skip left no trace anywhere. A peer independently reached a different
root cause (the @-picker guard) from the same symptoms; it was not that, and fixing only the @ path
would have left the lane frozen.
FIX: three parts. (1) `pane_bar_says_generating()` scopes the marker to the bottom 3 non-blank lines,
so prose cannot be a status. (2) The tick and the reactive deliverer now walk to the lane's NEXT row
on a refusal instead of abandoning the lane — one bad row can no longer freeze a queue. (3) Every
skip is logged with its reason, a lane whose oldest row exceeds 20 minutes is announced at WARN, and
`GET /api/debug/steering` exposes per-lane depth, oldest age and last refusal reason. Reproduced
end-to-end on a throwaway lane put into the same state: pre-fix predicate = 0 deliveries in 10 ticks;
bar-scoped predicate = both rows delivered, @-mention included.

---
## Six SPA-consumed API families 404 in production and nothing anywhere says so
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux-rust (RR-0130/0131 cutover sweeps)
CARD: AR-114, AR-115, AR-116, AR-118, AR-119, AR-120
SYMPTOM: The RR-0130/0131 live-data sweeps compared what the SPA READS against what the
  rust server SERVES. Six families the shipped dashboard calls answer 404 on the live
  server, and every one exists nowhere in `crates/`: `/api/channels/{a}/{b}/messages`
  (the DM drawer, polled every 2500ms), `/api/log-search`, `/api/memory/global`,
  `/api/observability`, `/api/review/week`, `/api/review/digest`. A seventh,
  `/api/metrics`, answers 200 with a completely different document than the SPA reads
  (`{board,events_journal,leases,queues,...}` vs the expected `data.sessions[]` +
  `data.system` + `data.server`), and the SPA calls `s.cpu_percent.toFixed()` on it
  unguarded. Nothing errored at cutover, no check went red, and the boundary registry
  (`/api/debug/boundary`) reports `proxied: []` — i.e. "everything is native" — because
  a family nobody implemented is not a family anybody proxied.
COST: These shipped broken at the python retirement and were still broken hours later;
  they were found only because someone diffed SPA call sites against live routes by
  hand. `/api/observability` is the entire Cost view, so 387,524 `token_ledger` rows
  have had no reader since cutover. Same failure shape as AMUX-2637 (board drive) and
  AMUX-2629 (submission): python-only capability, unported, invisible because absence
  does not raise.
FIX: The missing instrument is the one that would have caught all seven at once — a
  check that walks the SPA's own fetch call sites and asserts each resolves to a mounted
  route. `ROUTE_TABLE` already proves the reverse direction (claimed routes are routed);
  nothing proves the SPA's demands are met. `/api/debug/boundary` should report families
  the SPA calls that resolve to neither native nor proxied, so "unported" is a state the
  registry can express instead of one that reads as clean.

---
## Two rust call sites defer work to "while the Python server runs" — python is retired
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: amux-rust (RR-0131b sweep)
CARD: AR-117
SYMPTOM: `api/session_verbs.rs:5910` says `_write_claude_memory (symlink into
  ~/.claude/projects) is not ported — Python owns the memory composition during
  coexistence`, and `api/scope.rs:41` says `While the Python server runs (the migration
  soak) its next compose picks the edit up; the gap closes with the memory-compose port,
  not here.` Both are honest, well-written deviations — and both were made void the
  moment python was shut down. A worker memory write now updates
  `~/.amux/memory/<name>.md` and never composes `~/.claude/projects/<proj>/memory/
  MEMORY.md`. RR-0131b's own acceptance line ("MEMORY.md regenerated from migrated
  entries") cannot pass.
COST: Silent divergence between the memory a session edits and the memory Claude Code
  loads, for an unknown number of edits since cutover. Found only by grepping comments
  during a sweep; no test, no check and no doc references either site.
FIX: Deviations whose mitigation is "the other server covers it" need to be enumerable.
  A `GRACE:`-style marker (or a `python_covers_this` const the retirement checklist
  greps) would have turned python's shutdown into a list of exactly what stopped being
  covered, instead of a discovery process. RR-0154's shutdown criteria should include
  that grep.

---
## The gate-blocked 409 tells every agent to GET a route that does not exist
AREA: gates
SEVERITY: annoys
STATUS: open
DATE: 2026-08-09
SESSION: amux-rust (RR-0150 restart suite)
CARD: AR-123
SYMPTOM: Every gate_blocked 409 from `/api/board/<id>` carries
  `how_to_ack.contract: "GET /api/board/contract"` (`api/board.rs:1175` and `:1664`).
  `GET /api/board/contract` returns 404 `{"error":"item not found","id":"contract"}` on
  both a fresh build and the live server — it is being matched by the `/api/board/{id}`
  route as an item id. Hit it while making the restart suite move a card `todo -> doing`.
COST: Small on its own — the 409 also carries `gate` and `gate_ack`, so the escape is
  walkable without the contract. But it is ethos rule 6's exact shape: the one documented
  route out of a gate is the one action that leaves the sanctioned path, and it is the
  instruction amux itself prints. AMUX-2325 is the same defect one layer up.
FIX: Mount `/api/board/contract` ahead of `/api/board/{id}`, or delete the claim from
  both 409 bodies. Whichever — the test is that following the error message literally
  has to work.

---
## A worker whose pane died at launch reports `running: true` / `idle`
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux (cloud rust image, AMUX-2619)
CARD: AMUX-2644
SYMPTOM: Started a worker in the new cloud container. `GET /api/workers/<id>` returned
  `{"status":"idle","running":true,"state":{"state":"idle"}}` — a healthy-looking lane.
  `peek` showed what had actually happened: `--dangerously-skip-permissions cannot be
  used with root/sudo privileges for security reasons` … `Pane is dead (status 1)`.
  The tmux SESSION still exists after the pane dies (`remain-on-exit on`), so "the
  session is there" is true and "the agent is running" is false, and the status field
  reports the first while reading like the second.
COST: This is the single blocking defect of the cloud rust cutover — every agent lane in
  every workspace would have died at launch — and the worker list said nothing was wrong.
  It was found only because I peeked at a lane I had no reason to suspect. On the live
  host the same failure would present as "the fleet is idle", which is the one shape
  nobody investigates. `idle` is also what a correctly-waiting lane reports, so no
  amount of watching the status column can distinguish them.
FIX: `idle` must not be reachable when the pane is dead. tmux already knows
  (`#{pane_dead}` / `#{pane_dead_status}` are one `display-message` away, and the peek
  text carries `Pane is dead (status N)`), so this is a state the detector can express
  and currently does not. A `dead` state — or at minimum `running:false` — with the exit
  status attached. Related: the browser failure in the same container named its symptom
  (`CDP never answered within 12s`) and not its cause; both are the ethos rule 4 shape,
  where the diagnosis is impossible from what the instrument reports.

---
## `amux-rs why … | head` exits 101 with a Rust panic instead of 0
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-09
SESSION: rust-rebuild (RR-0109/0110 lane)
CARD: none — fixed in the same change that introduced the verbs; logging it because
  every OTHER verb in crates/amux-cli/src/main.rs still has it
SYMPTOM: `amux-rs why schedule SCHED-30` emits 220 lines for a schedule with 3,303
  recorded runs. Piped into `head -3` it printed the first lines and then exited
  **101** with a panic (`failed printing to stdout: Broken pipe`). Unpiped, the same
  command exits 5 — the verb's real "partial verdict" code. Rust ignores SIGPIPE, so
  `println!` unwraps a `BrokenPipe` write error into a panic.
COST: ~10 minutes chasing a phantom crash in the new endpoint. Worse than the minutes:
  `why` publishes exit codes as its machine-readable verdict (0 explained, 5 partial,
  6 cannot_tell), and a verb that exits 101 on the most ordinary shell idiom teaches
  a caller that those codes cannot be trusted. It also reads as "the instrument
  crashed on the case I was investigating", which is the worst possible false signal
  from a diagnostic tool.
FIX: `outln!` macro in crates/amux-cli/src/main.rs writes through a locked stdout and
  returns `Ok(0)` on a closed pipe. Applied to `search` and `why`. **The other verbs
  (`board list`, `board show`, `workers list`, `schedules list`, `health`) still use
  bare `println!` and still panic the same way** — `amux-rs board list | head` on a
  4,773-card board is the same bug waiting. The root fix is either resetting SIGPIPE
  to SIG_DFL at startup (needs a `libc` dep) or routing every verb through `outln!`.

---
## Uncommitted migrations reach the LIVE database within minutes, from another agent's server
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: rust-rebuild (RR-0109/0110 lane)
CARD: none — needs one; the mechanism is fleet-wide, not this lane's
SYMPTOM: I created `crates/amux-server/migrations/0013_search.sql` at 22:16:42 EDT and
  never installed or restarted anything. At 22:18:23 EDT the migration was applied to
  `~/.amux/amux.db` — the live 269MB database — creating 2 tables, 24 triggers and
  backfilling 5,021 rows. `scripts/rust-auto-build.sh` is NOT the culprit: it builds
  from a `git worktree` of HEAD and 0013 is not in HEAD. The cause is that some other
  session on this shared checkout ran a working-tree build of `amux-server` with the
  default `AMUX_DB`, which is the live file.
COST: No damage this time — the migration is additive and applied cleanly, and it is
  in fact the best live evidence I have. But I explicitly set out to test against a
  `.backup` copy precisely so I would not write to the live DB, and the live DB had
  already taken my schema before I made the copy. A session cannot honour "never touch
  the live database" when a peer's ordinary `cargo run` applies that session's
  uncommitted migrations to it. The same mechanism with a destructive or wrong
  migration is a data-loss event with no author and no audit line.
FIX: make the live database opt-IN for a locally-built binary. Either default
  `AMUX_DB` to a scratch path unless `AMUX_ALLOW_LIVE_DB=1`, or refuse to apply a
  migration whose version is absent from HEAD unless the same flag is set — the
  discriminator (`git cat-file -e HEAD:<migration>`) is one cheap call, and it exactly
  separates "this build is the deployed one" from "this build is someone's working
  tree". Right now nothing distinguishes them and the live file is the default.

## A continuously-busy lane starved its own queue forever: the boundary gate has no deadline
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: (agent, AMUX-2642)
CARD: AMUX-2642
SYMPTOM: `steer_lane_at_boundary` returns true only on `idle`, so a lane that works continuously has
no boundary and its queue never drains. Measured on the `amux` session — status `active` with a
6-second-old tool-hook report, correctly working — holding five messages queued 22:06..22:28, none
delivered; amux-rust held ten, the oldest 229 minutes. From the sender's side the only evidence is
"nothing is happening", which reads as a hung worker. Three of amux's five carried `@`-mentions, which
under the old picker guard could never be delivered to a busy lane at all, so the two faults hid each
other.
COST: two lanes not receiving the owner's instructions for hours while he asked twice why workers were
not moving; a sender concluding a healthy lane was hung; and — the part that made it expensive — five
messages aging past 20 minutes with no signal anywhere: no log line, no card field, no endpoint.
FIX: three parts, and the third is the one that generalises. (1) `AMUX_STEER_MAX_AGE_S` (default 10
min): boundary first, but past the deadline the message goes into the running turn, where Claude Code
queues it and folds it in at ITS own boundary — real queue semantics implemented by the agent instead
of by amux waiting forever. A selector is still never overridden: answering a pending tool is the
user's call, not amux's. (2) Picker-shaped text now goes through `paste-buffer -p` at any length —
measured live, `@`-text TYPED mid-turn is lost 1/1 while the same text PASTED mid-turn is accepted
4/4, because a bracketed paste never opens the autocomplete. That is what makes the overdue delivery
safe for `@` messages rather than just for plain ones. (3) The gate and the send path now share one
predicate (`pane_is_at_boundary`). They did not for one build, and the disagreement deadlocked
delivery in a way that was a bug in neither half: the gate read the frame as idle (so it never
consulted the deadline) while the send path read it as generating (so it refused) — every tick,
forever. A view that disagrees with the mechanism it describes is worse than no view.

## `git add -A` on the shared checkout committed module declarations for five files it did not stage
AREA: cli
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux (batch: AMUX-2618/2599/2636/2634/2609)
CARD: AMUX-2654
SYMPTOM: `main` has not compiled since 22:43. Someone committed with `git add -A`, which swept in three
sessions' in-flight edits to `api/mod.rs`, `runtime_jobs/mod.rs` and `ghost_rescue.rs` — including the
`pub mod offline_origin; pub mod sessions_git; pub mod search; pub mod why; pub mod pane_size;` lines —
while the FILES those lines name stayed untracked, because they belong to other sessions who had not
finished. `scripts/rust-auto-build.sh` builds a worktree of HEAD, so it sees only committed files:
`error[E0583]: file not found for module offline_origin` x5. Four builder cycles logged BUILD FAILED in
`~/.amux/logs/rust-auto-build.log` (83ab8ac, 0b156bb, 1155f25 twice) and the stamp is still stuck at
62e9bdd. Separately and on the same checkout, a one-line edit of mine inside `resize_pane`
(`note_resize`, AMUX-2634) was silently reverted by a concurrent writer of `session_verbs.rs` between
my edit and my test run.
COST: nothing whatsoever errors. The builder is designed to keep the last good build on failure, so the
server stays up and every session tonight believes its change will deploy; none will, and no session is
told. It cost me a wrong conclusion in the other direction too: I measured the pane-restore timing three
times and read the results as a lease bug in my own code, because the reverted line produced *plausible*
behaviour (a restore, just too early) rather than a crash — the unit tests could not catch it, since what
vanished was the CALL SITE, not the function. Roughly 25 minutes, and it was only caught because the
live end-to-end test disagreed with the passing tests.
FIX: two halves. (1) Immediate: `git add` the five missing files (or revert the declarations), then
confirm with the builder's own recipe — `git worktree add --detach $W HEAD && cd $W && cargo build`.
(2) Structural, and the one worth building: CLAUDE.md's Deploy section already warns that a push ships
other sessions' COMMITS, but the same hazard exists one step earlier, at STAGING, and it is worse —
`git add -A` produces a HEAD nobody can build, whereas a bad push at least builds. A `pre-commit` hook
that refuses when the staged set contains a `pub mod X;` whose `X.rs` is untracked would have caught
this exact commit in under a second, and it is checkable by the machine rather than by remembering.
The generalisable point: on a shared checkout the unit of work is a PATH, never `-A`, and the tell that
the rule is not being followed is a build that fails in a file its committer never opened.

## The dashboard's "New worker" button cannot create a worker (405)
AREA: cli
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2655
SYMPTOM: `POST /api/sessions` answers `405, allow: GET,HEAD`. The create dialog shows
  "Create failed: error 405" and stays open. `POST /api/workers` exists but writes a
  `workers` table row, a different substrate from the `~/.amux/sessions/*.env` registry
  the fleet actually reads — so it is not a workaround, it creates an invisible worker.
COST: the only way to make a worker for a UI test was to duplicate an existing one; a
  user with an empty fleet has no path at all. Found only because a test needed it.
FIX: `sessions_legacy::create_session_legacy` + `.post()` on the route (written,
  uncommitted — this session is barred from committing). Verified 201 + worker present.

## Board card Delete removes the card and never deletes it
AREA: board
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2656
SYMPTOM: `DELETE /api/board/{id}` -> 405 (the route was `get().patch()` only).
  `deleteBoardItem` filters the card out of `boardItems` and re-renders BEFORE awaiting
  the request, and does not roll back on failure — so the card disappears at ~40ms, the
  server still has it, and the next `fetchBoard()` brings it back.
COST: this is the reported "tons of board items are not moving". Every board delete
  since the cutover was a no-op that looked like a success.
FIX: `board_store::soft_delete` + `board::delete_item` + `.delete()` on the route, and
  rollback in `deleteBoardItem`/`updateBoardItem` (written, uncommitted). Verified: card
  gone at 21ms, DELETE 200, 404 on re-GET, stays gone after refresh.

## Two endpoints disagree about whether a worker is running, and the card believes the wrong one
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2657
SYMPTOM: after Stop, `GET /api/sessions` says `running: true` forever while
  `GET /api/sessions/<n>/info` says `false`. The list derives running from "a tmux
  session named amux-<n> exists"; `stop` deliberately leaves the tmux shell alive. The
  card therefore never shows the Start button and Stop reads as having done nothing.
COST: a full measurement pass concluded "Stop returns 202 and does not stop the
  session" — the agent WAS dead; only the card was lying. Wrong conclusion, ~20 min.
FIX: one batched `tmux list-panes -a -F '#{session_name}:#{pane_current_command}'` into
  `FleetSignals.shell_only`, plus `agent_running()` as the single accessor so the two
  answers cannot drift again (written, uncommitted). Verified both agree after Stop.

## Every server refusal reached the user as a bare status code
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2658
SYMPTOM: `apiCall` did `showToast('Error: ' + r.status)` and dropped the body. Archive
  on a PINNED worker returns `403 {"error":"cannot archive pinned session — unpin
  first"}` and the user saw "Error: 403". Board gate 409s carry the full checklist AND
  the exact `cli:` string that would work; none of it was ever shown.
COST: this is most of the reported "nothing happens if i delete or archive" — the
  server explained itself every time and the UI threw it away.
FIX: `_apiErrText()` surfaces `error`/`message` plus `cli` (written, uncommitted).
  Verified: "403: cannot archive pinned session — unpin first" and "409: already
  holding doing — try: amux board doing AMUX-X --override-doing".

## Editing static/app.js does not rebuild the embedded dashboard
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-09
SESSION: amux
CARD: AMUX-2659
SYMPTOM: `crates/amux-dashboard` has no `build.rs` and rust_embed did not invalidate.
  After editing `static/app.js`, `cargo build --release -p amux-server` recompiled only
  `amux-server` and produced a binary serving the PREVIOUS app.js — the page reported
  `APP_VER 0.9.553` while the file on disk said `0.9.555`. Only
  `touch crates/amux-dashboard/src/lib.rs` forced the re-embed.
COST: a full verification pass was run against the OLD client and reported the fixes as
  not working (the pinned-worker toast still said "Error: 403"). ~25 min, and it is the
  loud-wrong kind: the sweep produced confident, plausible, false results. Worse in
  production — a dashboard-only commit can deploy stale client code silently.
FIX: add `crates/amux-dashboard/build.rs` emitting
  `cargo:rerun-if-changed=static` (and assert the served APP_VER matches the file, so
  the check can fail).

---
## The staged-guard endpoint was unrouted on the rust server and the hook printed nothing
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-rust
CARD: AR-132
SYMPTOM: `POST /api/git/staged-guard` answered **405** on the rust origin (8822/8824) —
  ~1,147 calls/hour — while the generated `.git/hooks/amux-staged-guard` wrapped the call
  in `except Exception: return 0  # fail open` and printed NOTHING. Every commit on every
  shared checkout ran with cross-session sweep protection OFF, and nothing anywhere said
  so. Two independent things hid it: the hook's silent fail-open, and the fact that an
  unrouted `/api/*` path on this server answers **405 from the GET-only SPA catch-all**,
  which reads as "wrong method" rather than "no such route".
COST: Two sweeps landed on this checkout in one night with the guard nominally armed, and
  a third while I was fixing it — peer commit 572047d swept four uncommitted `pub(crate)`
  edits of mine in `session_verbs.rs` into an unrelated steering fix. Same guard had
  already regressed to silence once before (AC-261), and nothing detected either
  regression: the only signal was the absence of output, which is what a passing check
  also looks like.
FIX: Ported natively — `crates/amux-server/src/api/git_guard.rs`, mounted in `api/mod.rs`,
  registry row in `py_proxy.rs`, ROUTE_TABLE row in `request_log.rs`. The server never
  500s into a fail-open: it answers `undecided` + `reason` when nothing could be compared
  and `degraded` when the verdict may UNDER-report (e.g. a cotenant whose transcript it
  cannot read), so an empty verdict is no longer indistinguishable from a clean one.

---
## A guard's only client swallowed the failure it existed to report
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: amux-rust
CARD: AR-132
SYMPTOM: The hook had ONE `except Exception: return 0` covering the local git calls, the
  HTTP request, and the JSON parse. "Server unreachable", "route gone", "server broke" and
  "answered garbage" were the same silent exit 0. Meanwhile `scripts/install-hooks.sh`
  refused to install the guard and told the reader to "start the amux server for this
  work_dir" — advice that was true under python and false after the cutover, because the
  generator was deleted with `amux-server.py` and nothing in rust writes the hook.
COST: The advertised recovery path did nothing, and running install-hooks.sh would have
  made things WORSE: the tracked `scripts/git-hooks/pre-commit` had no staged-guard shim,
  so installing it DELETED the shim the retired python had injected — turning the guard
  off while printing `ok .git/hooks/pre-commit matches ...`. A second silent-disable path,
  sitting inside the tool meant to repair the first.
FIX: `scripts/git-hooks/amux-staged-guard` is now the tracked source (the previous
  "second producer" objection died with the generator); the shim is back in the tracked
  pre-commit; install-hooks.sh installs BOTH, verifies the shim link — not just file
  equality — and probes the live endpoint so an unrouted server is reported where someone
  is already looking at hooks. Three distinct failure messages in the hook; fail-open
  stays, silence does not.

---
## The fleet's only physical liveness signal was a tmux field that never moves
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-09
SESSION: (Claude Code in iTerm — not a fleet lane, hence no session stamp)
CARD: AMUX-2662
SYMPTOM: `derive_status` reads `#{session_activity}` as "when did this pane last paint".
  On tmux 3.6a that field does not track pane output for a DETACHED session, and every
  amux lane is detached. Measured: 60 of 63 live sessions had a `session_activity` more
  than 60s older than their `window_activity`, and `amux-rust` — mid-turn, spinner
  repainting ~6x/s — reported a `session_activity` that had not moved in 34.5 HOURS (it
  was still exactly `session_created`). `#{window_activity}` was current for all of them.
COST: Both consumers of physical liveness were silently inert for the whole fleet, for as
  long as the field has been read. `now - act < 60 -> active` could never fire; the guard
  demoting a stale `active` transition fired for EVERY session on EVERY request. So fleet
  status was whatever the self-reports said and nothing else — which is the precondition
  that let one fabricated `idle` report label a working lane idle for 1076s with nothing
  able to disagree. The wrongness was invisible because a lane with working hooks looks
  correct anyway: the dead signal only shows up when the reports are wrong, i.e. exactly
  when you need it.
FIX: `activity = max(session_activity, window_activity)`, parsed by a pure function
  (`parse_list_sessions_line`) so the rule is testable without a tmux server — the first
  version of that test re-typed the parse inline and passed against the bug. Uncommitted
  in the shared checkout at time of writing.

---
## A read-only fleet probe returned "0 problems" while examining 0 lanes
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-09
SESSION: (Claude Code in iTerm — not a fleet lane, hence no session stamp)
CARD: AMUX-2662
SYMPTOM: The live card-vs-pane consistency check selects lanes to probe by "painted
  recently". Run against the PRE-FIX derivation it printed: `63 tmux sessions, 0 painted
  inside the probe window, 0 of those mid-turn, DISAGREEMENTS: 0`. A clean bill of health
  computed over an empty candidate set, because the activity field it selects on never
  moves (entry above). Post-fix the same command reports `5 painted, 2 of those mid-turn,
  DISAGREEMENTS: 0` — the same verdict, now meaning something.
COST: Nothing yet, because the discrepancy between the two runs was visible side by side.
  The cost it WOULD have had is the whole point: a sweep step reporting 0 disagreements
  daily, forever, over a candidate set that is structurally always empty. This is the
  empty-grep trap with a denominator, and the denominator is the only thing that gives it
  away.
FIX: The check prints its denominators — fleet size, lanes probed, lanes confirmed
  mid-turn — beside the disagreement count, plus the full status histogram, and the sweep
  contract (`docs/rust-migration/log-sweep.md`, step 6) says to read them. A count of 0 is
  only meaningful next to the number of things counted.

---
## A peer's commit shipped this run's in-flight work to origin, mid-edit
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: (Claude Code in iTerm — not a fleet lane, hence no session stamp)
CARD: AMUX-2663
SYMPTOM: TWICE in ~40 minutes, by different peers. `e679bdb` ("fix(hygiene): five carded
  defects") took an in-progress `/report` attribution change in `api/session_verbs.rs` and
  a brand-new test file that had not yet passed — it was still 404ing on a missing rig
  fixture at that moment. Then `3b24fcd` ("fix(build): main has not compiled since 22:43")
  took the whole in-progress status derivation in `api/sessions_legacy.rs`, 495-line test
  module included, mid-refinement. Both are on origin/main
  (`git rev-list --count origin/main..main` = 0) before either was noticed.
COST: Benign by luck — the swept-up code passes now. But this run was explicitly
  instructed never to commit or push, and its work was pushed anyway, twice, once with a
  red test. Also cost the confusion of `git status` no longer listing files that were
  definitely modified minutes earlier.
FIX: Not a rule ("remember to `git add` specific files" is the kind of rule that does not
  run). Two things that would close it structurally: a pre-commit check that refuses a
  commit touching files whose most recent writer was a different session — the
  `Amux-Session` trailer machinery in `scripts/git-hooks/prepare-commit-msg` already makes
  the writer knowable — or per-lane git worktrees, which the harness already supports.
  CLAUDE.md's Deploy section documents the REBASE version of this hazard; this is the
  `git add -A` version, and it needs the same warning.

## `amux-rs board list | head` panicked with 254 bytes of Rust backtrace noise
AREA: cli
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-10
SESSION: amux-rust
CARD: AMUX-2653
SYMPTOM: `amux-rs board list | head -2` exited 101 and printed "thread 'main'
  panicked at library/std/src/io/stdio.rs:1165: failed printing to stdout: Broken
  pipe (os error 32)" plus a RUST_BACKTRACE note. Piping a verb to `head` is the
  most ordinary thing a user does with a CLI.
COST: Low per occurrence, but it makes every `| head` look like amux crashed, and
  it trains you to distrust exit codes from the CLI — which is expensive later,
  because a real failure and a pipe close were byte-identical from the caller.
FIX: e3acb7d — restore SIG_DFL for SIGPIPE once in main() instead of converting
  ~30 println! sites. Process-wide fault, process-wide fix: covers every verb
  added later too. Regression test crates/amux-cli/tests/sigpipe.rs, shown to
  fail with the call removed.

## A CLI probe measured a connection failure and it read as the bug reproducing
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux-rust
CARD: AMUX-2672
SYMPTOM: While reproducing AMUX-2653, every verb returned exit 1 whether piped or
  not. That reads as "the panic is everywhere". It was not: amux-rs defaults to
  https://localhost:8823, nothing listens there (8822 and 8824 both answer
  /health), so each verb died on connect before writing a byte. The real bug only
  appeared once AMUX_RS_URL was set by hand — and then only for `board list`,
  because the other verbs are too short to fill the pipe buffer.
COST: ~20 minutes and one wrong intermediate conclusion, which was then corrected
  only because 101 vs 1 did not match the card's claim. A less specific card would
  have let the wrong reading stand.
FIX: AMUX-2672 — point the default at a port that exists. The general shape is the
  one already in ethos rule 7: a probe whose failure mode is indistinguishable from
  the fault it is hunting will corroborate whatever you already believe. A
  connection error and an application error should not both surface as exit 1 with
  no discriminator.

## A stderr capture moved stdout off the pipe, so nothing could break
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-10
SESSION: amux-rust
CARD: AMUX-2653
SYMPTOM: Comparing panic noise before/after the fix with
  `amux-rs board list 2>&1 >/dev/null | head -2` returned EMPTY for both binaries.
  The redirection order sends stderr to the pipe and stdout to /dev/null — so
  stdout was never attached to a pipe, no EPIPE was possible, and the pre-fix
  binary could not panic. Both looked identically silent, which reads as "no
  difference, fine".
COST: Would have certified the fix on a probe that could not fail, in the same
  session that ran the pre-fix binary and saw exit 101 ten minutes earlier. Caught
  only because "0 bytes of panic noise BEFORE the fix" contradicted a measurement
  already in hand.
FIX: Capture stderr to a FILE and leave stdout on the pipe
  (`cmd 2>err.txt | head`). Generally: when a probe reports no difference between
  a known-broken and a known-fixed artifact, the probe is the candidate before the
  conclusion is. This is the "loud wrong probe" from ethos rule 7 — it answered,
  and its answer was agreeable.

## `amux board progress` printed "progress noted" and wrote nothing, for weeks
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-10
SESSION: amux-rust
CARD: AC-323
SYMPTOM: Wrote an outcome note to AMUX-2653, then to AMUX-2672. Both printed
  "AMUX-xxxx — progress noted", exit 0. Re-reading the cards minutes later:
  AMUX-2653 still showed only its original 209-char text and AMUX-2672's desc was
  empty. The rust cutover dropped `desc_append`; the server correctly named it in
  `ignored_fields`, but the CLI only checked that the reply contained an `id` and
  hid the rest behind `2>/dev/null`.
COST: Two outcome records lost outright, and every progress note fleet-wide since
  the cutover — silently. Worse than the lost text: this is the verb CLAUDE.md
  tells sessions to use to record an outcome BEFORE a gate transition, precisely
  because a gate-blocked PATCH discards the desc. So the sanctioned mitigation for
  losing your outcome text WAS losing your outcome text.
FIX: d0b2150 — server implements desc_append at python parity; CLI confirms at the
  FIELD (reads ignored_fields, exits 1, names the destructive {"desc":...} retry
  it must not reach for). The general shape is ethos rule 6: the mechanism that
  would have made this visible already existed and was unread. `ignored_fields`
  is only worth having if callers are made to look at it — consider whether any
  other caller in the tree ignores it too.
