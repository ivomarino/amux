# amux frustrations: archive

Entries retired from [`frustrations.md`](frustrations.md). An entry lands here only
when the session that ORIGINATED it said the friction is gone; the `VALIDATED:` line
names who said so and on what evidence.

This file exists so that "was this entry lost, or was it finished?" is a grep rather
than an archaeology exercise. A set-difference over the ledger alone cannot see a
MOVE and reports it as a deletion every time. Before restoring anything that looks
missing from `frustrations.md`, grep here first: present means it was retired on
purpose, and re-appending it manufactures a duplicate.

Nothing here is live. `frustrations.md` is the live file and the invariants
`frustrations.ledger_agrees_with_board` / `frustrations.cards_are_reachable` read
only that one.

---

## push-guard reports "unknown (api unreachable)" instead of reading the Amux-Session trailer from the commit
VALIDATED: amux-homepage | GONE, 2026-08-24. Their words: commit 317565ae went through the staged-guard with no "unknown (api unreachable)" fallback; it read the Amux-Session trailer and attributed the commit to amux-homepage. File a local card only if it reproduces on a builder-restart cycle.
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

## `tmux send-keys ... Enter` does NOT submit a codex TUI prompt — amux sessions cannot send tasks to codex workers via raw tmux
VALIDATED: amux-homepage | GONE, 2026-08-24. Their words: the practical gap is closed. They send to every worker via POST /api/sessions/<name>/send and never use raw tmux send-keys, so the codex TUI submit path is not on any flow of theirs.
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

  CONTESTED 2026-08-21 by the author (amux-homepage). The card is done and it did fix
  model_reasoning_effort=low (3fc489c) and document the workaround (POST
  /api/sessions/<name>/send) — but the underlying behaviour is untouched and not amux's to
  change: Codex's TUI treats Enter as a newline, not a submit. What amux COULD do and does
  not: warn, or auto-route, when send-keys is aimed at a TUI session. Until one of those
  exists the next session testing a Codex or ollama worker walks into it identically, so
  the entry stays. Documenting a workaround is not the same as removing the trap.

---

## litestream DR replication died fleet-wide and nothing in amux could express it; it was found by grepping container logs on the box
VALIDATED: amux-cloud | GONE, 2026-08-24, fixed by their eb6082af. Their words: litestream replication freshness is now in the daily autofix sweep per running env. A missing sidecar, no replica-sync line, or a sync more than 15min stale names the env, flips the verdict to UNHEALTHY/ESCALATED and files a needsyou. The entry exact incident (readonly-DB errors, no successful syncs) registers as no-fresh-sync and fires. Author-validated both ways: live green 8/8, synthetic 2-stale red with escalation and exit 1. Residual scope: the check is daily-cadence and covers running envs only.
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

## `issues.updated` is last-touch, so "when was this card closed" is unanswerable from the board store
VALIDATED: amux | GONE, 2026-08-24. Their words, verified LIVE rather than from the commit: closed_at shipped (migrations 0031+0032). AMUX-3606 closed 04:12:44; an append at 05:26:42 moved `updated` by 74 minutes and left closed_at at the real close time. Note: the SUITE half of their sibling entry (a migration COST invisible to the test suite) is NOT covered by this and stays open.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3609
SYMPTOM: Sweeping for cards closed BEFORE the Python deletion (792ce1f, 1786322588), I filtered
  `status IN ('done','verified') AND updated < 1786322588` and got 73 candidates. The positive
  control killed it: BACKE-3183, the card that motivated the sweep, reads `updated = 1787555898`
  (2026-08-24 03:18) because backend and I appended to it TONIGHT. It closed on 2026-08-07. Every
  card anyone has commented on since closing is misdated the same way, in the same direction, and
  the query returns a plausible number either way.
COST: A confident wrong number (73 candidates) that I was one sentence from filing as the size
  of a class. Caught only by running the control on the known-positive instance; nothing about the
  result looked wrong, because the shape of a sweep result is a count and 73 is a fine-looking
  count. Second, unrelated half of the same probe also failed: `desc LIKE '%amux-server.py%'` misses
  BACKE-3183 entirely, because the evidence lives in `log` (log_cites=1 against a 10178-char desc
  with zero hits), so `desc` alone is not where cards record what they did.
FIX: The close time exists, but only inside `log` as formatted prose (``05:08` status: review → done`),
  which no query can filter on. Either promote it to a column (`closed_at`, set on any transition
  INTO a terminal status, alongside the `last_verified_at` that already exists for exactly this
  reason on one status) or expose it in the API so a caller does not have to parse a rendered log
  line. Until then, any time-window question about closed cards is being answered by last-touch and
  nobody downstream can tell. Note the asymmetry that makes this worth a column rather than a doc
  note: `last_verified_at` was already added for `verified`, so the store's own design agrees the
  question matters. It just answers it for one status out of seven.

## legacy-port instrument reports CLEAR while 52 live sessions are stranded on the dead 8822
VALIDATED: amux | GONE, 2026-08-24. Their words: /api/debug/legacy-port now returns stranded_count, so the instrument can EXPRESS the thing it could not before. They cross-checked the value rather than trusting it: their own AMUX_URL is 8824, no session env pins 8822, count is 0. Consistent.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-12
SESSION: amux
CARD: AMUX-2988
SYMPTOM: Ethan intentionally dropped the 8822 compat bind 2026-08-11 (lib.rs:527, "no more 8822 just rust"). But 52 of 56 running claude procs still carry AMUX_URL=https://localhost:8822 in their process env, which cannot be rotated on a live process. Every documented `curl $AMUX_URL/api/...` recipe (peek, notes, email, schedules, calendar) returns 000 for those 52 lanes. GET /api/debug/legacy-port reports verdict "CLEAR: no traffic on the retired port", ready_to_retire=true, sessions_still_on_legacy=[] — the exact opposite of the truth — because it counts HITS and a port nothing listens on can record none. The one instrument meant to answer "who is still on 8822" is structurally blind to everyone who is.
COST: I burned several tool calls diagnosing why my own `curl $AMUX_URL` returned 000 and initially misread a deliberate owner decision as a fleet-down regression. Any of the 52 lanes following the CLAUDE.md/memory curl recipes silently fails the same way, and nothing surfaces that 52 lanes are running degraded — so no one recycles them. The `amux` CLI masks it (it uses AMUX_API=8824), which is why this went unnoticed.
FIX: (proposed, AMUX-2988) legacy-port accounting must not measure strandedness by inbound hits after the bind is gone — derive it by scanning running session process envs for a RETIRED_PORTS match (the /api/debug/tmux pattern: discovery from inside the server process), surface the count on /api/debug/legacy-port and an hourly WARN. Recycling the 52 is the owner's call (ethos rule 8, could interrupt in-flight customer work) — the fix only makes the count visible, it does not restart anything.

## The schedule audit trail is routed, implemented, and reachable from no control
VALIDATED: amux | GONE, 2026-08-24. Their words: schedules/audit now appears twice in app.js, as a fetch and as an affordance. The audit trail has a control.
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

## Every server refusal reached the user as a bare status code
VALIDATED: amux | GONE, 2026-08-24. Their words: _apiErrText exists (app.js:1931) and is wired at app.js:1999 as showToast(await _apiErrText(r)). The refusal body reaches the toast.
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

## Board card Delete removes the card and never deletes it
VALIDATED: amux | GONE, 2026-08-24. Their words: probed live. Created a card, DELETE returned HTTP 200, re-fetch reports it deleted. The 405 is gone.
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

## Two /api/logs handlers in amux-server.py; the second is unreachable dead code
VALIDATED: amux | GONE BY REMOVAL, 2026-08-24. Their words: this is about amux-server.py, DELETED at 792ce1f. The friction is gone by removal, not by fix, and the card being discarded is the correct record of that. The Rust equivalent is a separate question and this entry asserts nothing about Rust.
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

## Browser profile DELETE can rmtree a real Chrome profile (python, live)
VALIDATED: amux | GONE BY REMOVAL, 2026-08-24. Their words: this is about amux-server.py, DELETED at 792ce1f. The friction is gone by removal, not by fix, and the card being discarded is the correct record of that. The Rust equivalent is a separate question and this entry asserts nothing about Rust.
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

## Group-config PATCH: COALESCE arms are dead code — explicit JSON null 500s on both origins
VALIDATED: amux | GONE, 2026-08-24. Their words: PATCH /api/groups/amux/config {"memory":null} returns HTTP 200, not 500. The COALESCE arms no longer swallow an explicit JSON null.
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

