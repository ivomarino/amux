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
STATUS: open
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
