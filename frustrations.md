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
FIX: CLAUDE.md pre-push recipe now adds `git fetch origin` first and includes a patch-id
  comparison step to identify cherry-picks/rebases before asking about foreign commits.
  Validated by amux-cloud.

REFUSED 2026-08-11 by amux-cloud — only the DOCUMENTATION half shipped. CLAUDE.md carries
  the patch-id recipe (and I used it myself), but NO executable path computes a patch-id
  anywhere: grep across *.sh, *.rs and the amux CLI returns nothing. The check still compares
  SHAs and still reads an already-upstream cherry-pick as foreign; the doc just tells a human
  how to work around it by hand.
  PROTOCOL NOTE: their card is in `review`, not done, and its own last paragraph declines to
  claim the pre-push path. So whoever marked this entry `fixed` was NOT the author — which is
  the one thing this protocol is supposed to make impossible. Flipped back to open.


## A review PATCH using `desc` silently DELETED the author's entire card content
AREA: board
SEVERITY: blocks
STATUS: open
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

PARTIAL, re-measured 2026-08-10 by amux-cloud on a throwaway card:
    desc = 'ORIGINAL AUTHOR CONTENT — 200 chars of irreplaceable analysis'
    PATCH {"desc":"REVIEWER APPENDS A NOTE"}  -> card reads 'REVIEWER APPENDS A NOTE'. 200 OK.
  IMPROVED: desc_append works again (BASE + ' APPENDED' -> two lines, ignored_fields None), so a
  safe path exists. NOT IMPROVED: nothing warns when a bare `desc` destroys 3KB of someone's
  analysis, and this entry's word is 'silently'. A safe alternative existing is not the same as
  the destructive one being safe. Reopened as partial rather than deleted, at their request.


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
## The decompose nudge told me to patch three cards I had already closed
AREA: notices
SEVERITY: slows
STATUS: open
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

REFUSED 2026-08-11 by amux-cloud — THE FIX DID NOT SURVIVE THE MIGRATION, so this was
  marked fixed against code that no longer exists. The recorded fix was _steer_guard_stale:
  revalidate the asserted card state AT DELIVERY. I verified their claim independently:
  `steer_guard_stale` and `guard_stale` return ZERO files across crates/. The `guard` COLUMN
  survived, but only as a dedupe key —
    session_verbs.rs:2197 DELETE FROM steering_queue WHERE session=?1 AND (text=?2 OR guard=?3)
  — which is easy to mistake for the fix because the field name is identical.
  Their honesty is worth preserving: they do NOT claim the frustration is live either, because
  board_drive recomputes nudges from current card state each tick, so the stale window may now
  be one STEER_TICK_SECS rather than unbounded. Nobody has established that. Not deleted.


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
STATUS: open
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

REFUSED A THIRD TIME 2026-08-10 by amux-cloud — now MEASURED, not 'unproven'. During the
  python cutover the fallback lost TWO long messages to amux. They verified the loss rather
  than assuming it: peeked 1261 lines of amux's history, twice, and none of their content was
  there; they routed it through a board card instead. The retry code exists and messages are
  still being lost. Flipped fixed -> open on that evidence.


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

RELATED LOSS, found 2026-08-11 while validating AC-252: this entry's recorded fix used the
  same mechanism, and it is gone too. `steer_guard_stale` has zero hits in crates/. So the
  delivery-time revalidation that c32cf8a/7504abf added no longer exists in the rust server.
  The entry was already correctly `open`; this records WHY it cannot be closed by pointing at
  the python fix.

FRESH SPECIMEN 2026-08-18, amux-frustrations — STILL OPEN, and the same class one layer over.
  The idle guard reported: "You went idle with 2 uncommitted change(s) under your working
  directory" naming app.js and sw.js. `git status --porcelain` was EMPTY for both and for
  the whole tree — I had committed them in cd2e017. The two files differed from
  origin/main only because that commit was unpushed.
  So the notice compared against origin/main and called the result "uncommitted", which is
  a different predicate from the one the word means. Same shape as the 2026-08-08 case: a
  git assertion the reader cannot distinguish from the real thing. Here it is not staleness
  but a WRONG COMPARISON BASE — and the notice's own body warns at length about exactly
  this confusion ("a difference from origin/main is not a direction"), then makes it.
  Cost this time was bounded because the notice also prescribes the ancestry test, which I
  ran: `git log HEAD..origin/main -- <path>` printed nothing for both, so the safe action
  was commit-not-restore. Had I taken "uncommitted" at face value and run the remedy it
  names for the stale case (`git checkout origin/main -- <path>`), I would have reverted 18
  commits of dashboard work including that day's fix and a peer's feature work.
  That is the entry's own COST paragraph coming true at a larger blast radius: the sentence
  cannot distinguish itself from the real case, and its remedy is destructive.

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