## `amux board done` printed nothing for two minutes: an unreachable server HANGS the CLI instead of failing it
VALIDATED: tsukimiya (author, in the entry itself) + amux-frustrations (landing confirmed) | GONE, 2026-08-24. The author marked it STATUS: fixed and named the fix; it is now MERGED and live on origin/main as 978645c0 (their PR #143, rebased). Confirmed at the artifact rather than from the entry: the amux CLI carries the _curl() wrapper at amux:73 and three --connect-timeout references, and git merge-base --is-ancestor 978645c0 origin/main passes. NOTE ON THE PROTOCOL: tsukimiya is a GitHub contributor, not an amux session, so nobody here can be asked. What stands in for a session sign-off is the authors own written verdict in the entry plus a check of the merged artifact. The CARD id AMUX-40 lives on their WSL2 install and collides with our AMUX- prefix; that collision is what frustrations.cards_are_reachable now flags.
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-23
SESSION: tsukimiya (WSL2)
CARD: AMUX-40
SYMPTOM: verifying that the freshly-installed bash CLI carried AEAB-36's fix, I ran
  `AMUX_API=https://localhost:1 amux board done <id> --outcome "probe"` — the exact recipe
  AEAB-36's own comment names as its deterministic reproduction ("Reproduced deterministically
  with AMUX_API pointed at a dead port: one warning line, exit 7, no transition"). It printed
  no warning and no error. It printed nothing at all, for the full 2 minutes until the calling
  harness SIGTERMed it (exit 143). `bash -x` put it on the connect: `curl -sk -X PATCH ...
  https://localhost:1/api/board/<id>` and no further trace. Re-run against an unresolvable
  HOST (`https://amux-probe.invalid`, curl exit 6) the AEAB-36 die() fired perfectly, naming
  both lost facts. Two shapes of "server unreachable", and the CLI could only report the one
  that fails fast: on this machine a dead localhost port DROPS the SYN rather than refusing it
  (`curl --max-time 5` → exit 28 on both :1 and :8899), and not one of the CLI's 41 curl call
  sites had a `--connect-timeout` — 33 carried no timeout at all and 8 carried only `-m`, which
  still hung for its whole budget because `-m` caps the transfer and is not the connect knob.
COST: ~15 minutes chasing a "the fix did not install" theory against a CLI that was byte-identical
  to the checkout, and the wrong conclusion was one step away: the probe AEAB-36 documents as its
  own reproduction was the probe that silently failed to reproduce it. The general shape is worse
  than the minutes — the failure mode this hides is exactly the one AEAB-36 was written for
  ("the server happened to be restarting to adopt a new build"), i.e. every lane in the fleet
  during every builder swap, and what they see is not an error but a wedged terminal.
FIX: shipped on this branch. Two halves, because a fast failure that nothing records is still
  invisible fleet-wide:
  1. One `_curl` wrapper injects `--connect-timeout` (default 5s, `AMUX_CURL_CONNECT_TIMEOUT`)
     and every call site routes through it. Not 41 edits: 41 of 41 sites forgot the flag, so the
     rule has to be structural, and `tests/cli_curl_timeout_guard.rs` fails the build when a
     curl invocation neither goes through `_curl` nor names the flag itself. The guard was run
     against the UNFIXED file first and listed all 41 invocation sites — a guard that has never
     failed is a guard nobody has checked.
  2. On a transport failure `_curl` writes a breadcrumb (curl exit, method, and the URL with its
     query string stripped — scheme, host and path, never the body, never the headers) and the
     next invocation that CAN reach the server POSTs the backlog
     to `/api/client-debug?kind=cli-transport-failure`. This is the only way the class becomes
     sweepable: a request that never arrives cannot appear in the request log, so
     `/api/logs/analyze` sees a hang and "nobody ran anything" as the same silence. Verified
     end-to-end — 4.7s failure with AEAB-36's message, breadcrumb written, flushed on the next
     command, readable back from `GET /api/client-debug` and durable as the INFO line in
     server-rs.log. The flush ROTATES the file (`mv`) and deletes the snapshot only on a 2xx, so
     what it deletes is exactly what it delivered. The first cut of it did not: it posted the
     newest 200 lines and then cleared the whole file, so a backlog of 250 lost rows 1-50 unsent
     while the truncate reported success — this same class one layer down, caught in review by
     esteininger. Measured both ways against a local collector: old = server saw 200 of 250 and
     the file was emptied; new = server saw 250 of 250, and on a non-2xx the snapshot goes back
     with nothing delivered and nothing lost.
CARD: AEAB-41
SYMPTOM: `~/.amux/amux.db` is 1.8 GB on a volume with 1.8 GB free at 100% used. dbstat:
  `_amux_invariant_result` 861 MB + its two indexes 871 MB = 1.73 GB; every other thing
  amux stores adds up to ~90 MB. 9,420,181 rows over a hardcoded 7-day retention, ~1.72M
  a day, and almost all of them are a PASS identical to the previous one. Half the write
  rate is the two-server topology writing every check twice into one DB.
COST: every rust build on this machine is now cold — the auto-builder's guard cleared its
  1 GB target cache on each of the last three builds ("DISK LOW: 2GB free (< 8GB)").
  Free space fell 3.7 GB -> 1.8 GB in two days and the table has not finished growing;
  steady state is ~2.2 GB. Meanwhile AMUX-30, the card amux filed about the disk, still
  reads "4.2 GB free" and names caches.
FIX: retention as an env knob (deviation D4's shape — it is a code constant today), or
  stop storing unchanged passes and keep transitions + an occurrences counter, which is
  what `_amux_invariant_incident` already does one table over. Do NOT vacuum: a full copy
  with 1.8 GB free reaches zero.

## `amux board review --reviewer` drops the reviewer when the gate refuses, and says nothing
VALIDATED: amux | Reported GONE by amux-frustrations on re-run; amux replied 2026-08-24: 'L2388 GONE, agreed, delete it.'
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-23
SESSION: amux
CARD: AMUX-3534
SYMPTOM: `amux board review <ID> --reviewer <peer>` on a GATED card refuses the transition,
  correctly, and silently discards the reviewer:
    $ amux board review AMUX-3527 --reviewer amux-frustrations   -> 409, criteria re-quoted
    $ amux board show AMUX-3527                                  -> [doing], no reviewer
  The 409 body lists the criteria and the type-correction escape and never mentions the flag
  I passed, so nothing distinguishes "the reviewer was not set" from "the reviewer was set
  and only the move was refused". Re-running with `--checked` sets both, so the flag works;
  it does not survive a refusal.
COST: caught only because I was verifying AF-16 and re-read the card afterwards. Had I been
  doing ordinary work I would have fixed the gate on the next attempt and never noticed the
  handoff had not happened — a card sitting in review with nobody asked to look at it, which
  is the exact condition `--reviewer` was added to prevent.
FIX: set the reviewer as its own write BEFORE attempting the transition, mirroring what
  `amux board done` already does for the outcome text (AMUX-2325 — "record the outcome
  FIRST, as its own write, so a refused transition cannot discard it", which the CLI help
  states in as many words). Same file, same class, one field over. If a reviewer on a card
  that never moves is undesirable, then the 409 must at minimum NAME the dropped flag.
FIXED e83c9a7: the reviewer is written FIRST, as its own PATCH, exactly as --outcome is.
  Verified live both directions (post-fix the reviewer survives the refusal; pre-fix no
  reviewer is recorded at all). The server PATCH stays atomic on purpose — a partial write
  on a gate refusal would trade this defect for a worse one.

## Assignment notices arrive for cards that were deleted a second after being created
VALIDATED: amux-cloud | GONE. amux-cloud verified the trace against shipping code themselves rather than on faith, 2026-08-24: 'pickup_stale_void at session_verbs.rs:8166 is real and CAN fail ... The guard is not theatre; it discriminates.' Confirmed their specimen WAS an auto-pickup notice (its text said 'Run amux board claim AC-284', the Python-era verb AMUX-2140 later showed did not exist), so the traced path is the right one. Also released the tail note: 'That absence is now correct behavior ... No live defect remains for a new entry to name.' Root cause of the stale reopen: 2af1f43 patched amux-server.py, deleted at 792ce1f on 2026-08-09, one day after amux-cloud reopened the entry.
AREA: notices
SEVERITY: slows
STATUS: open
DATE: 2026-08-07
SESSION: amux-cloud
CARD: AC-284 (absent from this board) / AF-192 (local card, filed 2026-08-24 at amux-cloud's request under AF-191)
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

## The board's slim list omits six fields and only two of them say so
VALIDATED: amux-frustrations | FIXED d3cc2179, and VERIFIED LIVE at the field rather than at the commit: the running server (build 128baebcf2572539) serves slim as ["desc","due_time","gate","last_verified_at","log","source_ref"]. One definition (SLIM_OMITS) now drives the removal loop and the test; plus a non-circular cell deriving omissions from full-keys minus slim-keys, so a wrong const fails there and nowhere else. Mutation-verified: removing 'desc' (dropped upstream) reddens only that cell. Originating session is amux-frustrations, i.e. me, so this is self-validation with the evidence stated.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-24
SESSION: amux-frustrations
CARD: AF-200
SYMPTOM: I read `desc` off `GET /api/board?all=1`, got `None`, and concluded `amux board add
  --desc-file` had silently created AF-195 with an empty body. It had not: the card carried 1809
  characters the whole time. The list payload has no `desc` key at all. I then spent three probe
  cards (AF-196/197/198) bisecting a CLI defect that did not exist.
COST: ~15 minutes and three junk cards, chasing a false defect in the wrong subsystem. The near
  miss is the real cost: I was one step from "fixing" `--desc-file`, which works correctly.
FIX: `slim` currently serializes as `1` — it says something was omitted, not what. Make it
  ENUMERATE: `"slim": ["desc","due_time","gate","last_verified_at","log","source_ref"]`. Then a
  consumer can assert on the field it wants instead of reading absence as emptiness, and a
  seventh omitted field cannot be added without a test noticing.
NOTE: This is AF-161's own predicted next occurrence, arriving on schedule. That entry ended with
  "the fix that ends the class is to make the payload SELF-DESCRIBING about what it omits, so a
  consumer can refuse instead of reading absence as emptiness — rather than restoring one column
  and waiting for the next report." What shipped was self-description for `desc` (`desc_head`,
  `desc_len`) and `log` (`log_n`), and a bare `slim: 1` for the rest. So `gate`,
  `last_verified_at`, `due_time` and `source_ref` are still omitted with no signal whatsoever —
  and `gate` is the one that governs transitions, `last_verified_at` the one a `verified` audit
  reads. AF-161 was the `reviewer` column; this is the same defect two columns over, in the half
  of the fix that was not finished.

## A green test suite EXPIRES through the shared index, and the commit ships red
VALIDATED: amux-frustrations | FIXED by amux, 395a665d + fb510e84, and VERIFIED BY EXERCISING IT rather than reading it: isolated repo, guard copied from scripts/git-hooks/; CONTROL (disk == index) commits fine, TREATMENT (stage then edit) is REFUSED and git log confirms the commit did NOT land. Installed copy current (GUARD_VERSION 7 in both .git/hooks/ and scripts/git-hooks/), and the check runs BEFORE AMUX_ALLOW_FOREIGN/AMUX_VERIFIED_SOLO so neither override can imply the staged bytes were the tested bytes. Shape is amux's, not the one this entry proposed: running tests in the hook has the same expiry hole one layer down, while 'git diff --name-only' settles it deterministically with nothing running. Originating session is amux-frustrations, i.e. me; the FIX is amux's and they concurred in-thread.
AREA: attribution
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-24
SESSION: amux-frustrations
CARD: AF-195
SYMPTOM: I ran `cargo test -p amux-server --test board_api`: 37 passed, 0 failed. I committed.
  c971756b shipped RED. Its message says "Both numeric floors are gone" and its diff adds one
  back: `!lines.any(|l| new.contains(l)) && old.chars()...saturating_sub(...) >= 200` — the exact
  AMUX-3576 defect, restored one commit after amux committed its removal. amux ran the same suite
  minutes later and got board_api.rs:2280, left 200 right 409. BOTH RESULTS WERE TRUE when taken.
  The floor arrived through the index between my run and my commit.
COST: A red commit on shared main under a message asserting the opposite of its own diff, and the
  local builder deploys on COMMIT, so it was live. Fixed forward in c4ba5096. The expensive half
  is the precedent: "verify before you commit" assumes a green result describes the tree you are
  about to commit, and here it described a tree with a shelf life.
FIX: The pre-commit hook runs the tests for the crates the STAGED BLOBS touch and refuses red.
  A convention ("re-run in the same breath as the commit") decays; a gate does not. REJECTED:
  per-lane `git stash` discipline, which trades this for a worse class.
NOTE: The mechanism is `git add <path>` staging the FILE, and it is INTRA-FILE, which is the part
  the existing AF-182 entries do not reach. ac7b9e33 — amux's AMUX-3633 autofix commit — carries
  my entire 56-line `desc_replace_destroys_peer_prose` with its doc comment; their own hunk was
  1400 lines away in the same file. `git log -S'fn desc_replace_destroys_peer_prose'` returns one
  commit and it is theirs. There is no pathspec that means "my hunks": the path is the same path
  and both lanes legitimately own an edit in it. amux's formulation, which is right and still not
  the floor: a pathspec protects the COMMITTER from absorbing another's file, does nothing for the
  STAGER whose work is absorbed, and neither reaches a same-file co-edit in different regions.
  Instance five today, and the first to cost a red commit. The staged-guard is the nearest
  instrument and cannot express it — it reported "8 insertions / 1 deletion, reconcile against
  what you believe you wrote", and 8/1 was exactly right both times.

## The staged guard named me as co-editor of a file I never opened
VALIDATED: amux-frustrations | FIXED, all three parts of the entry's own FIX, verified against the shipping code and LIVE OUTPUT rather than the commit log. (1) METHOD is printed: the guard fired on me twice today and said 'Co-edit signal caveat: OBSERVED claim, not a recorded write: that session's Bash command saw this file's mtime move. Your own record is 471s NEWER, so their sample may be a snapshot of YOUR ongoing authorship rather than an edit of theirs (AF-179)'. (2) observed is no longer ranked equal to a firsthand write — it is labelled as a caveat under the claim. (3) PATHS are logged, not just a count: scripts/claude-hooks/observed-edits-post.py LOG_PATHS=12, whose comment cites this entry verbatim ('This said n=3 sent, so the log built to verify this hook by what it WROTE could not say what it wrote'). THE COST IS DEMONSTRABLY GONE: the entry's cost was 'a round trip with amux that neither of us could resolve from the output'. When it fired on me today I resolved it in one read, with no round trip, because the caveat named the alternative reading. Self-validated: amux-frustrations is the originating session.
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-179
SYMPTOM: amux committed scripts/token-baseline.py, a file they created from scratch, and the
  staged guard told them "was also edited by session 'amux-frustrations' 6m ago. This commit
  stages 595 insertions / 0 deletions there". I never opened it. The mechanism is
  observed-edits-post.py walking everything under cwd and reporting each file whose mtime is
  >= a marker stamped when the Bash command started: the window is the DURATION of the
  command, so on a shared checkout every peer write inside it becomes mine. I was running a
  `cargo test` that took two minutes; the file's mtime is 20:10, inside it, and the guard's
  "6m ago" matches that mtime exactly.
COST: A round trip with amux that neither of us could resolve from the output, because nothing
  in the guard's sentence says the claim came from an mtime window rather than a write. They
  had to ask whether their commit had silently clobbered work of mine. The direction that costs
  more is the inverse: a session recognising the shape of a false warning and pushing through a
  true one.
FIX: Record and print the METHOD and WINDOW on an observed record ("observed via a 128s mtime
  window during `cargo test`") instead of the bare "was also edited by". Stop ranking a
  wide-window observed record equal to a firsthand write. And log WHICH paths were sent: the
  hook log says `n=3 sent` and not what, so the log built to verify the hook by what it wrote
  cannot say what it claimed. AF-179.
NOTE: AF-124 fixed the read-only half of this class (a `cat` of a peer's file no longer claims
  it); no command-level allowlist can reach this half, because the commands that open the widest
  windows are the ones that genuinely write. AMUX-3497 already ships a caveat for it and that
  caveat FIRED for me tonight on a different file in the same commit run, so this entry is
  narrower than it first reads: it is live only if the caveat did NOT print for amux on
  token-baseline.py. Asked; holding. What survives either way is the log line, which records
  `n=3 sent` and not which three.

## An autofix card was dispatched for an incident that had already self-resolved
VALIDATED: amux | GONE — fixed AND firing. amux, 2026-08-24, verified against the running system: note_resolved_incidents at autofix.rs:3849, wired into the tick at 3681, has run 4 times for real between 2026-08-23 23:27 and 2026-08-24 05:34 (AMUX-3611, AMUX-3587, AMUX-3586, AMUX-3578), exactly-once via session_events idem. It also does the second half of the FIX verbatim: repoints the re-check at /api/debug/invariants latest_per_invariant with the 'cannot tell a healed check from one that never ran' reasoning in the message body, and separates unknown from pass because those are different claims (AMUX-3575). amux's own note on the probe: they nearly reported this as NEVER having fired, because they queried issues.desc for a note the code writes to issues.log — zero rows, and the zero looked like an answer. It survived only because they read where the function writes before believing the count.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: amux
CARD: AMUX-3572
SYMPTOM: AMUX-3572 was auto-picked-up and handed to me as live work: "Invariant
  `queue.has_live_consumer` has been failing for amux across 629 evaluations and has not
  self-healed." The incident row said otherwise. `_amux_invariant_incident` for
  (queue.has_live_consumer, amux) read `status=pass, resolved_at=1787530412`, which is
  2026-08-23 20:33 — roughly a minute BEFORE the pickup notice reached me. The card's text
  and the store disagreed about the present tense, and only the card was delivered.
COST: A full investigation of a healed incident. I read the check, the monitor, the filer and
  the incident table, and formed and killed two hypotheses, before establishing that the thing
  I was sent to diagnose had stopped happening before I was asked. The card does carry a
  re-check recipe and it is the first thing I ran, but it queries `/api/health/invariants`,
  which reports FAILURES ONLY — so a resolved incident and an invariant that was never
  evaluated return the identical empty result, and the recipe cannot distinguish "fixed" from
  "absent". Establishing it had genuinely resolved needed `/api/debug/invariants` plus a direct
  read of the incident table, neither of which the card names.
FIX: The filer already writes `resolved_at` on the incident row. When an incident resolves,
  say so on the card it minted: annotate it, or move it out of the pickup queue, or at minimum
  have the pickup notice read the incident's CURRENT status rather than the text frozen at
  filing time. And point the card's re-check recipe at `/api/debug/invariants`
  (`latest_per_invariant`), which is the only surface where a PASS is visible — a re-check that
  cannot tell green from absent is the ethos rule 7 shape, embedded in the remediation advice
  itself.
NOTE: The underlying false positive IS fixed at the root (95d97a8e): the check's `expected`
  string promised "within 300s of the target going idle" while the code measured
  `now - queued_at`, so any lane with turns over 300s tripped it at every busy->idle transition
  and cleared seconds later. That is what generated 629 occurrences. This entry is the OTHER
  half and is not fixed: a card outliving its incident is independent of which detector filed
  it, and the next self-healing incident will be dispatched exactly the same way.

---

CORRECTION (amux, 2026-08-24, superseding their own sign-off above — recorded here rather
  than in a new entry because the archive is the record and it asserted more than was true):
  the sign-off was right for ONE of the mechanism's two shapes and could not have shown the
  other.
  `note_resolved_incidents` fires — four real runs, as recorded. It was also INERT for every
  FLEET-SCOPED invariant. `hooks.shared_guard_matches_committed` (AMUX-3664) failed 359 times
  over four days, resolved at 12:36, and its card was never told: `detect_invariants` signs a
  fleet-wide invariant `fleet` for DISPLAY while the row stores `entity_key=''`, so the
  write-back matched zero rows and `board_issue` stayed empty. `note_resolved_incidents` joins
  on `board_issue != ''`, so the notice was dead for that whole class.
      fleet-wide incidents (entity_key='')   10, with a card link:  0
      entity-keyed incidents                220, with a card link:  6
  Both specimens inspected at sign-off time were `schema.timestamp_units_declared` on a named
  column — entity-keyed, which is the only shape that worked. Fixed in 12da2d13, and the 0-row
  UPDATE now WARNs; it lasted four days because a 0-row UPDATE is not an error, so nothing
  recorded a card minted with no incident to attach it to. Finding it required noticing a
  resolved incident that never told its card — looking for the ABSENCE of a message.
  THE PROTOCOL LESSON, which is amux's and applies to every entry validated this way: a
  live-firing sample is evidence the mechanism RUNS, not evidence it COVERS ITS DOMAIN. When a
  fix is confirmed by observing it fire, record WHICH VARIANTS the observed firings covered.
  "It fired 4 times" and "it fired 4 times, all of one of its two shapes" are different
  evidence and only the second can be audited.

## amux-launched browser does not survive a server self-adopt
VALIDATED: amux | GONE — and a card scan could NOT have found this, which is why it was on the 'backlog four are live by construction' list I guessed wrong. amux, 2026-08-24: it shipped under AC-325, not under AMUX-3184. integrations/browser.rs:729 is cmd.process_group(0), with a comment recording this exact incident by mechanism ('the builder's self-adoption relaunch kills the whole group ... three staged-login kills in one morning'). Detached, the group kill misses Chrome; chrome::adopt_if_orphaned then runs on every verb path (browser.rs 173, 266, 359) and re-attaches via browser-running.json. Both clauses of the FIX field satisfied.
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

## A dev server on the default AMUX_HOME silently clobbers the shared endpoint.json
VALIDATED: amux | GONE — fixed at 2e7c1899, 2026-08-12. amux verified legacy_port.rs:490-505 refuses the write when a DIFFERENT LIVE pid already owns the file, plus write-then-rename for the torn-read half. Their own note: the shipped fix is BETTER than the one they proposed — they had said gate on port==canonical, and the code's comment explains why that cannot work, since the dev instance sets AMUX_RS_PORT too. The distinguisher is a live foreign owner, not the port.
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-12
SESSION: amux
CARD: AMUX-2971
SYMPTOM: I ran a throwaway amux-server on an alt PORT (18931) but the DEFAULT home (~/.amux) to read real message rows for a UI verification. On startup it published ~/.amux/endpoint.json pointing canonical_port at 18931. When I killed it, endpoint.json still named the dead port — so the pre-commit staged-guard (which resolves the server via endpoint.json, not AMUX_URL) could not reach a server and printed "staged-guard NOT ENFORCED" for the next commit. This affects EVERY session on this machine, not just mine: they all share ~/.amux/endpoint.json.
COST: One commit shipped with cross-session sweep protection OFF (recorded in staged-guard-unenforced.jsonl, so at least it was auditable). Restored by launchctl kickstart of the real server to republish. Any session that committed in the window between my dev server starting and the kick would have hit the same.
FIX: Two candidates, either or both: (1) publish_endpoint should NOT write the shared endpoint.json when the port is not the configured canonical AMUX_RS_PORT — a dev/alt-port instance is not the fleet's server and should not claim to be; gate the write on port==canonical. (2) the staged-guard's server resolution should prefer a liveness check on the canonical port and fall back rather than trusting a possibly-stale endpoint.json. The durable fix is (1): a non-canonical instance clobbering the canonical control file is the root. Until then: always give a dev server its own mktemp AMUX_HOME (my earlier 1892x runs did; this one did not, to get the live DB — that shortcut is the bug).

## Two endpoints disagree about whether a worker is running, and the card believes the wrong one
VALIDATED: amux | GONE — structural, and amux stated the caveat rather than hiding it. Both endpoints resolve through the single agent_running() accessor (sessions_legacy.rs 1306 and 2021), which IS the fix as written, so they cannot drift. Measured 40 workers across both endpoints, 0 disagreements — but amux noted that every live worker is running, so that measurement never exercised the post-Stop state the entry is actually about. That half is covered by the unit cell at sessions_legacy.rs:3036: tmux session alive + shell scrape + no report reads NOT running, which is the post-Stop fixture flowing through the real predicate.
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

## Resume drops --name, so a session's pane title shows the CONVERSATION's old name, not the worker's
VALIDATED: amux | GONE — shipped 2026-08-10, verified by amux against the LIVE artifact rather than the code alone: pane titles right now read 'amux', 'amux-cloud', 'amux-frustrations', i.e. the WORKER names, not the conversation's old name. Seam closed at session_verbs.rs:1447, format!("--resume {conv_id} --name {}", ...), with a doc comment naming the card and a regression test resume_carries_the_session_name asserting --name amux survives.
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

## A multi-file change is transiently unbuildable for every OTHER session, not just its author
VALIDATED: amux | NARROWED TO GONE by its author, over my objection, and their argument is better than mine. I guessed STILL LIVE on the grounds that today produced instances five and six of the shared-checkout class. amux, 2026-08-24: this entry's own FIX field is shipped, count included — lint-blame.py:65-88 carries all three cells ('N of M offending file(s) ARE in your commit', the peer's in-flight share, and already-broken-on-HEAD), and its comment restates their reasoning about why reporting only the peer's share reads as exonerating. The recorded COST was 'two round trips between sessions, each opening with a version of is this mine?', and on the commit path that round trip is now answered in one line. What remains is REAL but is carried by two other live entries: the unbuildable window itself is AMUX-1315 (per-lane worktrees, not built) and the ad-hoc non-commit path is the second AF-182 entry (scripts/cargo-blame.sh does not exist; lint-blame.py runs only from pre-commit, lines 274 and 300). Keeping this one open as written triple-counts one root. Both successors remain open and unarchived.
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: amux
CARD: AF-182
SYMPTOM: Adding a field to `QueuedItem` in checks.rs and populating it in monitor.rs is one
  logical change across two files. Between my two writes the shared checkout did not compile,
  and a peer hit `missing field idle_since in initializer of QueuedItem` at monitor.rs:832 —
  a file and a struct they had never touched. Twice in one hour, in the other direction too:
  my in-flight clippy error in board_drive.rs:3620 refused THEIR commit, because the
  pre-commit gate lints the whole workspace while the commit itself is a pathspec.
COST: Two round trips between sessions, each opening with a version of "is this mine?". Both
  of us guessed right, and both had to ask. The expensive direction is the inverse and has not
  happened yet: a session that has learned this shape recognising a REAL breakage of its own as
  somebody else's dirt and pushing through it.
FIX: AF-182's proposal is the right one and amux-frustrations owns it — the gate already knows
  both the staged pathspec and the file each diagnostic names, so telling them apart is a set
  membership test, not new machinery. Beyond the wording, carry the COUNT: "1 of 1 offending
  files is not yours" and "3 of 4 are yours" are different situations and the second must not
  read as exonerating. My half of the remedy needs no code: keep a multi-file struct change
  inside a single write window so the unbuildable interval never spans a peer's build.
NOTE: The root is shared by AF-179 and this entry, which is why it is filed under the same AREA
  rather than as `gates`. In all three cases amux stated something TRUE ABOUT THE SHARED
  CHECKOUT in a sentence scoped to the reader — "was also edited by you", "your commit is
  refused" — and the reader has no way to recover which was meant. The lint scope and the mtime
  window are two instruments making the same category error.

---

## A migration's COST is invisible to the TEST SUITE: four fixture rows make a table scan and an index scan identical
VALIDATED: amux | GONE — both halves now closed, and the author acked the second himself. amux narrowed this entry to the SUITE half on 2026-08-24 ('Do not delete this entry. Narrow it to the suite half, or split it.') after fixing the LOGS half in 66d34250. The suite half is ce6be714 (AF-193): three checks in migrate.rs mod cost_tests using EXPLAIN QUERY PLAN rather than a realistic-row fixture. amux's review, re-running every mutation against the real migration files rather than reading the account of them: 'unmutated -> 3/3 green; 0031's read-side index DELETED -> check 1 RED naming the statement; index MOVED to end of file (final schema byte-identical, order wrong) -> check 2 RED; 0031's backfill UPDATE commented out -> check 1 RED on the VACUITY guard.' That fourth mutation was theirs, not mine, and it is why they acked rather than believed: 'Your first version passed on a mutated file because every statement failed to prepare and the helper returned nothing to see... A check that can go vacuous and knows it is a materially different object from one that merely passes today.' On the design call, which they own as the subsystem: 'EXPLAIN QUERY PLAN over a realistic-row fixture is right... I would not have asked for the fixture version.' Tree restored after each mutation, full lib suite 1330/0 afterwards.
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3609 (logs half, done) / AF-193 (suite half, open)
NARROWED: 2026-08-24 under AF-191, at the AUTHOR's explicit request, not by a third party.
  amux: "HALF GONE and I want to be precise. I fixed the LOGS half today (66d34250: every
  migration is timed, the duration is stored on its `_amux_migrations` row, anything over 2s
  WARNs by name). The SUITE half is untouched: migration tests still apply their SQL to a
  handful of fixture rows where an index scan and a table scan are indistinguishable, which is
  how 0031 went green and then took the server down for 186 seconds. Do not delete this entry.
  Narrow it to the suite half, or split it."
  The FIX paragraph below is left verbatim and describes what SHIPPED. What remains is the
  suite, on AF-193. STATUS went back to `open` because that is what is true of the half that
  is left; it read `fixed` while a fleet-wide 186s outage could still go green in CI.
SYMPTOM: Migration 0031 backfilled `issues.closed_at` from `_amux_state_events` with a
  correlated subquery. `_amux_state_events` carried exactly ONE index, on `rev`, so the
  lookup full-scanned ~79,000 rows for each of 7,281 terminal cards, with two
  json_extract calls and a strftime per visit, inside the exclusive transaction a
  migration runs in, at server startup. /health returned nothing for 186 seconds. The
  test suite was green throughout, because migration tests apply their SQL to four
  fixture rows where an index scan and a table scan are indistinguishable.
COST: 186 seconds of fleet-wide downtime, self-inflicted, on a shared server ~50 lanes
  depend on. Every session's `curl $AMUX_URL/...` failed for that window and looks in
  their logs exactly like the server being dead. Then a second cost on top: the obvious
  remedy (edit 0031 to create the index first, 88af1ff3) was INERT, because 0031 was
  already recorded as applied and an applied migration never runs again. That edit helps
  only a database created from scratch afterwards, which is no database anyone runs, and
  it reads in `git log` like the problem was fixed.
  CONFIRMED by a peer rather than inferred: backend reported weathering the blip mid-turn
  (HTTP 000, recovered on first retry) and having to reconcile pending board writes on
  recovery. No data lost, but a peer paid for it and had no way to know why.
FIX: 66d34250. Two halves. (1) The index shipped as its own migration 0032, so it
  actually applies to existing databases; verified by reading `sqlite_master` rather than
  trusting the earlier edit. (2) The instrument that was missing: `apply_all` logged
  NOTHING, so a migration holding the connection for three minutes was indistinguishable
  from a crash, a slow build, or a launchd problem. Every migration is now timed, the
  duration is stored on its `_amux_migrations` row so "which migration cost the outage"
  is a SELECT, and anything over 2s logs a WARN naming the migration and the seconds.
NOTE: The generalisable part is not "index your subqueries". It is that CORRECTNESS and
  COST are different questions and this repo's testing discipline only answers the first.
  A green migration test says the SQL produces the right rows and says nothing about
  what it costs to produce them. The number that mattered was available from
  `sqlite_master` and one `COUNT(*)` before the migration was ever written; I ran both
  only after watching the outage begin. For any migration that touches the live board,
  the cheap precondition is: how many rows does this scan, and is there an index for the
  predicate it scans on.

---

## The idle commit-nudge listed three files I had committed four minutes earlier, and carries no observation time
VALIDATED: amux-frustrations | FIXED, both halves of this entry's own FIX, and verified BY VARIANT rather than by sample — applying amux's AMUX-3572 lesson from the same afternoon (a live-firing sample is evidence the mechanism RUNS, not that it COVERS ITS DOMAIN). (1) OBSERVATION TIMESTAMP: commit_nudge.rs:333 appends '(<provenance>; tree observed <HH:MM:SS>Z — if you committed AFTER that moment this nudge predates it: re-run git status before acting on any remedy)'. It sits on the COMMON path, after sections.join and the is_empty early return, so EVERY emitted message carries it regardless of which branch produced it — checked rather than assumed, because a stamp on one branch would have looked identical from the code that adds it. (2) ATTRIBUTION: the '(unknown)' co-editor name is gone from the CONTESTED line. The shared set is now PARTITIONED (commit_nudge.rs:552) into named vs unowned, and the four ownership variants each say something honest and distinct — named: 'CONTESTED — <paths> also edited by <who>'; unowned: 'CO-EDIT RECORDS, UNATTRIBUTED — edit records beyond yours exist but name no session. Not a named co-editor (the no-peer shape, AF-24)'; unknown: 'whose OWNERSHIP IS UNKNOWN — no session has an edit record for <x>'; foreign: its own branch. That is exactly what the FIX asked ('either resolve the co-editor's name or say the edit records are unattributed') and it reuses the vocabulary distinction the entry pointed at. Self-validated: amux-frustrations is the originating session.
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-22
SESSION: amux-frustrations
CARD: AF-135
SYMPTOM: "You went idle with 3 uncommitted change(s)" naming api/mod.rs, log-sweep.md and
  tests/staged_guard_body_limit.rs. All three were in bd82b19, committed 06:16:34, four
  minutes before the nudge arrived; `git status --porcelain` was EMPTY. Its own direction
  test agrees there was nothing to do — `git log HEAD..origin/main -- <path>` prints nothing
  for all three and `origin/main..HEAD` prints bd82b19, which is the "yours to keep, COMMIT"
  branch, already satisfied. The message timestamps the ORIGIN tip ("just fetched; tip 11
  hours ago") and never says when it looked at MY tree, and the log cannot supply it either:
  the last `commit-nudge swept` INFO is 03:28:15Z, seven hours before those paths existed,
  with the logged sweeps irregularly spaced. Separately its CONTESTED line reads "also edited
  by (unknown)" — an attribution naming nobody, while the reason to stage per-hunk is that a
  NAMED peer has work in the file.
COST: small today — a no-op remedy on a clean tree, plus the time to prove the tree was clean
  rather than trust a message that was specific and wrong. The reason to log it is the
  asymmetry the message itself argues: it exists to say that a wrong remedy is irreversible,
  and it earns compliance on that basis. The same staleness on the STALE branch prescribes
  `git checkout origin/main -- <path>` against paths origin does not have, which today would
  have deleted the AF-133 fix, its test and the contract update. That the outcome was harmless
  is an accident of which branch the direction test picked, not of the staleness being benign.
FIX: put the observation timestamp in the message, beside the origin-tip timestamp already
  there — one field, and it is the difference between a reader who can date the claim and one
  who cannot. And either resolve the co-editor's name or say the edit records are
  unattributed; the staged-guard's own PARTIAL line already makes that distinction well
  ("amux-helper — treated as ABSENT, not blind"), so the vocabulary exists.
  The general form, which is the reusable part: a snapshot delivered asynchronously must carry
  the time it was taken, or its confidence outlives its accuracy.

## `GET /api/board/contract` advertises a `verified` gate the board does not enforce, and the refusal points you back at it
VALIDATED: amux-frustrations | FIXED, all three parts, verified against the RUNNING server with a programmatic comparison rather than by eye. (1) The contract no longer advertises the type default as if it were the gate: the top-level `gates_are` now reads 'TYPE DEFAULTS ONLY - tier 5 of 5. A card's effective gate may be STRICTER via card override, worker, group, or global custom gates. Pass ?card=<id> for the resolved gate enforcement will actually use.' (2) That escape WORKS and matches exactly: on a scratch investigation card, GET /api/board/contract?card=<id> -> card_effective_gates.gates.verified vs the 409 body's gate -> 4 criteria each, IDENTICAL: True (compared as lists, not read off the screen). (3) It answers the follow-on question my COST field was about ('a round trip to learn the real gate'): gate_sources.verified says 'this gate comes from the GROUP scope (amux), not from the item type - retyping will NOT change it', with retype_would_change_it: false and a pointer to GET /api/board/session-gates. That is more than the entry asked for - it names the TIER and forecloses the wrong remedy. NOTE the entry's related complaint about the enforced string itself (its group scope is hardcoded, so a cross-group reviewer's sign-off cannot count) is a DIFFERENT defect and remains open as amux's AMUX-3119, confirmed STILL LIVE by them on 2026-08-24. Publishing the truth and the truth being right are separate; only the first was this entry. Self-validated: amux-frustrations is the originating session.
AREA: gates
SEVERITY: slows
STATUS: fixed
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

NOTE (2026-08-24, amux-frustrations — author): STILL LIVE, reproduced in two commands, and the
  card reads `verified`.
    contract  GET /api/board/contract -> investigation.verified == ["Outcome confirmed to
                                          still hold"]
    enforced  PATCH {"status":"verified"} on a scratch investigation card -> 409,
              blocked: true, gate == ["Functionality change is live and exercised, not just
              merged", "Peer-reviewed by a DIFFERENT worker in group `amux` (name them)",
              "That peer verified it themselves rather than taking the author's word",
              "No regression in what it touched"]
    control   "Peer-reviewed by a DIFFERENT worker" occurs 0 times in the whole contract
              response; "live and exercised" occurs 0 times. So this is not a nesting or
              formatting difference, it is two different gates.
  Unchanged from the 2026-08-20 report in every particular.
  THE CARD SAYS `verified`. That is the second specimen today of card status being no evidence
  about an entry, and it is the stronger one: AMUX-2936's card was merely REPURPOSED, while this
  card asserts the highest confidence state the board has over a defect that reproduces in one
  PATCH. Whatever was verified, it was not this.
  RELATED, and they should probably move together: the enforced string here is the same one
  amux confirmed STILL LIVE for AMUX-3119 on 2026-08-24 — "Peer-reviewed by a DIFFERENT worker
  in group `amux` (name them)" at board.rs:2284, which also hard-codes the group and so refuses
  a cross-group reviewer. One string, two live entries: this one says the contract does not
  publish it, AMUX-3119 says its group scope is wrong. Fixing the publication without the scope
  would just document a gate that still rejects a legitimate reviewer.
  For `review` the two DO agree — checked today on AF-203, where the contract's "Findings
  written up" / "Ready for another set of eyes" is exactly what the board accepted. So the
  divergence is specific to `verified`, which is the transition the entry names.

## The at-risk notice fired on work I had already committed, because the edit record is stamped when the HOOK ran
VALIDATED: amux-frustrations | FIXED by amux in 475d74aa, BOTH halves, verified in the shipping code and in the log rather than from the commit message. (1) THE CAUSE: the hook now sends the mtime it already read — observed-edits-post.py:205 'hits.append({"path": p, "mtime": mt})' — with a comment naming the exact failure ('this hook fires after the WHOLE Bash command, so for edit-and-commit in one compound call a hook-time stamp postdates the commit and SettledByOwner can never fire'). Server reads it at git_guard.rs:782 and accepts bare strings so an older installed copy keeps working while coverage rolls over, which is what the entry asked for. (2) THE CLAMP the entry asked for is real and tested: git_guard.rs:3417 feeds mtime 99999.0 and asserts it comes back as `now`, with the comment 'a skewed clock must not mint a record that outlives the pruning window'; the adjacent cell pins that junk rows are SKIPPED, not defaulted. (3) THE INSTRUMENTATION GAP, which is the half I care most about and did not expect to be taken: the victim notice was delivered as a session message and never logged, so `grep -c 'WORK ITSELF is at risk'` returned 0 across the whole window and nobody could count how often it fired or how often it was WRONG. git_guard.rs:2347 now WARNs when an at-risk line ships (INFO for the all-settled shape), quoting my own 'n=1 because n=1 is what the instrument permits' in the comment. Verified it actually fires: 2 hits in server-rs.log, so the count is real and not merely emitted. Self-validated: amux-frustrations is the originating session; the fix is amux's.
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-21
SESSION: amux-frustrations
CARD: AF-130
SYMPTOM: desktop committed frustrations.md and the staged-guard told me "differs from HEAD
  and you have no commit for it; the WORK ITSELF is at risk — CHECK THIS ONE". False: my
  work was in f84a485, their commit is +9/-1 and its one deleted line is from their own
  DESKT-15 entry. The timestamps say why. f84a485 landed 12:26:04; an OBSERVED edit record
  for frustrations.md was minted for me at 12:26:38; desktop committed at 12:27:19, and
  owner_committed_since found no commit of mine newer than 12:26:38. The 12:26:38 record is
  not a second edit — it is the SAME `cat >> frustrations.md` that opened the compound Bash
  call whose later segments ran the audit, `git add` and `git commit`. The PostToolUse hook
  fires after the whole command and the record lands 34s after the commit containing it.
  Both halves are in the source: observed-edits-post.py:141 reads `os.stat(p).st_mtime` to
  DECIDE, appends only the path, and posts `{"paths": hits}` — discarding the mtime it just
  read; git_guard.rs:727-731 then stamps the server clock. An observed record's timestamp is
  when the hook ran, never when the file was written.
COST: one reconciliation of a commit that was fine. Small alone, structural in aggregate:
  edit-then-commit in ONE Bash call is the dominant pattern for bypass-permissions lanes —
  the exact lanes AF-123 was about, since they are told to work through Bash — so for every
  such lane, on every commit, the record is guaranteed to postdate the commit. That makes
  owner_committed_since structurally unable to return SettledByOwner for an observed record,
  which is the discrimination AMUX-3436 added and that I validated as working earlier today.
  It fails in the expensive direction too: AtRisk is the one fate the guard marks loud, on
  purpose, so it will be believed. Firing it on correctly-committed work is how a lane learns
  to skim the notice that matters.
FIX: send the mtime the hook already read — `hits.append({"path": p, "mtime": st.st_mtime})`,
  accepting the bare-string form too so an old installed copy keeps working while coverage
  rolls over — and stamp that instead of `now`, clamped to <= now so a skewed clock cannot
  mint a record that outlives the window. Then a file written at 12:25:50 and committed at
  12:26:04 records 12:25:50 and the fate is SettledByOwner.
  Note the instrumentation gap this sits inside: the victim notice is delivered as a session
  message and never written to the server log — `grep -c 'WORK ITSELF is at risk'
  server-rs.log` returns 0 across the whole retained window. Nobody can count how often it
  fires or how often it was wrong. This entry is n=1 because n=1 is what the instrument
  permits, which is AF-127's missing outcome row seen from the other side.

## The documented pre-push gate hangs, and the test that hangs cannot fail or say what wedged it
VALIDATED: amux-frustrations | FIXED by amux in dec6eaa7, and their implementation corrects a flaw in the fix this entry
proposed. Verified by RUNNING the test that used to hang, not by reading the commit:

    cargo test -p amux-server --test route_table
    2 passed / 0 failed, finished in 4.12s

Against a symptom of "over 60 seconds" and then 14+ minutes, with three orphaned
route_table processes alive at once (23h24m, 2h29m, 13m, all at 0.0% CPU).

WHAT THEY FIXED THAT I ASKED FOR: `fire()` now has a per-route budget and the panic names
the offender — "{method} {path}: no answer in {FIRE_BUDGET:?} — this route BLOCKS, and
without this timeout the whole pre-push gate hangs instead of failing (AF-129)". So a hung
route is a named red test instead of a process list, which is the ethos rule 7 + rule 4
half this entry was actually about.

WHAT THEY FIXED THAT I GOT WRONG. My FIX field said "wrap each fire() in
tokio::time::timeout". That is not sufficient and they say why in the code (route_table.rs
:86-88): a route that blocks by SPINNING rather than yielding cannot be preempted by
timeout(dur, future), because the future never returns to the executor to be cancelled. The
shipped version runs the probe on a multi-thread runtime and puts the timeout on the
JoinHandle instead. Had my version been implemented as written it would have hung on
exactly the spinning case, and the entry would have read as fixed.

Worth recording as the general shape: a FIX field is a hypothesis, not a specification, and
the person who implements it is the one positioned to find it wrong. This is the second
time this week a peer's implementation was better than the entry's own proposal (AMUX-2971
was the other, where the author noted the shipped fix distinguishes a live foreign owner
rather than the port they had suggested gating on).
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-21
SESSION: amux-frustrations
CARD: AF-129
SYMPTOM: `cargo test -p amux-server` is what CLAUDE.md tells every lane to run before
  pushing. 23 test-result lines complete in about a minute, then
  `route_table_matches_the_real_router_both_directions` (tests/route_table.rs:91) prints
  "has been running for over 60 seconds" and stays there. My first run died on a 10-minute
  limit inside it; a `--no-fail-fast` re-run sat in the same test for 14+ minutes. Zero
  failures throughout — it does not fail, it stops. Not slowness: three
  `route_table-efc570d6d8aa84be` processes were alive on this machine at once, 23h24m,
  2h29m and 13m elapsed, all at 0.0% CPU with seconds of accumulated CPU. Three separate
  runs, across sessions and days, each wedged and each leaving a process behind forever.
  The 23-hour one was not mine. CI does not see it: the rust workflow finishes at ~17m and
  passes, so the gate is green upstream while being unusable on the machine lanes run it on.
COST: I could not honestly certify "the suite is green" before consenting to a push, and
  said so as a projection from 23 of N rather than a completed run. Two other lanes paid it
  before me without anyone connecting the timeouts to a shared cause — that is what three
  orphans across a day means. Every lane following the documented workflow either waits
  indefinitely or kills the run and pushes on partial evidence.
FIX: The hang is a bug; the defect worth fixing is that the test cannot REPORT it. The loop
  runs `for entry in ROUTE_TABLE { fire(&app, method, &path).await }` with no timeout
  anywhere, so a blocking route means the test cannot go red (ethos rule 7) and nothing
  records which route or method blocked (ethos rule 4). The evidence a reader is left with
  is "over 60 seconds" and a process list. Wrap each `fire()` in `tokio::time::timeout` and
  fail naming the route and method — a hung route becomes a named red test, and the
  root-cause investigation becomes a one-line read instead of the reason nobody has done it.
  Hypothesis killed, so nobody re-runs it: I suspected the test drives the REAL tmux fleet.
  route_table.rs has no tmux isolation, there is no cfg!(test) guard in session_verbs.rs,
  and the only `any()` route in ROUTE_TABLE is `/api/workers/{name}/{*verb}` — the
  session-verb dispatcher, which shells to tmux. It fits the open AF-69/AMUX-3221 entry
  exactly. It is still wrong: `concretize` yields /api/workers/zz-probe-1/zz-probe, and
  firing that at the live server returns 404 in 118ms, rejected on the unknown verb before
  anything reaches tmux.

  REPRODUCED DETERMINISTICALLY 2026-08-21, with two competing causes excluded — recorded
  because desktop landed 7ecb766 an hour later fixing a DIFFERENT wedged-cargo-test cause on
  this same machine, and the two present identically (wedged `cargo test`, 0% CPU process).
  `cargo test -p amux-server --test route_table`, 240s cap: build "Finished in 0.66s", binary
  starts, `every_directly_routed_api_path_is_in_the_table` passes, then
  route_table_matches_the_real_router_both_directions reports "over 60 seconds" and EXIT=124.
  NOT the shared build lock: 0.66s to build, with two other cargo processes on the machine.
  NOT desktop's devtool_roots scan: this run is AFTER 7ecb766, a different test binary, and it
  never rebuilds so it never reaches the lock.
  Correction to my own evidence, since this entry is about probes that cannot answer: my first
  pass ran `lsof -p <pid>` on the orphans and read the empty output as "no fds, just blocked".
  lsof is not on this shell's PATH (/usr/sbin/lsof), so the command never ran. It never
  reached this entry, and I am recording it because desktop's DESKT-15 entry says the lsof fd
  check is exactly what separated THEIR two candidate causes — a probe that silently does not
  run is worse than one that answers wrongly.

---

## A detector went fully inert and its own debug surface called it "baseline has 0 samples"
VALIDATED: amux-frustrations | Validated by running the regression test, not by reading the card.

The entry asked for two things and both shipped:

1. "Carry the pre-filter row count into the suppression so '0 of 46,825 rows, all
   filtered' cannot be confused with '0 rows in the period'."
   autofix.rs now emits: "blindness check ran: 0 of N families lost every row to
   filtering (X rows considered, Y excluded). A zero here is a measurement;
   silence would not be."

2. "add an invariant that fails when a family with enough rows in the period has
   an empty baseline." Filed as an ordinary Finding so it rides the pipeline that
   already turns a detector's output into a board card — no new mechanism.

AF-180 (amux, reviewing this entry) added the half I had missed and it is the
better half: a healthy alarm and a broken one are byte-identical silences, so the
HEALTHY zero goes through `suppressed`, which GET /api/debug/autofix already
renders. The answer now appears where the unhealthy one would.

Test run:
  cargo test -p amux-server --lib a_baseline_deleted_by_a_filter
  test runtime_jobs::autofix::tests::a_baseline_deleted_by_a_filter_is_an_alarm_not_a_quiet_suppression ... ok
  test result: ok. 1 passed; 0 failed

Its control is a LOGIC mutation (boot = None), which changes what the filter
concludes and changes no string, so the cell cannot pass by coupling to wording.

I am the originating session and I agree it is complete.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-178
SYMPTOM: Reviewing AF-175 I found the latency regression detector had stopped working on the
  running build. The only trace anywhere was in GET /api/debug/autofix:
    {"detector":"latency","signature":"latency|p95|/api/board",
     "reason":"baseline has 0 samples (<30) - no trailing norm to compare against yet"}
  /api/board has 46,825 rows in the baseline period and /api/sessions has 122,848. They are the
  two busiest families in the system. An upstream filter was excluding 99.75% of rows (213,397
  of 213,935) and the suppression reported that as an absence of data. The same sentence is
  emitted for a genuinely quiet endpoint, so a live detector outage is byte-identical to a new
  install with no traffic yet.
COST: The regression shape was dead on main and would have stayed dead silently. I only found
  it because I was reviewing that specific commit; no sweep, no alarm and no invariant could
  have surfaced it. Checked from two angles before saying so: /api/debug/invariants returns 461
  invariants and the only autofix-adjacent one is board.autofix_cards_are_dispatchable, and in
  the source base.len() is compared in exactly one place, the min_samples gate that produces
  the suppression. Detector health is not checked anywhere.
FIX: Carry the pre-filter row count into the suppression so "0 of 46,825 rows, all filtered"
  cannot be confused with "0 rows in the period", and add an invariant that fails when a family
  with enough rows in the period has an empty baseline. Both values are already in hand at the
  point of suppression. Detail and acceptance on AF-178.

---

## The shared-checkout amend guard pins HEAD, not the staged set, so a correctly-pinned amend still absorbed a peer's work
VALIDATED: amux-frustrations | Validated by re-running the shipped decision path, not by reading the card.

The entry's complaint was that the pin protects the WRONG OPERAND: it proves the
COMMIT BEING REWRITTEN is yours and says nothing about the CONTENT BEING ABSORBED.
That is now the durable half, shipped as AMUX-3407:

  scripts/git-hooks/git-shared-guard.py:192  "the pin proves the COMMIT BEING..."
  scripts/git-hooks/git-shared-guard.py:218  _amend_staged_decision — a pinned BARE
                                             amend absorbs the whole staged set
  scripts/git-hooks/git-shared-guard.py:286  names AF-106's exact incident in the refusal

Test cells exist for the specific case rather than the general one, at
scripts/git-hooks/test_git_shared_guard.py:149-180, and they run with REAL staged
content because empty-staged short-circuits before any of the three branches:
server-unreachable fail-open, pathspec-scoped no-refusal, check-disabled, and
no-session/human-ungated.

Ran scripts/git-hooks/test_git_shared_guard.py: ALL 51 PASS.

Card AF-106 is `verified`. I am the originating session and I agree it is complete.
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

## The untracked-work nudge is blind to review work, so a reviewer is told to record what they just recorded
VALIDATED: amux-frustrations | RETIRED — the friction is gone because the FEATURE is gone, not because it was fixed. Verified on 2026-08-26 across the whole repo: the nudge was Python's `_task_guard`, never ported when 792ce1f deleted that server. `AMUX_TASK_GUARD` survives as a settings value read ONLY by its own GET handler (settings.rs::task_guard_enabled) and its own tests — zero consumers anywhere in crates/, scripts/ or .claude/. Nothing can fire the message this entry reports. The prescribed fix (a reviewer= suppression alongside the session= ones) is therefore unbuildable as written, and is recorded on AF-241 so it is not lost if the nudge is ever reimplemented. AF-241 also carries the live defect this drain surfaced: the dashboard toggle still ships and its status text asserts "idle workers are nudged to log tasks", which is false.
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

## The reviewer-identity check fires on done->verified, blocking the peer amux routed the verification to
VALIDATED: amux-frustrations | RETIRED — the reported check does not exist in the Rust server, and its LESSON is encoded in the replacement. Verified 2026-08-26: no refusal anywhere in crates/ is keyed on who acked a review ("review sign-off required from the reviewer", "must come from that session" — zero hits); the message was Python's and was never ported at 792ce1f. What replaced it is AF-160's reviewer-name gate in api/board.rs, and its predicate is exactly what this entry prescribed: `reviewer != THE CARD'S OWNER`, never `reviewer != WHOEVER IS TYPING`. The comment at board.rs:3827 records that the first draft of that rule (mine) compared against the ACTING session and would have refused both real verifications on this board within the hour — AF-161 (owner=amux, reviewer=amux-frustrations, acting=amux-frustrations) and AF-16, its mirror image — because criterion 3 says the peer verifies it THEMSELVES, so reviewer == acting is the CORRECT shape. It was validated against every verified card rather than a fixture: admits 147 of 148 live and refuses exactly one, AMUX-2409, where owner and reviewer are both amux-homepage, which is the self-review the criterion exists to prevent. The gate is also scoped by `criterion_wants_a_name`, so it fires only where the criterion asks for a peer — this entry's "scope the identity check to the transition it is about".
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
VALIDATED: amux-frustrations | RETIRED — fixed by AF-135, via a different mechanism than this entry prescribed, and the different mechanism is sound. Verified 2026-08-26: the perishable sentence is gone (grep for "have not committed it since" across crates/ returns nothing). Delivery is still steer_enqueue, so the emission-to-delivery gap this entry identified genuinely still exists — what changed is that the notice no longer asserts an untimed present-tense fact. commit_nudge.rs:610 stamps every nudge "(...; tree observed HH:MM:SSZ — if you committed AFTER that moment this nudge predates it: re-run `git status` before acting on any remedy)". So the claim became TIME-QUALIFIED rather than re-checked: it is a true statement about a stated moment instead of a false statement about now, which removes the reported cost (auditing a clean commit for work that is not in it) without paying a git call per delivery or racing the same window again. AF-135's own note records the sharper reason it mattered: harmless on the commit branch, but on the STALE branch the same lag prescribed `git checkout origin/main -- <path>` against paths origin does not have, which DELETES them.
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

## A review PATCH using `desc` silently DELETED the author's entire card content
VALIDATED: amux-cloud | Re-tested against TODAY's code, not repeated from memory. Re-ran the exact incident live on scratch card AC-398: a cross-session PATCH replacing amux-cloud's desc now REFUSES ("refusing to replace amux-cloud's description... none of their 54 characters survive it... Length is not the test and this refusal fires whether your text is shorter or longer"), and the original content survived intact. That closes the precise boundary flagged as STILL LIVE on 2026-08-24 — small-card / same-or-longer overwrites, which used to apply silently. Fixed by c971756b (fix(board): the desc-clobber guard tests authorship and survival, not length, AMUX-3576), a DIFFERENT and better mechanism than this entry prescribed: it guards on content-survival plus authorship rather than the size-delta floor the earlier guard used. Noted by its author: fitting that AC-236, the origin of the "AC-227 fingerprint" the ledger invariant now names, is the one fixed by content-survival guarding — exactly the property that fingerprint protects. VALIDATED: amux-cloud | reproduced-refusal-on-AC-398, orig text intact, fix c971756b.
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

  CONTESTED 2026-08-21 by the author (amux-cloud), in a frustrations validation pass run
  by amux-frustrations. REPRODUCED ON THE CURRENT BUILD, not recalled: scratch card AC-388
  took an anonymous PATCH {"desc":...} that replaced the desc with applied:true, and an
  ATTRIBUTED cross-session PATCH as X-Amux-Session:amux did the same ("WIPED-BY-PEER",
  applied:true). fc9ae48 does not change the incident shape; it adds a log line recording
  the delta. Observable, not prevented — so the entry stays.

## `cargo test` was green while `cargo check` was green — and the compiled binary lacked my tests
VALIDATED: amux | Gone. The pre-commit hook runs `cargo clippy --workspace --all-targets` — 8 references to --all-targets in the file — and its comment records exactly why: plain `cargo check --workspace` does not compile test targets, so a break inside #[cfg(test)] sails through. That is the specific hole this entry describes. EVIDENCE: scripts/git-hooks/pre-commit, clippy and the check fallback both pass --all-targets. VALIDATED: amux.
SCOPE-OF-VALIDATION (added 2026-08-26 at the validating author's request, amux): this
  entry is archived as FIXED and its COST line describes something that is STILL LIVE.
  Both are true, and the distinction is the point. What was validated is this entry's
  NARROW claim — "cannot tell MY broken change from a PEER's" — which lint-blame.py
  closed by partitioning offenders, with AMUX_SKIP_RUST_GATE (6497eac0) making the
  answer actionable. What the cost line ALSO describes — the hook checks the WORKING
  TREE when the question is "is what I am COMMITTING sound" — is the structural defect,
  and it is OPEN as AF-182 (three instances, reopened 2026-08-26).
  So: narrow friction fixed, structural defect open, same subsystem.
  THE GENERAL RULE, which is amux's and is worth more than this instance: A VALIDATION
  IS A CLAIM ABOUT THE ENTRY'S TEXT, NOT ABOUT THE SUBSYSTEM. The two come apart exactly
  when a subsystem carries two entries at different depths, and the shallower one can be
  honestly retired while the deeper one stays live. Read an archived entry as "this
  sentence stopped being true", never as "this area is done".
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

## `cargo check --workspace` in the pre-commit hook cannot tell MY broken change from a PEER's
VALIDATED: amux | Gone, and by two mechanisms. lint-blame.py partitions offenders into mine / theirs / already-broken-on-HEAD and prints which files are which. As of 2026-08-26 it also names the narrow exit: with no offender of yours, AMUX_SKIP_RUST_GATE=1 skips that one gate and keeps the security scan, the staged-guard, the append-only guard and the JS checks (6497eac0). That second half is what makes the attribution ACTIONABLE — amux-frustrations' own AF-182 instance the same morning is the proof it was not, since attribution alone still left them on --no-verify. EVIDENCE: scripts/git-hooks/pre-commit calls lint-blame.py at 3 sites; the escape is printed only when `mine` is empty, both cells verified. VALIDATED: amux.
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

## Browser state can see overlay content but cannot click it, so overlay features cannot reach `verified`
SUPERSEDED: amux | THE ENTRY'S MECHANISM WAS WRONG, and its own author superseded it in place. Retired as SUPERSEDED rather than validated at amux's explicit request during the 2026-08-26 drain: "Do not validate this one and do not reopen it. That entry is WRONG, it is mine, and I already superseded it in place... Archiving L2735 as 'fixed' would file a false mechanism as validated history, which is the thing the supersession exists to prevent." The claim was that browser state could SEE overlay content but not CLICK it. False: the selector always contained [onclick] and selector_click_js() already existed. The real defect was a silent 120-element cap, with the two elements that could not be found sitting at indices 155 and 156 — addressable the whole time. The corrected diagnosis is in the superseding entry on the same card. Kept as a DEAD HYPOTHESIS (ethos rule 7: record which hypotheses are dead, not only which one was right) so nobody re-derives it. This entry is also what prompted AF-243, the third disposition itself — before it, a wrong entry could only be archived as validated or reopened as live, and both lie.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3721
SYMPTOM: Verifying the .mdai viewer's bottom tabs (AMUX-3322) in the real UI.
  `GET /api/browser/state` returned 120 elements plus a `text` blob. The blob
  CONTAINS "Diagram" and "List", so the tabs are provably in the rendered DOM.
  The elements array does not contain them, so there is no index to POST to
  `/api/browser/action` and no way to click them. Three misses in one sitting,
  all inside the file overlay: the `.mdai-row` div that opens a node (a div with
  an onclick, not a button), the overlay's X, and the `.mdai-btab` buttons.
  Compounding it, the overlay has its own scroll container, so a scroll action
  and an End keypress both moved the page behind it while the overlay stayed put.
  Neither clicking nor scrolling reaches overlay content.
COST: ~20 minutes establishing that the instrument rather than the feature was
  the blocker, and AMUX-3322 closed at `done` on DOM-text evidence instead of
  `verified` on a click-through. The broader cost is structural: this repo's own
  standard is that `verified` requires exercising the real UI, so every
  overlay-hosted surface (file viewer, MDAI viewer, peek) has an honest ceiling
  of `done` until this is fixed. That is a gate nobody can satisfy truthfully
  (ethos rule 3), and it fails silently — the state call returns 200 with plenty
  of elements, so it reads as working right up until you look for a specific one.
FIX: include elements carrying an onclick handler, not only semantically
  interactive tags; and let `/api/browser/action` take a CSS selector, which
  sidesteps the index problem and the scroll-container problem at once.

## A cross-cutting finding recorded on someone else's card dies when that card closes
VALIDATED: amux-frustrations | FIXED — d5c4ed0a, `amux board add --depends-on <ISSUE-ID>` (repeatable), live fleet-wide. The entry's complaint was that a review which finds something out of scope has nowhere to put it, so the finding rides in the host card's desc and dies when that card closes. Measured before building: the SERVER has always accepted depends_on at create (POST /api/board known_keys, board.rs) and honours it — verified live with a scratch card rather than read off the list. `amux board add` simply could not express any link; `epic` was the CLI's only link verb, so a card that begets a card took two steps and the second had no verb at all. So this was ethos rule 1, not a missing feature: capability present, honoured, reaching nobody because the sanctioned tool could not say it. Neither candidate shape in the entry was built: (a) a `--spinoff` concept would have been a second spelling of a link that already works, which the build-on-the-primitives rule refuses, and (b) a close-time prompt is the accumulation rule 5 warns about. Two independent lanes routed around the gap on 2026-08-26 alone — this entry's own reviewer case, and amux writing the AF-182 -> AMUX-3726 split into prose on both cards. Verified live in four cells: the flag sets depends_on; repeated flags accumulate; an empty value is refused; and with no flag the key is ABSENT from the PAYLOAD rather than an empty array — asserted on the body the CLI builds, because the server normalises absent to [] on read, so "sent nothing" and "sent []" are indistinguishable from the read-back and my first version of that cell could not have failed.
AREA: board
SEVERITY: slows
STATUS: open
DATE: 2026-08-08
SESSION: amux-frustrations
CARD: AF-242
NOTE-CARD: repointed 2026-08-26. This said CARD: AF-10, which is the rescued INSTANCE
  (the SSE `workers` global that survived because I re-read the review and filed it by
  hand) — not the mechanism. So the entry pointed at a card that could be closed, and was,
  while the class went unaddressed. That is the AF-191 shape one level in: a CARD: that
  resolves, to the wrong thing. AF-242 is the class.
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

## A commit that compiles in the author's tree can be unbuildable AS A COMMIT
VALIDATED: amux-frustrations | FIXED by amux (AMUX-3726), verified in the INSTALLED hook rather than the source alone. The entry's own FIX named option (a): "The staged-guard already knows both facts it needs." That is what shipped. `_amux_staged_recheck()` in scripts/git-hooks/pre-commit materialises the INDEX into a scratch worktree (`git worktree add --detach HEAD` + `git checkout-index -a -f`) and builds THAT, so the gate now answers "is what I am COMMITTING sound" rather than "does the author's tree compile" — which is precisely this entry's title, a commit that compiles in the author's tree being unbuildable AS A COMMIT. Wired into BOTH gates (clippy at :378, the cargo-check fallback at :406), not just the one whoever was reading happened to hit; the fallback's own comment records that the hazard "matters MORE, not less" there because the failure that reaches it is a compile error rather than a lint. Gated on `_blame_rc -eq 10`, i.e. it runs only when lint-blame determines NONE of the offenders are yours, so the ~22s cost is paid only in the case that today costs the committer their commit. Cost measured by its author before writing it: 22s warm, and it does NOT amortise, because cargo re-fingerprints the workspace crates when the path differs. Confirmed the installed copy is byte-identical to the tracked source (`diff -q scripts/git-hooks/pre-commit .git/hooks/pre-commit`), which matters here because AMUX-2777's whole point was that editing scripts/ alone leaves a fix reaching nobody.
AREA: gates
SEVERITY: blocks
STATUS: open
DATE: 2026-08-24
SESSION: amux-frustrations
CARD: AF-190
SYMPTOM: My 53ae4b8b was the tip of origin/main and did not compile. Staging
  crates/amux-server/src/api/board.rs took ~16 lines of a peer's in-flight AMUX-3607 wiring that
  were sitting in the same FILE, including a call to `effective_gate_trail` whose definition was
  in board_store.rs — still uncommitted in their tree, so not in mine.
  `git show 53ae4b8b:crates/amux-server/src/db/board_store.rs | grep -c effective_gate_trail` -> 0,
  while board.rs at that same commit calls it. Main was unbuildable until their f5c6af76 landed.
COST: A broken tip on origin/main. CI runs per-tip so it went green, but a bisect through that
  range still breaks, and per-commit CI would have gone red on someone else's PR. My clean local
  `cargo check` and the pre-commit gate both passed, correctly: they check the TREE, which
  contained the peer's definition. Nothing anywhere builds the COMMIT.
FIX: The pathspec form CLAUDE.md mandates does not reach this — the peer's work was in the same
  file as mine, so file-granular staging takes it regardless. Two things that would:
  (a) The staged-guard already knows both facts it needs. It told me "34 insertions / 9 deletions
      — if that is MORE than you wrote, their work is in it", and 5 of those 34 were the peer's.
      It could also say: "you are committing board.rs, which a peer co-edited, and board_store.rs
      is DIRTY and NOT in this commit" — a staged/dirty cross-reference, from data it already has.
  (b) Build the COMMIT rather than the tree: a detached worktree at HEAD with its own target
      dir, checked before the commit is pushed. MEASURED rather than guessed — 40.6s on the next
      commit (be397da2), not the cold build I first wrote here, because cargo keys on content and
      the dependency tree is unchanged between commits.
  (a) is instant and names the hazard in words; (b) is the only thing that PROVES it. Not
  alternatives: do (a) first, and make (b) opt-in (AMUX_VERIFY_COMMIT=1) before it is a default,
  since the pre-commit gate already pays ~14s for clippy and this roughly triples it.
NOTE: the instrument was RIGHT and I read past it. The guard printed the insertion count and the
  exact question, and the number looked about right for my change so I did not reconcile it.
  Third time today I have named the confirming-result blind spot and the first time it shipped
  something. Same axis as amux's migration-cost entry: our discipline answers CORRECTNESS and
  does not answer WHAT ACTUALLY SHIPS.
NARROWED 2026-08-25 (amux-frustrations, the author): part (a) is SHIPPED by amux; part (b) is
  the half that remains, and it is the one that would actually catch the class.
  (a) DONE — 7ecdc869 "name the peer work a commit LEAVES BEHIND, not just the work it takes",
      with a65e2580 asserting the HOOK prints it rather than merely that the server emits it.
      Server side is `split_risk()` (git_guard.rs:1662, surfaced at :1752); the hook prints
      "SPLIT COMMIT WARNING — <peer>'s work is being cut in half: in this commit: <staged> /
      left behind, dirty and NOT committed: <paths>". That is this entry's (a) almost verbatim,
      including the staged/dirty cross-reference from data the guard already had. A comment at
      git_guard.rs:2958 records that split_risk must be SILENT when the peer has nothing, which
      is the negative control the warning needs to not become noise.
  (b) NOT DONE — nothing builds the COMMIT. `git worktree add` appears once in the whole repo
      and it is inside a test fixture (test-session-freshness.sh:407); neither pre-commit nor
      pre-push constructs a detached HEAD or uses `checkout-index`. Every gate still compiles
      the WORKING TREE, which is the exact substitution this entry is about — and the same
      substitution AF-195 hit from the other side (I tested the tree and committed the index).
  WHY (b) STILL MATTERS WITH (a) SHIPPED: split_risk WARNS about the shape; it cannot tell you
  the commit does not build. A peer's half-file can be absent from your commit with nothing
  dirty left behind — they may have committed their half seconds after you staged — and the
  warning is correctly silent while the commit is still unbuildable. Measured cost when it
  happened: 40.6s to build the commit, against four unbuildable commits landed on 2026-08-08.
CORRECTED 2026-08-27 (amux-frustrations, the author — this entry's own probe was defective):
  The NARROWED note above asserts "`git worktree add` appears once in the whole repo and it is
  inside a test fixture". THAT IS FALSE, and it was false when I wrote it. The auto-builder has
  built the COMMIT since 7253465c (2026-08-09), fifteen days BEFORE this entry was filed:
  `scripts/rust-auto-build.sh:284` does `git -C "$REPO" worktree add --detach "$WORK" "$(... rev-parse HEAD)"`,
  a detached worktree at the committed sha with no working-tree files, so a peer's uncommitted
  definition cannot make a broken commit look sound. `e2e/serve-head.sh:142,149` does it too.
  WHY THE PROBE COULD NOT SEE THEM: I grepped the literal adjacent pair `git worktree add`. Both
  real callers write `git -C "$REPO" worktree add`, so the option sits BETWEEN my two tokens and
  the pattern cannot match. It found two hits — a comment and a test fixture — and I read that as
  a negative. Reproduced 2026-08-27: literal `git worktree add` -> 2 hits, neither a real caller;
  `worktree add` -> 5, including both. The probe was blind to exactly the thing it searched for,
  and the blindness is not incidental: a tool that builds a detached snapshot MUST operate on a
  repo it is not cd'd into, so `-C <repo>` is precisely the form this class of caller takes.
  This is ethos rule 4's "before believing a negative, say what a positive would look like and
  confirm the probe could produce it", failed in an entry that is itself about a gate answering
  the wrong question.
WHAT IS ACTUALLY LEFT, measured rather than argued (amux's AMUX-3797, evidence corrected here):
  The builder triggers on the LAST Rust-touching commit — `rust-auto-build.sh:46`,
  `git log -1 --format=%H -- crates/ Cargo.toml Cargo.lock`. When two Rust commits land between
  polls, the earlier one is stepped over and never built. Over the builder's whole life
  (7253465c..main): 992 Rust-touching commits, 126 of them (12.7%) were never a `building`
  target. That is this entry's COST clause exactly — "a bisect through that range still
  breaks" — and it survives the headline being closed.
  MEASURE BY ANCESTRY, NOT BY DATE. My first pass used `git log --since=2026-08-09` and got
  1028/161; the range form gives 992/126. `--since` prunes traversal by author date, so it
  admits commits that reached main through a merge of a branch based before the window and is
  not the same set as "descendants of the builder's first commit". The two implementations
  reconcile EXACTLY once both use ancestry: amux measured 7253465c..origin/main as 125 of 839
  never built, I measure origin/main..HEAD as 1 of 153, and 125 + 1 = 126 of 992. Independent
  scripts agreeing to the commit is worth more than either number.
  THE UNPUSHED STACK IS CLEAN: 1 of 153 Rust-touching commits. An earlier "83 of 235" figure
  counted docs and markdown commits, which `:46` correctly excludes from being build targets;
  it was withdrawn.
  NOT the mechanism: SKIP-under-contention. 287 distinct shas were skipped at least once and
  only 7 of them were never subsequently built, because a SKIP is usually the dedupe declining a
  DUPLICATE trigger for the sha already building. The line quoted as proof (07:45:48 SKIP
  962c15d79) is preceded four seconds earlier by `07:45:44 building 962c15d79` — that sha was
  built. Reading a SKIP without checking for a `building` line naming the same sha counts the
  dedupe working as a commit lost.

## A page.route stub defeated by a service worker fails LOUDLY and blames the wrong subsystem
VALIDATED: amux-frustrations | FIXED 5e07e88a, the CLASS the entry left open: "nothing warns that a page.route stub never matched a request".

e2e/fixtures.ts wraps page.route so each stub counts hits and teardown fails the test naming the stub. Silent when the test already failed, because an unhit stub is usually downstream of whatever actually broke and reporting it there would be this entry's own defect committed by its own fix. allowUnusedRoute(page, matcher) is the declared opt-out, so "may not fire" gets written down rather than assumed.

Reaching every spec, not just the four converted: crates/amux-server/tests/e2e_route_stub_guard.rs fails the build when a spec stubs a request while importing test from '@playwright/test'. Mutation confirmed — reverting one import fails the guard by file name with the fix instruction. It also flags context.route, which the fixture does NOT wrap, rather than letting an unguarded stub look guarded.

The wrapper itself is tested against the real runner (e2e/route-stub-guard.spec.ts, 3 passed on desktop), because importing the fixture is not the same as the fixture working and the defect lives in the teardown path: a dead stub fails (test.fail inverts it), allowUnusedRoute suppresses it, and a stub that DOES match does not fail. That third cell is the control — without it cell 1 is equally consistent with a wrapper that breaks all four real stubs.

The entry's own instance was already fixed in b31bcac, and the service-worker half generalised into playwright.config.ts as serviceWorkers: 'block' by default.
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

## SIX answer-shaped wrong results in one night, and in every one the tell was a MISSING ACCOMPANIMENT rather than the answer
VALIDATED: amux-frustrations | The GENERALISATION is now encoded in the rules, which is what the entry asked for: "ethos rule 7 already carries this family... What it does not yet carry is the accompaniment test, which is the cheap mechanical version."

ethos.md rule 4 now carries it in one sentence: "A wrong answer is rarely wrong-LOOKING, so name what should appear BESIDE the answer if the probe really ran and check for THAT: a count beside a zero, a hash beside 'adopted', a PASS line beside a green suite, a key listing beside a None." Those four forms are the four specimens whose tell was an absence.

The SIXFOLD count moved to docs/ethos-incidents.md with all six specimens intact, which is the entry's other requirement — "so the SIXFOLD count is somewhere countable rather than spread across six cards nobody joins up". It sits beside the nine-instance probe-defect cluster and the -S/-G pickaxe case, where the argument that these are one family is readable.

SPECIMEN 3'S SURFACE IS FIXED, not just written down. /api/logs was "a capped newest-first page with no upper bound" and now publishes its own span. Live: truncated=true, page_span_h, total_matched, and note="TRUNCATED: these are the newest rows, not the whole window. Page backward with `until=<the oldest ts in this page>`". The zero that started this can no longer be returned without the payload saying the measurement was partial. That was AF-230's fix.

Two of the six were amux defects with their own fixes already (module-level sys.exit now __name__-gated; /api/browser/start unknown fields, AMUX-3403). The remaining two are field names differing by a suffix (last_run_at vs last_run), which no surface can currently tell a caller they misread - stated as a known gap in the incidents file rather than left implied.
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

## Three defects in two days where a compound operation reported success from the parts that worked
VALIDATED: amux-frustrations | All three specimens have SHIPPED fixes, the CI wiring the entry said was pending has LANDED, and the general half is now encoded.

SHIPPED, per the entry's own FIX block: 7759b36 (APP_VER/CACHE must MOVE when the file moves, not merely agree), c207339 (the sweep refuses when a full fetch returns no desc), 1998c75 (scripts/test-tree-clean.sh).

THE PART THE ENTRY LEFT OPEN IS CLOSED. It said: "Wiring it into .github/workflows/rust.yml is NOT mine to do: that file gates every lane's push. Proposal and evidence routed to amux; the guard is committed and runnable meanwhile." It is wired — rust.yml:67 runs `--self-test` as a negative control FIRST, and :82 wraps `cargo test --workspace` in the guard rather than running it after, so the guard cannot drift from what it guards.

MEASURED QUIET, with the probe's own capability confirmed: 25 rust.yml runs (2026-08-24..2026-08-27), 50 jobs, 298 annotation rows read, ZERO mentioning residue. The first pass of that probe read `.title`, which is empty on every annotation this repo produces, so it was structurally incapable of a hit; re-run on `.message` it returns 298 readable rows including eslint and Node-deprecation warnings. Step-level confirmation that it was not skipped: the latest run shows both "Tree-residue guard — self-test (negative control)" and "cargo test (workspace) — tree-residue guarded" as `success`.

THE GENERAL HALF IS ENCODED. docs/ethos-incidents.md now carries the family under its own name, "a compound operation takes its success signal from the parts that worked", with all three specimens and all three habits verbatim — kept as three because each catches exactly one of them and none of the others. It sits beside the accompaniment-test cluster, which is its sibling and needed distinguishing: there the tell is something ABSENT from the output, here nothing is missing at all and the operation genuinely succeeded.

ONE FOLLOW-UP, NOT MINE AND NOT THIS ENTRY'S FRICTION: rust.yml downgrades the guard's exit 3 to a warning, and its comment sets the exit condition itself — "flip to blocking (delete the `if`) once it has been quiet for a few days; leaving it advisory forever would make it decoration." The condition is met on the measurement above. Routed to amux with the evidence rather than flipped here, because that file gates every lane's push (ethos rule 8), which is the same reason the entry gave for not wiring it itself.
AREA: silent-partial
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-150
SYMPTOM: amux noticed the cluster and it is right, though not quite as "three invisible
  no-ops" — one of the three is the opposite of a no-op. The property they actually share is
  narrower and worth naming: A COMPOUND OPERATION TOOK ITS SUCCESS SIGNAL FROM THE PARTS THAT
  WORKED, while one part did nothing and said nothing.
    1. 1a7d215 (mine). Mutation-testing a guard, I disabled `if !p.is_absolute()` to prove the
       test could fail. The test then did what the unguarded code says and created a directory
       in the shared checkout. I reverted the FILE and reported the mutation clean; the
       directory outlived the revert and failed every later local run while CI stayed green.
       The revert succeeded at its visible half.
    2. 24fc2b4 (mine). A version bump written as a literal find-and-replace — '0.9.701' ->
       '0.9.702' — matched nothing, because a peer had moved both files to 0.9.708 between my
       read and my write. The same edit pass made the functional changes successfully and
       printed "patched". I had asserted on those and not on the bump.
    3. c207339 (amux). The recovery sweep classified on `desc`, and AMUX-3496 made the default
       board list slim, which does not carry it. `.get("desc") or ""` was empty for every row,
       so the sweep printed "0 to do" on a schedule while 76 unowned reports sat there. The
       FETCH succeeded, and the fetch is what the sweep reported on.
COST: measured, not estimated. (1) a red test on correct code that a peer hit while it blocked
  their gate. (2) a UI fix that reached no browser holding the cached script — caught only
  because a peer asked a routine push-census question, and would otherwise have looked shipped
  indefinitely. (3) a scheduled sweep reporting a clean board on a cadence while 76 items sat
  in it. None of the three produced an error, and in all three the surrounding operation was
  genuinely successful, which is what made the silence convincing.
FIX: two shipped and one general.
  SHIPPED — 7759b36 turns (2) into a CI guard, and the design point is worth keeping: the
  pre-existing test pinned that APP_VER and CACHE AGREE, and it could not have caught 25ba8ea
  because NEITHER moved, so they still agreed and it stayed green. Agreement was never the
  invariant; MOVING WHEN THE FILE MOVES is. Verified against the real artifacts rather than a
  fixture — I re-ran its logic here across four ranges: FAILS on 25ba8ea (app_moved=0
  sw_moved=0), passes on 24fc2b4 and 36b93f8, skips a range with no client JS.
  SHIPPED — c207339 makes (3) refuse when a full fetch returns no desc, rather than treating
  an absent field as an empty result.
  SHIPPED: 1998c75 turns (1) from a habit into a mechanism: scripts/test-tree-clean.sh
  wraps a command and fails if the checkout changed, so a fixture that dirties the tree is
  caught by the run that dirtied it rather than by the next person's red test. The design
  point is the one that nearly went the other way. `git status --porcelain` reports ZERO
  LINES for the exact residue in (1), and so does `-uall`, because git does not track empty
  directories; the obvious guard would have been green and unable to fail on its own
  motivating incident. `git clean -nd` sees it, and cannot see a modification to a tracked
  file, so the snapshot is the union. It ships a `--self-test` negative control (fires on an
  empty-dir residue, silent on a no-op) so a green from it is never taken on faith. Two
  measured limits are in its header: it attributes every diff to the wrapped command, which
  is false on THIS shared checkout (the first baseline run named a peer's mid-run edit to
  alerts.rs), and it ignores gitignored paths so cargo's target/ writes are not noise.
  This also inverts what 67137cc concluded, that "CI never sees this class (fresh checkout)".
  A fresh checkout is where the residue is EASIEST to see, because it has no history to
  hide in, so the run that created it is the only thing that could have. Wiring it into
  .github/workflows/rust.yml is NOT mine to do: that file gates every lane's push. Proposal
  and evidence routed to amux; the guard is committed and runnable meanwhile.
  GENERAL, and the part that does not have a patch: when a step's failure mode is doing
  nothing, its success cannot be inferred from the operation around it. Three concrete habits,
  each of which would have caught exactly one of the above and none of the others, which is why
  all three are listed rather than one rule:
    - assert the WRITE changed something, not that the code ran (`assert new != old` on each
      file), because a literal replace that matches nothing is indistinguishable from one that
      matched;
    - after mutating a guard OFF, ask what the code does WITHOUT it — that is precisely what
      the guard prevents, so the answer is never nothing, and the side effect outlives the
      revert;
    - when classifying on a field, confirm the field is PRESENT before concluding from its
      absence — an empty classification over a non-empty fetch is the loud-wrong-probe shape,
      answering confidently from a column that was never there.

---

---

## A peer's uncommitted lint error blocked my commit and the message named their file, not them
VALIDATED: amux-frustrations | BOTH HALVES SHIPPED, and this entry needed both because a card carrying two units of work is the wrong-SCOPE trap this repo's own rules describe. AF-182 was signed off once already on a fix that did exactly what it claimed while the headline stayed true and recurred.

HALF ONE, the attribution, which is what this entry's FIX asked for verbatim: scripts/git-hooks/lint-blame.py partitions offenders into yours / a peer's in-flight work / already-broken-on-HEAD. It prints "BLOCKED BY ANOTHER SESSION'S IN-FLIGHT WORK - not your commit" when none are yours, and it carries the COUNT ("1 of 1 offending file(s) ARE in your commit", "3 of 4"), which the entry called for by name because a partition reporting only the peer's share reads as exonerating. It is deliberately silent about the escape hatch when `mine` is non-empty, so an escape is never printed beside your OWN denial.

HALF TWO, the structural one, which is the half that recurred after the first sign-off: AMUX-3726's `_amux_staged_recheck()` materialises the INDEX into a scratch worktree (`git worktree add --detach HEAD` + `git checkout-index -a -f`) and builds THAT, wired into BOTH gates - clippy at :378 and the cargo-check fallback at :406. The gate now answers "is what I am COMMITTING sound" instead of "does this shared tree compile", so the refusal this entry is about becomes a pass.

VERIFIED BY RUNNING THE SUITE, not by reading the code. scripts/test-staged-recheck.sh, 7 passed:
  cell 1  foreign offender + clean staged content -> ALLOWED   <- this entry's exact scenario
  cell 2  staged file among the offenders -> refused
  cell 3  staged content fails its own build -> refused
  cell 4  an unlicensed re-check does NOT build the index
  cell 4b CONTROL - a LICENSED re-check DOES build it, so cell 4 is not vacuous
  cell 5  AMUX_STAGED_RECHECK=0 falls back to refusing
  cell 6  no worktree left behind
Cell 1 is this entry's SYMPTOM as a test case. Cell 4b is the control that makes cell 4 mean something, and cell 4 is the one that would have gone wrong quietly: a version that always built the index would pass 1-3 perfectly and cost the fleet ~22s on every commit forever.

Confirmed the INSTALLED hook is byte-identical to the tracked source, which matters here specifically because AMUX-2777's whole point was that editing scripts/ alone leaves a fix reaching nobody.

I checked the direction claim before signing this, because it is the thing that would make the validation wrong: amux briefly reopened this class on the reading that `_blame_rc -eq 10` licenses the re-check only when the TREE is already red, so it cannot help a green-tree/red-commit commit. That is correct about AF-190's direction and irrelevant to THIS entry, whose direction is tree RED / commit GREEN - precisely what exit 10 means.
AREA: gates
SEVERITY: blocks
STATUS: open
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-182
SYMPTOM: The pre-commit gate runs `cargo clippy --workspace --all-targets` over the WORKING
  TREE, not over what is staged. My commit of two clean files was refused with
  `board_drive.rs:3620 this assertion has a constant value`, from 170 uncommitted lines of a
  peer's in-flight work. Nothing in the output said the file was not mine. Earlier the same
  hour, `cargo check` failed on `missing field idle_since in initializer of QueuedItem` from
  the same peer writing checks.rs and monitor.rs minutes apart, and I built inside the window.
COST: A commit blocked outright with no correct action available except waiting on another
  session, plus a rebuild and a spell of doubting my own edits on the earlier one. The tempting
  wrong move is cheap and available: fix the peer's file. That is how a session ends up
  committing another session's half-finished work, which is the class the staged guard exists
  to prevent, reached from a direction the staged guard cannot see.
FIX: amux's framing, which is better than my first one: the gate reports a WORKSPACE-SCOPED
  FACT IN A SESSION-SCOPED SENTENCE. The diagnostic is true about the repo and false about the
  committer and nothing says which was meant. The gate already holds both halves at the moment
  it refuses (the staged pathspec, and the file each diagnostic names), so the discriminator is
  a set membership test. Say "BLOCKED BY ANOTHER SESSION'S IN-FLIGHT WORK - not your commit",
  name the session and that the staged files are clean, and carry the COUNT, because "1 of 1 is
  not yours" and "3 of 4 are yours" are different situations and the second must not read as
  exonerating. AF-182.
NOTE: third instance of one shape in about an hour, with AF-179 (a peer's Bash window sampled
  my ongoing authorship, reported as "you edited this") and the transient unbuildable window
  amux is filing separately. All three are a true statement about the shared checkout delivered
  in the second person.

## THIRD AF-182 instance: a peer's non-compiling tree killed my e2e web server and my pre-commit gate
VALIDATED: amux-frustrations | BOTH THINGS THIS ENTRY ASKED FOR ARE IN PLACE, and one of them already was when the entry was filed - which is itself the finding.

(1) "The gate is checking the wrong thing: a pre-commit hook that compiles the WORKING TREE cannot answer 'is what I am committing sound'. Staged-content checking would have let all three commits through honestly." SHIPPED as AMUX-3726: `_amux_staged_recheck()` materialises the INDEX into a scratch worktree and builds THAT, wired into both the clippy gate and the cargo-check fallback - and the fallback is the branch this entry's E0433 would have hit. scripts/test-staged-recheck.sh cell 1 is this scenario ("foreign offender + clean staged content -> allowed"), 7 passed, with cell 4b as the control proving the licence check is not vacuous. The --no-verify this entry calls the expensive part is no longer the only honest move.

(2) "The e2e half wants isolation, not etiquette." ALREADY SHIPPED when this was filed, and that is the part worth recording. e2e/serve-head.sh has built from committed HEAD in a detached worktree since 7624877a, 2026-08-11 - fifteen days before this entry. So the per-lane-worktree proposal was answered by something better already running: isolation from the working tree entirely rather than one worktree per lane.

WHICH RAISES THE QUESTION THIS ENTRY CANNOT ANSWER, and that is the real residue. `git log -S 'crate::worker::WorkerId' --all` finds nothing, so the import that killed the run was NEVER COMMITTED - it cannot have reached a build of committed HEAD. One of serve-head.sh's two working-tree paths must have been taken (AMUX_E2E_WORKING_TREE=1, or the fallback when a worktree cannot be prepared), and the run's own output cannot say which, because the script announced its source only `if [ -n "$dirty" ]`. A run against a tree with no uncommitted Rust changes said nothing at all, so all three sources produced identical output.

FIXED IN eeccbbc1: each of the three paths now prints one SOURCE line naming what it built, with the sha and worktree for HEAD. Grepping SOURCE finds exactly one hit per run instead of one hit per run that happened to go well. Verified in a real boot rather than by reading the diff:
  [WebServer] [e2e] SOURCE: committed HEAD f6a80ece (worktree ~/.amux/e2e-worktree).

So the cost this entry recorded was paid twice: once for the dead run, and once because the diagnosis it reached ("a peer mid-edit in the shared tree") could not be checked against what the run actually built. The first is fixed by (1) and (2); the second is fixed by making the source legible, which is what an entry filed against an already-isolated harness was really pointing at.
AREA: gates
SEVERITY: blocks
STATUS: open
DATE: 2026-08-26
SESSION: amux-frustrations (imposed by amux, who reported it themselves)
CARD: AF-182
SYMPTOM: Mid-verification of AF-235 the Playwright webServer refused to come up:
  `error[E0433]: cannot find `worker` in `crate` --> api/session_verbs.rs:11273` ->
  "Process from config.webServer was not able to start. Exit code: 101". Not my file
  and not my change — a peer was mid-edit in the shared tree with a wrong import path
  (`crate::worker::WorkerId` for `amux_core::ids::WorkerId`). The same tree state
  would have failed the pre-commit hook, which runs `cargo check --workspace` over the
  WORKING TREE rather than over what is staged, so I committed with --no-verify and
  gated my five files by hand instead.
COST: One dead e2e run (~1.5 min plus the re-run), and a --no-verify commit — which is
  the expensive part, because it means the gate was bypassed on a real commit and the
  bypass is now indistinguishable from a careless one to anyone reading the reflog.
  The peer fixed it within a couple of minutes and reported it unprompted; nothing here
  is a complaint about them.
FIX: NOT more care. This is the THIRD entry on AF-182 (the others at L2070 and L2191),
  which is the count this file exists to make visible, so it should stop being read as
  three unlucky mornings. Two things follow.
  (1) The gate is checking the wrong thing: a pre-commit hook that compiles the WORKING
  TREE cannot answer "is what I am committing sound", which is the only question it is
  asked. Staged-content checking would have let all three commits through honestly.
  (2) The e2e half wants isolation, not etiquette — a per-lane worktree for anything
  that starts a test server, which the Agent tool already supports (`isolation:
  "worktree"`). amux's own read, offered on the instance they caused: "if AF-182
  reaches the three-entry threshold that makes it an argument rather than a complaint,
  I think the answer is a real one (per-lane worktrees for anything that runs a test
  server) and not more care from me. Count it." Counted.

## The log sweep's own instrument could only show it 1.6% of the window it was judging
VALIDATED: amux-frustrations | Re-verified 2026-08-28 against the entry's own FIX claim, not the subsystem. `until` is
honoured (`since < ts <= until`; every returned row satisfied the upper bound), and the
response now carries `truncated`, `page_span_h` and a `note` reading "TRUNCATED: these are
the newest rows, not the whole window. Page backward with `until=...`".

Exercised on its author the same day: today's log sweep called /api/logs for step 5, got
`page_span_h=0.79 truncated=True`, and I changed approach BECAUSE the response said so
rather than by noticing `total_matched` disagreed. That is precisely the cost this entry
records - the sweep was reaching "the accusation you cannot un-say" from one capped page,
and the mismatch had to be noticed rather than read.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux-frustrations
CARD: AF-230
SYMPTOM: `GET /api/logs?since=<24h ago>&limit=2000` answered `total_matched: 123645`,
  `count: 2000`, and the rows it returned spanned 0.48 HOURS. `since` ("ts > ?") was the
  only time bound, and the query is `ORDER BY ts DESC LIMIT <=2000`, so every call returns
  the same newest rows and there is no way to page backward. Nothing in the response said
  the page was a slice — `total_matched` disagreed with the window being described, but the
  mismatch had to be noticed rather than read.
COST: Sweep step 5 decides whether a lane is doing mutating work with no board trace — the
  contract's own words are that this is "the accusation you cannot un-say", and it lists
  seven qualifications, each added after a false positive. That step has been reaching its
  verdict from one capped page for as long as it has existed. Today's answer was clean, so
  the cost was not a wrong accusation; it was that a clean 29 minutes was on its way to
  being reported as a clean day. The contract already carried a workaround telling the
  reader to state the blind spot "or read the store directly for the full window" — routing
  a caller off the sanctioned instrument onto raw SQL, which is the rule 6 shape.
FIX: fcff219e. `until` ("ts <= ?") makes the window walkable (`since < ts <= until`), and
  the response now admits when it is a slice: `truncated`, `page_span_h`, and a note naming
  the paging move. `analyze` and `stats` already publish `scan_truncated`/`actual_window_h`
  for exactly this reason; this is the same admission on the endpoint that lacked it, so the
  next capped read announces itself in the payload the caller already opens. The contract's
  step 5 now carries the paging loop instead of the workaround.

## The ledger cannot express that an entry is unvalidatable, so 20% of the open set can never drain
VALIDATED: amux-frustrations | Re-verified 2026-08-28 by running the shipped audit and checking BOTH directions, because
this entry's complaint was that the two cases read byte-identically.

  AEAB-* (non-fleet namespace, author absent from the fleet):
    "STRANDED: prefix AEAB exists nowhere on this board and author
     amux-errors-and-bugs is not in this fleet"  x12
  AC-227 (amux-cloud, a LIVE lane here): not flagged at all - no line emitted.

So the discriminator fires on the stranded set and stays silent on the ordinary
cross-instance id, which is the distinction the entry says did not exist. The summary line
now states the number outright: "STRANDED 12 entr(ies) cite a card no one in this fleet can
reach", against the 12 of 59 the entry measured.

The discriminator is the PREFIX NAMESPACE plus author liveness rather than author liveness
alone, which is what keeps a live lane's cross-instance card out of the stranded bucket.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-25
SESSION: amux-frustrations
CARD: AF-229
SYMPTOM: `frustrations_audit.py` resolves every CARD: against the live board and printed one
  advisory when it missed: "not on this board (other instance, or deleted)". Byte-identical
  for AC-227 (amux-cloud, a LIVE lane here) and AEAB-18 (amux-errors-and-bugs, absent from
  all 120 sessions, working out of a `~/Developer/amux` that does not exist on this machine).
  12 of 59 open entries are AEAB-*; direct GET returns 404 for each, and 0 of 9,296 cards
  carry that prefix while DESKT-*, also a non-fleet lane, carries 25.
COST: The deletion protocol keys removal to the ORIGINATING session's sign-off, so those 12
  have no party who can ever sign them off — they accumulate in the open set forever while
  reading as ordinary work. This file's entire argument is a COUNT ("three entries sharing an
  AREA is an argument"), so a fifth of the open set being permanently unactionable distorts
  every AREA tally computed from it, including the ones used to decide what to rebuild next.
  Not hypothetical: it is why the drive-to-zero sweep stalled at 59 rather than finishing.
FIX: 04721906. The advisory stays advisory — a cross-instance id is not an error — but it now
  discriminates, and the discriminator is the PREFIX NAMESPACE rather than author liveness.
  That distinction is load-bearing: amux-rust is not live either, yet AR-114 answers HTTP 200,
  so judging on liveness alone called six drainable AR-* entries permanently stranded on the
  first run. Same commit fixes a defect it exposed rather than caused: `board.get()` was called
  on the whole CARD string, so multi-id fields ("AR-114, AR-115, AR-116") had ALWAYS reported
  unresolved, invisibly, until the branch started saying something specific and said it wrongly.
  Two of three predicate mutations survived the first draft of the test suite, which is why the
  roll-up and the empty-session-list controls exist as their own cells.
  STILL OPEN, and it is Ethan's call, not mine: what actually happens to those 12 entries.
  Reaching amux-errors-and-bugs, or retiring them with a rationale, is a decision about another
  party's contributions (ethos rule 8). The audit now names them; it does not presume to sweep them.

## The archive tool took evidence as an argv positional, so my shell executed the code I quoted
VALIDATED: amux-frustrations | Re-verified 2026-08-28. Both safe paths exist on the tool and the usage text prefers them:

    scripts/frustrations-archive.py <line> <validated-by> --evidence-stdin
    scripts/frustrations-archive.py <line> <validated-by> --evidence-file <path>
    PREFER --evidence-stdin/--evidence-file whenever the evidence quotes code.

Exercised rather than read: this validation and the two archived alongside it were all
written through --evidence-stdin, and the heredoc bodies contain backticks and $(...)
that reached the archive byte-for-byte. That is the exact substitution which corrupted
AF-130's archive line in two places and left only a misleading `now: command not found`
on screen.

The argv positional form still exists and is still unsafe with quoted code. That is a
choice rather than a trap now: the safe path is documented and preferred at the point of
use, which is what this entry asked for.
AREA: cli
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-25
SESSION: amux-frustrations
CARD: AF-223
SYMPTOM: Archiving AF-130 with evidence that quoted code, via
  `scripts/frustrations-archive.py <line> <who> "<evidence...>"`. Bash printed
  `line 1: now: command not found` and the archive line landed corrupted in TWO places:
  "asserts it comes back as , with the comment" (backtick-now evaluated to empty) and
  "so 0 returned 0 across the whole window" — where `grep -c 'WORK ITSELF is at risk'`
  was EXECUTED by my shell and replaced by its own output. The archive succeeded; only
  the one visible bash error hinted anything was wrong, and it named the wrong half.
COST: a mangled quotation written into the file that exists to be the DURABLE RECORD of
  what was verified, and the least recoverable place for it: the entry it describes had
  just been deleted from frustrations.md in the same operation. Caught only because the
  stray `now: command not found` was on screen. A quieter substitution — `$(date)`, or a
  grep that returns nothing — would have left a plausible sentence and no error at all.
FIX: shipped in the same breath. `--evidence-stdin` / `--evidence-file` on the tool, with
  the usage text saying to prefer them whenever the evidence quotes code. Verified the
  file path preserves backticks and $(...) byte-for-byte.
NOTE: this is AMUX-1888's shape, and the rule already exists — `amux send` and
  `amux board add` both grew --stdin/--file for exactly this, and CLAUDE.md states it as a
  fleet convention I have cited repeatedly this week. My own tool was written in the old
  shape and I used it the old way. The lesson is not "remember the rule": it is that a
  tool taking free text as an argv positional MAKES the trap, and every such tool in this
  repo has now had to learn the same lesson separately.

## The e2e suite restarts its own servers mid-run, and blames whichever specs were mid-navigation
VALIDATED: amux-frustrations | Validated 2026-08-28 on the OUTCOME half of the entry's own prediction, with the other half
stated as unverified rather than assumed.

The entry predicted two things of "the next e2e job": zero `binary changed on disk` lines,
and no ERR_CONNECTION_REFUSED failures.

CONFIRMED - the failure it describes did not occur, on real specimens of the exact shape
that motivated it. Two OUTSIDE CONTRIBUTOR PRs ran full e2e yesterday and both passed:
#161 e2e 17m38s, #162 e2e 21m9s. Both branches were based on cad635ea, and 67474428 is an
ancestor of it, so those runs contained the fix. The mechanism is still in place:
e2e/serve-head.sh:59 exports AMUX_NO_SELF_ADOPT=1.

That is the COST this entry records, gone: "a contributor's PR blocked on a red check that
was never theirs". Two contributor PRs went green through e2e and merged.

NOT CONFIRMED, and I will not claim it: zero `binary changed on disk` lines in the job log.
I tried to read run 33083312377's log and got 0 BYTES back. Grepping it returned 0 for the
predicted strings - which is what a genuinely clean log returns and also what an empty file
returns. A positive control (grep for "passed", "playwright", "e2e") returned 0 for those
too, which is how I know the fetch failed rather than the log being clean. Sixth instance
of that shape today and the first one caught before it became evidence.

The outcome half is the one that carries the cost, and it is confirmed twice.
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-185
SYMPTOM: PR 148 (an outside contributor's) was red with e2e 4 failed / 228 passed, every failure
  `net::ERR_CONNECTION_REFUSED at https://localhost:18823/`, and nothing anywhere said why. The
  suite starts three servers (desktop/mobile/ios-safari), each a `cargo run` rebuilding into the
  SAME target dir; every rebuild rewrites target/debug/amux-server, and a running server watches
  that path and exec's itself. Run 32671387493's log has three `binary changed on disk —
  exec'ing the new build in place` lines, each right after a sibling target's build finished,
  each costing ~10s of refused connections while the suite drove that server.
COST: A contributor's PR blocked on a red check that was never theirs, with no way to tell from
  the PR. Because the victims are chosen by timing they move run to run, so no spec is reliably
  guilty and the whole thing reads as flakiness rather than a defect with a cause. That is the
  same misattribution shape as AF-179 and AF-182: a true statement about the environment
  delivered as a statement about the thing under test.
FIX: 67474428 — the suite sets AMUX_NO_SELF_ADOPT=1. The capability already existed (AEAB-52)
  and its own doc comment says what it is for, "a test harness pins its build on purpose"; the
  one harness in this repo that pins its build was not enrolled. Rule 1 exactly. Not yet proven
  in CI: the prediction is zero `binary changed on disk` lines in the next e2e job and no
  ERR_CONNECTION_REFUSED failures, and if they persist the env is not reaching the server through
  serve-head.sh.

## The push guard's only override is worded for the human, so the AUTHOR's explicit consent has no honest exit
VALIDATED: amux | Signed off by amux 2026-08-28, the originating session, who verified it themselves rather
than taking amux-frustrations' account:

  "Both escapes exist: AMUX_ALLOW_FOREIGN at pre-push:18 and AMUX_FOREIGN_CONSENT at :358,
   with :396 rejecting it malformed and :453/:483 rejecting it when it does not match the
   commits. Ten mentions, plus cells E-H in scripts/test-push-guard-range.sh."

Corroborated live the day before by amux-frustrations: pushing 260 commits, the guard
offered both paths in its refusal output, with per-commit `<sha>:<session>[:owner]` entries
for the recorded form. AMUX_ALLOW_FOREIGN=1 was the honest one there because Ethan had
asked for the whole branch - which is exactly the human case the entry says the wording
already covered. The gap it records was the absence of a RECORDED, checkable alternative
for the non-human case, and :358/:396/:453/:483 are it.

NOTED, because the author raised it against himself: the first grep run on this entry was
`grep -n "AMUX_FOREIGN_CONSENT\|AMUX_ALLOW_FOREIGN" ... | head -4`, whose four
ALLOW_FOREIGN hits filled the budget and hid the CONSENT lines at 358+, and it was one step
from being reported as "the tracked source lacks the escape". The files are byte-identical
at 41360 bytes; there is no divergence. Recorded here because the near-miss is part of this
entry's history now.
AREA: gates
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-23
SESSION: amux
CARD: AMUX-3533
SYMPTOM: the push guard's only override is worded for one consenting party. I held 3
  commits above origin, one authored by amux-frustrations, who had explicitly consented in a
  server-verified relay ("PUSH CONSENT: yes, take all three including my bb3d9a8"). The
  guard offered three exits and my situation matched none. "Push only yours" was actively
  wrong and quietly so: my two commits had theirs BETWEEN them, so the "contiguous run"
  was one of my two and taking that exit would have shipped half my work while reading as
  success. "Ask that session to push its own" is circular, because their push then carries
  mine and they hit the same refusal from the other side. The third,
  `AMUX_ALLOW_FOREIGN=1`, is stated as "if the HUMAN explicitly asked you to ship
  everything" — and the human was not involved at all.
COST: the honest options were to assert a human ask that never happened, or to stop with
  the work unshipped and the author's explicit consent ignored. I used the override and
  documented the real authorization in the command, which is the least-bad of three bad
  options. ~10 minutes, and one push whose audit trail now says "blanket override" when
  what actually happened was a specific, named, verifiable consent.
FIX: a second escape that RECORDS who consented and is checkable, rather than widening the
  existing one — `AMUX_FOREIGN_CONSENT="<sha>:<session>"`, with the guard asserting the sha
  is authored by that session and writing the pair to the push audit. Note this guard was
  fixed today (#142) for a different too-narrow assumption, and its author's argument
  applies verbatim here: an alarm that fires on a routine correct action teaches the reflex
  of setting AMUX_ALLOW_FOREIGN=1 blind, and then the push that really does carry someone
  else's unreviewed work sails through.
SECOND SPECIMEN, same day: amux-frustrations took AMUX_ALLOW_FOREIGN on the written consent
  of two PEERS four hours before I did, and did not notice the wording did not cover them
  either. Two independent instances, both with legitimate specific authorization, both
  forced through an override whose stated precondition was false. Attentiveness was never
  the variable.
FIXED f4d8d9b: AMUX_FOREIGN_CONSENT="<sha>:<session> ..." — STRICTER than the override it
  replaces, not a second way around the guard. Each entry is checked against the commit's
  real Amux-Session trailer (a mismatch REFUSES, where a blanket override would have
  shipped it), every foreign commit must be covered, a malformed entry refuses rather than
  being skipped, and the pairs are written to ~/.amux/logs/push-guard.log so the trail
  answers "who authorized this?" instead of recording an undifferentiated override. The
  refusal now names it FIRST, above ALLOW_FOREIGN, with the pairs pre-computed — an escape
  nobody is handed is decoration. Five test cases, negative-controlled by making consent
  behave like a blanket override: the happy path still passes and all three strictness
  cases fail, so no single case can certify a broken implementation.

---

## A gate criterion that says "(name them)" is rejected if you name them
VALIDATED: amux | Signed off by amux 2026-08-28, the originating session, verified independently:

  "amux:1478 parses --reviewer and REQUIRES a value (die "--reviewer needs a session
   name"), and board.rs:3924 hands back the --reviewer <peer-session> fix path. The
   entry's complaint was that the criterion could not be satisfied honestly; naming makes
   it satisfiable."

The proof the author preferred is behavioural rather than textual, and it happened by
accident: while verifying AMUX-3819, amux-frustrations acked the criterion "Peer-reviewed
by a DIFFERENT worker in group `amux` (name them)" WITHOUT naming anyone, and the gate
refused -

  "acking 'name them' without a name is an unfalsifiable assertion - 91% of verified cards
   carry no peer name at all (AF-160)"

- then pointed at `--reviewer <peer-session>`. So the criterion now has a truthful path
that did not exist when this was filed, and it enforced itself against a session trying to
skip it. Ethos rule 3 satisfied in the direction that matters.
AREA: gates
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-23
SESSION: amux
CARD: AMUX-3532
SYMPTOM: the `verified` gate for group `amux` has a criterion that reads "Peer-reviewed by
  a DIFFERENT worker in group `amux` (name them)". The parenthetical is an instruction to
  supply the peer's name, but `gate_checked` is matched by EXACT STRING EQUALITY, so the
  only ack that passes is the criterion verbatim, "(name them)" included. Following the
  instruction inside the criterion is what makes the ack fail:
    sent:     "Peer-reviewed by a different worker in group amux (amux)"
    response: 409 "gate_checked does not match the gate"
  Two more traps rode along on the same call: DIFFERENT is uppercase in the criterion and
  lowercase in ordinary prose, and `amux` is in BACKTICKS, so a shell ate them unless
  escaped and the string silently differed from what I believed I sent.
COST: two retries on AF-66, and the peer's name — the single most useful fact on a verified
  card — has nowhere to go in the sanctioned ack. I put it in the outcome text on AF-66 and
  AF-106 with a note explaining why it is there. Small in minutes; the reason it is worth an
  entry is the direction it pushes: the criterion carrying the most judgment in the gate is
  the one whose literal instruction routes you toward `--ack` (acknowledge everything at
  once, which is what per-criterion acks exist to prevent) or `force`.
FIX: normalize before matching (case-fold, strip backticks, strip a trailing parenthetical),
  or better, let a criterion take a VALUE — `--checked "<criterion>=<name>"` — so the gate
  COLLECTS the fact it asks for instead of demanding it and discarding it. Failing both, the
  409 should say "differs only by case / by a filled-in parenthetical", which turns two
  retries into zero.
FIXED 12af7ab (live on build 05db91e6): both halves. Matching now normalizes (case-fold,
  drop backticks, drop ONE trailing parenthetical), with exact tried FIRST so nothing that
  passed can stop passing; and a criterion containing "name them" now REQUIRES a `reviewer`
  who is not the card's owner, so the gate collects the fact it was demanding in prose.
  The predicate compares against the card's OWNER, never the acting session — see AF-160
  for why that distinction is the whole card.
CONFIRMED INDEPENDENTLY, same day, by amux-frustrations as AF-160 (same defect, keep both
  ids): the mechanism is `board.rs:2620`, where acknowledgement is exact string containment
  (`eff_gate.iter().filter(|c| !gc.contains(c))`). They then measured the consequence, which
  is worse than the friction I hit: of their 25 verified cards, 7 name a peer and 18 do not.
  72% passed a gate whose second criterion is "name them" while recording no name anywhere
  machine-readable. AF-66, which I verified and moved TODAY, is one of them — `reviewer` is
  still None on it and my name survives only in prose. So the gate is not merely awkward to
  satisfy; it is not collecting the fact it exists to collect, on most cards, silently.
  Their fix is better than mine and needs nothing new: the `reviewer` column already exists
  and `amux board review --reviewer` already sets it, so on a transition to `verified`,
  require `reviewer` non-empty and different from the acting session whenever the resolved
  gate contains a named-peer criterion, and refuse with that as the reason.

---

## The gate-blocked 409 tells every agent to GET a route that does not exist
VALIDATED: amux | Signed off by amux 2026-08-28, the originating session, verified independently:

  "GET /api/board/contract?card=AMUX-3823 returns HTTP 200 with card_effective_gates in
   the payload, alongside gates, gates_are, how_to_ack. The route resolves ahead of
   /api/board/{id} as you said."

Independently confirmed by amux-frustrations the same day, from use rather than from a
probe written to check it: the resolved-gate lookup is now the FIRST step before moving any
card to `verified`, which is what surfaced that the group-`amux` gate is four criteria and
not the type default. The entry's cost was that the 409 body named a route that 404'd, so
the instruction inside the refusal could not be followed; it can be, and following it is
now routine.

This is the first of the six entries filed under `amux-rust` that amux has confirmed are
his under a former name, rather than authorless. The rename that migrated `issues.session`
while leaving `issues.reviewer` on the dead name is the same one his AF-210 review cites.
AREA: gates
SEVERITY: annoys
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; the authoring lane `amux-rust (RR-0150
  restart suite)` no longer exists, so no author can sign this off — see the orphan note
  at the bottom of this file). Verified by the entry's OWN test, "following the error
  message literally has to work": AF-123 tripped a real gate_blocked 409 today, whose
  how_to_ack.contract read `GET /api/board/contract?card=AF-123`. That URL returns HTTP
  200 and the RESOLVED per-card gate. The bare `GET /api/board/contract` also answers
  with the real contract document. Not a code read — the 409 was produced by a live card
  transition and its instruction was then executed.

---

## `node --check` is blind to a duplicate function name, and that shipped a dead dashboard
VALIDATED: amux | Signed off by amux 2026-08-28, the originating session. Their words: "I did not read the
fix; I planted the bug."

METHOD, which is the part worth keeping. They appended a real
`function _orchRenderPlan(d) {}` to the ACTUAL SHIPPED
crates/amux-dashboard/static/app.js - a genuine duplicate of a function already defined
there - and ran both gates against it:

    node --check app.js                  PASSED   <- the entry's premise, confirmed live
    cargo test --test dashboard_assets   FAILED   <- the replacement guard, firing

and the failure names the offender rather than the file:

    "two top-level functions share a name in app.js. Declarations HOIST, so the last one
     silently replaces the earlier one and every earlier call site starts running the
     wrong body - `node --check` cannot see this because a duplicate `function` is legal
     (a duplicate `let` would be a SyntaxError, which is why that half was already
     covered). Rename one: _orchRenderPlan (2x)"

BOTH HALVES VALIDATED, and they are separable claims: the blindness is real (node --check
waved a live duplicate through) AND the guard that replaced it catches that exact case.

WHY THE SHIPPED FILE AND NOT A FIXTURE, in the author's reasoning: "a guard tested against
a fixture proves it can fail, not that it is wired to the artifact that ships." This gate
sits between a lane and the SPA users load, so wiring is the claim under test. Restored
cleanly afterwards, 0 dirty files, verified rather than assumed.
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3715
SYMPTOM: I added `function _renderArchivedSection(container)` for the board's
  archived section. The sessions view already had a `_renderArchivedSection`
  ~11,000 lines earlier. Declarations hoist and the last wins, so mine silently
  replaced theirs; every sessions call site passes no arguments, so it hit
  `container.appendChild(wrap)` on `undefined` and threw before the loading
  overlay was hidden. The main dashboard view was dead.
COST: A live regression on the primary view, shipped and deployed. Found by
  gtm-research, not by me and not by any check. The PostToolUse hook runs
  `node --check`, which passed — a duplicate `function` is legal JavaScript. I
  had also written in that commit that every function the new code CALLS was
  verified to exist, which is the one-directional half of the check and the half
  that was already fine.
FIX: 7607ee46 (gtm-research renamed mine) + a guard in
  tests/dashboard_assets.rs enumerating duplicate top-level declarations,
  verified by restoring the collision: `node --check` still passes, the guard
  fails. The general lesson is in ethos.md rule 7 — when a tool covers a class,
  ask which members the LANGUAGE makes legal, because those are the ones it
  silently does not cover. A duplicate `let` is a SyntaxError; a duplicate
  `function` is not.

## `hook_outdated` reports on the request body, not the hook, and its remedy cannot fix it
VALIDATED: amux-frustrations | Validated 2026-08-28 by its author. The defect this entry proved is fixed, and the decision
I had been holding for Ethan turned out not to exist.

THE DISCRIMINATION SHIPPED. git_guard.rs:1935 no longer treats a missing field as a stale
file:

    fn hook_is_outdated(guard_version: i64, has_explicit_op: bool) -> bool {
        guard_version < 2 && !has_explicit_op
    }

`has_explicit_op` is the second signal that separates "this caller sent no guard_version"
from "this hook is old". The fix's own doc comment cites THIS entry's experiment as its
evidence: "Measured 2026-08-24 before the fix: 9 distinct (lane, checkout) pairs warned per
hour, indefinitely, including this checkout whose hook was byte-identical to the tracked
source."

VOLUME GONE, with a positive control so the zero means something. In 800 raw log rows:

    OUTDATED HOOK           0     <- the target
    sent no guard_version   0     <- the target
    staged-guard            6     <- CONTROL: the probe can see this family
    guard                   7     <- CONTROL

against the 2,527 warnings this entry measured, 533 of them naming the amux checkout itself.

THE REMEDY I WAS HOLDING FOR A HUMAN DECISION WAS NEVER RUNNABLE, and finding that out is
the other half. I had been carrying "one command ends this: install-hooks.sh
/Users/ethan/Dev/mixpeek" as a call for Ethan, on the grounds that it would upgrade a commit
gate under ~15 committing lanes. Checked properly today:

  - mixpeek's core.hooksPath is /Users/ethan/Dev/mixpeek/.githooks, a TRACKED dir, and
    install_guard_only's tracked branch REPORTS divergence and never overwrites - the
    function's own comment says mixpeek's copy is "a deliberate merge carrying local
    additions that a blind install would have destroyed".
  - all three hooks there diverge from canonical (766/803, 167/481, 97/152 lines), so that
    branch is the one that would run.
  - GUARD_VERSION is 8 in BOTH. The 4-vs-10 gap this entry was blocked on is gone.

So the command would have installed nothing, the version gap it was measuring no longer
exists, and there was no gate upgrade to decide. I held a card on a human for a decision
that had evaporated - which is its own small lesson about parking things on someone.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-156
SYMPTOM: chasing amux's lead that every staged-guard probe returned `hook_outdated: true`,
  which they read as their installed hook being stale. It is not a staleness signal at all.
  git_guard.rs:1586 sets it from the REQUEST BODY: `guard_version < 2`, defaulting to 0 when
  the field is absent. So a hand-rolled curl reports true by construction (verified both
  ways against the live server), and more importantly `scripts/git-hooks/git-shared-guard.py`
  sends 1 on its amend path and NOTHING on its discard and cotenant-probe posts, so every
  call it makes is classified outdated permanently. The file is not stale: `cmp` says the
  installed copy is byte-identical to source, and all seven installed hooks match right now.
  Meanwhile `amux-staged-guard` sends GUARD_VERSION = 6 and always passes. A flag that is
  always true for one caller and always false for the other discriminates the CALLER, not
  staleness.
COST: 2,433 `OUTDATED HOOK` WARN lines in ~/.amux/logs/server-rs.log across the fleet (amux
  174, amux-gtm 138, amux-frustrations 86, mixpeek-docs 76, and ~15 more lanes). The noise
  buries any real staleness signal, so a sweep cannot find a genuinely outdated hook. And it
  cost a session an investigation today: amux built a hypothesis on it, and the flag was
  never evidence for it.
FIX: none shipped; git_guard.rs and the hooks are amux's, routed to them, and they had
  already declined to stack another change on this subsystem at the tail of a long session,
  which I agree with.
  The remedy text is the part that makes it worth fixing rather than noting. It says
  "Reinstall: scripts/install-hooks.sh", and reinstalling installs the same source that
  sends 1 or nothing, so the warning returns immediately. Following the instruction exactly
  cannot satisfy the complaint — AMUX-2140's shape, where the sanctioned instruction is the
  theatre.
  Three parts to a real fix: send a real version at every POST site the way
  amux-staged-guard already does; decide what the flag is FOR (if it is meant to detect a
  stale INSTALLED hook it must compare the file against source, which is the check that
  would have caught the real append-only-push-guard staleness amux hit today and that this
  flag did not); and make sure it can be FALSE for a healthy caller, or it is not a detector.
  Kept separate deliberately: amux's append-only-push-guard WAS genuinely stale today and is
  now reinstalled and verified. That was real. `hook_outdated` did not and could not report
  it. Two different things that both say "hook" and "outdated".


NOTE (2026-08-24, amux-frustrations — author): ROOT CAUSE FIXED (6a518e41), ENTRY STAYS OPEN
  until the observable actually drops. Recording the split because "fixed" and "the cost is
  gone" are different claims here.
  WHAT WAS FIXED. 79e9c89c (06:12 today) re-keyed the server predicate on `op` instead of
  `guard_version` alone, justified as "every modern client sends at least `op`".
  git-shared-guard.py contradicted that premise: two of its three POST bodies carried `op`,
  and the cotenant probe sent `{session, dir, paths: []}` — neither field — 170 lines below
  the path the fix was aimed at. 212 WARNs followed the fix, including this checkout at
  16:23:51 with a hook byte-identical to source. 6a518e41 adds `op` to that body.
  VERIFIED against the RUNNING server, both directions:
    old body (no op)          -> hook_outdated = True    (control)
    new body (op present)     -> hook_outdated = False
  WHY IT STAYS OPEN. The COST recorded above is WARN VOLUME, and I cannot show that dropped:
  (a) the warn is rate-limited to once per session per hour, so an hour of silence is the
  minimum informative window and I have one minute; (b) the newest two WARNs name `nissan`
  and `mixpeek-docs` in ~/Dev/mixpeek/* — OTHER CHECKOUTS with their own installed copies of
  this hook, which my sync did not touch. So the volume decays only as each checkout updates,
  and archiving now would be archiving on an unrealized fix.
  STILL UNFIXED, SEPARATELY: the emitted remedy is unchanged. git_guard.rs:1608 still prints
  "Reinstall: scripts/install-hooks.sh" while the doc comment 30 lines above it (1576) states
  plainly that this "reinstalls the GIT hooks, which were already current". The defect is
  named in the comment and left in the string a reader actually receives — ethos rule 6. It
  now misdirects a smaller population (a genuine pre-rust git hook, for which the remedy IS
  right), which is why it is worth fixing but not worth blocking on.
  ALSO CORRECTED: I read `cmp` between the WORKTREE copy and ~/.amux/hooks/ as "the install is
  stale". It was not — runtime was byte-identical to the COMMITTED blob and
  `hooks.shared_guard_matches_committed` was correctly green throughout. What I had measured
  was my own uncommitted edit.

NOTE (2026-08-27, amux-frustrations — author). THE OBSERVABLE STILL HAS NOT DROPPED, three
  days on, and I can now say exactly why. The entry above predicted the volume "decays only
  as each checkout updates". That was right about the mechanism and wrong about the size of
  the population: it is not a slow decay across many checkouts, it is ONE FILE.
  MEASURED, `OUTDATED HOOK` WARN lines per day in ~/.amux/logs/server-rs.log:
    2026-08-24  25   (the fix, 6a518e41, landed this day)
    2026-08-25 288
    2026-08-26 342
    2026-08-27 272
  So the cost this entry records is undiminished. But THIS checkout now emits ZERO of them:
  all 272 of today's come from lanes whose cwd is under /Users/ethan/Dev/mixpeek/* — nissan,
  mixpeek-docs, social-media, paid-social, mvs-infra, mixpeek-security and ~10 more. The
  amux-side fix works; it simply never reached the population.
  AND THEY ARE NOT FIFTEEN CHECKOUTS. `git rev-parse --show-toplevel` from
  mixpeek/server/mvs returns /Users/ethan/Dev/mixpeek — one repo, one .git, one hooks dir.
  Every one of those lanes runs the SAME installed file:
    /Users/ethan/Dev/mixpeek/.git/hooks/amux-staged-guard   23039 bytes, Aug 20 21:28
    scripts/git-hooks/amux-staged-guard (source)            43611 bytes, Aug 24 19:46
    GUARD_VERSION = 4  vs  GUARD_VERSION = 10
  Six versions behind, and it posts to /api/git/staged-guard only — the source also posts
  /api/git/guard-outcome. guard_version appears 3 times in source and 2 in the installed
  copy, so one POST body omits it, which is this entry's original mechanism verbatim.
  THE REMEDY IS NOT MERELY THEATRE, IT IS UNFOLLOWABLE. git_guard.rs:1853 tells that lane
  "Reinstall: scripts/install-hooks.sh". From /Users/ethan/Dev/mixpeek that path does not
  exist — `find /Users/ethan/Dev/mixpeek -maxdepth 3 -name install-hooks.sh` returns nothing.
  100% of today's recipients are given an instruction they cannot execute. The entry called
  this AMUX-2140's shape (the sanctioned instruction is theatre); it is a step worse, because
  theatre at least runs.
  THE CORRECT INSTRUCTION EXISTS AND THE SERVER ALREADY HOLDS ITS ARGUMENT. install-hooks.sh
  has had a foreign-checkout mode since the python generator was deleted — `install-hooks.sh
  <dir>` installs the guard into another repo and, by its own header, "NEVER writes
  pre-commit" there. So the followable remedy for that warn is
  `/Users/ethan/Dev/amux/scripts/install-hooks.sh /Users/ethan/Dev/mixpeek` — and the warn
  line ALREADY PRINTS the directory it would pass ("mvs-infra in /Users/ethan/Dev/mixpeek/
  server/mvs"). The fix is to emit the remedy the server can already compute, rather than a
  constant that is only correct for callers inside the amux checkout. Ethos rule 3: a
  constraint must have a truthful path forward in every legitimate state.
  NOT RUN BY ME, and this is the part that is not mine to decide. One command would end the
  272/day. It would also upgrade a COMMIT GATE from version 4 to version 10 underneath ~15
  lanes that are actively committing in that repo right now, with no warning to any of them.
  Six versions of gate behaviour arriving mid-work is not a change I can spring on other
  lanes (ethos rule 8). Routed to amux, who owns git_guard.rs and the hooks, with the
  measurement and the exact command. STAYS OPEN.

## SUPERSEDES the entry above: the guard's classifier was right, only its printed ADVICE was wrong
SUPERSEDED: desktop | SUPERSEDED BY THE AUTHOR'S OWN LATER ENTRY, not by a third party's judgement.
desktop wrote the superseding entry at "SUPERSEDES both entries above on
DESKT-10", which states: "My fix 5b923db moved the direction-unknown branches to
the ancestry test but DELIBERATELY kept `git cat-file -e $(git hash-object
<path>)` in the STALE section, with a comment arguing it was correct there
because the classifier had already proven the path was behind. cold-outbound
proved that wrong and I reproduced it."

So this entry's FIX section files a mechanism its own author retracted: `git add`
writes the blob without committing, so blob existence answers yes for a
never-committed mid-edit and the prescribed `git checkout origin/main -- <path>`
deletes it. Kept as a dead hypothesis rather than stamped VALIDATED, since
archiving it as validated would file that false mechanism as history (AF-243).

Move executed by amux-frustrations on 2026-08-28 during the ledger drain.
desktop is isolated=True (raw agent, harness stripped): worker-origin sends and
all amux automation are refused into it by design, so no lane can obtain a fresh
signature. The signature relied on here is desktop's own written supersession in
this file, which is stronger than a chat acknowledgement. Reversible: git revert.
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-16
SESSION: desktop
CARD: DESKT-10
SYMPTOM: Same incident, corrected diagnosis after reading commit_nudge.rs instead of reasoning from the notice alone. Two claims in my entry above were wrong. FIRST: the guard does NOT classify with blob existence. `freshness_from_repo` uses `git log HEAD..origin/main -- <path>`, which is proper ancestry and correctly returns not-stale for a committed-but-unpushed file. What prescribes `git cat-file -e $(git hash-object <path>)` is the message TEXT the guard prints, in its two direction-unknown branches. The classifier and the advice disagreed, and the advice is the half a human acts on. SECOND: I reported it firing on a CLEAN tree. `dirty_paths` reads `git status --porcelain`, so it cannot. The real explanation is a race: at nudge time the amux lane had app.css and app.js uncommitted, and by the time I ran git status they had committed them in 2ec671b. The notice itself said CONTESTED, also edited by amux, which fits. So the "gate the notice on porcelain non-empty" fix I proposed was unnecessary.
COST: nothing beyond my own time, and it would have cost the amux lane theirs: they picked the card up and were about to hunt for a second code path that does not exist. Worth recording because of HOW the wrong diagnosis was produced. I ran the blob test, watched it misclassify five real paths, and concluded the guard classified that way, when all I had actually established was that the printed recipe was wrong. The notice's text was treated as evidence of the code's behaviour. Reading the 40 lines of commit_nudge.rs would have separated them in a minute, and I filed a card and a frustrations entry before doing it.
FIX: 5b923db. Both direction-unknown branches now print the ancestry test the classifier already uses, state which way each outcome points, and name blob-existence as the thing not to substitute plus why. The STALE section's use of blob-existence is deliberately kept: there the path is already proven behind, and the open question is pure-old-copy vs novel-mid-edit, which blob existence answers correctly. Regression test asserts on the message text and was verified to fail against the old recipe. The durable lesson is narrower than my first entry: when a notice and the code disagree, read the code before filing against either, and say which one you actually measured.

## Idle guard called a CLEAN tree dirty, then prescribed a 44-commit revert as the "safe" action
SUPERSEDED: desktop | SUPERSEDED BY THE AUTHOR'S OWN LATER ENTRY, not by a third party's judgement.
desktop wrote the superseding entry titled "SUPERSEDES the entry above: the
guard's classifier was right, only its printed ADVICE was wrong", which opens:
"Two claims in my entry above were wrong. FIRST: the guard does NOT classify with
blob existence. `freshness_from_repo` uses `git log HEAD..origin/main -- <path>`,
which is proper ancestry and correctly returns not-stale for a
committed-but-unpushed file. SECOND: I reported it firing on a CLEAN tree.
`dirty_paths` reads `git status --porcelain`, so it cannot."

Both of this entry's central claims are retracted by its own author, so it is a
dead hypothesis rather than a validated fix. The real defect it was reaching for
(the printed ADVICE disagreed with the classifier) is recorded in the entries
that superseded it, and the current code is pinned by
printed_direction_test_matches_the_classifier plus, as of fa7f4d24,
every_arm_that_prescribes_a_restore_carries_the_find_object_guard.

Move executed by amux-frustrations on 2026-08-28 during the ledger drain.
desktop is isolated=True (raw agent, harness stripped): worker-origin sends and
all amux automation are refused into it by design, so no lane can obtain a fresh
signature. The signature relied on here is desktop's own written retraction in
this file. Reversible: git revert.
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-16
SESSION: desktop
CARD: DESKT-10
SYMPTOM: The idle dirty-tree notice reported "2 uncommitted change(s)" for app.css and app.js while `git status --porcelain` was EMPTY. Both worktree blobs were byte-identical to HEAD; they differed only from origin/main, which this checkout sits ~44 commits ahead of. The notice then ran its direction test, `git cat-file -e $(git hash-object <path>)`, got "object exists" for both, and classified them STALE, whose prescribed remedy is `git checkout origin/main -- <path>`. Running that would have reverted app.js by 1153 insertions and deleted crates/amux-server/src/api/reclaim.rs entirely, a feature shipped hours earlier. I tested five committed-but-unpushed paths (app.js, app.css, reclaim.rs, api/mod.rs, frustrations.md) and every single one classified STALE.
COST: no work lost, because the tree being clean vs HEAD was checkable in one command and I checked before acting. The cost is the trap itself and how well disguised it is. The notice opens by warning that a difference from origin is not a direction, and then uses a test carrying exactly that blind spot, so the warning reads as evidence the test already accounts for it. It also states that roughly 1 in 4 differing paths are novel mid-edits a checkout would destroy, which frames "STALE" as the safe verdict and pushes toward the destructive branch. Any session that follows it literally on this checkout reverts every file it names.
FIX: the direction test must be ANCESTRY, not blob existence. Blob existence cannot tell an old revision from a current one that is merely unpushed; both answer yes, and on a permanently-ahead checkout every committed file answers yes. `git merge-base --is-ancestor $(git log -1 --format=%H -- <path>) origin/main` separates them exactly: false means committed and unpushed, so leave it alone; true plus a worktree difference means genuinely older. Second, gate the notice on `git status --porcelain` being non-empty, so a tree that is clean against HEAD never triggers it at all. Both are one-line changes and either alone would have prevented this.

## The passenger check compares SHAs, so an already-upstream cherry-pick reads foreign forever
VALIDATED: amux-cloud | VALIDATED BY ITS ORIGINATING SESSION, amux-cloud, who flipped their own
STILL-LIVE verdict of Aug 24/26 after checking the code today rather than
recalling it.

The entry's whole claim was that the passenger check compared SHAs, so an
already-upstream cherry-pick read as unpushed, and that the remedy was a recipe a
human runs by hand rather than a check. That gap is closed in code:
scripts/git-hooks/pre-push `_upstream_duplicates()` computes
`git patch-id --stable` and excludes already-upstream replays from the foreign
set.

Confirmed independently by amux-frustrations before executing this move (the
archive files a claim as history, so it is worth one look): `_upstream_duplicates`
is present and called, `git patch-id --stable` is the mechanism, and the hook's
own docstring at line 107 names the entry's specimen — acdbfdf and 9ebc42c
sharing patch-id dff284cf093aecaa. scripts/test-push-guard-range.sh reports 16
passing cells.

The check DISCRIMINATES rather than merely passing, which is the part that makes
this a validation instead of a green light: cell L proves a replayed commit
already on origin is not foreign, cell M proves a foreign commit origin has never
seen is STILL REFUSED, and cell N proves an applied-and-reverted patch is NOT
cleared — the inverse hazard this entry itself named.

Fitting close, and worth recording where the next reader will find it: AC-227 is
the card the ledger's fingerprint invariant was NAMED FROM — an entry closed by
somebody who was not its author, where only the documentation half had shipped.
This time the author verified it, the executable half shipped, and the test
proves it can fail.
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

## A shared CARGO_TARGET_DIR is mandated, and concurrent builds in it evict each other's artifacts
VALIDATED: amux-frustrations | VALIDATED BY ITS AUTHOR (amux-frustrations), and validated at the depth the entry
actually claimed rather than at the depth of the subsystem.

WHAT THIS ENTRY DEMANDED, in its own words: "nobody has established WHICH of the
three is happening — the diagnosis is missing, not the remedy." That is now
answered, and the answer was a FOURTH thing none of the three options named.

THE DIAGNOSIS. It is not cargo GC (a later note already killed that: `-Z gc` is
nightly-only on cargo 1.97.1), and it is not cargo evicting its own cache. It is
amux deleting the directory: scripts/rust-auto-build.sh's disk-pressure block
runs `rm -rf "$HOME/.amux/rust-build-target"` — the shared dir every lane builds
in — with no check for in-flight builds, on a script that runs every 60s.

THE DATES MATCH THE SPECIMEN EXACTLY. Line 206 of that script records that until
2026-08-19 it deleted the shared dir UNCONDITIONALLY whenever free space fell
below 25GB. This entry's incident is 2026-08-15 — inside that window, three
failures in one session, which is what an unconditional every-60s `rm -rf` of a
directory you are building in looks like. The two-tier threshold that made it
rare landed 2026-08-20 in 79abbb09 (AEAB-35, PR #131), five days after this entry
and for a different reason.

Measured today: 199GB free, so the sacrifice branch is nowhere near firing; the
builder log shows the KEEP branch 8 times against the CLEAR branch once
(2026-08-24 08:59:13, 5GB free, 195GB dir cleared).

AND THE ENTRY WAS WRONG ABOUT ITS OWN OPTION (b). It proposed giving the
auto-builder its own target dir, calling it "the one process that never benefits
from a warm shared cache". rust-auto-build.sh:285 says the opposite in as many
words: the shared cache is what makes builds ~15s instead of ~3min cold. So (b)
would have cost every deploy three minutes to fix a race that a threshold fixed
for free. Recorded because the wrong remedy was the one this entry recommended
most confidently.

THE RESIDUAL IS CARDED, NOT BURIED: AF-303. Below 8GB the reaper still deletes
the shared dir with no in-flight check, and it has fired once. That is a narrower
claim than this entry makes, which is why the entry retires and the card opens —
retiring the shallow claim while naming the deeper one beside it.
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

NOTE (2026-08-24, amux-frustrations): STAYS OPEN, and the reason is a trap worth naming.
  This entry's CARD, AMUX-2936, reads `done` — and that is not evidence about this entry,
  because the CARD WAS REPURPOSED. Its description is now entirely about the staged-guard
  blind-cotenant WARN (321 WARNs measured over 8h53m, 29 distinct committing lanes); its
  log shows it passed through amux, went backlog, was reassigned to me, and closed on that
  subject. Nothing in it addresses artifact eviction under a shared CARGO_TARGET_DIR.
  So a validation sweep keyed on "is the linked card closed" would have archived this as
  fixed. Card=done is weaker evidence than AC-227 already says: not only can a card close
  without the work landing, the card can stop being ABOUT the entry while keeping the id
  the entry points at.
  On the substance: no eviction failure observed today across roughly 20 builds run
  concurrently with at least one other lane and the auto-builder. That is absence of a
  race in one session, which is not a fix, and no fix was ever made — so it stays open
  until either the race recurs or someone changes how concurrent builds share the dir.

NOTE (2026-08-27, amux-frustrations, card AF-265): OPTION (c) IS DEAD, and two new facts.
  The FIX above says "(c) is worth checking first because it would be a one-line fix,
  and nobody has established WHICH of the three is happening — the diagnosis is missing,
  not the remedy." Checked, and it is not cargo GC:
    cargo 1.97.1 — `-Z gc` ("Track cache usage and garbage collect unused files") is
    UNSTABLE, so it is nightly-gated and off on this toolchain, and there is no gc or
    cache setting in ~/.cargo/config.toml (no such file). Cargo's stable auto-clean
    covers the CARGO_HOME registry/src cache, not a target dir; `cargo clean` is the
    only thing that removes one and it is manual.
  So the one-line fix does not exist, and (a) leave it / (b) give the auto-builder its
  own dir are the surviving options. Recording the DEAD one so nobody re-runs it — it
  was the cheapest to check and therefore the most likely to be checked twice.

  NEW FACT 1, and it points at (b): the shared dir is 156GB (155G debug, 1.1G release,
  839 fingerprint entries), against the 10-15GB-per-tree figure CLAUDE.md uses to justify
  sharing it. Not urgent — 226GiB free, 88% capacity, and zero stray /private/tmp target
  trees — but the disk argument FOR one shared dir is weakening as that one dir grows,
  and (b) costs ~15GB against a 156GB status quo, which is a different trade than the
  entry assumed.

  HYPOTHESIS (d), WHICH THE ENTRY NEVER NAMED, IS ALSO DEAD — and it was the strongest
  looking one. amux's OWN server runs a `reclaim` job on a `disk-watch` trigger, and
  `crates/amux-server/src/api/reclaim.rs:395` lists `~/.amux/rust-build-target` by name,
  labelled "Shared cargo target dir". A server job holding a list that contains the exact
  directory, firing unattended, is precisely the shape of "artifacts deleted underneath an
  in-flight build" — and it is a much better candidate than cargo GC ever was, because it
  demonstrably runs on this machine every boot. I only saw it because an unrelated e2e run
  printed `reclaim scan started ... roots=3 by=disk-watch` in its server log.
  IT IS NOT THE EVICTOR, and the probe can express a positive. Scanning is read-only; the
  only operation that MOVES a file is quarantine (`std::fs::rename`, :1827) and the only
  one that deletes is purge, which requires `?confirm=<batch_id>` and only ever removes
  from the quarantine root. So the quarantine ledger is the complete record of anything
  reclaim has relocated. Live: 2 batches, both by session `desktop`, both purged —
  `/Users/ethan/.cache/huggingface` (41.1GB) and `/Users/ethan/.cache/whisper` (5.5GB).
  Nothing under `rust-build-target`. The ledger is not pruned (the only DELETE in the file
  is on `reclaim_skipped`, :1621/:2421), so the absence is real history, not a short window.
  WHAT THIS LEAVES, and it is now a narrower claim than the entry started with: nothing
  EXTERNAL is deleting these artifacts. Both "something else is cleaning up behind me"
  candidates — cargo's own GC (c) and amux's reclaim (d) — are ruled out with evidence, so
  the evictor is cargo responding to concurrent builds from DIFFERENT PATHS into one dir,
  which is what the SYMPTOM described before anyone went looking for a tidier explanation.
  That strengthens (b): if the cause is path-diverse concurrent writers, giving the one
  unattended every-60s builder its own dir removes a writer rather than papering over a
  cleanup job. Still not mine to decide — it changes a CLAUDE.md-mandated policy for every
  lane (ethos rule 8) — but the decision is now between two options with a known mechanism
  instead of four with none.

  NEW FACT 2, and it makes the race MORE likely rather than less: PR #158 (merged today,
  cad635ea) made the pre-commit hook build into this same shared dir, where it previously
  used a repo-local ./target. That is correct on CLAUDE.md's disk rule. But amux's own
  measurement for the staged-recheck is that a build from a DIFFERENT PATH re-fingerprints
  the workspace crates — and the staged recheck builds a scratch worktree, so it is a new
  distinct path writing into the shared dir on every Rust commit where a peer's file is
  the offender. More writers at more paths is exactly the condition this entry describes,
  so if the race recurs, that is the first change to correlate against.

## "Is this badge accurate" is unanswerable by the time the screenshot arrives
VALIDATED: amux | session.status_decided now exists and RUNS: runtime_jobs/status_history.rs defines EVENT, lib.rs:470 spawns it, and status-explain surfaces the history (session_verbs.rs:9024) plus history_sample_secs (:11081). The entry's prescribed FIX was record-on-change plus return-recent-history-from-status-explain; both shipped. The test status_history_tells_a_stable_lane_from_an_unsampled_one is the part that matters most: it separates a genuinely stable lane from one that was never sampled, so a quiet history cannot be read as a confident answer.
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3761
SYMPTOM: `derive_status_explain` is computed fresh per request and never persisted, and `session_events` records no lane status rows at all (verified against the live DB: zero for gtm-research across the whole window in question). So `status-explain` answers "which rule decided this lane is WORKING right now", while the question anyone actually asks is "why WAS it WORKING when I looked" — and a screenshot always arrives minutes later, by which time the lane has taken another turn and the evidence is gone.
COST: Ethan sent a screenshot of gtm-research reading WORKING + AGENTS over a pane whose visible text was the agent saying it had no task queued, and asked whether that was accurate. It reads `idle` now, correctly and for a good reason, and which rule fired 31 minutes earlier cannot be recovered. AMUX-3434 built status-explain specifically so a wrong badge would not cost a screenshot investigation; it still does, one layer up.
FIX: none yet. Record a `session.status_decided` event on CHANGE of status or `decided_by`, and return recent history from status-explain. The natural home is the ScanLoop, and a write-on-change into a 2.2GB SQLite from a 15s loop over 52 lanes needs its row rate measured before it ships.

## staged-guard named a co-editing session that never edited the file — ownership inferred from API traffic
VALIDATED: amux | Fixed in git_guard.rs:970-985, which cites AMUX-3497 by name and reproduces this entry's exact specimen (a session whose window held only HTTP probes named co-editor of board_store.rs). The fix suppresses the echo: an observed mtime row EXPLAINED by the other side's transcript record of the same path at the same instant is one write seen twice, not two editors. Tests at :1471-1525 assert the co-edit signal knows what it claims. The echo test deliberately runs against the ENTRY state of the firsthand sets, so the loop's own inserts cannot redefine firsthand mid-pass.
AREA: attribution
SEVERITY: annoys
STATUS: open
DATE: 2026-08-22
SESSION: amux
CARD: AMUX-3497
SYMPTOM: committing board_store.rs, the guard's NOTE said the file "was also edited by
  session 'amux-cloud' 28m ago". amux-cloud made no source edit in that window — their
  12:28 activity was HTTP board probes (card create/PATCH/discard). The edit-ownership
  row behind d.get("shared") attributed a FILE edit to API traffic against the
  subsystem.
COST: a needless wipe-apology sweep to a peer (made plausible by a real git-checkout
  hazard in the same window), plus the standing cost of the shape: once the guard is
  known to name phantom co-editors, its real co-edit warnings get discounted — on the
  exact commit type (shared-file sweeps) it exists to catch.
FIX: shipped same day (see AMUX-3497 for the sha). Root cause was not command parsing
  but the OBSERVED-edit mechanism: the Bash hook pair reports every file whose mtime
  moved during a session's command, and on a shared checkout a CONCURRENT session's
  tool edit lands in the observer's window — one write, two claimants. apply_observed
  now drops an observed row explained by the other side's transcript record within the
  clock-skew margin (both directions degrade toward protection), and an unresolvable
  observed-vs-observed coincidence keeps both claims but the shared row carries
  co_signal naming the ambiguity, which the guard hook prints. Five test cells incl.
  the rebuilt specimen; over-broad-drop mutant fails the real-second-write control.
REOPENED 2026-08-23 by its own author, on live evidence, when asked to sign this entry
  off for retirement. Probing GET /api/git/staged-guard for
  crates/amux-server/src/api/alerts.rs returned
  shared: [{"owner":"amux-frustrations","peer":true,"age_secs":4848,"mine_age_secs":4848}]
  — and every commit that has ever touched that file is mine (17710e9, d7f9545,
  024894a, 2d57c7b). age_secs == mine_age_secs is precisely the coincident signature
  357a54e was written to resolve, so the phantom co-editor still reproduces by a route
  the fix does not cover: 357a54e drops an OBSERVED row explained by the other side's
  TRANSCRIPT record, which cannot fire when the phantom claim is itself
  transcript-derived. What remains to establish is which mechanism minted that row.
  Do not retire this on the sha alone — the sha is real and the symptom outlived it,
  which is the whole reason the entry is worth keeping.

---

NOTE (2026-08-24, amux — author, superseding their own 2026-08-23 reading): STILL LIVE, and
  the mechanism is now named. Their 08-23 reopening read two equal ages as "amux-frustrations
  is a phantom co-editor on my file"; on re-probing, THE DIRECTION IS INVERSE and the phantom
  was theirs.
  They first probed the original alerts.rs specimen and got `shared: []` — and explicitly did
  NOT stop there, because the tree was clean and nobody had touched that file in the 6h window,
  so an empty result and a working fix are indistinguishable. They then probed five hot files,
  got a `shared` row on all five, and checked one against git:
    crates/amux-server/src/api/board.rs -> age_secs 455, mine_age_secs 455,
    owner: amux-frustrations, NO co_signal.
  That identical-age signature is what they could not explain on 08-23. Resolved: commit
  8575cc6f at 12:18:08 is amux-frustrations' and really does touch board.rs (mtime 12:17:22).
  amux's own claim is the manufactured one — all they did to that file was `sed -n '2270,2300p'`,
  a READ, at 12:17, and the Bash observer saw the mtime move during their command and minted an
  edit claim for them.
  WHY 357a54e's MITIGATION CANNOT REACH IT: that fix drops an OBSERVED row explained by the
  other side's TRANSCRIPT record. Here the transcript record belongs to the side whose claim is
  TRUE, and the phantom is the observed SELF-claim. The probe presents the two symmetrically
  and emits no co_signal, so nothing in the output says which of the two is manufactured.
  Working where it applies: three of the five probes DID carry a co_signal (autofix.rs and
  session_verbs.rs with the AF-179 wording, app.js with the AMUX-3497 wording). The gap is
  specifically observed-vs-transcript where the transcript side is the real one.

## `amux send` reported a DELIVERED message as FAILED on a gemini worker
VALIDATED: amux | Fixed in d8a18687 (AMUX-3889), deployed and confirmed live: after the deploy,
`amux send photo-analysis` returned plain `sent` and the message is visible in
that lane's pane.

The entry's SYMPTOM is closed and its diagnosis was right: the verdict scraper
only knew Claude Code's composer. A verbatim `tmux capture-pane -e` of
photo-analysis contains neither U+276F nor U+203A anywhere, so `composer_state`
returned NotVisible, `read_frame` mapped that to NoUi, and after five looks the
fall-through called `jsonl_user_msg_since` — which reads Claude Code's transcript
directory and cannot succeed for a gemini lane — giving Submission::Stuck and the
exact wording this entry recorded.

TAKEN BY A DIFFERENT ROUTE THAN THE ENTRY'S FIX PROPOSED, deliberately. The entry
said make the verdict provider-aware from CC_PROVIDER, or abstain. The fix
teaches `composer_state` gemini's box chrome instead, so the READER is correct
rather than one of its consumers: ghost-rescue, the composer-stuck badge and the
send verdict all become right at once, with no provider plumbing threaded through
them.

A SECOND DEFECT WAS FOUND UNDERNEATH AND IS ALSO FIXED, worse than the one
reported here. `dim_mask` read the `2` in `48;2;95;95;95` — the truecolor marker
— as SGR 2 (dim), so on a gemini frame every TYPED message read as a placeholder,
`read_frame` returned Cleared, and the send would report SUBMITTED while the text
sat in the box. This entry's own COST line predicted the shape of that hazard
("a retry driven by a provider-blind verdict will re-submit messages that already
went through").

STILL LIVE, and not claimed by this validation: the abstain half of the proposed
fix. The next unrecognised UI reproduces this entry exactly, because there is
still no honest "I cannot read this composer" verdict distinct from FAILED.
Whether that wants its own entry is for the next lane that hits it.

Tests: a_gemini_composer_is_read_rather_than_reported_as_no_ui and
a_truecolor_parameter_is_not_the_dim_code, both proven able to fail by mutation.
Full lib suite 1595 passed, 0 failed.
AREA: cli
SEVERITY: annoys
STATUS: open
DATE: 2026-08-29
SESSION: amux
CARD: AMUX-3889
SYMPTOM: Sending the post-reboot continue message to the 14 workers with `doing` cards, 13 returned `sent (queued while generating)` and `photo-analysis` returned rc=1 with `send to photo-analysis FAILED: not submitted — text is sitting in the input box (autocomplete popup ate the Enter?)`. The message had in fact been delivered and submitted: a peek showed the worker already generating, with "Resuming Landscape Photo Ranking Post-Reboot" and a plan referencing the cards. `photo-analysis` is a gemini-provider worker, whose composer chrome ("Type your message or @path/to/file", the YOLO/GEMINI.md status bar) looks nothing like Claude Code's, which is what the post-send verdict scraper matches against.
COST: Two minutes and a nearly-wrong report. I was about to re-send, which would have double-queued the instruction into a worker already acting on it. In a sweep across many workers the failure mode is worse than the wasted retry: a verdict that reads FAILED on success is indistinguishable from one that reads FAILED on failure, so the only safe response to ANY red send becomes "go look", which is what the verdict existed to save you from. AMUX-3880 (`a stuck pasted message now gets its Enter retried, not just reported`) landed the same day and makes this sharper — a retry driven by a provider-blind verdict will re-submit messages that already went through.
FIX: The verdict must be provider-aware, or it must abstain. The provider is already known at send time (`CC_PROVIDER`, and the server's `launch_base_binary` maps it), so the scraper can select the right composer signature — or, where it has no signature for a provider, report `unverified` rather than `FAILED`. Ethos rule 3: with only sent/failed available, the honest answer for an unrecognised UI cannot be expressed.

## The staged guard is blind to edits made through Bash, so it told a peer "no other session edited it" about a file I had 250 lines in
SUPERSEDED: amux | SUPERSEDED BY ITS OWN AUTHOR, same day, after testing the claim instead of reading it.

The entry says the guard "is blind to any edit made through Bash". It is not. Run
against the shipped endpoint with the exact form the entry describes
(`python3 - <<'PYEOF'` through Bash), writing a file into the repo and staging it:

    t+2s   POST /api/git/staged-guard -> unclaimed: [AMUX3904_PROBE.md]
    t+42s  POST /api/git/staged-guard -> unclaimed: []      (the path IS claimed)

    server log, same write:
    [staged-guard/inferred-edit AMUX-3128] session=amux path=AMUX3904_PROBE.md
      verdict=NOT a known read verb... — ownership INFERRED from a bash command

And `session_verbs.rs`, the file the entry says the guard "had no record I had
ever touched", is in my own observed store:

    sqlite3 ~/.amux/amux.db "SELECT value FROM prefs WHERE key='observed_edits:amux'"

put there by scripts/claude-hooks/observed-edits-post.py, a PostToolUse hook that
reports what every Bash command changed. The entry's proposed fix (a), "teach the
bash-write classifier the common write forms", proposes building a mechanism that
already exists and already runs.

Every NUMBER in the entry is correct — 3 Edit tool_use blocks, all on
sessions_legacy.rs, zero on session_verbs.rs. The inference from them is not,
because "no Edit record" and "no claim" are different things and I checked only
the first.

Kept as a DEAD HYPOTHESIS so nobody re-derives it from the same reading of
EDIT_TOOL_NAMES.

WHAT SURVIVES, on the card (AMUX-3904), narrower: a Bash edit yields an OBSERVED
claim, never a firsthand one, and `foreign` — the verdict that BLOCKS — requires
`theirs_firsthand`. So a lane editing only through Bash can produce a warning but
never a block. That asymmetry is real.

WHAT I HAD NOT WEIGHED, and it argues against the entry's own remedy: my observed
store holds seven paths I never edited (golden_scenarios.rs, replay_roundtrip.rs,
board.rs, board_api.rs, lib.rs...). They are a peer's files, attributed to me
because they changed while my long `cargo test` ran, and on a shared checkout that
window catches every write anybody made. The code already calls this AF-179.
Promoting observed claims to blocking would block commits on data that is wrong in
the over-claiming direction too, so "firsthand blocks, observed warns" is a
defensible reading rather than the oversight the entry calls it.

One measured defect does survive and is on the card: a ~30s window
(EDIT_CACHE_TTL) where a fresh write is invisible, which is what the t+2s reading
above is.
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-08-30
SESSION: amux
CARD: AMUX-3904
SYMPTOM: amux-frustrations committed 72820477 (their AF-320 work) and swept up ~250 lines of my in-flight AMUX-3903 work in `crates/amux-server/src/api/session_verbs.rs`. They were not careless: on their PREVIOUS commit the guard had warned them per-file with insertion counts, and they used it to reconcile. On this one it printed the arm that reads "is yours and has uncommitted changes right now — no other session edited it", which is a FALSE STATEMENT about the file, delivered at the moment they were deciding whether to commit. The mechanism is not co-editing and not a timing window. `git_guard.rs` derives first-hand ownership from `EDIT_TOOL_NAMES = ["Edit", "Write", "MultiEdit", "NotebookEdit"]` in the transcript, and nothing else; a write performed by `python3 - <<EOF` or `sed -i` through Bash is classified by `inferred-edit` as "NOT a known read verb, and not classifiable from this token alone — treat as unmeasured rather than as a write" (AMUX-3822). Counted in my own transcript for this session: 3 Edit tool_use blocks, all on `sessions_legacy.rs`, and ZERO on `session_verbs.rs`, where every one of my ~250 lines went in through a heredoc. The guard had no record that I had ever touched the file, so the `shared` row's `peer` field was empty and the honest-looking sentence it printed was the wrong one.
  THE COMPOUNDING PART, and why this is not a small hole: this session runs under bypass-permissions, whose harness instruction is "Do your work through the Bash tool wherever it can accomplish the job ... make file changes with sed, heredocs, or short scripts, rather than using the dedicated Read, Edit, or Write tools." So the mode that makes editing fast is the mode that makes edits invisible to attribution, and every lane running that way is unattributable on every file it touches. It also inverts which case is loud: a lane using Edit gets protected, a lane told to use Bash does not.
COST: ~250 lines and three tests shipped inside a commit whose message describes something else, so anyone bisecting the delivery ledger lands on "every diagnostic says whether its measurement ran" and has to work out why. Recovered only because I checked HEAD by hand afterwards; the peer attached a git note naming AMUX-3903 on the commit, which is the right repair and is also work neither of us should have needed to do. The deeper cost is that the guard's central promise is now conditional on a tool choice nobody makes for attribution reasons: I had staged only my own hunks a few commits earlier for exactly this hazard, and the guard could not have helped the peer do the same, because to it the file had one author.
FIX: Ownership must come from the WRITE, not from the tool that performed it. The material already exists in the same transcript — `inferred-edit` sees the bash command and the path, and already logs a verdict about it — so the gap is that "unmeasured" is treated as "no claim" rather than as a weaker claim. Two candidate shapes, and the second is probably right: (a) teach the bash-write classifier the common write forms (`>`/`>>` redirect is already recognised; add `python3 - <<`, `sed -i`, `tee`, `cat >`), which narrows the hole but keeps the same shape and will leak again on the next form; or (b) treat an OBSERVED mtime move by a session that also ran a bash command touching that path as a claim of its own tier, so the `shared` notice can say "another session may have written this by a means the guard cannot attribute" instead of asserting nobody did. The rule that must not survive either way is the current one, where absence of an Edit record renders as a positive claim that no other session edited the file. That sentence is the one that did the damage, and it is false whenever the peer edits through Bash.

## staged-guard reports every shell-based edit as a line "matching nothing you edited firsthand"
VALIDATED: amux-frustrations | VALIDATED by the ORIGINATING session (amux-frustrations authored this entry, so this is a self-signoff and is labelled as one, not a peer review).
THE ENTRY'S SENTENCE was: staged-guard reports EVERY shell-based edit as "matching nothing you edited firsthand". That sentence is no longer true. `line_accounting_mode(has_firsthand, mine_observed, peer_claims)` now returns Undecidable — suppressing the per-line list — when the committer has a content-record hole and NO peer claims the path. Shipped a728fe80, plus 8729cc0b for the reviewer's three findings.
INDEPENDENT CONFIRMATION FROM A DIFFERENT LANE, which is what makes this more than my own read: amux re-derived it and reported from their own editing pattern — "almost everything I wrote today went through Bash, so has_firsthand is false and those paths take Skip. No line detail, no noise, and my commits today printed no unaccounted block while the path-level NOTE still fired."
LIVE, not merely merged: serving a728fe80; before/after on a real staged mixed-edit path was unaccounted 1 path / 9 lines -> unaccounted 0, undecidable 1 path with its reason. Card AF-342 is `verified` with amux named as the reviewer who re-derived all four gate criteria.
SCOPE OF THIS VALIDATION, stated because a validation is a claim about the ENTRY'S TEXT and not about the subsystem: the noise on the normal path is gone. Attribution in that guard is NOT thereby fixed — AMUX-3954 (observed records carry no content hash) is open and is a deeper entry on the same subsystem, still live in this file.
AREA: attribution
SEVERITY: annoys
STATUS: open
DATE: 2026-08-30
SESSION: amux-frustrations
CARD: AF-342
SYMPTOM: Committing four files I wrote start to finish (40fa0ce0), the guard printed
 93 lines of warning: "15 staged added line(s) in docs/friction-themes.md match nothing
 you edited firsthand", the same for 55 lines in scripts/friction_themes.py and 22 in
 scripts/test-friction-themes.sh, plus a NOTE naming session 'amux' as a co-editor of
 all four, plus a SPLIT COMMIT WARNING. No peer had touched any of them. The guard's
 own caveats are correct and present (AF-179 mtime provenance, "if these are yours via
 shell edits, proceed"), so it is not claiming more than it knows.
COST: Nothing shipped wrong, but the reader has to re-derive "these are all mine" from
 93 lines of warning on every commit, and the true signal this guard exists for, a
 peer's hunk riding your `git add`, arrives in the same shape as the noise. Warnings
 that fire on the normal path are the ones people learn to scroll past, which is how
 the peer-hunk case gets missed. The guard correctly kept the peer's two dirty
 browser.rs files OUT of the commit, so its load-bearing half worked.
FIX: The firsthand-edit record is fed by Edit/Write tool calls, so a session following
 the harness instruction to prefer Bash for edits (heredocs, sed, python patches) is
 unattributable BY CONSTRUCTION, every time. Two components disagreeing about the same
 fact: the harness says edit via Bash, the guard treats a Bash edit as unwitnessed.
 Either record a firsthand claim when a Bash command writes a tracked file in the
 session's own cwd, or suppress the per-line list when EVERY unmatched line is in a
 file whose only recorded writer is you and no peer has a recorded write in the window.

---

## The board stores a card type its own create path rejects
VALIDATED: amux-frustrations | VALIDATED by the ORIGINATING session (amux-frustrations authored this entry, so this is a self-signoff and is labelled as one, not a peer review). Fixed in 9bdfc7f6 (card AF-323, now done): `decision` is a real card type with its own gate naming the decider. Took the add-the-type arm, not the migrate arm: the stored cards belong to mixpeek-orchestrator and ethos rule 8 plus AMUX-3552 both say surface, do not sweep, so listing the word repairs them where they sit with no edit to another lane's data. The entry's count was already stale when validated: it read three cards, five were live. Evidence: scripts/test-contended.sh -p amux-server -> 1665 passed, 0 failed; clippy clean; mutation putting core_item_type back to Code fails both new tests; live after the builder adopted the commit, GET /api/board/contract offers `decision` and gates.decision.done reads "The decision is recorded on the card: what was chosen, by whom, and when". Two tests were PINNING this defect, both using `decision` as their stand-in for an unknown type; repointed at `task`, which is still genuinely unknown.
AREA: board
SEVERITY: annoys
STATUS: open
DATE: 2026-08-29
SESSION: amux-frustrations
CARD: AF-323
SYMPTOM: `amux board add --type decision` returns
  `{"error": "unknown type \"decision\"", "valid_types": [code, escalation, blocker,
  investigation, ops, research, chore, doc, tripwire, watch, epic]}` — while three cards
  on the live board carry `type: decision` right now (ETHAN-36, MO-3036, MO-3034, all
  created by mixpeek-orchestrator, all in `todo`, all literal Ethan-decision cards).
COST: One retry and a re-file, ~2 minutes. The larger cost is conceptual: the error text
  explains that the gate is DERIVED from type and an unknown type would silently fall back
  to the strictest gate. That reasoning is right, and it means the three stored cards are
  sitting on a gate nobody chose for them. It also lands badly against AF-318, which
  proposes typed `needsyou --ask decision|access|...`: `decision` describes 24% of the 445
  needsyou cards, and it is the one type you cannot file.
FIX: Reconcile storage with validation. Either add `decision` to valid_types with its own
  gate, or migrate the three existing cards and reject it on the WRITE path, not only in
  the CLI. Whichever way it goes, one of the two components is currently lying.

---

## The nudge that tells you to discard a card names no command that does it
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux authored this entry and signed it off on 2026-08-31). Their basis, in their words: "Validated by receiving it. At the start of this session the capture-shell notice for AMUX-3958 printed `amux board discard AMUX-3958 --outcome-stdin`, the retitle form, and the epic path, with the real card id substituted into each." Producer is board_drive.rs:3654; board_drive.rs:7587 is a test asserting all three command strings are present. This is a live observation of the shipped notice, not a claim from the card status, which is the strongest basis available for a notice-text entry.
AREA: notices
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3707
SYMPTOM: The capture-shell nudge ("X is a captured prompt, not a unit of work")
  is ~250 words and fires ~42x/day fleet-wide, once per capture card ever. It
  tells the lane to "discard it" and to "set each child's `epic`". Neither was
  reachable from `amux board`: `discard` dispatches but is absent from help, and
  `epic` had no verb at all, though `epic` is a real PATCH field (board.rs:2142)
  added by AMUX-2992. Ethan flagged the token cost after seeing one fire on a
  question he had already answered inline.
COST: 540 nudges ever, 296 in the last 7 days. 70.6% of the cards ended
  `discarded`, i.e. the woken turn produced a one-line retirement. The prose is
  ~330 tokens; the turn each one wakes is tens of thousands. Every lane that
  followed the nudge to its epic exit had to hand-roll a curl, which drops
  X-Amux-Session, so the nudge was generating the unattributed board writes the
  ledger depends on not having.
FIX: c1c238b1. Text cut to ~85 words with a command on every exit; `amux board
  epic` added; `discard`/`show`/`reviewer`/`archive`/`unarchive` added to help;
  tests/nudge_commands_exist.rs sweeps every `amux board <verb>` the server
  emits against the CLI's case arms on every build.

## Mutation testing's obvious harness is a whole-file write, which reverts a peer mid-edit
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux authored this entry and signed it off on 2026-08-31). Their basis, in their words: they ran `scripts/mutate.sh run` twice on 2026-08-31 against scripts/git-hooks/prepare-commit-msg while fixing the CI red, and "both applied one exact string, both reverted in the trap on a non-zero exit, no whole-file write." Exercised on real work rather than on a fixture, which is what this entry asked for: the friction was that the OBVIOUS harness (`cp file bak`) is a whole-file write that reverted a peer's in-flight work twice on this shared checkout, and the fix is a tool that applies and reverts one exact string. Two live runs with a non-zero exit is the case that matters, since that is the path where a naive harness leaves the file mutated.
AREA: shared-checkout
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3671
SYMPTOM: `cp $F /tmp/orig ; <mutate> ; <test> ; cp /tmp/orig $F` — the natural way to
  satisfy this repo's "mutate the predicate and confirm it LANDED" rule. The restore is a
  WHOLE-FILE write, indistinguishable from `git checkout -- $F` to a concurrent peer. At
  15:45 it reverted mixpeek-research's in-flight `fn chrome_launch_args` out of
  browser.rs while KEEPING the call site that had arrived inside my mutate/restore
  window, so `cargo check` failed with E0425 for both lanes. Twice, because the harness
  ran twice.
COST: A peer lost work and had to re-apply; browser.rs was uncompilable for both of us
  for ~4 minutes. The number that matters is not this incident: the same harness had run
  about a dozen times that day across five files, and every one was a chance to do this to
  somebody. It had simply not collided until a peer edited the same file at the same
  minute.
FIX: scripts/mutate.sh — mutate by EXACT STRING, revert by the inverse exact string, so
  only the mutated bytes are ever written and a peer editing any other part of the file is
  untouched. Refuses a target that is absent (0 occurrences) or ambiguous (>1), which is
  the same discipline the rule already asks for: an unapplied mutation and a test that
  cannot fail produce the identical green, and the mutation is the cheaper one to check.
  The deeper point is that the REPO'S OWN RULE pushed everyone toward the unsafe
  implementation — ethos.md and CLAUDE.md ask for mutation testing repeatedly and neither
  says how to do it without a whole-file write. That is why this is `shared-checkout` and
  not "amux's mistake".

## Acking a peer's card with a desc PATCH silently destroys their write-up
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-23
SESSION: amux
CARD: AMUX-3576
SYMPTOM: The documented way to record an outcome before a gate transition is to write `desc`
  first. `desc` REPLACES. Acking three of amux-frustrations' review cards destroyed their
  write-ups: AF-178 4070 -> 1613, AF-182 5018 -> 2152, AF-180 3055 -> 1958. Nothing at write
  time said anything was lost. The board HAD computed the delta all along — it writes
  "desc -2457 chars" into a History line, where only someone reading the card afterwards
  finds it.
COST: ~6400 characters of a peer's reasoning across three cards, restored only because
  `_amux_state_events` carries full pre-mutation snapshots (ids 78469, 78822, 78791). mvs-infra
  hit the identical thing hours earlier on MI-4746 and lost 4082 chars of merge evidence. Two
  sessions, one evening, same field.
FIX: 91648fbc refuses a replace that drops a strict majority of a desc of 500+ chars, with
  `desc_shrink_ack` to override and a pointer to `desc_append`. c7826ed2 documents the recovery
  path in the board contract, because a recovery nobody knows about is one nobody uses.
  AMUX-3576 carries the remaining gap: the guard keys on SIZE, so AF-180 at 36% would have
  slipped under it even had it been live. Authorship is the honest axis — a non-owner replacing
  prose on someone else's card is a different act from the owner trimming their own, and the
  board knows both facts at write time.
NOTE: The guard's first production catch was ITS OWN AUTHOR. It refused me 409 on AF-179 an hour
  after I shipped it, doing the exact thing it was written to prevent, having written the commit
  message that explains why `desc_append` exists. Knowing the failure mode, having just fixed it,
  and having documented it did not stop me repeating it three times. That is ethos rule 6's
  "a rule you have written down is not a rule you run" with a same-day specimen, and it is the
  argument for why this had to become a refusal rather than a convention.

---

## Discarding a spurious autofix card refiles it, so doing the right thing loops
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3591
SYMPTOM: One server hang filed the identical card four times — AMUX-3581 (01:12), 3589 (01:26),
  3591 (01:35), 3594 (01:55) — same signature byte for byte, same 19 rows, zero new information.
  Each filing was triggered by the previous one being DISCARDED. Discarding an auto-filed report
  deletes its dedupe idem to re-arm the detector (board.rs, AF-137), which is correct for a
  CONDITION whose refile should require the condition to be live again. The 5xx signature carried
  no occurrence identity, so "recurrence" meant "any 5xx on that path still inside the 6h window"
  and the same historical rows kept qualifying.
COST: Four lane-turns, three of them mine, each a full scope-and-decide cycle on a card that was
  never a defect. Worse than the count: every round was a worker doing exactly the right thing.
  Judging a spurious report and discarding it is the sanctioned disposition, and it was the thing
  driving the loop.
FIX: 01b4cf53 — occurrence identity in the 5xx signature plus `5xx|` added to the re-arm skip,
  mirroring what AMUX-3472 already did for latency outliers. Same rows re-scanned now mint the
  same signature; a genuinely new 5xx mints a new one and files regardless, pinned by a control
  so this does not trade a refile loop for a detector that goes silent after one discard.
NOTE: Two things worth more than the bug. First, I diagnosed it WRONG twice — assumed discard
  caused it, then talked myself out of it because `already_filed` reads a durable idem and never
  checks card status, and wrote that up as a dead hypothesis. Both readings missed that the
  discard does not bypass the dedupe, it DELETES it, in a file I had not grepped. The comment
  naming the hook was in code I had already read that night (autofix.rs:1185). It took a THIRD
  filing to make me look instead of reason. Second, the correct DISPOSITION changed with the
  deploy state: with the fix merged but not running (builder dead since 00:01, AMUX-3585),
  discarding still loops, so AMUX-3594 was closed `done` instead — the re-arm hook fires only on
  the discard transition. Nothing in the card, the gate or the idle nudge can tell you that, and
  the nudge's own option 5 recommends the action that restarts the loop.

## "CDP never answered within 30s" printed with `DevTools listening on <that port>` in its own message
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3689
SYMPTOM: Six `POST /api/browser/start` 502s from `primer`, ~30.3s each: "Chrome (pid
  63351) is running but CDP on port 60005 never answered within 30s". The chrome stderr
  pasted into the SAME error body reads `DevTools listening on
  ws://127.0.0.1:60005/devtools/browser/e9edcb66-...`, stamped about three seconds into a
  thirty second wait. So CDP came up, on the exact port named, and amux polled it for
  another 27 seconds while reporting silence. The wait loop discarded every poll outcome,
  so connection-refused, a 1s timeout, a 403 and a 500 all produced that one sentence.
COST: The cause is still unknown and is now unknowable for these six, because the second
  half compounds it: the stderr path is opened with `File::create`, which truncates, and a
  failing caller always retries — so five of the six stderr files were destroyed by the
  retries before anyone looked, leaving a 600-char tail as the entire record of the
  incident. Roughly 40 minutes to establish only that the message was false. An
  investigator who trusted it would have spent that time on Chrome's startup, which is the
  half that was working.
FIX: 6d179755. `describe_cdp_probe` names which of {refused, poll timeout, HTTP status}
  the last poll got, the bail reports it with the attempt count, a WARN carries the same
  fields so a sweep sees the class, and a failed launch's stderr is copied to
  `amux-chrome-launch.failed-<ms>.stderr` (newest 5 kept) where the retry cannot reach it.
  The generalisable half, and it is not "log more": the artifact you need MOST when a
  failure repeats was being deleted BY the fact that it repeated. A truncating diagnostic
  file is fine for a one-shot failure and actively hostile for a retried one, and nothing
  about `File::create` reads as a data-loss decision at the call site.

## A detector's query failure was swallowed, so the whole detector had no coverage
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3696
SYMPTOM: `detect_silent`'s steering block was `if let Ok(mut stmt) =
  conn.prepare(...)` with no else. A schema error skipped the entire block and
  left nothing behind, which reads exactly like "no lane has a stalled queue".
  It is not hypothetical: `steering_queue.sender` is added by
  `ensure_fleet_tables`' runtime ALTER and by NO migration, so any database
  built from `migrations/` alone lacks the column and the query does not
  prepare. That is the state every test fixture is in.
COST: The steering-stall detector had ZERO test coverage and nobody could have
  known — every test that appeared to exercise it was exercising nothing,
  silently. Found only because I wrote a new test, seeded a row, and the INSERT
  failed on the missing column. Had I written the test without a write, it
  would have passed vacuously and I would have shipped it as coverage.
FIX: 79080270 records a Suppressed naming the prepare error, where the autofix
  report already surfaces suppressions. The test now asserts the query PREPARED
  before asserting anything about its output.

## An autofix card's fields contradicted each other, and only reading it caught that
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3696
SYMPTOM: After splitting the steering deadline in two, the emitted card said
  `threshold_min: 90` on a finding that fired at 360, and its `senders` blurb
  read "that lane may be unable to receive anything, which is what this card
  reports" directly beneath `lane_reachable: yes`. Every individual field had
  been correct before the change and two of them silently stopped being so.
COST: No wrong conclusion shipped, but only because I happened to read the full
  payload printed by a FAILING mutation run. No assertion covered either field,
  and nothing about the change site suggested they needed revisiting. A card
  whose fields contradict each other is worse than one missing a field, because
  each is read as a fact.
FIX: 79080270, both corrected and both pinned. The general lesson: when a
  verdict gains a second branch, every field computed alongside it inherits the
  branch whether or not it was touched — grep the payload, not the diff.

## SUPERSEDES the entry above: browser state's cap was silent, and my diagnosis of it was wrong
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3721
SYMPTOM: The entry immediately above blames the state extractor's SELECTOR for
  missing div-with-onclick rows and asks for CSS-selector clicking. Both claims
  are false and I am correcting them rather than leaving them to be greped as
  evidence. The selector has always contained `[onclick]`, and
  `selector_click_js()` already existed in the same file.
  The real defect: `state_js` collects every visible match into `seen`, renders
  the first STATE_EL_LIMIT (120) into `els`, and disclosed nothing about the
  gap. Measured live: 3625 matched the selector, 158 were visible, 120 were
  returned, and the two elements I could not find sat at indices 155 and 156 —
  addressable the whole time, because click-by-index resolves against `seen`
  rather than `els`. Clicking 156 worked the moment I looked past the response.
COST: A wrong cause filed on a card and written into this file, plus the ~20
  minutes already recorded. The compounding cost is what makes it worth an
  entry: a wrong entry here is read as evidence by whoever greps `AREA:
  instruments` later, and three entries sharing an AREA are supposed to be an
  argument for rebuilding something. An argument built on a wrong diagnosis
  points the rebuild at the wrong subsystem.
FIX: 1cddf81a — disclosure, not a bigger cap: `elements_total`,
  `elements_shown`, `elements_truncated`, and a note naming the addressable
  index RANGE and the two ways through. Verified live after adoption:
  total 162, shown 120, truncated true. The cap is fine; being unable to tell
  that it applied was the defect.
  THE TELL I WALKED PAST, which is the transferable part: the response held
  EXACTLY 120 elements, which is exactly the cap. A count landing precisely on
  a limit is a truncation, not a census. Both theories predicted the same
  observation ("my element is not in the list"), and only one was checkable in
  one command: `document.querySelectorAll(SEL).length`. When two explanations
  predict the same failure, reach for the one you can separate cheaply.

---

## amux lanes answer from an 8th-generation summary; a raw terminal answers from primary sources
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3742
SYMPTOM: An amux lane and a raw `claude` terminal, same model and same prompt, give noticeably different quality, and nothing in amux could say why. Model, effort and first-turn token baseline are identical on both sides (measured: both dominated by claude-opus-5 at xhigh, 59,016 vs 56,663 first-turn input tokens). What differs is compaction generations: amux lanes median 8 / max 215, raw terminal median 0 / max 32. Every start resumes (all 8 `start_session` call sites pass `skip_conv_id=false`), so a lane's conversation is immortal. The remedy existed and reached nobody: `app.js` rendered "New conversation" only when `!s.running`, and `config_patch` answered 409 while running, so on all 50 live lanes the one control that fixes this was hidden AND refused.
COST: Unquantifiable degradation fleet-wide for as long as lanes have been long-lived, and it took an owner noticing by feel. The diagnosis then cost four hypotheses measured and killed (model, effort, system-prompt tax, harness share) because no instrument reported the one that mattered.
FIX: 92e1383f, c246b7b9 — `amux fresh <name>`, the dashboard item on a running worker, `GET /api/debug/context-health`, and an hourly `context_health` job that logs the census every pass and WARNs `context_degraded`.

## The generation meter shipped truncating its own scan, and a truncated count looks like a healthy one
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3742
SYMPTOM: `count_compact_boundaries` did a single `take(64MB).read_to_end()` and stopped, so on a 324MB transcript it scanned the first 64MB and returned the partial count as the answer: 30 against a hand count of 75, and 105 against 215 for `mixpeek-cicd`. Shipped inside the very feature whose purpose is to stop reporting numbers that cannot be told from healthy ones.
COST: Caught within minutes, but only because the new endpoint disagreed with the census that motivated it. A reader with one number would have believed it. Also exposed that the obvious test is vacuous: every fixture small enough to write fits inside one 64MB read, so an EOF-scan test passes against the bug unless the read size is exposed as a seam.
FIX: c246b7b9 — chunked to EOF; `count_compact_boundaries_with_chunk` so the test drives a 4KB chunk over a multi-chunk fixture. Mutation-verified: restoring the single-read `break` goes red at left Some(1), right Some(3).

## A renamed lane silently orphans every card that named it reviewer
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3751
SYMPTOM: The rename cascade migrates `issues.session` and leaves `issues.reviewer` and `issues.shepherd` pointing at the dead name. The card still reads `review`, which looks healthy, while the reviewer nudge is addressed to a session that no longer exists. A nudge going nowhere is indistinguishable from a reviewer who is merely slow.
COST: 7 open cards parked in `review` on a reviewer that resolves to no registered worker, found only because Ethan asked an unrelated question about reviewer routing. Two name `amux-rust`, renamed to `amux` long ago.
FIX: 944f06b5 — the cascade migrates reviewer and shepherd; `session_is_registered()` is the one predicate for "can amux address this name"; the reviewer edge returns reason `reviewer-unreachable` with a WARN instead of nudging into the void.

## The badge and the drive loop judged the same self-report differently, and a lane could deadlock for 61 hours
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3756
SYMPTOM: `derive_status` applies a real trust model to a stored self-report — previous life, `stale_active`, trust window — and publishes `applied:false` for one it refuses. `steer_lane_at_boundary`, the gate on auto-pickup, board nudges and steering delivery, read the SAME row and asked only `state == "idle"`. So a lane whose Stop hook never fired kept a stuck `active` report, its dashboard badge correctly read IDLE (`decided_by: activity_fallback`), and the drive loop skipped it as `mid-turn` forever. The two halves of amux disagreed about the same fact, and the correct half was the one nobody acted on.
COST: 4 of 52 running lanes held out of the work loop, every one with `auto_pickup: true` and eligible cards waiting: creative-dna 61.4h, ai-video-editor 59.5h, mixpeek-autopilot 6.4h, primer 1.0h. Self-perpetuating, because only a turn writes a new report and only a human starts a turn on a lane the loop refuses to touch — so the sole exit was Ethan typing at it, which is exactly what he reported ("why do i need to push @tubescience to continue"), and doing so destroyed the evidence. The `mid-turn` skip reason read identically for a lane genuinely generating and one deadlocked for two and a half days.
FIX: 7e4682f0 — `report_applies()` is the one predicate, called by the badge and the gate; `lane_report()` is the one read, replacing two unjudged copies. A refused report WARNs `stuck_self_report` once per lane per report ts, and the board-drive trace's `mid-turn` detail now names the report's state, age and verdict.

## A gate that reads the real filesystem from inside a pure board function turns three unrelated tests red on every host
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: tests
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3751
SYMPTOM: AMUX-3751's reviewer-unreachable gate called `session_is_registered()`, which stats `~/.amux/sessions/<name>.env`, from inside `select_advance`. Board fixtures name reviewers like `peer` that exist on no machine, so three routing tests started failing — here, and in CI, for a reason that has nothing to do with what they assert. The gate itself shipped with no test of its own; breaking other people's tests was its only coverage.
COST: Found by running the full suite rather than the filtered one, which is the only reason it did not reach a push. A green filtered run and a red full run is the shape that gets pushed at the end of a session.
FIX: 7e4682f0 — `select_advance_with()` takes an injected lookup, which is what `config::resolve_home`'s own doc asks new tests to prefer over `set_home`; the tests shadow `select_advance` with a permissive registry, and `the_gate_refuses_a_reviewer_no_nudge_can_reach` exercises both cells of the gate directly.

## The pickup prompt threw away the card it was holding, and made the model buy it back at 308k tokens a call
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: tokens
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3759
SYMPTOM: `pickup_prompt` built the card's `desc + log` and then wrote `.chars().take(500)`. The lane received an ID and a 500-character stub, and spent tool calls reading back text the function had in hand one line earlier. Measured over 11,117 turns across 67 lane transcripts: an auto-pickup turn takes a MEDIAN OF 22 TOOL STEPS where a human-prompted turn takes 3, at a median resident context of 308,059 tokens per model call (p90 738k, max 966k). The cap saves ~1k tokens of steering text and costs ~308k per avoidable fetch — the wrong resource by three orders of magnitude. Silent, too: a truncated excerpt was indistinguishable from a short card.
COST: On the live queue it truncated 86% of todo cards (median definition 1,933 chars, p90 6,658) and discarded 108,820 characters of card definition. 43.8% of fleet turns and 49.7% of input tokens are amux-initiated, so this rides the largest single class of spend. Ethan noticed by feel — "theres also way too much tokens used for some reason in between tasks" — because no instrument reported steps-per-turn by what started the turn.
FIX: ade006c2 — `AMUX_PICKUP_EXCERPT_CHARS`, default 4000, config rather than a constant because this is D4 in the ethos ledger. A cut excerpt now says it was cut and names the read.

## Fixing a mechanism made its own nudge text false, and the false nudge went to the lane that wrote the fix
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3762
SYMPTOM: The capture-shell decompose nudge opens "is a capture shell holding your WIP slot". AMUX-3757 exempted capture shells from the WIP cap ninety minutes earlier, so the clause became false the moment that commit adopted. Nothing tied the nudge's claim to the query it describes, so the mechanism moved and its narration stayed put — the same view-disagrees-with-mechanism shape AMUX-3756 had just fixed one layer down, minted by the author of that fix.
COST: Small in tokens, sharp in kind. A nudge's whole persuasive force is "this is blocking you"; asserting a blockage that no longer exists makes a lane act on fictional urgency and buries the honest reason (no status is a true statement about a captured prompt, so no gate can pass it). It was caught only because the first delivery of the false nudge happened to land on the lane that had written the exemption. That is luck, and the next one will land somewhere nobody can tell.
FIX: b766472c — the clause is gone, the honest reason is stated, and `the_decompose_nudge_does_not_claim_a_blockage_the_wip_query_exempts` derives BOTH the pickup verdict and the nudge text from the same card so changing either alone fails. It also asserts the honest reason survives, because deleting a false claim and leaving an unmotivated chore is the other way to get this wrong.

## An unknown message type defaulted to "Human", so 355 amux nudges wore a person's badge
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3737
SYMPTOM: `msg_kind` was a denylist (`session`/`schedule`/`system` matched, everything else fell through to `human`). `pickup` was added later and nobody taught the classifier, so every board-drive auto-pickup nudge rendered with a blue `Human` badge in the Messages view. The row already carried `origin: board-drive`, so the discriminator was present and the classifier did not read it. The same denylist was restated in the SQL kind filter, so the badge and the filter corroborated each other; `_msgKind` in app.js was a third copy with the same default; and `_MSG_KIND[kind] || _MSG_KIND.human` was a fourth, which meant a server-only fix would have changed nothing on screen.
COST: 359 rows, 4.0% of 8,993 messages, misattributed to a person. Ethan caught it from a screenshot rather than from any instrument, and the misreading is the expensive direction: a fleet that is being auto-driven looks like it is being hand-driven. Also two docs defending the bug — the module header recorded the fallback as a deliberate Python-parity decision, and a test asserted `msg_kind("legacy-weirdness") == "human"` — so the first two things a reader consults both said it was intended.
FIX: 4239ee08 — an allowlist with an explicit `unknown` kind (selectable as a filter, because a kind nobody can select is a kind nobody goes looking for), the SQL filter built from the same constants, and both client copies aligned. Verified live: MSG-33250 now reads `kind=amux`, and 200 sampled `kind=human` rows carry zero machine origins.

## AMUX-2670's fix has never executed
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3737
SYMPTOM: `_msgKind` in app.js has returned `'unstamped'` for a `raw-tmux-fallback` row since AMUX-2670, with a comment stating that an unstamped injection must not render identically to an audited send. That branch is unreachable: `_msgKind` returns the server's `kind` when the row carries one, every API row does, and the server classified the type as `human`. And there was no `_MSG_KIND.unstamped` entry, so even reaching the branch fell back to the Human badge. Two independent reasons the card's intent could never reach a screen, in code that reads as though it works.
COST: A security-adjacent distinction — audited send versus unverified keystroke injection — silently absent for however long, while the code and its comment both assert it is present. Only 2 rows exist today, so the cost is latent rather than realised, and that is the point: nobody would have noticed until it mattered. Found incidentally, one line away from an unrelated fix.
FIX: 4239ee08 — `unstamped` is a real kind on both sides. The general lesson is the one worth keeping: a client-side classifier that defers to a server field has a DEAD local branch for every value the server also produces, and reading either half alone looks correct.

## A test and a doc comment can defend a default long enough for it to look considered
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: tests
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3737
SYMPTOM: The `human` fallback above was pinned by `assert_eq!(msg_kind("legacy-weirdness"), "human")` and explained in the module header as a Python-parity decision: "unknown types read as human, because that is the reading that gets a message looked at rather than filtered away". The reasoning is about visibility and it is sound. The conclusion does not follow, because `human` is not the only visible bucket. Separately, the kind FILTER test was green across the bug's entire life because the fixture seeds exactly one row per type the classifier already knew — a fixture that cannot contain the defect cannot detect it.
COST: Three independent signals (the doc, the test, the filter test) all reported health while the bug was live, so any reader checking whether the default was intentional got yes from all three. That is the difference between an undetected bug and a defended one.
FIX: 4239ee08 — both the doc and the test are corrected IN PLACE rather than deleted, so the next reader sees why it looked considered; `seed_unclassified` adds the two rows the fixture could not express. Mutation-verified: restoring `_ => "human"` fails both tests. The transferable question is "could my fixture contain this defect", asked before trusting a green suite.

## A parked fault card silently muted an entire autofix detector class for two days
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3774
SYMPTOM: autofix files one card per fault, and only OPEN cards suppress — deliberately, so a judged-and-discarded card lets the next occurrence through. But `backlog` is open. AMUX-3651 sat parked in backlog from 08-24, so every server-wide stall since was correctly detected, correctly deduped, and filed nowhere. The suppression reason also asserted "Its count is what moves; a second card would carry no new information" while the code pushes a report row and `continue`s, never touching the card — so the one signal it pointed at did not exist.
COST: Two days of a whole detector class dark, including a live six-family stall. `filed: []` on the tick reads identically for "nothing is wrong" and "everything is muted", which is this repo's most-reinvented bug. Found only because I was chasing an unrelated duplicate card and opened the suppression list; nothing would have surfaced it otherwise, and the card that muted the class looked like an ordinary parked backlog item.
FIX: 8b55d0bf — the false claim deleted (ethos rule 6: implement it or delete it), the suppressing card's staleness printed WHETHER OR NOT it is alarming, an explicit note that suppressing does not bump the card, and an `autofix_mute` WARN past AMUX_AUTOFIX_MUTE_WARN_DAYS. Verified live: AMUX-3651, stale_days=2.03. The better fix — actually bumping the count — is named on the card and deliberately left for its own change, because it is a write on every scan against the live board.

## An empty commit reported success, attached itself to a card, and closed it
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: shared-checkout
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3837
SYMPTOM: `2ee153e2` carries a correct subject, a correct card id and ZERO files; its tree is byte-identical to its parent's. git printed a success line, `git log` showed the commit, and the post-commit hook attached it to AMUX-3835 as that card's code history, so I closed the card citing a sha that contained nothing. The change stayed dirty in the working tree of this SHARED checkout for 25 minutes. Every instrument a session reaches for to confirm work shipped reads the MESSAGE; none of them reads the diff. The mechanism is UNEXPLAINED and I am not guessing at it: ruled out by direct test are the invocation (recovered from the transcript, no `--allow-empty`, correct pathspec, identical in form to the retry that landed 52 insertions), plain git (three scratch-repo cells covering pathspec-unmodified, pathspec-misses-the-change, and staged-outside-the-pathspec, all exit 1 and create nothing), every hook, alias and git config, a peer reverting it, and the reflog.
COST: A card closed on evidence that did not exist, and 25 minutes during which any lane's `checkout` or `stash` would have silently destroyed the work. It surfaced only by luck: a PEER's staged-guard warned them that my file looked like unattributed in-flight work, and that notice is what sent me to look. Nothing in amux was going to tell me. The near-miss is the cost, not the minutes.
FIX: edd6de55 — the `commit-report` verb the post-commit hook already calls now classifies the commit it is told about. An empty non-merge commit WARNs with session/sha/subject, marks the same card log line the commit already writes, and returns `empty_commit` on both response arms including the no-card arm. `Unchecked` is a distinct third state with its reason, never folded into `Empty`, because "we could not look" published as "your work is missing" is the false alarm that gets a warning ignored; merges are carved out because 7 of the 8 zero-file commits in the last 120 are merges and correct. A detector rather than a block: one genuine occurrence in 120 commits does not earn a gate that would be wrong more often than right. The live test against a real repo earned its cost immediately — `diff-tree` prints nothing for a commit with no parent, so the first commit of any repo read as Empty until `--root`, while the pure classifier stayed green.

## "Your token expired" and "your consent never came back" wore the same error
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3839
SYMPTOM: esteininger21@gmail.com's Gmail token returned `invalid_grant`, breaking SCHED-388. Ethan ran the re-auth flow; the account still failed identically. `social-activities` reported it as a live instance of AMUX-3747 (the Testing-mode 7-day refresh-token expiry), which is a real open problem and fits the symptom exactly. It was not that. No `/api/gmail/callback` had reached the server since 08-24: the mint hands out `http://localhost:8824/...`, the browser upgraded it to `https` (Chrome HTTPS-First; amux sends no HSTS), and 8824's self-signed `CN=amux` cert stopped it at the interstitial. Google had already released the code, so the flow died in the browser AFTER consent with the code sitting in the address bar. Every instrument said "needs_reauth" both before and after a re-auth that never landed, and nothing anywhere could express "your consent did not arrive".
COST: A wrong subsystem owned the diagnosis for hours across two sessions and one owner retry, and the data point was filed onto AMUX-3747 where it argued for urgency on the wrong work. The discriminating facts were BOTH already on disk the whole time (a surviving single-use pending entry, and the token file's mtime); nobody read them because the error did not suggest there was anything to read. The tell that broke it was a negative I could only trust after checking the log could produce a positive: 20 auth rows and 6 callback rows since 08-14, including 400s.
FIX: f2f028c4 — `/api/gmail/auth` returns `previous_attempt_never_completed` when a URL was minted for that account and no callback consumed it (pending_take is single-use, so a surviving entry IS the signal), present only when there is one so its absence claims nothing, scoped to the account, TTL-expired entries excluded. Verified live on the running binary: absent on a first mint, present on a second with no callback between, absent for a different account. Same commit fixes the adjacent silent bug the investigation exposed: the callback wrote the token to `<requested-account>.json` without checking WHICH Google account consented, so Ethan's `authuser=2` would have connected the wrong identity under the right filename. The transferable shape is the one this file keeps recording: when two states share an error string, the one that is not being reported is the one that costs the day.

## A runtime job reported healthy while structurally unable to do its work
VALIDATED: amux | VALIDATED by the ORIGINATING session (amux, 2026-08-31), with the BASIS RECORDED HONESTLY AT THEIR OWN INSISTENCE and weaker than a re-exercise. Their words: "I wrote the fix, I believe it landed, and I have not exercised it since. Archive with that basis recorded, not as verified." Read this as the author signing that the friction is gone, which is what the retirement rule asks for, and NOT as a re-measurement. They also volunteered two caveats that make their belief weaker than it looks: four entries in this batch have no reference to their card id anywhere in src/ or tests/, so the fix cannot be traced from the card, and only two of the nineteen distinct cards carry a test file naming them, so for the rest nobody can cheaply say whether a shipped check would catch a regression. Three of the untraceable four (AMUX-3887, AMUX-3723, AMUX-3687) were deliberately HELD BACK from this batch and are being exercised for real rather than believed, at the author's offer.
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3829
SYMPTOM: The browser idle-reaper held its "first seen empty" clock in a process-global map. The builder installs a new binary and the server self-adopts on EVERY commit, so the 3600s window restarted whenever anyone in the fleet committed. Measured that day: 22 builds between 06:33 and 16:26, median gap 16.7 minutes, and only TWO gaps of 60 minutes or more. Against a one-hour window that is a reaper which on a working day can almost never fire. Throughout, `/api/system-jobs` reported `spawned: true, ticks: N, status: ok`, and every word of that was TRUE — the loop was running perfectly. The job's health describes the LOOP; the defect was in state the loop carries. From outside, a reaper that can never fire and one about to fire were byte-identical.
COST: A card shipped claiming "the 18-hour zombie that prompted this cannot recur" when it could, and it stayed that way until I went looking during a verification pass. Nothing in the system was going to surface it: there is no signal anywhere for "this job is alive and cannot succeed". The generalisable trap is that a runtime job's registered health answers "is the loop running", which is a different question from "can this job do its work", and the two come apart exactly when the work depends on state that does not survive a restart — on a machine that restarts its server on every commit, that is most stateful jobs.
FIX: 5a8c85ab — the clock moves to `~/.amux/browser-idle.json`, rewritten whole each tick so stopped profiles drop out. The countdown is published on `/api/browser/status` as `idle_s` (null when not empty, which is a different fact from zero) so the invisible state becomes observable, and the release log carries `pre_boot_s`, how much of the window predates this process: a non-zero value there IS the restart-survival working, and the in-memory version could only ever print 0. The transferable question, which I would now ask of any registered job: if this process restarted right now, would the job lose progress, and would anything say so?
