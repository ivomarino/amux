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