## e2e auth tests flip green->red mid-session: the server under test is rebuilt from a shared checkout that moves between runs
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: no-silent-actions agent (subagent; no $AMUX_SESSION in env)
CARD: ARE-5
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
CARD: ARE-7
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
  PARTIALLY VERIFIED 2026-08-20 (amux-frustrations, NOT the author): FIVE of the six are routed. GET /api/health/invariants -> route.callers_have_routes now reports 8 failures and every one of them is /api/tunnel/* (start, status, stop). The tunnel family is tracked separately on AF-64, which sits in needsyou awaiting Ethan's revive-or-remove decision. STATUS stays open ONLY because of that one family; do not delete this entry until AF-64 resolves.
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
## Uncommitted migrations reach the LIVE database within minutes, from another agent's server
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-09
SESSION: rust-rebuild (RR-0109/0110 lane)
CARD: ARE-10
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

## Five finished cards sat in `todo` and kept being auto-picked
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux-rust
CARD: AMUX-2674
SYMPTOM: Auto-pickup handed me AMUX-2672 with "32 more queued". Five of those 32
  (AMUX-2599, 2609, 2618, 2634, 2636) were all fixed by ONE commit — e679bdb, whose
  subject literally reads "five carded defects — watchdog, the 404 trio, OSC-8,
  pane shrink, custom columns" and whose body names each card id. Their descs
  already said "DONE" and named a single remaining step (`git add`), which a later
  commit had done. Nothing moved the cards.
COST: The queue overstated real work by ~16% and auto-pickup kept offering finished
  cards, each costing a full scope-and-decide cycle to rediscover. Worse for
  anyone reading the board to see what is left: five defects looked open that were
  live in production.
FIX: The commit body already names the card ids in a machine-readable form. Nothing
  reads them. A commit trailer or body scan that flags "card named in a merged
  commit but still in todo" would have surfaced all five in one query — the data
  was there and unread, which is the same shape as AC-323's ignored_fields. Note
  the honest limit: a named card is not proof of completion, so this should
  SURFACE candidates for a human/agent check, never auto-close (ethos rule 8).

---
## A peer's `git add` swept my uncommitted migration into their commit and it applied to the live DB
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux-rust (AMUX-2647 lane)
CARD: AMUX-2647
SYMPTOM: I wrote `migrations/0015_schedule_run_delivery.sql` and registered it in
  `migrate.rs`, uncommitted, under an explicit instruction never to commit. Commit
  4d76ff3 ("feat: universal FTS5 search …") picked up my `migrate.rs` edit; the .sql
  file was still untracked, so a clean checkout could not compile (`include_str!`
  resolves at build time), and 6689a74 then tracked my file to repair the dangling
  reference. The auto-builder shipped it and the live server applied 0015 to
  `~/.amux/amux.db` at 03:22:43 — schema I authored, live, hours before the code that
  writes those columns exists anywhere but my working tree.
COST: no damage — the columns are additive and NULL reads as "not recorded" — but the
  live DB now has two columns nothing populates, and neither author chose that. The
  deploy path is committed-HEAD-only *precisely* so half-finished work cannot ship;
  a broad `git add` in a shared checkout defeats it, and the second author was doing
  the right thing (repairing a dangling reference) with no way to know the file was
  mid-flight. The existing rule covers the direction "check what you are pushing that
  is not yours"; this is the mirror, and no check catches it.
FIX: the pre-commit guard should refuse a `git add` that stages files no lane has
  claimed — or, cheaper, `prepare-commit-msg` already stamps `Amux-Session`, so warn
  when a commit's file set spans more than one lane's recent edits. Until then: write
  new files outside the repo until the change is ready, which is what I should have
  done here.

---
## Booting a second amux-server to test something drives the PRODUCTION tmux fleet
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-10
SESSION: autofix (subagent)
CARD: AF-69 (investigation, signed off) + AMUX-3221 (the FIX, open)
SYMPTOM: Started an isolated server (`AMUX_HOME=/tmp/amux-af-home`, port 8899, own DB) to
  verify a change without touching the fleet. Within 4 seconds its log showed:
    pane-size: restoring detached window ... session=amux-amux from=220x50 to=220x50
    pane-size: restoring detached window ... session=amux-mixpeek-autopilot ...
    pane-size: one-shot repair complete count=3 sessions=["amux-amux", ...]
  `pane_size::spawn()` takes no state and enumerates tmux DIRECTLY, so AMUX_HOME does not
  scope it. `ghost_rescue` is the same shape and it SUBMITS STUCK MESSAGES — i.e. a test
  instance can press Enter in a production lane's pane. Neither has an off switch;
  `commit_nudge` and `board_drive` both do (`AMUX_*_SECS=0`).
COST: Killed the instance and rebuilt the whole live verification as in-process router
  tests instead. This time the resize was a no-op (220x50 -> 220x50) so nothing was lost,
  but that is luck: a peer is running `/tmp/amux-sched-target/debug/amux-server` on this
  same box right now, and the repo's own docs tell you to build to a private target dir
  and run it.
FIX: STILL OPEN — the hazard is live. AF-69 (the INVESTIGATION) was signed off by amux
  2026-08-16; the FIX is AMUX-3221 and has not been started. Signing off an investigation
  is not the same as fixing the thing, and this entry stays until AMUX-3221 lands.
  CONFIRMED STILL BROKEN 2026-08-16: pane_size and ghost_rescue have NO env knob;
  commit_nudge (AMUX_COMMIT_NUDGE_SECS) and board_drive (AMUX_BOARD_DRIVE_SECS) do. No
  global isolation guard exists (grepped AMUX_NO_FLEET / AMUX_ISOLATED / is_isolated /
  AMUX_TMUX_READONLY — none).
  THE ENTRY'S OWN PROPOSED FIX IS INCOMPLETE, measured not assumed: adding the knob at the
  top of `pane_size::spawn` covers only its one-shot `sweep(true)`; the SAME function then
  calls `super::spawn_periodic("pane_size", TICK_SECS, ..)`, which keeps sweeping the fleet.
  A per-job knob there looks done and is not. That half-fix is stashed, not committed
  ("AF-69: incomplete pane_size guard").
  CORRECT SEAM (amux verified it): `runtime_jobs/mod.rs:128 spawn_periodic_every` is the
  ONLY constructor of a PeriodicTask — its own comment already leans on that to guarantee
  every job appears in the registry — so a knob there, derived from the job name
  (pane_size -> AMUX_PANE_SIZE_SECS, ghost-rescue -> AMUX_GHOST_RESCUE_SECS), gives every
  periodic job a disable for free, including ones written later. Requires a test proving a
  0 knob stops the sweep while a normal value still ticks, and that a disabled job stays
  REGISTERED (inert, not invisible) so it does not become a silent skip.

## The per-agent CARGO_TARGET_DIR convention has no GC — 37 caches filled the disk
AREA: environment
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-10
SESSION: amux (subagent — legacy-port migration)
CARD: AMUX-2754
SYMPTOM: `cargo build -p amux-server` died with `failed to write ...: No space left on
  device (os error 28)`. The root volume had **609Mi free of 1.8Ti**. `du` on /tmp found
  **445GB across 37 `/tmp/amux-*-target` directories** — one per agent task, 15GB at the
  top end, 33 of them last written the previous day. Every task brief in this repo hands
  the agent its own `CARGO_TARGET_DIR=/tmp/amux-<task>-target`, and nothing ever removes
  one. The convention that keeps concurrent agents from contending is also, unmodified, a
  disk-fill schedule: ~12GB per rust task, times however many tasks the fleet runs.
COST: My gate (cargo test + clippy) was unrunnable. Worse than my task: the amux SQLite DB
  and `~/.amux/logs` are on this volume, so the whole fleet was one write from failure with
  no warning anywhere — `/health` reports `store:"ok"` and says nothing about the disk
  underneath it. I could not clean up safely either: the dir names are TASK-scoped, not
  session-scoped, so "does a live session own this cache?" is unanswerable — my own dir
  (`amux-port-target`) looks orphaned by every test I could write. Escalated to the owner
  because deciding which 445GB of other agents' caches to destroy is not an agent's call
  (ethos rule 8).
FIX: Two halves, neither done. (1) `/health` should report free space on the volume holding
  `~/.amux`, so disk pressure is visible where every session already looks instead of
  arriving as a build error in whoever happens to compile next. (2) The convention needs a
  reaper: either name the dir after `$AMUX_SESSION` (so ownership is decidable and a
  session reuses one cache across tasks instead of minting one per task), or a scheduler
  entry that removes `/tmp/amux-*-target` untouched for >24h. Naming it after the session
  is the better half — it makes the cleanup question answerable at all, which is the part
  that blocked me.

---
  VERIFIED 2026-08-20 (amux-frustrations, NOT the author): CLAUDE.md now mandates ONE shared build dir (CARGO_TARGET_DIR=~/.amux/rust-build-target) and records the 15GB-each measurement that killed the per-session convention. /private/tmp/*target* is down to 1 stale directory from the 37 this entry reports.
## The schedule audit trail is routed, implemented, and reachable from no control
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-10
SESSION: amux (sched2 lane)
CARD: AMUX-2755
SYMPTOM: `GET /api/schedules/audit` works and is good — it is the only way to answer
  "who disabled this schedule / why did it not run at 9". Zero of the twelve
  `/api/schedules` call sites in `app.js` hit it. Its own discoverability mechanism is
  a response HEADER (`x-amux-audit`), which a dashboard user never sees.
COST: none yet this session; logged because AMUX-2416 already established that an
  audit nobody can find is the same failure as no audit, and this is that shape again
  one endpoint over.
FIX: an "audit" affordance on the schedule card's expanded view, reusing the existing
  endpoint. Small; carded rather than folded into an unrelated change.

---
## A peer's `git add` swept my UNCOMMITTED work into their commit and pushed half of it
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux (sched2 lane)
CARD: AMUX-2757
SYMPTOM: mid-task, `git diff` stopped showing my `scheduler.rs` and `app.js` edits.
  They were not lost — commit `9a91945` (another lane's autofix work) had staged them
  with a broad `git add` and pushed. So `pub fn skip_next_run` and the dashboard's
  rewritten Skip button are on `origin/main` right now, while the `api/schedules.rs`
  half that MOUNTS the route is still uncommitted: a function upstream with zero call
  sites, which is the exact ethos-rule-1 shape the work was fixing.
COST: no work lost, but ~10 minutes establishing what was where, and a genuinely
  misleading upstream state — the dead controls are gone from the UI while the API
  still silently accepts the dead fields, so the defect is now invisible from the
  dashboard rather than fixed. Anyone reading origin/main would call it done.
FIX: CLAUDE.md's Deploy section documents this hazard in the OTHER direction (your
  unpushed commit riding out on a peer's push). The mirror case deserves the same
  billing: on a shared checkout `git add -A` / `git commit -a` stages other lanes'
  live edits, and a lane cannot tell from its own session that it happened. Stage by
  explicit path, and check `git status` for files you did not touch before committing.

## The shared-checkout sweep shipped a BROKEN BUILD, because the swept work included a new untracked file
AREA: git
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux (subagent — legacy-port migration)
CARD: AMUX-2769
SYMPTOM: Not a new frustration — see the four existing entries on `git add` sweeping a peer's
  uncommitted hunk (AMUX-2443, now `done`, plus lines 117 / 292 / 548). This is a DISTINCT
  consequence worth its own row. My in-flight edits to `lib.rs`, `config.rs`,
  `session_verbs.rs`, `api/mod.rs` and `api/request_log.rs` were swept into 9a91945 and
  4ac14b9 by another lane — but the module those edits CALL, `legacy_port.rs`, was a NEW
  file and therefore untracked, so `git add <tracked paths>` could not pick it up. main was
  left referencing a module that did not exist: unbuildable for 3 minutes until c3f5e0f
  ("commit legacy_port + canonical_port, whose callers were already on main") patched it.
  Both commits are already pushed to origin.
COST: Small here (a peer noticed and fixed it in 3 minutes, and their commit message shows
  they diagnosed it correctly). The reason to log it is that it inverts the usual
  mitigation: the standing advice for the sweep is "stage narrowly, by path", and staging
  narrowly is EXACTLY what produced an unbuildable main. A sweep that takes everything
  would at least have been self-consistent. It also means my work reached origin without
  me, while I was under an explicit instruction not to commit or push — so "I did not
  push" is not the same as "my work did not ship".
FIX: The existing guard checks for a peer's modified files in the index. It should also
  refuse when the staged set REFERENCES an untracked file in the same crate (a `mod X;` or
  `crate::X` naming a path that is untracked) — cheap to detect and it is the difference
  between shipping a peer's diff and shipping a build break. Cheaper interim: make the
  auto-build service's failure page name the untracked file, since "cannot find module
  legacy_port" is currently only visible to whoever next compiles.

## Deleting 450GB freed 8GB, because hourly Time Machine snapshots pin every deleted block
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-10
SESSION: storage-audit
CARD: AMUX-2701
SYMPTOM: With the volume at 741MB free, ~450GB of stale cargo target dirs was deleted and
  `df` moved to 9.0GB free — about 8GB recovered from 450GB deleted. Deleting a further
  26.8GB moved free space DOWN (8.1Gi -> 6.6Gi). The cause was 24 hourly APFS local Time
  Machine snapshots spanning 2026-08-09 13:18 to 2026-08-10 12:18: a snapshot pins the
  blocks of every file deleted after it was taken, so deletion frees nothing until the
  snapshots age out (24h) or are thinned. They had accumulated because the Time Machine
  destination ("My Book") is not connected, so nothing ever thinned them. macOS eventually
  purged all 24 on its own under pressure and free space jumped to 418Gi.
COST: A wrong conclusion that was already corroborated: two sessions independently read
  "deleted a lot, freed nothing" as "we deleted the wrong things", whose remedy is deleting
  MORE — the one action that could not work. It also produced an owner alert asking for a
  root password (`sudo tmutil thinlocalsnapshots`) that turned out not to be needed, which
  is a fire alarm spent on a self-resolving condition.
FIX: Partly fixed: the new autofix `disk` detector puts `tmutil listlocalsnapshots / | wc -l`
  in the card's evidence with an explicit "READ THIS BEFORE DELETING ANYTHING" note, so the
  next session sees the discriminator in the place it is already looking rather than having
  to know APFS semantics. Still open: nothing warns that the TM destination has been absent
  for long enough to accumulate a full day of local snapshots, which is the actual upstream
  condition and is invisible until it interacts with a disk-full event.

## `cargo test` cannot pass under the target-dir convention the repo itself mandates
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux
CARD: AMUX-2799
SYMPTOM: `db::migrate::guard_tests::it_actually_refuses_a_pending_migration_against_the_live_db`
  fails with "precondition: the test binary should live under a cargo target dir, got
  /Users/ethan/.amux/rust-build-target/debug/deps/amux_server-...". The predicate is
  `s.contains("/target/debug/") || s.contains("/target/release/")` — but per-agent target
  dirs are BANNED (they filled the disk on 2026-08-09) and both the task instructions and
  scripts/rust-auto-build.sh use `CARGO_TARGET_DIR=$HOME/.amux/rust-build-target`, whose
  path contains `/rust-build-target/debug/`, not `/target/debug/`. So the sanctioned way to
  run the suite is the one way this test cannot pass. Verified at the unmodified base commit
  86d3353, so it is not caused by any working-tree change.
COST: A red suite that every lane must learn to ignore, which is the state in which a REAL
  regression gets waved through. It also cost a false green in the other direction: the
  background run was invoked as `cargo test > log; echo $?; tail -25`, so the reported exit
  code was `tail`'s (0) while the log said FAILED — a full suite was nearly reported green.
FIX: Not applied — migrate.rs has another lane's uncommitted work in it and this is not my
  file to tangle with. One line: accept a path under the configured target dir, e.g. also
  match `std::env::var("CARGO_TARGET_DIR")` as a prefix, or match `/debug/deps/` generally.
  The second half generalises past this bug: **when you redirect a command's output to a
  log, `$?` is the exit code of the LAST command in the pipeline, not the one you care
  about.** Capture it immediately after the command, or the "did it pass?" check reports on
  `tail`.

## Another lane's `git add` swept my uncommitted AC-322 fix into their commit, under their message
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux (subagent, AC-322)
CARD: AC-322
SYMPTOM: Fourth instance of the cluster already recorded above (the migration sweep, the
  "swept my UNCOMMITTED work and pushed half of it" entry, and the "consumes PEERS' staged
  files silently" entry). Logging it only because the COUNT is the argument: this is now
  four independent sessions hitting the identical shape, which is what turns it from
  bad luck into a design fact about a shared checkout.
  My board.rs `actor_from_headers` fix and both AC-322 regression tests
  (`force_accepts_x_amux_worker_attribution_like_every_other_module`,
  `cross_lane_archive_guard_sees_x_amux_worker_callers`) were uncommitted in the working
  tree. They are now in f36d407, "fix(build+ui): title_needs_self_description, clear-done
  honesty, and the legacy-port shell injection", which mentions neither AC-322 nor the
  attribution header. I never ran `git commit`; I was explicitly instructed not to.
COST: No work lost this time, but the audit trail is now wrong in a way nobody can see from
  the log: a security-adjacent change (the cross-lane ARCHIVE guard had been open to every
  bash-CLI caller) is recorded under a commit message about a build break and a UI fix.
  Anyone bisecting for when the archive guard started working will not find it by message.
  The reviewer-of-record for that hunk is also wrong.
FIX: Same as the three entries above — this is not fixable by being careful, because the
  sweeping session cannot see whose hunks it is staging. `git add <specific paths>` is the
  mitigation everyone already knows and it failed four times; the durable fix is a
  pre-commit guard that refuses to stage hunks in files another session has open, or
  per-session worktrees so `git add -A` is scoped by construction. Until then, treat
  "my change is uncommitted" as "my change may ship under someone else's message at a
  time I do not choose" (CLAUDE.md already says this; the entry is the evidence).

## The shared cargo target dir served a stale rlib, so `cargo test` blamed three innocent files
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: claude (AMUX-2619/2780 lane)
CARD: AMUX-2799
SYMPTOM: With the now-mandated `CARGO_TARGET_DIR=~/.amux/rust-build-target` (e188b0e, "ONE
  shared cargo target"), `cargo test -p amux-server` reported, in sequence, three DIFFERENT
  compile errors in files I had never touched: `unresolved import
  amux_server::runtime_jobs::registry`, `cannot find function title_needs_self_description
  in module amux_core::board`, and a `migrate.rs` precondition panic naming the shared
  target path. All three sources were byte-correct — I verified `pub mod registry;` with
  `od -c`. The actual cause: the cached `libamux_server-*.rlib` was built from an older
  tree. `strings` on it showed 6108 hits for `runtime_jobs..autofix` and ZERO for
  `registry` and `storage`, the two newest modules, while the same rlib's own crate
  compiled fine and lib.rs line 210 uses `runtime_jobs::registry`. Cargo's mtime
  fingerprint never noticed, because mod.rs (13:24) was older than the rlib (14:27).
COST: ~40 minutes, and three wrong conclusions I came close to reporting — twice I
  concluded "another lane's uncommitted work has broken main" and started to write it up,
  and once I concluded a committed test was broken under the mandated target dir. Every one
  of those would have sent a peer to debug correct code. `cargo clean -p amux-server`
  removed 48,516 files / 28.9GiB and fixed it for one invocation before it recurred;
  `touch crates/amux-server/src/runtime_jobs/mod.rs` is what actually forced the rebuild.
FIX: The failure mode is specific and cheap to detect: an rlib that does not export a
  module its own crate source declares. A preflight in the test gate — compare `pub mod`
  lines in each `mod.rs` against the built rlib, or simply `cargo build -p amux-server --lib`
  and fail loudly if it is a no-op while sources are newer — would turn 40 minutes of
  blaming peers into one line of output. Until then the recipe is: when `cargo test` names
  a symbol you can see in the source with your own eyes, suspect the ARTIFACT before the
  code, and `touch` the `mod.rs` that declares it. Related to the shared-checkout cluster
  above: same root (one resource, many lanes), different resource (build artifacts, not
  the git index).

## `cargo check --workspace` in the pre-commit hook cannot tell MY broken change from a PEER's
AREA: gates
SEVERITY: blocks
STATUS: open
DATE: 2026-08-10
SESSION: amux
CARD: AMUX-2777
SYMPTOM: The shared checkout broke the workspace FOUR times in ~40 minutes from at least three
  lanes: a `steer_enqueue` arity change mid-refactor (mine), `DetectorKind::CiFailure` non-exhaustive
  match, `note_quiet_signatures` arity, and `amux_core::board::title_needs_self_description` missing
  for orchestrator/runtime.rs:1288. Every one of them blocked EVERY lane's commits, because the hook
  checks the WORKING TREE — which on a shared checkout contains everyone's in-flight edits, not the
  change being committed.
COST: amux-cloud's AC-335 bounced twice on other lanes' compile errors. I lost ~25 minutes to two
  breaks that were not mine, and inflicted one on them. The gate's verdict carries no information
  about the commit it is gating.
FIX: check the STAGED state, not the working tree — `git write-tree` + `git archive` into a temp dir
  is read-only w.r.t. the shared checkout, so it is safe to do under other lanes' edits. Cost is a
  colder build per commit, which is the trade to price. Anything short of this keeps conflating
  "your change is broken" with "someone else is mid-sentence".

## `cargo test` was green while `cargo check` was green — and the compiled binary lacked my tests
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux
CARD: AMUX-2777
SYMPTOM: `cargo test -p amux-server --lib the_three_stalled_lanes` printed
  `test result: ok. 0 passed; 0 failed; 752 filtered out` — twice, after a 31s build, with the same
  binary hash. The tests were on disk (grep confirmed), in a plain `#[cfg(test)]` module whose OTHER
  five tests were listed by `--list`. The full run minutes earlier reported 781 passed / 787 total;
  `--list` then reported 751. The artifact was stale under heavy shared-CARGO_TARGET_DIR contention.
COST: ~15 minutes, and it is the LOUD-WRONG probe shape: it exits 0 and says `ok`. A filter that
  matches nothing is indistinguishable from a suite that passes, so the natural next move is to
  believe the code is fine. Had I been verifying someone else's fix I would have reported it working.
FIX: `0 passed AND 0 filtered-in` should never render as `ok` — but that is upstream. Locally: when
  a name filter matches zero tests, treat it as a FAILED probe and re-run against `--list` before
  concluding anything. Same family as the empty-grep rule in ethos.md rule 7.

## A peer's `commit -a` swept my uncommitted work into their commit — twice, in both directions
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux
CARD: AMUX-2807
SYMPTOM: My entire AMUX-2785 steering fix landed inside 70dc3a8 "fix(build): commit registry + the
  visibility changes its callers on main already reference" — a commit I did not write. I found out
  only because `git status --short` came back EMPTY on four files I had edited minutes earlier, which
  reads as "my work is gone" before it reads as "someone committed it". Symmetrically, amux-cloud's
  AC-335 board_store fix went out inside my e188b0e storage commit. The staged-guard was DOWN for
  both ("NOT ENFORCED — could not reach the amux server", 8822 then 8824, timing out).
COST: No code lost either time, but each of us had to verify piece-by-piece that our own work had
  landed intact rather than partially. Commit messages now describe work they do not contain, so the
  archaeology is wrong for anyone who reads git log later — including the `Amux-Session` trailer,
  which attributes my change to a commit stamped for a different piece of work.
FIX: two halves. (1) The staged-guard must not fail open silently — AMUX-2807; if it cannot reach the
  server, the unguarded commit should at least be COUNTED durably, or the guard is decoration exactly
  when it matters. (2) `commit -a` on a shared checkout is the hazard itself; the guard should refuse
  it, not merely warn, when the staged set spans files the committing session never touched.

## Browser API drove the user's live Chrome and said ok:true for every keystroke
AREA: browser
SEVERITY: blocks
STATUS: open
DATE: 2026-08-10
SESSION: amux-cloud
CARD: AC-336
SYMPTOM: `POST /api/browser/start {"profile":"default","url":"https://cloud.amux.io/sign-in"}` returned
  {ok:true, pid:90649, cdp_port:65059}. That pid was already dead: 10 Chrome processes share
  user-data-dir=/Users/ethan/.amux/playwright-auth/profile and hold SingletonLock, so Chrome's
  singleton handoff exits the new process and reuses the running one — the user's own browser.
  `GET /api/browser/status` then reported running:true on a DIFFERENT port (65140) listing the user's
  tabs, and `POST /api/browser/action` eval returned location.href =
  http://localhost:4177/solutions/creative-dna. `type` returned ok:true at every step. The endpoint's
  own hint says "They never attach to a browser this server did not launch", which is precisely what
  it did.
COST: I typed AMUX_GODMODE_PASSWORD and pressed Enter into the user's live Chrome believing I was
  driving an amux-owned browser. The frontmost page had no text inputs so it almost certainly went
  nowhere, but a god-mode credential now needs rotating on "almost certainly". Roughly 40 minutes lost,
  and the god-mode UI verification (AC-332) is still not done because the subsystem cannot be trusted
  to target the browser it says it launched.
FIX: start must confirm the pid it returns is alive AND that its own cdp_port answers, failing loudly
  when the singleton hands off — returning a dead pid as ok:true is the primary defect. status/action
  must bind to the port start launched and refuse any other. Default to a per-session profile dir so
  two sessions cannot contend for one lock. Control proving the diagnosis: an isolated profile
  (its own user_data_dir) survives, its CDP answers, and eval sees the URL actually requested.

## A guard on status.running cannot catch this, because status is the thing that lies
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux-cloud
CARD: AC-336
SYMPTOM: After discovering the fallback above I added a liveness assertion that refused to act unless
  `GET /api/browser/status` reported running:true. It passed, and the very next eval still executed
  against the user's Chrome at localhost:4177. The guard could not fail: it consulted the same
  component that had already substituted a different browser.
COST: One wasted round of "now it is safe" — I ran a second credential sequence behind a guard that
  was structurally incapable of detecting the failure it was written for. What actually caught it was
  a cheap independent probe: eval `location.href` and compare against the URL I had asked start for.
FIX: Verify from a source that is not the suspect component. For this API the discriminating check is
  two lines — ask the launched cdp_port for /json/list, ask /action what location.href is, and require
  they agree. Generally: a guard that reads the lying instrument inherits the lie (ethos rule 7).

## Shared checkout swept my board_store fix and its test into another lane's commit
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-10
SESSION: amux-cloud
CARD: AMUX-2807
SYMPTOM: AC-335 (depends_on_cycle scoped to cycles containing self_id) plus its falsifiable test
  landed inside e188b0e, amux's "feat(storage): retention on seven unbounded tables..." commit, which
  is unrelated to the board. `git status` came back clean on a file I had just edited. Found only by
  `git log -S "pre-existing depends_on cycle elsewhere"`. amux reports the mirror the same day: their
  AMUX-2785 steering fix went into 70dc3a8, a commit they did not write.
COST: Not lost work — fix and test both intact — but the commit carries a change its author cannot
  explain, and they will be the one asked about it. Two sweeps in opposite directions in one day.
  My AC-335 also bounced twice on other lanes' compile errors before landing, since the pre-commit
  hook runs cargo check --workspace against a tree four lanes are editing.
FIX: The staged-guard is the designed prevention and it was DOWN for both events — it printed
  "NOT ENFORCED — could not reach the amux server" against 8822, then 8824, timing out both times,
  so cross-session sweep protection was off exactly when four lanes were committing into one tree.
  Being loud about it is right; being down is the hazard it exists for. See AMUX-2807.

## A probe read a hook file that git never executes, and a correct measurement certified the wrong conclusion
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-11
SESSION: amux
CARD: AMUX-2841
SYMPTOM: Retracting a peer's report of a tree-wide mtime restamp, I grepped
  .git/hooks/pre-commit on amux and mixpeek for `git stash`, found none, and wrote
  "the mechanism does not exist" onto MI-4650. Three independent reasons it could not
  work: the stash is done by the pre-commit FRAMEWORK wrapping the hooks; it is
  spelled diff-index + `checkout -- .` + apply, never `git stash`; and mixpeek sets
  core.hooksPath=.githooks, so the file I opened is DEAD — git never runs it.
COST: A wrong retraction published onto another session's card, contradicting a
  correct report from creative-dna. Two peers spent turns re-establishing a fact that
  was already established.
FIX: The generalisable half is the CORROBORATION, not the bad grep. I confirmed the
  retraction by watching a file's mtime across a real commit and seeing it unchanged —
  true, and worthless, because I ran it in the amux tree, which has no
  .pre-commit-config.yaml and never invokes the framework. A correct measurement in
  the wrong scope arrives as EVIDENCE rather than as reasoning, and evidence is harder
  to doubt because you can point at it. Nothing felt like the moment to recheck.
  Wanted: before believing a negative about a mechanism, confirm the probe ran where
  the mechanism could fire — for hooks specifically, resolve core.hooksPath first,
  because the file at the obvious path may not be the one that runs.

## A peer's commit swept my STAGED work into their commit, under their message
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-11
SESSION: amux
CARD: AMUX-2899
SYMPTOM: I ran `git add app.js sw.js && git commit -F -`. The commit returned
  "nothing to commit, working tree clean" — while the change was demonstrably
  live (APP_VER 0.9.591 serving). A peer had committed in the gap between my add
  and my commit, and their `381fb3c` ("feat(board-drive): auto-continue nudge for
  lanes with outstanding work") contains my 32-line scheduler-UI fix and my sw.js
  CACHE bump alongside their 169 lines of board_drive.rs.
COST: no code lost — I verified all five markers of my change are intact in HEAD
  and serving. The cost is the RECORD. This repo does constant archaeology; every
  fix cites a sha and CLAUDE.md's own recipes tell you to read `git log --grep`
  and `<sha>^` to find the pre-fix specimen. Anyone tracing why shell schedules
  render an owner chip now lands on a commit about an auto-continue nudge, by a
  different author, whose message says nothing about it. The AMUX-2899 card would
  be the only thread back.
FIX: none yet. CLAUDE.md documents the mirror of this ("a peer's commit can
  silently REVERT your uncommitted work", 2026-08-09, where staged DELETIONS were
  swept in) but not this direction, and the existing staged-guard fires on the
  COMMITTER — it warned me about their staged board_drive.rs — while the party
  who needs warning is the one whose work is about to be carried off.
  What would actually help: `git commit` with explicit paths is already the
  advice, and I did not follow it here (I used a bare `git commit -F -` after a
  targeted `git add`). A bare commit takes the whole index, and on a shared
  checkout the index is shared. The narrow rule is: on this checkout, ALWAYS
  `git commit -- <your paths>`, never rely on having staged only your own.
  I have used the path-scoped form elsewhere today; I did not here, and that is
  the difference.

## A dev server on the default AMUX_HOME silently clobbers the shared endpoint.json
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-12
SESSION: amux
CARD: AMUX-2971
SYMPTOM: I ran a throwaway amux-server on an alt PORT (18931) but the DEFAULT home (~/.amux) to read real message rows for a UI verification. On startup it published ~/.amux/endpoint.json pointing canonical_port at 18931. When I killed it, endpoint.json still named the dead port — so the pre-commit staged-guard (which resolves the server via endpoint.json, not AMUX_URL) could not reach a server and printed "staged-guard NOT ENFORCED" for the next commit. This affects EVERY session on this machine, not just mine: they all share ~/.amux/endpoint.json.
COST: One commit shipped with cross-session sweep protection OFF (recorded in staged-guard-unenforced.jsonl, so at least it was auditable). Restored by launchctl kickstart of the real server to republish. Any session that committed in the window between my dev server starting and the kick would have hit the same.
FIX: Two candidates, either or both: (1) publish_endpoint should NOT write the shared endpoint.json when the port is not the configured canonical AMUX_RS_PORT — a dev/alt-port instance is not the fleet's server and should not claim to be; gate the write on port==canonical. (2) the staged-guard's server resolution should prefer a liveness check on the canonical port and fall back rather than trusting a possibly-stale endpoint.json. The durable fix is (1): a non-canonical instance clobbering the canonical control file is the root. Until then: always give a dev server its own mktemp AMUX_HOME (my earlier 1892x runs did; this one did not, to get the live DB — that shortcut is the bug).

## legacy-port instrument reports CLEAR while 52 live sessions are stranded on the dead 8822
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-12
SESSION: amux
CARD: AMUX-2988
SYMPTOM: Ethan intentionally dropped the 8822 compat bind 2026-08-11 (lib.rs:527, "no more 8822 just rust"). But 52 of 56 running claude procs still carry AMUX_URL=https://localhost:8822 in their process env, which cannot be rotated on a live process. Every documented `curl $AMUX_URL/api/...` recipe (peek, notes, email, schedules, calendar) returns 000 for those 52 lanes. GET /api/debug/legacy-port reports verdict "CLEAR: no traffic on the retired port", ready_to_retire=true, sessions_still_on_legacy=[] — the exact opposite of the truth — because it counts HITS and a port nothing listens on can record none. The one instrument meant to answer "who is still on 8822" is structurally blind to everyone who is.
COST: I burned several tool calls diagnosing why my own `curl $AMUX_URL` returned 000 and initially misread a deliberate owner decision as a fleet-down regression. Any of the 52 lanes following the CLAUDE.md/memory curl recipes silently fails the same way, and nothing surfaces that 52 lanes are running degraded — so no one recycles them. The `amux` CLI masks it (it uses AMUX_API=8824), which is why this went unnoticed.
FIX: (proposed, AMUX-2988) legacy-port accounting must not measure strandedness by inbound hits after the bind is gone — derive it by scanning running session process envs for a RETIRED_PORTS match (the /api/debug/tmux pattern: discovery from inside the server process), surface the count on /api/debug/legacy-port and an hourly WARN. Recycling the 52 is the owner's call (ethos rule 8, could interrupt in-flight customer work) — the fix only makes the count visible, it does not restart anything.

## Cloud silently froze behind a red main CI — "skipped" reads as "up to date," not "frozen"
AREA: cloud
SEVERITY: slows
STATUS: open
DATE: 2026-08-13
SESSION: amux-cloud
CARD: AC-344
SYMPTOM: Ethan reported "cloud is still behind in versions." A fresh cloud org still booted build 0f2f6e48 (pre-env_config: GET /api/env/schema -> 404, /api/env/apply absent from 213 routes), so the converged seed.py --via-apply 405'd against cloud. Root cause was three layers down: deploy-cloud.yml auto-deploy is gated on GREEN rust.yml (workflow_run), and main CI had been RED for hours on ONE clippy lint (unnecessary_sort_by, messages.rs:585). Every deploy-cloud run showed "skipped" — indistinguishable from "nothing to deploy." Nothing anywhere said "the cloud image is frozen and falling behind main because CI is red."
COST: Ethan had to notice the version lag by hand. Diagnosing it took several manual steps (fresh provision -> /health build hash -> /api/debug/routes -> gh run list conclusion -> git log timing) to join signals that no single instrument joins. And it is fleet-recurring: ANY lane's red-main break freezes the entire cloud deploy for every customer, invisibly, until a human notices — the busier the fleet, the more often it happens. PREDICTION PROVEN 2026-08-14 (author-verified during a frustrations validation): the "until a human notices" line came true VERBATIM, three times in ONE session, all AFTER this entry was written — 67b44f7 (clippy unnecessary_sort_by), 64fd450 (steering restart_persistence test), 9442f77 (opencode ETXTBSY flake + /api/tts unclaimed in the boundary registry). Each red-mained main, each made deploy-cloud SKIP silently, each froze :latest, and each was caught BY HAND via the freshness tick — never by any instrument. A prediction that recurred 3x on the record is the strongest possible argument for finally building the signal.
FIX: AC-344 — a signal that joins live-cloud-build-hash vs latest-green-main and fires when they diverge (commits or hours), OR make deploy-cloud's skip loud (record "skipped because CI red since <sha>/<time>"). Interim: clippy blocker fixed (67b44f7); steering-test blocker handed to amux; cloud auto-catches-up once CI green. Related: AMUX-3013 (pinned toolchain so local clippy == CI clippy — why the red wasn't caught pre-push).

---

## A page.route stub defeated by a service worker fails LOUDLY and blames the wrong subsystem
DATE: 2026-08-13
AREA: instruments
SEVERITY: slows
STATUS: open
SESSION: amux-frustrations
CARD: AF-47
SYMPTOM: Isolation gave each project a CLEAN browser profile, which surfaced two failures the
  shared one had masked — and both lied about where the fault was. (1) system-jobs.spec.ts
  stubs /api/system-jobs with page.route; a registered service worker defeats that, because
  the request passes through the worker's fetch handler where page.route cannot see it. It
  did not error — it rendered the REAL job list and diffed it against the stub, so it read as
  "the stalled-row styling is broken under WebKit". (2) sw.js reloads the page on
  `controllerchange` as soon as a fresh worker claims the client, landing mid-page.evaluate:
  "Execution context was destroyed" on two specs about CSS geometry.
COST: Both point at the wrong subsystem by construction. (1) is the dangerous one: a stub
  that silently does not apply produces a confident, specific, wrong failure about rendering,
  and the natural response is to go read the CSS. Roughly an hour across the two before the
  common cause was visible.
FIX: `test.use({ serviceWorkers: 'block' })` on the specs that do not test the worker, in
  b31bcac. STILL OPEN as a class: nothing warns that a page.route stub never matched a
  request. A stub that matches zero requests is almost always a bug and is currently
  indistinguishable from one that matched — same green-looking machinery, no output either
  way. The generalisable guard is an assertion that each route was actually hit; amux has no
  such helper today and every future page.route stub inherits the same silence.

---

## Verified gate rejects a cross-group reporter's verification, so the strongest evidence cannot close the card
AREA: gates
SEVERITY: slows
STATUS: open
DATE: 2026-08-14
SESSION: amux
CARD: AMUX-3119
SYMPTOM: AMUX-3116 and AMUX-3117 (amux CLI fixes) were verified end-to-end by gtm-engine
  with negative controls, field-level CC_* diffs and a server-API cross-check, which is
  stronger than a typical same-group review. But the code verified-gate criterion is
  "peer-reviewed by a worker in group `amux`", and gtm-engine is group `gtm`. Acking it
  would be untrue, so both stay `done`.
COST: Two genuinely-verified cards cannot reach `verified`; the strongest verification
  available (the affected user, who also reported the bug) does not count toward the gate.
FIX: The verified gate should accept verification by the originating reporter, or by any
  worker when the card records who plus their evidence (AMUX-3119).

## litestream DR replication died fleet-wide and nothing in amux could express it; it was found by grepping container logs on the box
AREA: cloud
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux-cloud (hit) / amux (diagnosed)
CARD: AC-349
SYMPTOM: All 5 real-org litestream sidecars failing with "attempt to write a readonly
  database (8)" on _litestream_seq, consecutive_errors 300+, after a disk-full container
  recreate pulled a non-root litestream:latest. No /api endpoint, invariant, or job report
  expresses DR-replication health: /api/logs/analyze, /api/debug/*, and
  /api/health/invariants all say nothing about a sidecar that has stopped replicating. The
  signal lived only in the litestream container's own stderr and its Prometheus metrics,
  neither of which amux reads.
COST: The failure was invisible until a human noticed and hand-diagnosed it: reproducing on
  the box, rm-ing state dirs, and reading container logs per org. A DR-coverage gap ran
  overnight (08-14 into 08-15) with nobody able to see it from amux; had a customer db
  actually corrupted in that window, the first signal would have been data loss rather than a
  probe.
FIX: Root cause fixed at the template (AMUX-3127, b8b358f: pin litestream 0.5.16 + user:0,
  plus a deploy guard that trips on reintroduction). The OBSERVABILITY half is AC-349 (routed
  to amux-cloud): the gateway should poll each sidecar's replica lag / consecutive_errors and
  expose it via /api/observability or a health invariant, so the next DR failure
  self-announces. Open until that runtime signal exists; the CI guard only catches the repo
  reintroduction, not a live replication stall.

## amux send to a bare REPL worker: origin header is submitted as its own message, prompt body is not
AREA: notices
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux-cloud
CARD: AC-354
SYMPTOM: Driving a qwen3.8:27b ollama worker, `amux send qwen-eval "<prompt>"` returned
  `sent (origin-stamped): sent`, but the peek showed the model had received and answered only
  the `[amux-origin: amux-cloud ...]` HEADER (qwen reasoned about it as a possible
  social-engineering attempt and asked what I wanted), while the real prompt sat in the REPL
  input typed-but-unsubmitted (`Press Enter to send`). I had to `tmux send-keys Enter` by hand
  to get an answer. The steering/delivery choreography is claude-UI-shaped: it injects an
  origin header the bare REPL treats as content, and it does not submit the body.
COST: The send reported success while the payload never ran — a false "delivered" (ethos rule
  4). Every eval prompt needed a manual Enter, so the amux worker plumbing could not drive the
  model unattended; I fell back to tmux for the model eval.
FIX: REPL-aware delivery (AC-354, routed to amux, who owns the send/steering path): for
  bare-REPL providers, do not inject the origin header as a submitted message (omit it or make
  it a non-submitted preamble), and ensure the body is actually submitted. Verify by peeking
  that the model answered, not by trusting `sent`. Same message->worker seam as [[amux-project-reference]]
  AC-353 (env-apply can't message a not-yet-started worker).

## A peer's `git commit -a` swept my uncommitted feature edits into their unrelated commit
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux-cloud
CARD: AC-355
SYMPTOM: Mid-edit on session_verbs.rs (14 lines enabling ollama as a provider, AMUX-3145),
  another lane committed `13d66f4 fix(ledger)` with a `git commit -a`-shaped `git add
  <file>`, which staged the WHOLE shared file — sweeping my uncommitted ollama edits into
  their ledger-fix commit. `git diff` went empty under me mid-`git add`; `git status` showed
  the file clean while `grep -c ollama` still found my lines — they were committed, under a
  peer's authorship, in a commit whose message is about the ledger. The CLAUDE.md documents
  this exact class (2026-08-09) and it recurred here.
COST: ~8 min diagnosing where my edits went (were they reverted? committed? by whom?),
  splitting my remaining work into a second commit (b992c99), and a commit whose message does
  not describe half its diff. No code lost, but the attribution + commit-message coherence is
  wrong and only a manual peer heads-up reconciled it.
FIX: The durable fix is not "remember not to `git add <sharedfile>`" — it is per-lane
  isolation for in-flight edits (worktrees), or a pre-commit guard that refuses to stage a
  file another live session has uncommitted hunks in (the staged-guard already KNOWS cotenant
  edits — it warned about env_config.rs minutes earlier — but it warns the committer, not the
  victim, and does not block). Filed AC-355. Same family as [[amux-project-reference]]
  shared-checkout races; three attribution entries now share this seam.

## The shared git INDEX let my `git commit` sweep a peer's STAGED work (mirror of AC-355)
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux
CARD: AC-355
SYMPTOM: I ran `git add <my 3 files>` then `git commit -F` for AMUX-3148. The commit landed
  14 files, not 3 (440 insertions), sweeping 11 files of amux-cloud's and amux-frustrations'
  work (provider/*, backend/*, workers.rs, opencode/*, app.js, the amux CLI) into 4becada
  under MY message and trailer. Root mechanism is worse than AC-355's `git add <sharedfile>`:
  the git INDEX (.git/index) is SHARED across every session in this checkout, so a peer who
  had `git add`-ed their files but not yet committed had their STAGED work committed by MY
  commit — path-scoped `git add` gives no isolation when the index already holds a cotenant's
  staged hunks. The staged-guard NOTE fired ("also edited by amux-frustrations 37m ago") but,
  exactly as AC-355 says, warned me and did not block. When I tried to un-sweep, the
  shared-checkout guard correctly BLOCKED `git reset --soft HEAD~1` (moving shared HEAD can
  decapitate peers' commits), so there was no clean recovery: revert would delete their work.
COST: The work is safe (compiles, not pushed, preserved in 4becada) but attribution is wrong
  on 11 files and there is no in-repo way to fix it without owner sign-off; reconciliation was
  a manual heads-up to two peers. ~15 min. The un-sweep being unreachable from the sanctioned
  tooling is itself an ethos-6 gap.
FIX: Two concrete, either closes it: (1) before committing, assert the staged set equals your
  intended paths — `git diff --cached --name-only` must match what you added, and any extra is
  a cotenant's staged work to `git restore --staged` (scoped, guard-allowed) BEFORE commit; a
  pre-commit hook could enforce this automatically (refuse a commit whose staged set contains a
  file with another live session's uncommitted/staged hunks). (2) per-lane worktrees so the
  index is never shared. Same seam as AC-355; four attribution entries now share it, which is
  the argument for worktree isolation rather than another warning nobody can act on.

---

## `tmux send-keys ... Enter` does NOT submit a codex TUI prompt — amux sessions cannot send tasks to codex workers via raw tmux
DATE: 2026-08-15
SESSION: amux-homepage
AREA: codex-integration
STATUS: open
CARD: AH-81
SEVERITY: slows
TITLE: `tmux send-keys ... Enter` does NOT submit a codex TUI prompt — amux sessions cannot send tasks to codex workers via raw tmux
SYMPTOM: Tested qwen worker (codex --oss --local-provider ollama). Used `tmux send-keys -t "amux-qwen" "task text" Enter` to send prompts. Enter appended a NEWLINE to codex's multi-line input buffer rather than submitting — the prompt accumulated silently, never reached the model. Discovered only after ~45 min of apparent "no response" — the model was idle, not processing. Same issue hit xhigh reasoning effort (qwen does not support extended thinking), which added ~30 min of wasted wait time. Eventually discovered that `POST /api/sessions/<name>/send` correctly submits (amux uses the pane's send protocol that delivers Ctrl+Enter or similar). After switching to the API send, the agent immediately started Working and produced correct output.
COST: ~75 min (45 min for unresponsive session + 30 min debugging xhigh), wrong conclusion that the worker was broken (it was not — the submission method was wrong).
NORMALISED 2026-08-17 by amux-errors-and-bugs, not rewritten — see AEAB-19. This entry
  used `WHAT HAPPENED:` where the contract says `SYMPTOM:`, so `grep '^SYMPTOM:'` could
  not find it, and carried no `SEVERITY:`, so `grep '^SEVERITY: ...'` could not either.
  Renaming the label changed no words. The one INFERRED value is `SEVERITY: slows`,
  derived from this entry's own COST line ("~75 min ... wrong conclusion that the worker
  was broken"); amux-homepage should correct it if that is wrong. The non-contract
  `TITLE:` line duplicating the heading was left alone as harmless.
FIX: `POST /api/sessions/<name>/send` is the correct way to send tasks to codex/ollama workers. `tmux send-keys ... Enter` is wrong for codex TUI — it inserts a newline, not a submit. No amux docs or session-card says this; it is an easy mistake for any session testing a codex worker. Also: codex's global config `model_reasoning_effort = "xhigh"` is incompatible with local qwen models (qwen does not support extended thinking API); workers using `--oss --local-provider ollama` need `-c model_reasoning_effort=low` to be responsive.

---

## A shared CARGO_TARGET_DIR is mandated, and concurrent builds in it evict each other's artifacts
AREA: build
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux-frustrations
CARD: AMUX-2936
SYMPTOM: `error: extern location for serde_core does not exist: ~/.amux/rust-build-target/debug/deps/libserde_core-0d2476c6ed9be3cc.rmeta`, and separately 42 errors inside the `nix` crate ("cannot find type `ControlFlags` in this scope") — artifacts deleted underneath an in-flight build, three times in one session.
  CLAUDE.md requires ONE shared build dir (~/.amux/rust-build-target) and the reasoning is sound — per-session dirs filled the disk with ~37 copies at 10-15GB each. But with several lanes plus the auto-builder building concurrently, I hit repeated hard failures of the form "extern location for serde_core does not exist: .../libserde_core-<hash>.rmeta" and 42 errors inside the `nix` crate, i.e. artifacts deleted underneath an in-flight build. Not a lock contention wait, which is what the CLAUDE.md note measured and correctly called cheap; this is cache eviction, and the only recovery is a full rebuild. Hit it three times in one session, roughly 4 minutes of rebuild each.
COST: ~12 min of pure rebuild, and worse, it masqueraded as a code error twice — the first failure looked like my own change had broken the build, which is exactly the wrong instrument reading (a red result on code you just verified by hand means the instrument is a candidate before the code is).
FIX: Not fixed; needs a decision, not a workaround. Options: (a) leave it — the failure is loud and self-recovering, just expensive; (b) give the auto-builder its own target dir, since it is the one builder that runs unattended every 60s and is the most likely evictor, accepting ~15GB for the one process that never benefits from a warm shared cache; (c) find whether this is cargo GC (CARGO_GC / cache auto-clean) rather than eviction, in which case pinning the retention setting fixes it outright and costs nothing. (c) is worth checking first because it would be a one-line fix, and nobody has established WHICH of the three is happening — the diagnosis is missing, not the remedy.

## amux-launched browser does not survive a server self-adopt
AREA: browser
SEVERITY: slows
STATUS: open
DATE: 2026-08-15
SESSION: amux
CARD: AMUX-3184
SYMPTOM: Driving the dashboard for the ollama UI E2E, the amux-launched Chrome (POST /api/browser/start, a Playwright/CDP child of the server) vanished twice mid-test. Each time the trigger was the local auto-builder adopting a fleet commit: the server self-adopts (exits for launchd to relaunch) and the Chrome child dies with it. On a shared checkout where ANY session's commit swaps the binary every ~60s, any browser-driven task longer than a build cycle loses its session.
  CORRECTION (verified after filing, and it is the more useful lesson): my first report also claimed the failure was SILENT, that /api/browser/screenshot returned {"path": null} with no error. That was MY probe, not the endpoint. The handler returns a clear, actionable body, {"error":"no amux-launched browser is running, POST /api/browser/start ... first", "hint": ...}, and it already WARNs on wedged captures. My extraction was `python3 -c "print(json.load(sys.stdin).get('path'))"`, and an error response carries no `path` key, so it printed "None" and I read the None as a silent null. Exactly the ethos rule 7 trap: a blank result on code I had not yet read means the INSTRUMENT is the candidate before the code is. The instrument half of this card is a non-bug; the endpoint errors clearly today.
COST: ~8 minutes. ~6 across two browser restarts (re-open the peek via openPeek eval; the tmux pane re-rendered its shell setup so the worker's response had to be read from the peek history API), plus ~2 chasing a "silent failure" that my own extraction script invented and I filed a card for before reading the handler.
FIX: The real residual is lifecycle, not instrumentation. Launch Chrome DETACHED (not a server child) and persist its cdp_http/cdp_port/pid (the start response already returns all three), so a freshly self-adopted server re-attaches to the still-alive Chrome instead of orphaning it. Until then, a browser-driven task must expect to restart the session across a builder swap. The instrument half needs nothing.

## Cloud freshness tick's served-APP_VER probe returns empty because app.js 302s to /sign-in
AREA: cloud
SEVERITY: slows
STATUS: open
DATE: 2026-08-16
SESSION: amux-cloud
CARD: AC-360
SYMPTOM: The CLOUD FRESHNESS TICK step-1 probe `curl -sk https://cloud.amux.io/app.js | grep APP_VER` returns EMPTY. Unauthenticated `app.js` now 302s to `/sign-in` (http=302, size=0). `/health` and `/version` also 302; `/api/health` and `/api/version` 401. There is no auth-free endpoint on the gateway that reveals the served build, so the recipe's `served=` is always blank for whoever runs the tick.
COST: A blank `served` compared against a non-empty `head` reads as "cloud is behind origin/main" and, taken literally, would dispatch `recreate=yes` — which STOPS every worker container and does not restore them. That is precisely the false-positive-recreate-before-a-demo harm the 2026-08-12 guard was added to prevent, and here the trigger is a broken probe rather than a real drift. Caught only because I recognised the empty read as a probe fault, not a signal (ethos rule 7: an empty grep is not a measurement). A less careful run recreates prod to "fix" a drift that does not exist.
FIX: (proposed on AC-360) Drop the app.js scrape. The robust, auth-free freshness signal already exists: the newest SUCCESSFUL `deploy-cloud` run's headSha vs origin/main. `gh run list --workflow=deploy-cloud.yml --json headSha,conclusion` -> newest `success` sha, then `git rev-list --count <sha>..origin/main`; 0 means the deployed image is current. Used exactly that this tick (last success 31914150334 built 73fce92; origin/main tip == 73fce92, 0 behind). The recipe change is Ethan's to make (it is his standing scheduler prompt, ethos rule 8) — proposed, not silently rewritten. Distinct from AC-344 (deploy-cloud SKIP is silent); this is the tick's own comparison being unable to read the served version at all. FIXED 2026-08-16 (c026008 log + SCHED-344 command PATCH): the tick now runs the deploy-sha probe, verified live on a later firing.

## Metrics disk gauge read 0.6% used while the volume was 90% full and failing writes
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-16
SESSION: desktop
CARD: DESKT-3
SYMPTOM: `/api/metrics` reported `disk_used_gb: 11.4`, `disk_total_gb: 1858.2`, `disk_percent: 0.6` — a green "low" gauge on the dashboard — while `df -h /System/Volumes/Data` showed 1.7Ti used of 1.8Ti, 26Gi free, 99% full. Writes were already failing elsewhere on the box. Cause: `collect_system_metrics` ran `df -k /` and parsed its Used column. On APFS `/` is the SEALED READ-ONLY system snapshot (~11GB, essentially static); everything a user owns lives on `/System/Volumes/Data`. So the gauge was reading a different volume than the one that fills up, and it will read ~0% forever no matter how full the machine gets.
COST: the disk filled to 99% with the dashboard showing green the whole way, and nobody had a reason to look. The failure mode is the expensive one: not a missing instrument (which prompts a manual check) but a confident wrong one that is trusted and read first. CLAUDE.md already records a related incident where ~37 abandoned target dirs took the volume to 741MB free with a 50-session fleet running — the gauge that was supposed to make that visible could not, by construction, ever have shown it. Also cost this session a wrong initial read of how much headroom the machine had.
FIX: fixed in the same breath as building the reclaim scanner. `/api/metrics` now derives disk from `statfs($HOME)` through `reclaim::df_bytes` — asking about a PATH rather than a mount point is correct on APFS by construction, and sharing the helper with the scanner means the two views can never disagree (a second spelling would drift). Adds `disk_free_gb` because free space is the number you act on, and emits `tracing::warn!(disk_percent, free_gb, "disk critically full")` at >=90% so the NEXT disk-fill announces itself in a log sweep instead of waiting for someone to open the dashboard. Regression test `disk_metrics_measure_the_data_volume_not_the_sealed_system_snapshot` was verified to FAIL against the pre-fix code with "metrics reports 0.6% used but the data volume is at 90.56%".

## `git commit` after `git add`-ing only my files still ships a peer's pre-staged file
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-16
SESSION: amux (file-manager subagent)
CARD: AMUX-3249
SYMPTOM: I `git add`-ed exactly my four dashboard files, then `git commit`. The
  commit landed FIVE files: my four plus `crates/amux-server/src/runtime_jobs/mod.rs`,
  a peer's (`desktop`/`amux`) comment-only em-dash cleanup that was already sitting
  staged in the shared index. `git commit` commits the whole index, not just the paths
  I added, so a file another session left staged rides along under my SHA and my card.
COST: a foreign Rust file shipped in a dashboard-UX commit (8b802e1) against an explicit
  "do not touch Rust files / only git add your exact files" instruction. Harmless here
  (comment-only, compiles, same-session attribution) but the mechanism ships un-reviewed
  peer code silently. Caught only because `git show --stat` listed 5 files, not 4.
FIX: the durable habit is `git commit -- <exact paths>` (path-scoped) or
  `git diff --cached --name-only` + unstage foreign paths BEFORE committing — the second
  AMUX-3242 commit used `git commit -- <paths>` and landed exactly 2 files. Worth putting
  the path-scoped form in CLAUDE.md's Deploy section as the default on this shared checkout,
  since a headerless `git add` of your files does not protect you from a peer's pre-staged index.

## staged-guard can't see a subagent's own edits, so it blocks the subagent's real work as "foreign"
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-16
SESSION: amux (file-manager subagent)
CARD: AMUX-3249
SYMPTOM: The pre-commit staged-guard bases its verdict on per-session EDIT RECORDS in a
  time window, not on the staged diff. Running as a subagent, my Edits to app.css /
  index.html / sw.js produced no edit record under my session, so the guard reported
  "they wrote it (transcript); you have no edit record on this path" and BLOCKED the
  commit, naming `desktop` as sole author of files I had just rewritten this session.
COST: the commit was blocked; I had to read the FULL staged diff of app.css and index.html
  by hand to confirm every hunk was mine, then use `AMUX_VERIFIED_SOLO=1` to override. The
  guard's own advice ("keep only your hunks") assumed the peer's work was mixed in when it
  was not. The dangerous edge: a subagent conditioned to reach for AMUX_VERIFIED_SOLO on
  every commit will eventually rubber-stamp a diff that DOES carry foreign hunks, since the
  guard cries wolf on every subagent commit.
FIX: the guard needs a signal a subagent's edits actually exist — attribute Edit-tool writes
  to the running (sub)agent session, or fall back to the staged diff (not edit records) when
  no edit record exists for EITHER party. Basing the verdict on the staged diff directly
  would make it correct regardless of who recorded what.

## Idle guard called a CLEAN tree dirty, then prescribed a 44-commit revert as the "safe" action
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-16
SESSION: desktop
CARD: DESKT-10
SYMPTOM: The idle dirty-tree notice reported "2 uncommitted change(s)" for app.css and app.js while `git status --porcelain` was EMPTY. Both worktree blobs were byte-identical to HEAD; they differed only from origin/main, which this checkout sits ~44 commits ahead of. The notice then ran its direction test, `git cat-file -e $(git hash-object <path>)`, got "object exists" for both, and classified them STALE, whose prescribed remedy is `git checkout origin/main -- <path>`. Running that would have reverted app.js by 1153 insertions and deleted crates/amux-server/src/api/reclaim.rs entirely, a feature shipped hours earlier. I tested five committed-but-unpushed paths (app.js, app.css, reclaim.rs, api/mod.rs, frustrations.md) and every single one classified STALE.
COST: no work lost, because the tree being clean vs HEAD was checkable in one command and I checked before acting. The cost is the trap itself and how well disguised it is. The notice opens by warning that a difference from origin is not a direction, and then uses a test carrying exactly that blind spot, so the warning reads as evidence the test already accounts for it. It also states that roughly 1 in 4 differing paths are novel mid-edits a checkout would destroy, which frames "STALE" as the safe verdict and pushes toward the destructive branch. Any session that follows it literally on this checkout reverts every file it names.
FIX: the direction test must be ANCESTRY, not blob existence. Blob existence cannot tell an old revision from a current one that is merely unpushed; both answer yes, and on a permanently-ahead checkout every committed file answers yes. `git merge-base --is-ancestor $(git log -1 --format=%H -- <path>) origin/main` separates them exactly: false means committed and unpushed, so leave it alone; true plus a worktree difference means genuinely older. Second, gate the notice on `git status --porcelain` being non-empty, so a tree that is clean against HEAD never triggers it at all. Both are one-line changes and either alone would have prevented this.

## SUPERSEDES the entry above: the guard's classifier was right, only its printed ADVICE was wrong
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-16
SESSION: desktop
CARD: DESKT-10
SYMPTOM: Same incident, corrected diagnosis after reading commit_nudge.rs instead of reasoning from the notice alone. Two claims in my entry above were wrong. FIRST: the guard does NOT classify with blob existence. `freshness_from_repo` uses `git log HEAD..origin/main -- <path>`, which is proper ancestry and correctly returns not-stale for a committed-but-unpushed file. What prescribes `git cat-file -e $(git hash-object <path>)` is the message TEXT the guard prints, in its two direction-unknown branches. The classifier and the advice disagreed, and the advice is the half a human acts on. SECOND: I reported it firing on a CLEAN tree. `dirty_paths` reads `git status --porcelain`, so it cannot. The real explanation is a race: at nudge time the amux lane had app.css and app.js uncommitted, and by the time I ran git status they had committed them in 2ec671b. The notice itself said CONTESTED, also edited by amux, which fits. So the "gate the notice on porcelain non-empty" fix I proposed was unnecessary.
COST: nothing beyond my own time, and it would have cost the amux lane theirs: they picked the card up and were about to hunt for a second code path that does not exist. Worth recording because of HOW the wrong diagnosis was produced. I ran the blob test, watched it misclassify five real paths, and concluded the guard classified that way, when all I had actually established was that the printed recipe was wrong. The notice's text was treated as evidence of the code's behaviour. Reading the 40 lines of commit_nudge.rs would have separated them in a minute, and I filed a card and a frustrations entry before doing it.
FIX: 5b923db. Both direction-unknown branches now print the ancestry test the classifier already uses, state which way each outcome points, and name blob-existence as the thing not to substitute plus why. The STALE section's use of blob-existence is deliberately kept: there the path is already proven behind, and the open question is pure-old-copy vs novel-mid-edit, which blob existence answers correctly. Regression test asserts on the message text and was verified to fail against the old recipe. The durable lesson is narrower than my first entry: when a notice and the code disagree, read the code before filing against either, and say which one you actually measured.

## SUPERSEDES both entries above on DESKT-10: blob existence is unsound in the STALE section too
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-17
SESSION: desktop
CARD: DESKT-10
SYMPTOM: My fix 5b923db moved the direction-unknown branches to the ancestry test but DELIBERATELY kept `git cat-file -e $(git hash-object <path>)` in the STALE section, with a comment arguing it was correct there because the classifier had already proven the path was behind. cold-outbound proved that wrong and I reproduced it: commit v1, edit to v2, `git add` without committing, and cat-file -e reports EXISTS while `git log --all --find-object=<blob>` is empty. `git add` writes the blob into .git/objects, so cat-file -e answers "ever written to the object DB", not "ever committed". The prescribed `git checkout origin/main -- <path>` then deletes the never-committed mid-edit. cold-outbound hit a live 4-minute near-miss on server-fast-checks.yml, mid-keystroke.
COST: a destructive false positive shipped into standing advice for every lane, for about 14 hours, and a near-miss on someone else's uncommitted work. The gap is not exotic: any session that stages incrementally produces it constantly, and it fires in the delete direction rather than the redundant-commit direction.
FIX: `git log --all --find-object=<blob>`; empty means never committed anywhere. `--all` matters, since a blob committed only on origin or another branch reads empty under a HEAD-only search, which errs safe but still misclassifies. amux has a fix agent in flight across commit_nudge.rs, the shell guards and session-freshness.sh, with a regression test; I am staying off those files rather than being a second editor. What generalises past this bug: I decomposed the question correctly (once a path is known behind, ask pure-old-copy vs novel-mid-edit) and then never checked that the instrument answered the sub-question I had just posed. A correct decomposition makes the wrong instrument feel already-validated, because the reasoning that selected it was sound. Verify the mechanism, not the verdict, applies to the sub-question too, and I had quoted that rule at another session hours earlier.

---

## push-guard reports "unknown (api unreachable)" instead of reading the Amux-Session trailer from the commit
DATE: 2026-08-17
AREA: attribution
STATUS: open
SEVERITY: annoys
SESSION: amux-homepage
COST: 5 minutes of false diagnosis and a wrong message sent to amux
CARD: AEAB-21
SYMPTOM: push-guard emits "currently: unknown (api unreachable)" when it fails to resolve the owning session's identity via the API. The Amux-Session trailers ARE present and correct in git log, so the attribution data exists — the guard just cannot reach the API at that moment and falls back to "unknown" instead of reading the trailer directly from the commit. This caused me to message amux asking them to push as if something was wrong with their commits, when the real issue was a guard API lookup failure.
REPRODUCE: trigger a push that the guard will block; if the server is mid-restart (builder cycle), the guard resolves "currently: unknown (api unreachable)" even for sessions with fully-attributed commits.
FIX: the guard should fall back to the Amux-Session trailer in the commit itself when the API is unreachable, rather than reporting "unknown". The data is already in the commit; the API is just a secondary confirmation. Never let an API timeout degrade a git-native source.
NORMALISED 2026-08-17 by amux-errors-and-bugs, not rewritten — see AEAB-19. This entry
  arrived in 18590ca8 with no `## ` heading, so the audit's parser folded it into the
  entry above and the file's own greps could not see it. Changes were: added the heading
  (worded from this entry's own first clause), `DESCRIPTION:` -> `SYMPTOM:`,
  `FIX DIRECTION:` -> `FIX:`, `SESSION:` filled from the commit's own Amux-Session
  trailer, and `CARD: (file one)` -> AEAB-21, which I filed on this entry's behalf
  because it asked for one. Not a word of the account was altered. The one INFERRED
  value is `SEVERITY: annoys`, derived from this entry's own COST line ("5 minutes of
  false diagnosis"); amux-homepage should correct it if that is wrong.
## `amux board add` folds every flag into the title, and `--help` files a card
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-17
SESSION: amux-errors-and-bugs
CARD: AEAB-17
SYMPTOM: `amux board add` takes only a positional title. Unknown flags are concatenated
  into it, exit 0, no warning — proved with a control rather than inferred from missing
  help: `amux board add "PROBE..." --totally-bogus-flag xyz` filed AEAB-16 with the flag
  in the title. Separately, `amux board add --help` does not print help; it creates a
  card titled `--help` (AEAB-15). The verbs that DO take flags are inconsistent with it:
  `retitle` accepts `--stdin/--file/--desc-stdin` and `--type`, every status verb accepts
  `--checked/--ack/--outcome-file`; `add`, the verb you reach first, accepts none.
COST: All five cards filed in one log-review run (AEAB-9..AEAB-13) were created with
  desc_len 0 or near-0 — every diagnosis written for them went nowhere — titles up to
  259 chars carrying raw `--desc-file /private/tmp/claude-501/.../c26.md` paths that
  will not exist tomorrow, and the wrong TYPE, so two cards about a production outage
  and a dual-server fault were gated on "Implemented and merged" + "Tests / lint pass".
  An empty card is worse than no card: it asserts the work is tracked. And the
  discovery path is the polluting action, so the mistake is unlearnable in advance —
  the AMUX-2140 shape (`amux board claim`) with a new verb.
FIX: Cards repaired via the documented path (`retitle ... --desc-stdin`) and verified
  from fresh reads; probe cards discarded after confirming they were mine. CLI not
  fixed — not mine. Wanted: reject unknown flags, print help for `--help`, accept
  `--type`/`--desc-file` on `add` like `retitle` does, and a title-length sanity check
  (a 259-char title containing `/private/tmp/` is never intentional).
MY OWN HALF, recorded because ethos.md predicted it verbatim: I verified `type` and
  `status` on those cards from fresh reads and never checked `desc` — the exact
  desc_append/AMUX-2161 failure rule 7 describes ("the habit gave the feeling of rigour
  while pointing at the wrong field"). I had read that line earlier in the same session.
  Reading a rule does not install the habit; verify the operand you just wrote.
  VERIFIED 2026-08-20 (amux-frustrations, NOT the author; amux-errors-and-bugs is not a reachable fleet lane): `amux board add --help` prints usage and files NOTHING. Checked with a control rather than by eye — 0 cards created on this board in the 3 minutes around the call.
## The auto-builder ships any branch to the live fleet with no announcement
AREA: deploy
SEVERITY: blocks
STATUS: open (the live deviation is fixed; the hazard is not)
DATE: 2026-08-17
SESSION: amux-errors-and-bugs
CARD: AEAB-12
SYMPTOM: `~/amux` is the BUILD SOURCE, and the builder rebuilds on any local HEAD move
  regardless of branch; the server self-adopts in 5s. I committed a9aa7177 on a feature
  branch there at 00:02; at 00:03:43 the builder installed it and it served the whole
  fleet until 09:45 — 9h42m of an unreviewed, un-CI'd commit in production. The same
  condition left the machine 29 commits behind origin/main, so SCHED-1 ("keep me on the
  latest") fired at 09:00 and could not do its job.
COST: 9h42m of unreviewed code live, plus the owner's standing "keep me on the latest"
  request silently unmet while every indicator looked healthy. Diagnosing it took the
  first ~30 minutes of a log review that was supposed to be about something else.
FIX: Live deviation fixed — ~/amux back on main, fast-forwarded to 9d5aebf4, verified
  by build-stamp change (663a3a84 -> ec3228af), store=ok, 0 panics/0 ERRORs since. The
  hazard is NOT fixed and should not be fixed by refusing non-main HEADs: this machine
  survived weeks deliberately pinned to an unmerged fix branch, so that is a supported
  mode. The defect is that a deliberate pin and an accidental feature branch are
  byte-identical to the builder and the accidental one is announced nowhere. Wanted:
  one line in rust-auto-build.log naming the branch when the revision is off main, and
  the same fact on /health or the dashboard. Workaround that works today and belongs in
  CLAUDE.md: never develop in ~/amux — `git worktree add` and leave its HEAD on main.

## Two amux servers on one SQLite DB, and endpoint.json points at the wrong one
AREA: port
SEVERITY: blocks
STATUS: open — owner's decision
DATE: 2026-08-17
SESSION: amux-errors-and-bugs
CARD: AEAB-11
SYMPTOM: Two launchd jobs both run the Rust server against `~/.amux/amux.db` —
  `com.amux.server-rs` (pid 22521, port 8824, last exit -9) and `com.amux.serve`
  (pid 22053, port 8823, exit 0) — same binary, same build, both logging "schedule loop
  starting (FIRING)". Every `starting amux-rust` line before today was 8824 and single;
  8823 starts begin 2026-08-17 03:53:41.
COST: One batch of request-log rows was DROPPED (`request-log insert failed; rows
  dropped error=database is locked`, 04:07:34) — the first and only lock error in the
  file, all time, inside the dual-instance window. `endpoint.json` now advertises 8823,
  so every hook self-healing a stale AMUX_URL off it reaches the OTHER server; my own
  sync-github.sh resolver (frustration above / LR-22) now resolves to 8823 and works
  only because 8823 happens to answer. And it doubled the log: both instances tick the
  same 5s stall loop, so those warnings appear twice ~200ms apart, which is 77% of the
  24h log volume and buried the lock error above.
  DOCS NOW WRONG, second time for this class: CLAUDE.md asserts as ground truth
  "re-measured 2026-08-06" that "com.amux.serve.plist is the only server plist on disk"
  and gives `launchctl kickstart -k gui/$(id -u)/com.amux.serve` as THE restart command.
  There are two server plists now, and that command restarts 8823, not the canonical
  port. The note is emphatic that a wrong label costs a debugging session; it is now
  wrong itself.
FIX: Not applied — choosing which job is canonical can take the dashboard down, and a
  dev instance with its own AMUX_HOME is a legitimate configuration this could also be
  (ethos rule 8). Needed: decide, `launchctl bootout` the loser, delete its plist,
  correct CLAUDE.md's launchd note.

## frustrations.md logged from ~/Developer/amux is stranded — that checkout cannot push
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-17
SESSION: amux-errors-and-bugs
CARD: AEAB-18
SYMPTOM: Two copies of this file exist and the one a session is pointed at is the one
  that cannot reach anyone. `~/Developer/amux/frustrations.md` holds 25 entries /
  43,934 bytes; `~/amux/frustrations.md` (origin/main) holds 116 / 207,952. Same file,
  same lineage — the local one is a stale revision that has ALSO diverged, holding at
  least one entry that never reached origin. CLAUDE.md and `.claude/rules/frustrations.md`
  both live in the stale checkout and say to append to "frustrations.md at the repo
  root", which for a session cwd'd there resolves to the stranded copy. The append
  succeeds. There is no error.
COST: All four frustration entries from today's log review went into the stranded copy.
  The whole argument for this file is that a single frustration is a complaint and a
  cluster is an argument — three entries sharing an AREA is the signal. That only works
  if they are in the file everyone reads. Mine were invisible to every other session and
  to any AREA tally run upstream, and would have stayed so indefinitely: the unblocker
  is the 4-unpushed-commit divergence that has been an open owner decision since
  2026-08-13.
  Distinct from that divergence rather than a restatement of it: that one is "the
  checkout cannot fast-forward", which announces itself. This one is "the documented
  place to log friction is INSIDE that checkout", so the divergence silently swallows
  new writes instead of blocking a read.
FIX: Migrated today's four entries here and verified against
  `scripts/frustrations_audit.py` — no new structural problems, all four CARD ids
  resolve on the live board. The underlying choice is open and worth making
  deliberately: (a) resolve the divergence so the checkout syncs again — owner's call,
  needed regardless; (b) point the rule at the build source, which can push, and say why;
  (c) have the rule REFUSE to append to a checkout that is behind origin, or at minimum
  warn. (c) is the one that survives the next time two checkouts drift, because this
  failure is silent by construction.

## session /start reports ok:true for a commandless session but starts nothing and logs nothing
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-18
SESSION: amux-cloud
CARD: AMUX-3364
SYMPTOM: After a recreate=yes cloud deploy (AC-374) I restarted each POC's tmux sessions via
  POST /api/sessions/<name>/start. The 12 real workers came up. The seeded `hello-world`
  scaffold (command/cmd/agent all null) returned `{"ok":true,"message":"starting","resumed":false}`
  in all 3 orgs, yet no tmux session materialized and ~/.amux/logs/server-rs.log had ZERO lines
  mentioning it — grep for the name returned nothing. A start that spawned nothing was
  byte-indistinguishable from a start that worked.
COST: ~10 min chasing a phantom "1 of 4 sessions failed to restart" before confirming the missing
  one was a no-command scaffold, not a real worker. A log sweep or autofix loop could never catch
  this class: the no-op leaves no trace at all.
FIX: (1) /start should refuse a commandless/agentless session with an explicit
  `{"ok":false,"error":"no command configured"}` instead of an optimistic ok:true. (2) a start
  that spawns no tmux session must WARN (session=<n> reason=no-command) so the next occurrence
  self-announces. Routed to amux (owns session lifecycle) as AMUX-3364.

---
## Prompt auto-capture writes a credential the user typed verbatim into a fleet-readable board card
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-19
SESSION: amux-cloud
CARD: AMUX-3384
SYMPTOM: Ethan typed a NetSuite login password in chat and said to keep it in the PRIVATE
  amux-GTM repo only. amux auto-captured the prompt into a board card (AC-381) with the
  password value verbatim in the TITLE. The board is fleet-readable (every session can GET
  /api/board/<id>), so a credential meant for a private repo was sitting in shared state.
COST: A credential leak into fleet-shared state that directly contradicted the user's stated
  scoping. Had to hand-scrub the card (overwrite title+desc) and confirm the value was gone;
  a discarded card is still retrievable, so the value transited the board regardless. ~5 min
  plus the exposure window.
FIX: the capture path should redact obvious secret shapes (long base64/hex, values after
  pw|password|secret|key=) before writing the card, OR skip capturing a prompt flagged as
  carrying a credential, OR keep the capture body out of the shared board store. Filed as AMUX-3384
  to amux (capture/board is their domain). Same family as AC-214: a secret reaching a store the
  owner did not intend.
---
## The two causes behind that outage are not amux bugs, and amux had nothing to say about either
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-18
SESSION: amux-errors-and-bugs
CARD: AEAB-28
SYMPTOM: The machine was up and on the network at 15:18; amux did not start until the
  console login at 18:28 — 3h10m later. All four amux units are user LaunchAgents in
  `~/Library/LaunchAgents` with no `LimitLoadToSessionType`, so they are `Aqua`: they
  load at GUI LOGIN, not at boot. `ls /Library/LaunchDaemons | grep -i amux` -> none.
  `RunAtLoad=true` is doing exactly what it says; "load" just never happened. Separately,
  the machine died in the first place from a hardware undervoltage fault
  (`Boot faults: uv,vdd_boost_uvlo`, `Boot failure count: 2`) — AEAB-30.
COST: Turned a ~75-minute hardware outage into a 4h26m amux outage. On a headless box
  this is unbounded: it ends when a human happens to sit down.
FIX: Owner's call, and genuinely a trade — `LimitLoadToSessionType = Background` starts
  at boot but leaves the login keychain locked, so lanes needing provider credentials
  may fail in a way that looks like a broken lane rather than a locked keychain;
  automatic login is simpler but is incompatible with FileVault and is a posture change
  on a Tailscale-reachable machine. Filed rather than chosen (ethos rule 8). What is NOT
  the owner's call and should ship regardless: `install.sh` says nothing about this
  property, so every amux install has it and no operator has been told.

---
## `amux board done --outcome-stdin` printed a warning about the outcome and silently applied NOTHING
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-19
SESSION: amux-errors-and-bugs
CARD: AEAB-36
SYMPTOM: Closing AEAB-34, the entire output was:
    warning: outcome NOT recorded — server sent no JSON
  Verified against the API immediately afterwards: status still `review`, desc_len
  unchanged at 2792, no new log line. NEITHER the outcome NOR the status transition
  landed. Re-running the identical command with the identical ~2.9KB input succeeded
  completely (`AEAB-34 → done`, EXIT=0, desc +2915 chars). Nothing appeared in
  server-rs.log for the failed request.
COST: Caught only because I checked the operand I had just written — the habit this repo
  learned from desc_append/AMUX-2161. Without that check the card would have sat in
  `review` while I reported it closed, and the next nudge about it would have read as the
  board misbehaving rather than as my write evaporating. The warning actively misleads:
  it names ONE of the two things the command does, so the natural reading is "status moved,
  prose lost" — the opposite of what happened.
FIX: The CLI cannot know what landed when the server sends no JSON, so it must say exactly
  that ("no change may have been applied — re-run and verify") and exit non-zero, rather
  than emitting a field-scoped warning that implies the rest succeeded. Separately, a
  request that produces neither a response body nor a server log line is its own defect —
  whatever path this took leaves no trace, which is the AMUX-2140 shape. Note this is the
  SANCTIONED path: `--outcome-stdin` exists precisely so a gated transition never needs a
  hand-rolled curl, so a silent no-op here pushes people back to curl, which is how
  attribution gets lost.
---
## I wrote a keep-warm branch that could never execute, hours after writing about that exact failure mode
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-19
SESSION: amux-errors-and-bugs
CARD: AEAB-35
SYMPTOM: AEAB-34's fix added an early exit to the builder's disk guard — reclaim the idle
  cache, and if free space is back above the floor, keep the shared cache so the build
  stays warm. The branch fired ZERO times and could not: it tested against
  `AMUX_BUILD_MIN_FREE_GB` (25GB) on a volume with 4GB free, so after reclaiming the idle
  cache the condition was still false and the shared cache was destroyed anyway.
  `grep -c "reclaimed to" rust-auto-build.log` -> 0.
COST: Half a fix presented as a whole one. The card AEAB-34 is titled "so every build is
  cold" and every build stayed cold; only prod verification caught it, and only because I
  looked for the branch's OUTPUT rather than re-reading the code. Had I not, the card would
  have closed claiming a result it did not deliver.
FIX: Two thresholds, because there are two questions —
  `AMUX_BUILD_MIN_FREE_GB` (25) asks "is the FLEET at ENOSPC risk", which is what decides
  whether to reclaim at all; `AMUX_BUILD_SACRIFICE_CACHE_BELOW_GB` (8) asks "can THIS BUILD
  proceed while keeping its cache". Between them, reclaim the idle caches and stay warm.
  Two test cases that EXECUTE the branch, driving the shipped script with a fake `df`
  placed in `$HOME/.cargo/bin` — the script exports its own minimal PATH whose first entry
  is exactly that, so no test-only branch is added to production code. Mutation-checked:
  restoring the single threshold fails the keep-warm case specifically.
WHY IT IS WORTH AN ENTRY EVEN THOUGH I FIXED IT: the ethos file records "a threshold below
  the baseline is not a detector", and this is its mirror — a stop condition ABOVE the
  achievable maximum is not a stop condition. I had cited that rule in a commit message the
  same morning. Authoring a rule does not install the habit; that is the third instance of
  that sentence being true in this session, and the pattern is only visible because the
  earlier two were also written down. The generalisable check: when adding an early exit,
  state the RANGE OF INPUTS under which it fires, and confirm the live system produces
  values in that range.
---
## `set -euo pipefail` turned a deliberately non-fatal board write into a silent abort
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-19
SESSION: amux-errors-and-bugs
CARD: AEAB-36
SYMPTOM: SUPERSEDES the earlier entry "board done --outcome-stdin printed a warning about
  the outcome and silently applied NOTHING" — that entry recorded the symptom correctly and
  called the cause unknown. It is now known and reproduced deterministically.
  The outcome write is `curl ... | python3 -c ...`, and the python exits 0 on failure ON
  PURPOSE so a lost outcome never blocks the transition that follows. `set -euo pipefail`
  (amux line 19) defeats that entirely: on a TRANSPORT failure curl exits non-zero,
  `pipefail` makes the pipeline return curl's status rather than python's, and `set -e`
  aborts the script right there — AFTER the warning has printed. One warning, scoped to
  the outcome, and the transition never ran and never said so.
  The transition write was worse. Not piped at all, so `result=$(_board_transition ...)`
  aborted the caller directly: `amux board done <ID>` against an unreachable server printed
  ABSOLUTELY NOTHING and exited 7.
  Reproduced with `AMUX_API=http://127.0.0.1:9`: one warning + exit 7 with an outcome,
  total silence + exit 7 without one — byte-identical to the live incident.
COST: A card reported closed that was still in `review`, caught only by re-reading the
  operand. The live trigger was the server restarting to adopt a build I had just merged —
  so the failure fires exactly when amux is deploying itself, which is when a session is
  most likely to be closing cards. And the warning's wording actively misleads: naming only
  the outcome makes "status moved, prose lost" the natural reading, which is inverted.
FIX: Capture curl's OWN exit status at both sites instead of piping it, and on a transport
  failure say plainly that NOTHING was applied — naming both writes — then stop. A dropped
  connection and a refused gate now read completely differently, which matters because one
  should be retried and the other must not be.
  `scripts/test-board-transport-failure.sh` (11 cases, wired into checks.yml). Its (c)/(d)
  controls are the load-bearing half: a CLI that shouted "cannot reach the board" at every
  error would pass both failure cases while destroying that distinction, and
  mutation-checking confirms it fails (c) and (d) specifically.
  THE GENERAL LESSON, worth more than the fix: `set -e` + `pipefail` silently overrides
  every `sys.exit(0)` written to make a step non-fatal. Any `cmd | python3 -c '...exit(0)'`
  in this file has the same defect latent. Grep for the pattern before trusting that a step
  is really optional.
---
## The "oldest undeployed commit" age was the NEWEST one, so an overnight lag read as 3 minutes
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-20
SESSION: amux-errors-and-bugs
CARD: AEAB-39
SYMPTOM: The SessionStart hook printed "the RUNNING SERVER is 4 commit(s) behind
  origin/main / built 9f92a631f; oldest undeployed commit landed 3 minutes ago". The oldest
  undeployed commit was 23 HOURS old (7ac99632). The line computed
  `git log -1 --format=%cr <range> | tail -1`, and `-1` limits git to one commit — the
  newest — so `tail -1` never sees a second line.
COST: The count was right; the number that makes it ACTIONABLE was always small. This line
  exists to separate "just merged, the builder is about to pick it up" from "a fix has sat
  undeployed overnight", and only the second is worth acting on — so the alarming case is
  precisely the one it could never display. It would have read as reassuring every time,
  forever. I shipped it yesterday in the same commit whose message argued that a bare count
  "reads as bookkeeping" and the age is what makes it actionable.
FIX: `git log --reverse --format=%cr <range> | head -1`. New case (i2) in
  test-session-freshness.sh builds a repo with one commit dated 2020 and one dated now, so
  taking the newest says "seconds ago" and taking the oldest says "years ago". The 25-case
  suite passed with the bug present because every other case asserts the COUNT — a whole
  suite around a line, none of it testing the field that carries the meaning.
  Fourth instance this session of a check I wrote reporting something plausible but wrong
  (LR-37 filter, LR-42 vantage, LR-43 proximity grep, this). The common shape is not
  carelessness: each probe RAN and returned something usable, so nothing prompted a
  recheck. What catches them is comparing the output against an independently-known value —
  here, `git log` over the same range without `-1`.

---
## A ghost-rescue log quoted a pane scrape as the user's message, and it reads as corruption
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-20
SESSION: amux-errors-and-bugs
CARD: AEAB-38
SYMPTOM: At 00:50 EDT the owner sent a real message from their phone; the send path failed
  to submit it and ghost-rescue recovered it. The WARN line quoted it as
  "Canyoufightthisforme?Don'tsendbutdodraftandaddresstotherightperson" — every space gone.
  That reads unmistakably as the prompt being corrupted somewhere in delivery.
COST: Nearly a filed user-facing data-corruption bug, on the owner's own overnight message,
  which is about the worst thing to be wrong about. Cost ~10 minutes to disprove.
FIX: The preview is a PANE SCRAPE. `composer_state` builds it with
  `plain.extend(p.split_whitespace())` — words concatenated, whitespace discarded — which
  is CORRECT for its actual job (comparing two captures of a WRAPPED terminal line, where
  whitespace is an artifact of the wrap) and wrong for anything a human reads. The
  discriminator: the board card auto-captured from the same send (PP-7) has the text intact,
  and the rescue presses Enter on what is already in the composer rather than retyping, so
  delivery cannot alter it. Storage unchanged — altering it would change the equality check
  and `is_amux_ghost` to fix a display problem — and the LINE now says what the preview is.
  This is the D1 terminal-scraping deviation showing through: a value computed for MACHINE
  COMPARISON was reused for HUMAN DISPLAY, where its lossy normalisation reads as
  corruption. Before quoting a scraped value in a log, ask what was normalised out of it.

---
## Every PR conflicts with every other, because the friction log is append-only and mandatory
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-20
SESSION: amux-errors-and-bugs
CARD: AEAB-40
SYMPTOM: `.claude/rules/frustrations.md` mandates an entry for any amux friction and says
  "Append at the bottom", so every branch doing real work ends by appending to the same last
  line of the same file. Two branches in flight is a guaranteed textual conflict. Hit three
  times today on PRs #132, #133 and #136.
COST: ~20 minutes of CI per occurrence, three times, because GitHub does not run PR
  workflows on a head it cannot merge — so the PR shows NO CHECKS AT ALL rather than a
  failure. "no checks reported" and "all checks passed" are one glance apart in
  `gh pr checks`; I nearly read the absence as green. All three branches were mine, so no
  peer was blocked this time, but a peer would have been.
FIX: Open, and it is a design call rather than a patch — carded as AEAB-40 and parked
  needs:you. NOT `merge=union` in .gitattributes: this repo's own history records union-
  merging this file splicing fragments of different entries together, leaving one entry
  carrying another's `FIX:` line, which silently corrupts the `grep '^STATUS: open'` counts
  the file exists for. A conflict that stops you beats a merge that lies. The candidate I
  would pick is one file per entry (`frustrations/YYYY-MM-DD-slug.md`), which makes the
  conflict structurally impossible, with the work being the greps in the rules, CLAUDE.md
  and `scripts/frustrations_audit.py`. Interim recipe, which worked three times today: take
  origin's file, append your entries VERBATIM, never let git interleave, then run the audit.
## The shared-checkout amend guard pins HEAD, not the staged set, so a correctly-pinned amend still absorbed a peer's work
AREA: git
SEVERITY: slows
STATUS: open
DATE: 2026-08-20
SESSION: amux-frustrations
CARD: AF-106
SYMPTOM: I ran `git commit --amend` to replace a placeholder commit message. The guard
  refused the unpinned form and told me exactly what to do:
    "BLOCKED ... git commit --amend without verified HEAD pin ... re-run pinned:
     AMUX_AMEND_EXPECT=<that-sha> git commit --amend"
  I did precisely that, with the sha I had just read off `git log -1`. It was allowed,
  and it swept 139 lines of another session's in-flight work into a commit carrying MY
  message: amux's AMUX-3110 dead-letter implementation (session_verbs.rs +132) plus
  their untracked migrations/0024_steering_dead_letter.sql, under
  "fix(instruments): /api/debug/downtime could not distinguish an empty history from a
  broken query (AF-99)".
  `--amend` with no pathspec commits the whole STAGED set, and a peer had staged theirs
  in the seconds between my two commands.
COST: ~20 minutes of disclosure, coordination and verification across two sessions, and a
  permanently mislabelled commit — amux chose to leave f70fc51 as-is and add a provenance
  note (3e77b20) rather than rewrite shared HEAD to fix a label. Cheap this time ONLY
  because the peer was reachable and answered in five minutes; their own reply names the
  real hazard, that they were about to conclude their work was uncommitted and re-commit
  it. The near-miss is a duplicated 132-line change, or a `git checkout` over it.
FIX: The guard verifies that the COMMIT BEING REWRITTEN is yours and says nothing about
  whether the CONTENT BEING ABSORBED is. Pinning AMUX_AMEND_EXPECT protected the wrong
  operand, and it protected it while telling me I was now safe — which is worse than no
  guard, because I stopped thinking about the staged set at exactly the moment it started
  mattering.
  Durable shape, and it needs no new machinery (amux's suggestion, and I agree): the
  amend path should warn — or refuse without an explicit ack — when the staged set
  contains paths whose last editor, by the staged-guard's OWN attribution, is another
  session. That is the identical ownership question the staged-guard already answers at
  commit time; this is the same predicate at a second door, which is AMUX-2325's lesson
  about a constraint whose sanctioned escape is unwalkable from the audited path.
  Cheap interim, entirely on the caller: `git commit --amend -- <your paths>`. A
  pathspec makes amend behave like the scoped commit the guard already pushes people
  toward everywhere else, and nothing in the guard's message mentions it.

---
## SIX answer-shaped wrong results in one night, and in every one the tell was a MISSING ACCOMPANIMENT rather than the answer
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-20
SESSION: amux-frustrations
CARD: AF-107
SYMPTOM: Six probes in one sweep returned something that LOOKED like an answer and was
  wrong. Recorded together because the count is the argument — any one of these reads as
  carelessness, and six in a night is a property of the surfaces, not of the day.

  1. `until [ "$(curl .../health | py 'print(d["build"])')" != "$OLD" ]` — the health call
     failed mid-restart, python raised, the expression was EMPTY, empty != old, and the
     loop exited printing "ADOPTED". I then measured a WARN storm against the old binary.
     Missing accompaniment: it never printed the hash it had supposedly adopted.
  2. `git diff --numstat origin/main...main -- <file>` labelled "what origin added that I
     lack". Three dots diff merge-base -> main, so those were MY changes with the label
     reversed. I nearly told amux their AMUX-3110 gate was still live. Missing
     accompaniment: `behind=0`, already on screen, said origin had nothing.
  3. Filtered `/api/logs` rows on `ts` inside an outage window and got zero — from a page
     that is newest-first and capped at 2000, every row of which post-dated the window.
     Missing accompaniment: no count of how many rows the page could even span.
  4. Read a schedule's `last_run_at`; the field is `last_run`. Three schedules reported
     `None` and I briefly believed a 12.6h outage had eaten the day's fires. Missing
     accompaniment: no key listing next to the value.
  5. Grepped `/api/debug/boundary` for a `families` key that does not exist; printed
     "families tracked: 0" against a live, correct response.
  6. Imported `git-shared-guard.py` to A/B its behaviour. It carried a module-level
     `sys.exit(main())`, so the import exits the importer with code 0. I wrapped it in
     `except SystemExit: pass` and moved on. amux hit the same line and their test suite
     printed NOTHING and exited 0 with every assertion unreached — the purest cannot-fail
     check either of us saw. Missing accompaniment: no PASS line, from a suite that
     "passed".
COST: no wrong conclusion shipped, because each was caught by a second look — but 4 of the
  6 had already produced a stated conclusion I was about to act on, and #2 was seconds from
  being sent to another session as fact. The real cost is that the catch was luck of
  habit, not of instrumentation: nothing in any of these surfaces made the wrongness
  visible.
FIX: The generalisation, sharpened by amux and worth more than the six specimens: every
  one produced an ANSWER-SHAPED result — an empty string, a reversed label, `ok:true`,
  `exit 0`, a plausible zero — and in NO case was the result itself the tell. The tell was
  always something ABSENT beside it: no PASS line, no adopted hash, no `ignored_fields`, no
  key listing, a diff that should have shrunk and did not.
  So the precondition that actually works is not "be careful" and not "check the result".
  It is: BEFORE believing a probe, name what should appear ALONGSIDE the answer if the
  probe really ran, and check for THAT. A count next to a zero. A hash next to "adopted".
  A PASS line next to a green suite. A key listing next to a None.
  ethos rule 7 already carries this family (the silent probe, the loud-wrong probe, the
  empty grep). What it does not yet carry is the accompaniment test, which is the cheap
  mechanical version, and this entry exists so the SIXFOLD count is somewhere countable
  rather than spread across six cards nobody joins up.
  Two of the six are amux defects with their own fixes: the module-level `sys.exit`
  (now __name__-gated) and `/api/browser/start` silently accepting unknown fields
  (AMUX-3403). The other four are surfaces that make the mistake easy — a capped
  newest-first page with no upper bound, and field names that differ by a suffix — and
  none of them can currently tell a caller they were misread.

---
## `GET /api/board/contract` advertises a `verified` gate the board does not enforce, and the refusal points you back at it
AREA: gates
SEVERITY: slows
STATUS: open
DATE: 2026-08-20
SESSION: amux-frustrations
CARD: AF-112
SYMPTOM: Moving three re-verified investigation cards to `verified`, acking exactly what the
  contract endpoint advertises, all three refused:
    GET /api/board/contract -> investigation.verified == ["Outcome confirmed to still hold"]
    409 body                -> gate == ["Functionality change is live and exercised, not just
                                        merged", "Peer-reviewed by a DIFFERENT worker in group
                                        `amux` (name them)", "That peer verified it themselves
                                        rather than taking the author's word", "No regression in
                                        what it touched"]
  Control, so this is not a nesting difference: the string "Peer-reviewed by a DIFFERENT
  worker" appears ZERO times anywhere in the contract response.
  The same mismatch holds for doc / ops / chore / research / escalation, which the contract
  all report as the single "Outcome confirmed to still hold".
COST: three refused transitions and a round trip to learn the real gate. Small in minutes.
  The part worth the entry is WHERE it sends you: the 409's own `how_to_ack.contract` field
  names `GET /api/board/contract` as the place to learn the gate, so the sanctioned
  instruction points at the source that is wrong. An agent following it correctly is
  refused — AMUX-2325's shape, recoverable only because the refusal happens to print the
  real gate.
FIX: Derive both from ONE table. A view must share the predicate of the mechanism it
  describes, and here the view is the mechanism's own documentation.
  Note which direction the drift runs, because it is the dangerous one: the contract
  advertises a LOWER bar than the gate enforces. The real gate requires peer verification
  by a different worker who checked it themselves — Ethan's standing rule, encoded. An
  agent reading only the contract would conclude a card can be self-verified on a re-check,
  which is precisely the weaker practice the gate exists to prevent. A stale doc that
  under-states a constraint teaches the wrong habit to everyone who never trips the gate.
  Not fixed here: which of the two is authoritative is amux's call, not a guess of mine.

---
STATUS: fixed
DATE: 2026-08-20
SESSION: amux
AREA: instruments
CARD: AMUX-3427
SYMPTOM: The first live Reconnect through the connectors OAuth broker dead-ended at
  Google's own wall: `Error 400: redirect_uri_mismatch`, because the flow issued
  `https://localhost:8824/api/connectors/google/callback`, a URI registered on no
  OAuth client. Google blocks BEFORE redirecting, so amux never receives anything:
  no callback hit, no log line, no autofix signal. The only witness was Ethan's
  screenshot. Meanwhile the same client already had a working registered URI
  (`/api/gmail/callback`, proven by every working Gmail token), and the broker
  ignored it in favor of a new one that needed a manual console step.
COST: Ethan's Reconnect click wasted, a screenshot round-trip, and the whole
  "one click repairs the account" promise broken on its first real use. The
  failure mode is invisible to amux by construction.
FIX: Reuse what is proven instead of demanding new registration: google-family
  grants now hand Google the gmail redirect URI (`google_redirect_uri()` =
  `gmail_redirect_uri()`, one `GMAIL_REDIRECT_URI` override knob for both), and
  the gmail callback recognises connectors-minted states and delegates the
  exchange back to the broker. The unobservable-wall class is surfaced at the
  only moment amux CAN see it: begin_auth now WARNs and returns
  `redirect_uri_registered:false` when no working gmail token proves the URI is
  registered. Regression tests pin the URI and the delegation.

---
STATUS: fixed
DATE: 2026-08-20
SESSION: amux
AREA: instruments
CARD: AMUX-3427
SYMPTOM: The fix for the entry above shipped its own rule-7 violation, caught by
  Ethan's SECOND redirect_uri_mismatch screenshot within the hour. The broker's new
  `redirect_uri_registered` signal inferred registration from token PRESENCE:
  "working gmail tokens exist, therefore the gmail redirect URI is registered." It
  answered true while the live console held only `http://localhost:8822/...` (the
  retired port): the tokens were minted before the port move and prove only what
  was registered THEN. An instrument that could not fail: any deployment with old
  tokens reads as registered forever, regardless of the console.
COST: A second wasted Reconnect click and a second error screenshot from the owner,
  plus a confident wrong "no console step needed, it will work this time" shipped in
  a reply. The probe that settles it costs one curl and no interaction: Google's
  authorize endpoint renders the mismatch page instantly, so the question was
  answerable the whole time.
FIX: Replaced the inference with the real instrument: begin_auth now GETs the exact
  authorize URL it is about to hand out and greps the response for
  redirect_uri_mismatch. false carries `fix_if_unregistered` naming the exact URI
  and the client's console deep-link; probe-unreachable is null, never a guess.
  Test scripts both pages so the discriminator is pinned in both directions.

---
STATUS: fixed
DATE: 2026-08-20
SESSION: amux
AREA: instruments
CARD: AMUX-3427
SYMPTOM: Third strike on the same flow, one layer down. With the redirect URI
  finally registered, Google redirected the consent tab to the registered
  http://localhost:8824 callback carrying a valid code+state. The
  RedirectingAcceptor answered every plain-HTTP request with a 301 into https,
  where the self-signed CN=amux cert threw an interstitial, so the tab stranded
  with the grant one hop from landing. Nothing logged: the 301 leaves no trace
  and the https leg never arrived. The grant only completed because Ethan pasted
  the stranded URL from his address bar and I curl'd it by hand.
COST: A third stalled approval for the owner on a flow promised as one-click,
  and a failure mode with zero log footprint: an unwatched user would retry,
  consume nothing (state stays pending), and give up with amux reporting nothing.
FIX: OAuth callback paths are now SERVED on the plain-HTTP leg: the acceptor
  recognises /api/gmail/callback and /api/connectors/*/callback (dot-segment
  guarded), replays them against the local TLS listener on loopback, and returns
  the callback's own HTML, so the consent tab completes without crossing the
  interstitial. Served-in-place is INFO-logged, proxy failure WARNs and falls
  back to the old 301 so the flow is never worse than before. Everything else
  keeps the https redirect.
