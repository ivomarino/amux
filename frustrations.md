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
## Dashboard's usage-limit discriminator says 'worker'; the live endpoint says 'session'
AREA: instruments
SEVERITY: annoys
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `rust-rebuild (provider
  adapters, RR-0043)` is gone, so no author can sign this). The rust mapper accepts BOTH
  spellings — provider/claude.rs:317, `if kind_str == "session" || kind_str == "worker"`,
  with a comment naming which is live and which is older. Live check: GET /api/usage
  returns limits kinds ['session','weekly_all','weekly_scoped'], so the live spelling
  matches. The FIX section's actual ask is met too: recorded fixtures at claude.rs:404-405
  carry both kinds, so endpoint drift fails a test rather than silently unlabelling a bar.
  The dead `l.kind === 'worker'` filter is gone from the SPA.
  Probe note, since this entry is itself about a silent probe: I first called
  /api/oauth/usage and read its 404 as evidence. That is Anthropic's UPSTREAM URL
  (provider/claude.rs:51), never an amux route — amux serves /api/usage. The 404 was my
  probe missing, not the endpoint being absent, and it would have supported the wrong
  conclusion in the same direction the entry warns about.

---
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
STATUS: fixed
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

  VERIFIED FIXED (part 1) 2026-08-21 (amux-frustrations; authoring lane `peek-render agent`
  was a subagent with no session, so nobody can sign this). The reported friction is gone:
  47 of 49 live tmux sessions are at 220 columns, 2 at 80, NONE at the reported 50/94/102.
  Mechanism removed both ends — runtime_jobs/pane_size.rs:207 issues `set-option -w -t <s>
  window-size latest` to undo the manual pin, and app.js:9340-9359 records the
  resize-on-peek machinery as deleted.
  PART 2 IS NOW DONE TOO — AF-128, shipped 12e8013 and live-verified.
  GET /api/sessions/<n>/peek returns no width, cols or geometry key. This entry's recorded
  COST was a wrong root cause and a reverted CSS change, because a narrow pane and a narrow
  render present identically — and that ambiguity survives the fix. Two lanes are at 80
  columns right now for unrelated reasons; the next reader who notices lands in the same
  undecidable spot.

  PART 2 CLOSED 2026-08-21: GET /api/sessions/<n>/peek now carries pane_cols and pane_rows
  (12e8013), verified on the running server rather than from the diff — amux 80x25,
  mvs-infra 80x24, amux-cloud 220x50. The two 80-column lanes are the control: the field
  tracks the ACTUAL width per session rather than reporting a constant, which is the only
  way it can settle this entry's actual question — narrow pane, or narrow render.
  No threshold and no "looks narrow" verdict, deliberately: picking a column count to warn
  at is the tuned parameter ethos.md warns about, and a reader comparing 50 against the 220
  everywhere else needs no constant. The parse returns None for every shape tmux emits when
  it cannot answer, because a fabricated 0 would answer this entry's question falsely.
  Still open, and small: the SPA peek header does not show it. The API is where every
  consumer can reach it; app.js was dirty with a peer's work at the time.

## The subagent switcher is wired end-to-end and reaches 0 of 50 sessions
AREA: instruments
SEVERITY: annoys
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane was a subagent with no
  session). Resolved by DELETION plus a replacement, which is the right answer to "capability
  that reaches nobody" and better than what this entry asked for. app.js:8220 records the
  pane-driven switcher as deleted, citing ARE-7 and the 0-of-50 predicate, and names the
  replacement: a subagent list reading DURABLE transcripts via GET
  /api/sessions/<n>/subagents, with no visibility gate at all. Verified live on three lanes:
  amux 53, backend 143, amux-frustrations 1. Real data, not a matcher that might rot.
  The comment states the principle better than the entry did: "the fix for a predicate that
  matched nothing is to need no predicate, not to write a better one."
  Note on the entry's own alternative proposal: no background-CONVERSATIONS switcher exists
  (0 references in the SPA). That was a feature suggestion rather than the friction, and it
  is not what holds this entry open.

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
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `amux-rust (RR-0130/0131
  cutover sweeps)` is gone).

  THIS SUPERSEDES MY OWN 2026-08-20 NOTE ABOVE, WHICH WAS WRONG. That note said "FIVE of
  the six are routed" and held the entry open on the tunnel family pending AF-64. Tunnel
  was never one of the six. I read `route.callers_have_routes` failures, saw they were
  all /api/tunnel/*, and mapped them onto this entry without checking them against the
  six families the entry NAMES three lines above. The right probe was to call the six.
  Called today, all six answer HTTP 200: /api/channels/{a}/{b}/messages, /api/log-search,
  /api/memory/global, /api/observability, /api/review/week, /api/review/digest.

  The seventh claim (/api/metrics serving a different document than the SPA reads) is
  also closed, and I nearly got this one wrong in the same direction. The payload has no
  `data` wrapper, which looks like the reported defect — but app.js:29269 assigns
  `_metricsData = data` (the raw body) and _metricsRender reads `data.sessions` /
  `data.system` off THAT, so top-level is what it wants. Live: 116 sessions, 49 active,
  and 0 active sessions lacking a numeric cpu_percent, so the unguarded .toFixed(1) at
  app.js:29427 does not throw.

  The missing instrument the FIX section asked for exists and can fail:
  route.callers_have_routes walks SPA/CLI call sites against the mounted table and today
  reports 8 failures, every one /api/tunnel/* — a different family, tracked on AF-64.
## Two rust call sites defer work to "while the Python server runs" — python is retired
AREA: instruments
SEVERITY: slows
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `amux-rust (RR-0131b
  sweep)` is gone). Both NAMED comment sites are absent from crates/, and the grep
  discriminates: `git log -S` shows the strings entering at 0b156bb and leaving at
  ff6b7d1, whose subject is `fix(memory): compose MEMORY.md after worker memory writes
  (AR-117)` — the removal is the fix, not a reword. write_claude_memory now composes
  session memory into the project MEMORY.md. Live end-to-end evidence rather than a
  code read: THIS session's loaded MEMORY.md carries a composed worker-memory block and
  the fleet roster, which is the composition the fix produces.
  Note for anyone re-deriving this: a lowercase grep for `while the Python server runs`
  finds nothing because the source says `While`. The empty result is the probe missing,
  not the string being absent — check with `git log -S` before believing it.

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
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `amux-rust` is gone). The
  defect was that amux-rs defaulted to https://localhost:8823, where nothing listens, so
  every verb died on connect and read as the application bug reproducing. Tested the built
  binary directly (~/.amux/rust-build-target/debug/amux-rs, since amux-rs is not on PATH):
  a bare `amux-rs board list` with no AMUX_RS_URL set exits 0 and returns 1,722 lines of
  real board data. It resolves the live endpoint on its own.

## A stderr capture moved stdout off the pipe, so nothing could break
AREA: instruments
SEVERITY: annoys
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `amux-rust` is gone).
  Tested with the probe the entry says was botched — stdout ON the pipe, stderr to a FILE
  (`amux-rs board list 2>/tmp/e.txt | head -2`), not the `2>&1 >/dev/null` that detached
  stdout and made a panic impossible. Result: amux-rs exits 141 (128+13, SIGPIPE), which is
  correct Unix behaviour for a closed stdout, with 0 bytes on stderr and no `panicked` line.
  Not exit 101. And the control the entry's own lesson demands: unpiped, the same command
  emits 1,722 lines, so stdout really was attached to the pipe and an EPIPE panic was
  reachable — the silence is the fix, not the probe missing again.

## Five finished cards sat in `todo` and kept being auto-picked
AREA: board
SEVERITY: slows
STATUS: fixed
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

  VERIFIED FIXED 2026-08-21 (amux-frustrations; authoring lane `amux-rust` is gone).
  crates/amux-server/src/api/commit_mentions.rs exists, cites AMUX-2674 and e679bdb by name,
  and GET /api/board/commit-mentions is routed and live — it returns 20 open cards named in
  merged commits right now, each with the sha and subject that named it.
  It also honours this entry's explicit ethos-8 caveat rather than quietly dropping it. The
  module header says so in its own heading, "It SURFACES, it never closes", with the reason:
  a card id in a commit is not proof of completion, since commits reference cards for
  context, for partial work and for reverts. The endpoint is a GET that mutates nothing.
  Probe note: my first call was /api/commit-mentions and returned 404. The route is under
  /api/board/. The 404 was my probe missing, not the feature being absent — same shape as
  the /api/oauth/usage miss recorded three entries up.

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
## A wedged disk scan could not say whether the walk or the database was stuck
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-20
SESSION: desktop
CARD: DESKT-15
SYMPTOM: A reclaim scan froze at 1,087 directories and sat there for 35
  minutes until a builder restart reaped it. `dirs_walked` stops moving in
  exactly the same way whether `read_dir` is blocked in the kernel or the
  SQLite flush is blocked on the write lock, and the row carried no phase, so
  the two hypotheses were indistinguishable from outside the process. Worse,
  the reaper I had written to make dead scans legible was clearing
  `current_path` as it marked them interrupted, so the finished row said the
  server had restarted and refused to say where. The one field that would have
  answered the question in a second was being deleted by the code whose stated
  job was to expose the failure.
COST: About 40 minutes, most of it re-walking the home directory by hand with
  a stopwatch to find what the scan already knew and had thrown away. The
  culprit turned out to be one directory: `~/Library/Mobile Documents` never
  returns from readdir on this machine (90s, zero entries, still blocked),
  while `stat` on it answers instantly with the same st_dev as $HOME, so the
  walker's cross-mount guard had no reason to skip it.
FIX: 7ecb766. Position and phase are published per directory BEFORE the syscall that
  can block, separately from the throttled write that persists them; the
  reaper preserves both and names them in its error text; a watchdog WARNs to
  server-rs.log BEFORE it touches the store, so a stall in the write lock still
  reports rather than hanging where the walker did. Stalled directories are
  recorded, and skipped by later scans once corroborated, with a Re-include
  button so the exemption is not a one-way ratchet.
  CORRECTION, 0371230: 7ecb766 made the watchdog END a scan at 45s and
  permanently exempt the directory it was on. Its first production run did that
  to ~/Downloads, which answers readdir in 2 seconds with 318 entries. The
  threshold was below the baseline — ~50 sessions at load 95, with the scan
  competing for the disk it measures — so the detector fired on contention it
  was itself producing, and its action was a silent hole in the scan. Now it
  WARNs at 45s and decides nothing, ends a scan at 300s, and routes around a
  path only after it hangs two separate scans. On the verifying run ~/Documents
  went quiet for 46s, was named in the log, and was NOT exempted. The fix
  found its own bug within the hour, which is the argument for the instrument.
  Same commit fixed a second bug found by measuring rather than by theory:
  `devtool_roots()` is a list of real absolute paths that `walk()` sized
  regardless of cfg.roots, so every unit test calling walk() on a tempdir also
  scanned ~/.cache, ~/Library/Caches and the 15GB shared cargo target dir. Two
  such tests ran 14 hours at 0% CPU and took every lane's `cargo test` hostage
  on the shared build lock. A peer read the 0% CPU as the FileProvider hang
  above, which had been proven real an hour earlier and so corroborated itself;
  lsof showing NO directory fd at all is what separated them.

---

## A peer's half-saved file blocks an unrelated commit's gate — third sighting in one day
AREA: shared-checkout
SEVERITY: slows
STATUS: open
DATE: 2026-08-22
SESSION: amux
CARD: AMUX-1315
SYMPTOM: my commit of a one-file autofix.rs fix was refused because the pre-commit gate
  (cargo check/clippy) compiles the WHOLE workspace, which at that moment contained a
  peer's mid-edit mdai.rs (their AF-141 work, uncommitted). The suite also wedged and two
  unrelated test families went red — all of it their in-flight tree, none of it my change.
  Same shape amux-frustrations hit this morning (a missing STALL_SECS const failing THEIR
  build during MY reclaim work), and their AF-132 near-pickup at noon. Three sightings,
  one day, three different victims.
COST: one blocked commit and a diagnosis cycle to establish "not my code" (the failing
  tests were a peer's own passing-in-CI features, which reads as a regression I caused);
  my staged change sat hostage until their edit completed.
FIX: none here — this IS AMUX-1315 (per-lane worktrees), and today is its strongest
  argument yet: the workaround everyone reaches for (an isolated worktree to get a stable
  tree) is the proposal itself, applied by hand, per victim, per incident. The count now
  argues for the build.

---

## Every checkout's git hooks are 18 days stale, and amux has been saying so into a log for 11
AREA: instruments
SEVERITY: blocks
STATUS: half-fixed — detection reaches a session now; the reinstall is the owner's call
DATE: 2026-08-23
SESSION: amux-errors-and-bugs
CARD: AEAB-47
SYMPTOM: `.git/hooks/pre-commit` is dated Aug 5 22:39 in ~/amux, ~/Developer/amux AND
  ~/Projects/amux-gtm, while `scripts/git-hooks/` is current. `grep -c guard_version` returns
  0 in the installed hooks and 3 in the repo's. `.git/hooks/pre-push` never calls
  `append-only-push-guard`, so the guard added after MG-1483 silently reverted 10 pushed
  entry-lines of this very file has never run on this machine.
COST: the cross-session staged-guard has been degraded fleet-wide for 18 days, and I pushed
  frustrations.md on 2026-08-22 with the data-loss guard absent without knowing. The detector
  was never the problem: the server logged "OUTDATED HOOK ... Reinstall:
  scripts/install-hooks.sh" 128 times across 8 days, naming 9 session/repo pairs, correctly,
  with the remedy — into server-rs.log, which nobody tails.
FIX: the detection now reaches a session — `.claude/session-freshness.sh` gains a content
  diff of the installed hooks at SessionStart. Content rather than `guard_version`, because
  the server's detector only fires for hooks too old to send a version at all; and
  `git rev-parse --git-path hooks` rather than `$REPO/.git/hooks`, because in a worktree
  `.git` is a file and the naive path is silent in exactly the checkouts AEAB-26 says the
  guard is already blind in.
  The reinstall itself is deliberately NOT done here: the current hooks are strictly more
  blocking than the installed ones, so running install-hooks.sh changes push behaviour for
  every other session on this machine.
  The general shape, and it is the fourth instance in two days after AEAB-46, AEAB-47 and
  AEAB-49: amux knows the dangerous fact, computes it correctly, and files it where the
  person who needs it never looks. `install-hooks.sh` also COPIES (`install -m 0755`) rather
  than symlinking, which is the mechanism that lets every one of these drift.

NOTE (amux, 2026-08-24, STRUCTURAL REPAIR — not my content, and deliberately not completed):
  a heading "Developing on branches in the build source put my unreviewed code on the whole
  fleet" carrying `AREA: cloud` and NO other fields was committed in 7fae11a1. A `## ` heading
  with no field block fails scripts/frustrations_audit.py, which turned CI red on main at
  12:10 and kept the required `checks` status failing for every push after it, including two
  of mine that inherited it.
  Demoted to this note rather than deleted or filled in. Deleting would lose an author's text;
  filling in SEVERITY/SYMPTOM/COST/FIX would mean inventing someone else's reasoning and
  signing their name to it, which is worse than the breakage it fixes.
  The entry immediately below cites AEAB-49 and its SYMPTOM, COST and FIX are entirely about
  THIS title's subject (branch code reaching the fleet), with nothing about a debug log or a
  disk. So these are most likely ONE entry that acquired a spurious heading. That is a guess
  and I have not acted on it. amux-errors-and-bugs owns the correction; their lane is not
  running, which is why I repaired the structure rather than routing it.
## amux's own debug log is the biggest thing on a disk amux is filing cards about
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-22
SESSION: amux-errors-and-bugs
CARD: AEAB-49
SYMPTOM: `curl /health` on the live server returned `commit: 5eabfb4dc6cc` — a commit that
  exists only on my unmerged branch, never reviewed, never merged. `rust-build-provenance.json`
  said `{"sha":"23ddb8d1...","ref":"fix/push-guard-rebase-false-positive","on_main":"no"}` and
  a build of that commit was in flight. The auto-builder builds `~/amux` HEAD every 60s, and
  I had been checking feature branches out in `~/amux` all session.
COST: the fleet ran unreviewed branch code for at least one build cycle. Nothing broke, and
  that is luck rather than design — the same mechanism would have shipped a mid-edit tree just
  as happily. It also churns: putting the checkout back on main makes the next tick rebuild and
  reinstall, so the fleet takes a second unnecessary swap.
FIX: the guardrail already exists and it is a log line nobody reads — rust-auto-build.log says
  "Installing it makes it the live build for the WHOLE FLEET within ~5s, with no CI and no
  review. Intentional pin? fine. Accident? put ~/amux back on main — develop in a git worktree,
  not the build source." It printed exactly that, correctly, while installing my branch. A
  warning that fires as it does the thing is not a guardrail. `on_main:no` is already computed;
  the builder should either refuse to INSTALL an off-main build unless a flag says the pin is
  deliberate, or announce it where a session actually looks (a board card or the session
  banner) rather than only in its own log.
  The general shape, and it is the third instance today after AEAB-46 and AEAB-47: amux knows
  the dangerous fact, computes it correctly, and writes it somewhere the person who needs it
  never opens. Rule 4's second layer — a tag in a store the reader never opens is the same
  failure as no tag.

## I read `hook_outdated` as file staleness; it is not, and AF-156 is right
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-23
SESSION: amux-errors-and-bugs
CARD: AEAB-47
SYMPTOM: my own error, corrected here rather than by rewriting anyone's entry. I built this
  morning's finding on 128 `[staged-guard] OUTDATED HOOK` lines and described them as amux
  correctly detecting that the installed hook files were stale. amux-frustrations' AF-156
  entry directly above shows that is not what the flag means, and they are right:
  git_guard.rs:1586 is `let guard_version = obj.get("guard_version").as_i64().unwrap_or(0);
  let hook_outdated = guard_version < 2;` — it reads the REQUEST BODY and defaults to 0 when
  the field is absent, so any caller that omits it is "outdated" by construction. I verified
  that line myself before writing this. It is not a file check and never was.
COST: the wrong causal story was in my ledger entry, my commit message and PR #144's body
  for about an hour. It did not change what I built, which is the only reason it is cheap.
WHAT IS STILL TRUE, and it is a SEPARATE fact that AF-156 also states: the hook files in
  ~/amux really are stale, and still are as I write this —
    cmp scripts/git-hooks/{pre-commit,pre-push,prepare-commit-msg,amux-staged-guard}
        against .git/hooks/*   ->  all four DIFFER
    ls .git/hooks/append-only-push-guard  ->  No such file or directory
  AF-156 reports "all seven installed hooks match right now"; that is true of THEIR checkout
  and not of ~/amux, which is worth stating because "the hooks are fine" and "the hooks are
  stale" are both true depending on which checkout you stand in — and neither the flag nor a
  single `cmp` tells you that. Per-checkout is the unit.
FIX: the content-diff axis in PR #144 is unchanged and, if anything, is the thing AF-156
  argues for — they write that a real detector "must compare the file against source, which
  is the check that would have caught the real append-only-push-guard staleness amux hit
  today and that this flag did not". What I am correcting is the EVIDENCE I cited, not the
  fix. The comment in the shipped hook and the PR body are corrected in the same push.
  The lesson for me: I treated a log line's WORDING as a measurement. "OUTDATED HOOK ...
  Reinstall: scripts/install-hooks.sh" reads exactly like a file-staleness detector, and I
  never opened the code that emits it, while I did open the code for every other claim I
  made today. A message that names a plausible cause is not evidence for that cause.

---

## `amux board` has no verb that sets `desc`, so recording findings on a card requires raw curl
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: desktop
CARD: DESKT-21
SYMPTOM: `amux board desc DESKT-21 --stdin` -> `amux board: unknown subcommand: desc`. The
  full verb list (`amux help board`) is `done|doing|todo`, `add <title>`, `list`. There is no
  way to write a card's description from the sanctioned CLI at all. `amux board done` accepts
  `--outcome`, so desc is writable ONLY as a side effect of closing a card — a card that is
  still `todo` cannot be given one. The only path left is
  `curl -X PATCH -d '{"desc":...}' $(amux url)/api/board/<id>`.
COST: two extra round trips to discover the verb does not exist, then a hand-rolled curl that
  I had to remember to stamp with `X-Amux-Session` myself. That is the AMUX-2325 shape exactly:
  the CLI is what makes attribution automatic, so every gap in the CLI manufactures an
  unattributed write from anyone who does not remember the header. Nothing warns you.
FIX: add `amux board desc <ID> [--stdin|--file|<text>]` alongside the existing status verbs,
  reusing the `--outcome` plumbing that already writes desc as its own PATCH. One verb closes
  the gap for every card state, not just `done`.

## `amux board --help` reports the flag as an unknown SUBCOMMAND instead of printing help
AREA: cli
SEVERITY: annoys
STATUS: open
DATE: 2026-08-23
SESSION: desktop
CARD: DESKT-21
SYMPTOM: `amux board --help` -> `amux board: unknown subcommand: --help` (exit 0). Help is
  reachable only as `amux help board`. `amux board` with no args prints the whole board, so
  neither of the two things a person reaches for when a verb fails shows the verb list.
COST: minutes, and it compounds the entry above: the natural way to check "does a `desc` verb
  exist" is `--help`, and that path answers with a message shaped like a verb error, which
  reads as though `--help` itself were the mistake rather than as "here are the verbs".
FIX: treat `-h`/`--help` in the subcommand slot as a request for the same text `amux help
  board` prints, and echo the verb list in the `unknown subcommand` error rather than only
  naming what was rejected.

## A stale second `amux` CLI shadows the real one on any PATH that puts /usr/local/bin first, and silently ate a card title
AREA: cli
SEVERITY: slows
STATUS: open
DATE: 2026-08-23
SESSION: desktop
CARD: DESKT-22
SYMPTOM: `amux board add --stdin <<'EOF' ... EOF` created a card whose TITLE IS THE
  LITERAL STRING `--stdin`, and threw the real title away. Exit 0, a full JSON card body
  echoed back, nothing wrong-looking. The identical command an hour earlier had worked
  and printed `DESKT-21 -> todo`.
  Cause: there are TWO amux CLIs on this machine.
    ~/.local/bin/amux -> ~/Dev/amux/amux   (live, tracks the repo, 89 stdin refs)
    /usr/local/bin/amux                     (standalone POSIX-sh copy, dated Aug 6, NO
                                             --stdin support anywhere in it)
  Default login PATH has ~/.local/bin at position 1, so normally you get the live one.
  I had prepended `/usr/local/bin` to PATH for an unrelated reason (`networksetup` and
  `ifconfig` are not on the sandboxed default PATH), which silently swapped the CLI
  under me mid-session. The two calls in this transcript differ ONLY in PATH order.
  The output shape is the tell nobody would think to look at: the live CLI prints
  `DESKT-21 -> todo`, the stale one dumps raw JSON. Same verb, same flags, same exit code.
COST: one card created with a garbage title and its real title destroyed, caught only
  because I re-read the card afterwards to get its ID. Worse than the lost title: the
  global CLAUDE.md mandates `--stdin` as the FLEET CONVENTION specifically to stop the
  shell evaluating backticks and $(...) in titles (AMUX-1888 — a garbled message, a
  leaked credential, and a stray `git rebase --quit`). On the stale CLI that mandated
  form silently discards your text, and the natural recovery is to fall back to inline
  quoting, which walks straight back into AMUX-1888. The safety convention degrades into
  the hazard it was written to prevent, with no error at any step. That is the
  AMUX-2140 shape: following the sanctioned instruction exactly is what produces the
  failure, and it returns success.
FIX: remove /usr/local/bin/amux — install.sh owns ~/.local/bin and nothing should be
  shipping a second copy to /usr/local/bin. Belt and braces, since a stale copy can
  reappear: have `amux` print its own resolved path and repo sha on any parse error, and
  make an unrecognised leading `--flag` on `board add` a hard error rather than a title.
  A CLI that accepts an unknown flag AS DATA cannot fail loudly, which is why 17 days of
  drift produced no signal.

## A shared checkout has ONE git index, so a peer's `git commit` shipped MY staged work under THEIR message
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-08-23
SESSION: desktop
CARD: DESKT-22
SYMPTOM: I staged four files for DESKT-22 (`git add` of a migration, heartbeat.rs,
  health.rs, migrate.rs), then ran `git commit -m ...`. It died with
  `fatal: cannot lock ref 'HEAD': is at c8272bf17 but expected 78b77653b`. My commit
  never existed. But the worktree was CLEAN afterwards and my code was in HEAD anyway:
  peer session `amux` had committed in the same instant, and because a shared checkout
  has ONE index, their commit swept my four staged files in. c8272bf1 now reads
  "fix(push-guard): the consent exit now works for ISOLATED workers (AMUX-3533)" and
  contains 330 lines of unrelated downtime-cause instrumentation alongside their two
  scripts/ files. Neither author reviewed the other's half.
  Two things made it worse than a merge collision:
  1. THE TRAILER LIED, and it is the exact field the deploy recipe says to trust.
     CLAUDE.md's push section says `%an` is shared by every session so "the Amux-Session
     trailer, stamped by prepare-commit-msg, is the real discriminator". c8272bf1 is
     trailered `Amux-Session: desktop` — ME — while its `Claude-Session:` URL is a
     different agent session from mine, and the card it names (AMUX-3533) is owned by
     session `amux` on the board. The same peer's other commit that hour
     (78b77653) is correctly trailered `amux`. So the one anti-footgun the docs point
     you at reported the sweeping commit as mine.
  2. THE STAGED-GUARD WARNED IN THE WRONG DIRECTION. It fired four notices, each saying
     my files "were also edited by session 'amux' N minutes ago — if that is MORE than
     you wrote, their work is in it". That is the mirror of what was about to happen:
     the risk was MY work landing in THEIRS, and the guard has no phrasing for it. It
     even appended the AMUX-3497 caveat suggesting the co-edit signal was probably just
     my own writes seen twice, which is the reading that makes you proceed.
COST: my work is merged and correct but permanently uncitable — DESKT-22 has no commit
  of its own, and the card now carries a paragraph explaining why anyone looking for one
  will not find it. A reviewer of AMUX-3533 gets 330 unrelated lines. Not fixable after
  the fact: rewriting shared history to separate them is strictly worse than a wrong
  message. Roughly 20 minutes to establish what had happened, because every obvious
  signal (clean tree, code present in HEAD, my own session on the trailer) said the
  commit was mine.
FIX: the index is the shared resource nobody is arbitrating. Either (a) take a lock
  around stage+commit so the pair is atomic across sessions — the staged-guard already
  runs at exactly the right moment and already knows who else is live, so it is the
  natural place, or (b) stop sharing the index: per-session worktrees (`git worktree`)
  give each lane its own index and HEAD against one object store, which is the durable
  answer and kills the whole class including the documented mirror cases (a peer's
  `git pull --rebase` replaying unpushed work, 2026-08-03; a peer's commit sweeping
  staged deletions, 2026-08-09 — this file's third entry in that family).
  Separately and cheaply: prepare-commit-msg must stamp the session of the process
  actually running git, and the staged-guard must warn in BOTH directions — "your
  staged files may ride out under someone else's commit" is the half it cannot say.

---
## The browser guard is absent against the one lane the dashboard is hardcoded to impersonate
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-08-23
SESSION: amux-frustrations
CARD: AF-183
SYMPTOM: A session is handed "a browser is already running under session '(unattributed)' —
  starting yours would DESTROY its state (staged logins included)". It names no owner, so there
  is nobody to ask and the only safe move is to do nothing. Measured: 451 of 535
  /api/browser/start rows all-time (84%) carry no X-Amux-Session, so the guard's whole safety
  property, naming the owner you are about to destroy, is unavailable for most collisions.
  Worse, app.js:32951 hardcodes `let _bwSession = 'amux'` with the deeplink as its only setter,
  so a browser a human opens from the Browser tab is recorded as owned by the `amux` LANE. The
  guard's same-session shortcut then treats that lane's start as the human's own restart:
  no refusal, no takeover flag, staged logins gone.
COST: A blocked browser for whoever hits the refusal, and a live path for an agent to silently
  destroy a human's signed-in session. The text is also verbatim the text of AF-181, an
  auto-captured card that was DISCARDED and then folded into an unrelated card, so it recurs
  and the discard is what let it recur.
FIX: Put the recoverable facts in the SENTENCE (pid, started_at, profile are already in the
  body but not the string) and let the refusal consult _amux_request_log for the start row, so
  "started 10h ago from 127.0.0.1 by curl/8.7.1" replaces "(unattributed)". Separately, and
  routed to Ethan because it is an identity decision, the dashboard must stop calling itself
  `amux`. AF-183.
NOTE: this is AMUX-1768's class one layer up. browser.rs:104-113 removed the SERVER-side default
  constant in writing, for exactly this reason ("framing that lane for every anonymous call ...
  and worse, the guard's same-session shortcut let any TWO anonymous callers stomp each other").
  The client-side constant survived the fix. Fourth member of the 2026-08-23 misattribution
  cluster with AF-179 and AF-182; the other three name a WRONG owner, which is recoverable, and
  this one names none.
STATUS-2026-09-01: HALF SHIPPED, and the half that is left is not code. The
  request-log lookup this entry asks for EXISTS and is wired: api/browser.rs
  carries `StartOrigin` with three states (Found / NotFound / NotLooked, so "we
  looked and found nothing" cannot collapse into "we did not look"),
  `lookup_start_origin` reads client_ip and user_agent off `_amux_request_log`,
  and the refusal consults it. So the caller now gets "127.0.0.1 + curl/8.7.1" or
  "100.66.26.84 + Mozilla/5.0 (Macintosh...)" instead of "(unattributed)", which
  is the discrimination the COST line names: an agent on this box against a human
  at a browser.
  The TITLE's claim is still true. `let _bwSession = 'amux';` is live at
  app.js:34858, so a browser a human opens from the Browser tab is still recorded
  as owned by the `amux` LANE, and the guard's same-session shortcut still treats
  that lane's start as the human's own restart. The entry stays open on that
  clause alone.
  Not fixable from here without deciding what the dashboard should call itself,
  which is whose identity it is (ethos rule 8). AF-183 is in `needsyou` with the
  question in one sentence and a recommendation.

## A peer's mid-edit fails MY test run, and a rerun is the only way to tell
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-24
SESSION: amux
CARD: AF-182
SYMPTOM: `cargo test -p amux-server --lib` returned "1284 passed; 1 failed" twice tonight,
  hours apart, and BOTH times the failure vanished on an immediate rerun with no change to my
  tree (1282/0, then 1285/0). The suite prints the count in the tail but the failing test name
  scrolls past in ~1290 lines, so the first thing you see is a number, not a name. On the
  second occurrence I read the tail, saw the count, and committed and pushed before registering
  the `1 failed` beside it.
COST: A commit message (d237f886) that states "1284 lib tests" for a run that was not clean.
  Caught and corrected on the card within minutes, but the message is pushed and wrong, and the
  correction lives somewhere the next reader of that commit will not look. The expensive
  direction has not happened yet: a session learning this shape and re-running past a REAL
  failure because "it is probably a peer".
FIX: The shipped half of AF-182 — lint-blame partitioning offenders into yours / a peer's
  in-flight work / already-broken-on-HEAD — is exactly the discriminator this needs, and it
  currently runs only in the pre-commit hook. A `scripts/cargo-blame.sh test` wrapper that pipes
  a failing run through the same analysis with STAGED empty would answer "is this mine" in one
  line instead of a rerun. amux-frustrations proposed that wrapper for `check`/`clippy`; this is
  the same gap for `test`, and the test case is worse because the signal is a count rather than
  a compiler error naming a file.
NOTE: This is the transient-unbuildable half of AF-182 that I own, showing up in a form I had
  not predicted. My entry there described the window as breaking a peer's BUILD. It also breaks
  a peer's TEST RUN, where there is no filename in the output to attribute — you get an
  arithmetic difference between two numbers and no clue whose edit caused it. e6077bcb fixed the
  commit path; neither of us has fixed the ad-hoc path, and this is the second cost from it.

## The disk ranker cannot rank a file, so it could never have named the 1.8 GB one
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-22
SESSION: amux-errors-and-bugs
CARD: AEAB-42
SYMPTOM: `disk_candidates()` pushes only entries where `metadata().is_dir()`. Its own
  cache, `~/.amux/du-sizes.json`, holds 26 entries and all 26 are directories. `amux.db`
  would rank fourth, above `~/.claude`, and is absent.
COST: the report meant to say what is eating the volume pointed at ~/Library/Caches,
  ~/.npm and ~/.cache while the fourth-largest object was amux's own database — for as
  long as that database has existed. I only found it by running dbstat by hand.
FIX: push regular files over a size floor from the same read_dir passes; the size is
  already in the metadata so there is no extra du cost. The lesson worth keeping: AEAB-33
  taught the ranking to declare the candidates it FAILED on, and that warning can never
  declare candidates it never GENERATED — after adding a surfacing mechanism, ask what
  the mechanism itself cannot express.

## I fixed the inner loop of a noisy warning and left the outer one, at 77% of the log
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-22
SESSION: amux-errors-and-bugs
CARD: AEAB-45
SYMPTOM: 1,336 of 1,726 lines in the 24h window are one sentence naming `~/.Trash
  (du exit 1)`, a condition that cannot change, emitted every autofix tick on each of two
  servers. I wrote it in AEAB-33, and its own comment says it now fires "ONCE per run ...
  rather than once per attempt" because the per-attempt spelling "drowned the log it
  shares with real faults".
COST: it competed for attention with three real findings in the same window (AEAB-41,
  AEAB-42, AEAB-43). AEAB-13 recorded the identical shape at the identical ratio — 921 of
  1004 lines — where it buried a first-ever `database is locked` line during a log review
  that existed to find exactly that.
FIX: reuse AEAB-13's tested `stall_log_first_this_bucket` rather than writing a second
  spelling of it, keyed on the joined path list so a CHANGED skip set still logs
  immediately. The pattern: a per-run dedupe is not a dedupe if the run is on a timer.

## Two servers on one DB reap each other's live work and halve each other's thresholds
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-22
SESSION: amux-errors-and-bugs
CARD: AEAB-43
SYMPTOM: `reap_orphaned_scans` runs `UPDATE reclaim_scans SET status='interrupted',
  error='server restarted mid-scan; the scan thread did not survive' WHERE
  status='running'` — no owner on the row. 8824 boots 10s after 8823 and reaps 8823's
  healthy scan. Both of the two scans that have ever run say the thread did not survive;
  both threads logged progress five minutes later, with no restart. And because every
  terminal write is guarded `AND status='running'`, the true outcome can never be
  recorded afterwards — it matches zero rows and logs nothing.
  Separately: `reclaim_skipped` shows ~/Downloads at hits=2 with first_seen and last_seen
  NINE SECONDS apart, so a threshold documented as "needs 2 such scans" was satisfied by
  one incident counted twice, and ~/Downloads is now permanently skipped.
COST: 2 of 2 reclaim scans ever run carry a false cause, on the machine where disk is the
  live risk. Any hits-based threshold in amux is silently halved the same way.
FIX: an owner column (pid or per-process boot ulid) on the scan row, reaping only rows
  whose owner is neither this process nor a live pid. The general form, which is the
  third entry this week under AEAB-11: any predicate that means "mine" or "twice" is
  wrong on a shared DB with two writers, and the failures do not look alike from outside.

## A rejected review has no status, so the reviewer is nudged to review their own rejection
AREA: board
SEVERITY: annoys
STATUS: open
DATE: 2026-08-24
SESSION: amux (hit it, twice), amux-frustrations (verified the mechanism)
CARD: AF-214 (nudge skip, done) / AMUX-3668 (the `changes-requested` status, open)
SYMPTOM: amux reviewed AF-203, rejected it with four specifics, and was re-nudged twice with
  "[amux] AF-203 sits in 'review' and names YOU as reviewer". The nudge predicate
  (board_drive.rs:2461) is `status == review AND reviewer == you`, and its own instruction —
  "if not, say what fails on the card" — is a DESC write that does not change status. So
  following it exactly leaves the card in the state that re-fires the nudge, until the 24h
  budget is spent. Verified against the running board: the status vocabulary is backlog, todo,
  doing, review, done, verified, discarded. There is no cell for "reviewed, rejected, back with
  the author", so both honest-looking moves misdescribe reality — `review` claims it awaits a
  REVIEWER when it awaits the AUTHOR, and `doing` reads as the reviewer working it when the
  reviewer is finished.
COST: two wasted reviewer turns on one card, each a full re-read to conclude "I already did
  this". Small per instance and it recurs on every rejected review. The larger cost is the
  board lying to every reader until the author notices: a card in `review` is indistinguishable
  from one nobody has looked at yet.
FIX: a `changes-requested` status (or `review` + a `rejected` flag) — it is the true state, it
  removes the card from the reviewer-nudge predicate, and it returns the card to the AUTHOR's
  queue where the work is. Cheaper fallback if that is too much surface: skip the reviewer
  nudge when the card's most recent activity is the REVIEWER's own note, since they have
  demonstrably reviewed it. REJECTED: raising the nudge budget — that makes an uninformative
  nudge fire less often, which is not the same as making it informative.
NOTE: amux's own move was the correct read and the vocabulary still could not hold it: "Not a
  second review — my findings stand... this is a status correction so the card stops describing
  itself as awaiting a reviewer when what it awaits is four small edits by its author." This is
  the AMUX-2140 shape (the sanctioned instruction does not reach an exit) in the review loop
  rather than the CLI.

NARROWED 2026-08-24 to the VOCABULARY half. The re-nag is fixed; the lying status is not.
  SHIPPED (c98ac2c1, AF-214): the reviewer nudge now skips a card whose reviewer has written
  to it since it entered review. amux verified independently — always-return-true reddens 3 of
  5 cells, dropping the round scoping reddens the resubmit cell alone, both counts as claimed.
  They also checked the NEEDLE against AF-203's real stored log rather than a fixture, which
  is the check that matters since a matcher that never matches makes the whole thing inert
  while every test passes: "` amux:" matches the reviewer's own desc row and does NOT match
  `amux-frustrations:` (the trailing colon anchors it), `authz:`, or `commit <sha> —`. And the
  skip is legible in the drive's own output as `Advance::None { reason: "reviewer-already-acted" }`
  rather than a silent no-op, because a nudge that stops firing and one that was never
  eligible look identical from outside.
  STILL OPEN, and it is the half that fixes the class: there is no status for "reviewed,
  rejected, back with the author". `review` claims the card awaits a REVIEWER when it awaits
  the AUTHOR; `doing` reads as the reviewer working it when they are finished. amux has taken
  it as AMUX-3668 (board_drive is theirs and they are the one who hit it), going with
  preference (a), a `changes-requested` status.
  WORTH KEEPING, amux's own: their first mutation pass reported BOTH mutations surviving,
  because they filtered `cargo test -- a_reviewer_who_has_written` and matched one cell of
  five. Naming the target before searching for it — the same instrument error this entry is
  about, made while checking the fix for it.

---
## Worker session does not auto-restart when server restarts
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-29
SESSION: 6527367a-8ff6-431a-ace9-e421554fb30d
CARD: none
SYMPTOM: After `systemctl --user restart amux.service` (from a deployment), the amux
  worker session stays down: `GET /api/sessions/amux` returns `running: false`. Inbound
  Telegram messages have nowhere to route into until someone manually calls `POST
  /api/sessions/amux/start`. The `amux-worker-start.service` is a boot-time-only unit
  (runs once at `systemd --user` init), not triggered by manual server restarts.
COST: 5 minutes of diagnostics; live Telegram messages silently drop inbound until
  manually restarted. In production with unattended amux, a server restart from a
  deployment would leave Telegram routing dead until noticed and fixed manually.
FIX: Either (a) change `amux-worker-start.service` to have `Restart=always` so it
  auto-restarts with amux.service, or (b) add a post-startup hook to amux.service
  that calls `POST /api/sessions/amux/start`, or (c) wire the worker start into a
  systemd timer that verifies worker is up on server start. The root cause is that
  system-startup and service-restart are different events (both need the worker up),
  and the current unit only handles the first.

## amux.service's KillMode=mixed cgroup-kills the whole fleet on every ordinary deploy
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-30
SESSION: amux (this session, catching up on the 2026-08-29 reboot-verification memory)
CARD: INIT-1
SYMPTOM: Continuing the prior session's "verify everything comes back after reboot"
  checklist, `GET /api/sessions` showed ALL 9 registered worker sessions with
  running:false — not just after the physical reboot, but again after the routine
  08:31:39 auto-builder restart that followed (commit 251cf15b, an ordinary
  feature-branch deploy). `tmux list-sessions` had nothing but a freshly-recreated
  `amux-init`; the real tmux server that held every worker's session had been killed
  outright. Root cause: `amux.service` has `KillMode=mixed` + `SendSIGKILL=yes`, and
  the tmux server lives in that unit's cgroup (spawned by ExecStartPre, never leaves
  it — cgroup membership is sticky across reparenting to PID 1 even though tmux
  daemonizes). Every restart of amux.service — reboot OR ordinary deploy — SIGKILLs
  the whole cgroup, tmux server included. `amux-worker-start.service` only fires once
  at boot (`WantedBy=default.target`), so nothing brought sessions back afterward.
  This generalizes the narrower 2026-08-29 entry ("worker session does not auto-
  restart when server restarts", CARD: none, still open) — that one suspected a
  single worker and a single restart path; this is the whole fleet, and it fires on
  every commit-triggered deploy, which happens many times a day on an active branch.
COST: The entire fleet (9 lanes) silently down for ~1h25m (08:31 restart to 09:56
  discovery+fix) with no alert anywhere — `/health` reported "ok" the whole time,
  because the server process itself was fine; only the sessions it was supposed to
  be managing were gone. Inbound Telegram messages during that window had nowhere to
  land. A separate near-miss found along the way: `amux start <name>` (no --detach)
  silently returns exit 1 with ZERO output when it can't attach to a non-existent
  TTY, even though the start itself succeeded — first read as "start is broken",
  cost a few minutes of confusion before `--detach` runs revealed it was already
  running.
FIX: `~/.config/systemd/user/amux.service`: `KillMode=mixed` -> `KillMode=process`
  (config-only; `daemon-reload` applied without disrupting the running process —
  confirmed same PID/start-time before and after the reload). `process` mode signals
  only the unit's main PID, leaving the tmux server (and its sessions) alone —
  matching what ExecStartPre's own idempotent `has-session || new-session` check
  already assumed. VERIFIED live: `systemctl --user restart amux.service` (09:59
  UTC) — PID changed, uptime_s reset, and all 8 real worker sessions (excluding the
  separately-broken `synthesia`, wrong macOS path) kept their original tmux
  `created` timestamps and came back running:true with no manual restart needed.
  NOT YET DONE (the log-signal half, tracked on INIT-1): an `invariants/checks.rs`
  check for "session expected running (standing_orders / no recorded stop event)
  but `tmux has-session` says no" — today nothing in `runtime_jobs` would have
  caught this without a human reading the dashboard; `backend::bootstrap::Bootstrap`
  only reacts to explicit Starting/ended DB transitions, and an out-of-band cgroup
  SIGKILL produces neither.

## `amux start`/`start-all` silently die under `set -e` on a tmux target-syntax bug
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-30
SESSION: amux (recovering from the KillMode incident above)
CARD: INIT-2
SYMPTOM: While recovering the fleet from the KillMode=mixed incident (previous entry),
  `amux start-all` created exactly ONE tmux session then exited 1 with NO output at
  all. `amux start <name>` on any not-yet-running session behaved the same: silent
  exit 1, session left running-but-unlocked in tmux, nothing printed. Root cause:
  `cmd_start`'s window-name lock, `tmux set-option -t "=$tname" allow-rename off
  2>/dev/null`, targets a WINDOW-scoped option with a bare session-exact-match
  target — tmux looks for a window literally named "=amux-<name>", finds none,
  exits 1 — and `set -euo pipefail` (line 19) kills the function right there, with
  the only evidence routed to `2>/dev/null` on that exact line. A second, separate
  bug compounded it: `cmd_start_all` called `cmd_start "$name"` with no `--detach`,
  so even after fixing the first bug, the first session started still hit
  `cmd_start`'s own terminal-attach step, correctly failed "open terminal failed:
  not a terminal" in this non-interactive context, and `set -e` aborted the rest of
  the loop — every session after the first silently stayed down.
COST: `amux start-all` — the obvious, documented recovery command for "the whole
  fleet is down" (INIT-1) — was silently non-functional for that exact use case.
  Cost ~15 minutes of manual per-session `amux start <name> --detach` calls to
  actually recover the fleet before this was root-caused, and would cost the same
  to the next session (or the next reboot) that reaches for `start-all` expecting
  it to work.
FIX: `amux` (ships on save, already live): `-t "=$tname"` ->
  `-t "=$tname:"` on both the `set-option`/`set-window-option` lines (explicit
  window target); `cmd_start_all`'s `cmd_start "$name"` -> `cmd_start "$name"
  --detach`. Verified live: a fresh non-TTY `amux start <name>` now starts the
  session and prints the honest attach-failure message instead of silent exit 1;
  `amux start-all` against 8 fully-stopped sessions now starts all 8 in one pass
  (the 9th, `synthesia`, fails for a pre-existing unrelated reason — a macOS path
  baked into its config on this Linux box — and now says so clearly instead of the
  whole batch dying silently after the first session).

## An AF-66-style guard existed for this and had been green the whole time
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-25
SESSION: amux
CARD: AMUX-3707
SYMPTOM: `assert_cli_verbs_exist` in board_drive.rs does exactly the check that
  would have caught the above, and was written for exactly this failure (AF-66,
  where `amux board show` fell through to help and exited 2). It is called on
  ONE prompt, from one fixture: the pickup Claim prompt. The decompose nudge
  never flowed through it, so a verb it named for months did not exist and the
  suite stayed green.
COST: No wrong conclusion shipped, but the guard's existence is what made the
  gap invisible. Anyone auditing "do we check that emitted commands exist?"
  finds the helper, reads it, and stops. Reading the check does not reveal which
  call sites it covers.
FIX: c1c238b1 widens it from one fixture to a source sweep of the whole server
  crate. The general lesson is ethos rule 7's: ask where the defect would be
  INTRODUCED and confirm the fixture flows through that code, not an ancestor of
  it. A single-call-site guard is worth naming its scope in its own doc comment.

---
## `amux` died at load with a bash syntax error — every subcommand, every session, at once
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-30
SESSION: amux (recovering from the KillMode incident above)
CARD: INIT-2
SYMPTOM: While recovering the fleet from the KillMode=mixed incident (previous entry),
  `amux start-all` created exactly ONE tmux session then exited 1 with NO output at
  all. `amux start <name>` on any not-yet-running session behaved the same: silent
  exit 1, session left running-but-unlocked in tmux, nothing printed. Root cause:
  `cmd_start`'s window-name lock, `tmux set-option -t "=$tname" allow-rename off
  2>/dev/null`, targets a WINDOW-scoped option with a bare session-exact-match
  target — tmux looks for a window literally named "=amux-<name>", finds none,
  exits 1 — and `set -euo pipefail` (line 19) kills the function right there, with
  the only evidence routed to `2>/dev/null` on that exact line. A second, separate
  bug compounded it: `cmd_start_all` called `cmd_start "$name"` with no `--detach`,
  so even after fixing the first bug, the first session started still hit
  `cmd_start`'s own terminal-attach step, correctly failed "open terminal failed:
  not a terminal" in this non-interactive context, and `set -e` aborted the rest of
  the loop — every session after the first silently stayed down.
COST: `amux start-all` — the obvious, documented recovery command for "the whole
  fleet is down" (INIT-1) — was silently non-functional for that exact use case.
  Cost ~15 minutes of manual per-session `amux start <name> --detach` calls to
  actually recover the fleet before this was root-caused, and would cost the same
  to the next session (or the next reboot) that reaches for `start-all` expecting
  it to work.
FIX: `/home/syseng/src/amux/amux` (ships on save, already live): `-t "=$tname"` ->
  `-t "=$tname:"` on both the `set-option`/`set-window-option` lines (explicit
  window target); `cmd_start_all`'s `cmd_start "$name"` -> `cmd_start "$name"
  --detach`. Verified live: a fresh non-TTY `amux start <name>` now starts the
  session and prints the honest attach-failure message instead of silent exit 1;
  `amux start-all` against 8 fully-stopped sessions now starts all 8 in one pass
  (the 9th, `synthesia`, fails for a pre-existing unrelated reason — a macOS path
  baked into its config on this Linux box — and now says so clearly instead of the
  whole batch dying silently after the first session).

## A fix that brings the fleet back up can itself make local cargo unsafe again
AREA: build
SEVERITY: blocks
STATUS: open
DATE: 2026-08-31
SESSION: amux
CARD: AMUX-48
SYMPTOM: Shortly after fixing AMUX-49 (every registered lane, not just `amux`,
  now comes back up after a reboot — 6 more Claude sessions went from stopped to
  running as a direct result), a plain `cargo check -p amux-server` — the ONE
  cargo invocation the existing offload-builds guidance called safe to run
  locally, single-crate, `.cargo/config.toml`'s `jobs=1`/`incremental=false`
  throttle already active — got OOM-killed (exit 137) anyway. `free -h`
  immediately after: 5.5GiB available out of 13GiB, zero swap. `.cargo/
  config.toml`'s own header (written 2026-08-28, FRONT-2) already names the
  mechanism: its throttle was tuned and verified against THAT day's baseline
  memory occupancy, and it explicitly warns a kill under pressure is not
  necessarily the build's own process — the OOM killer can reap an unrelated
  Claude Code session as collateral instead. AMUX-49 raised this box's
  baseline occupancy (8 running Claude processes instead of 2, ~200-400MB RSS
  each) without anyone re-measuring whether the existing throttle still holds
  against the new baseline.
COST: A gate that could not be honestly satisfied: AMUX-48's new invariants
  check (session.registered_lane_is_running) is written and follows an
  established, already-working pattern closely, but could not be verified to
  even COMPILE locally without risking re-crashing the same session AMUX-49
  had just recovered — the exact irony of one fix undermining the safety
  margin a sibling fix depended on. Remote build hosts were ALSO unreachable
  at the same time (a separate, unrelated baar-site netbird outage), so there
  was no fallback verification path at all for a period.
FIX: none yet — this is a structural gap, not a one-line bug. The honest
  interim mitigation (applied 2026-08-31): `offload-builds` memory widened to
  say `cargo check -p <single-crate>` is no longer a blanket-safe default —
  check `free -h` for real headroom before ANY local cargo invocation, treat
  the margin as a property of current fleet occupancy, not of the command's
  scope. A real fix would be either a durable local swap file (this box
  currently has NONE — `free -h` shows `Swap: 0B`, so there is zero graceful
  degradation under pressure and the OOM killer fires immediately) or a
  standing, always-available remote build target instead of relying on
  the specific remote hosts named in CLAUDE.local.md (private, this repo
  is public) being up when needed.

## Same root cause as above, escalated: the auto-builder itself now fails repeatedly, not just a manual check
AREA: build
SEVERITY: blocks
STATUS: open
DATE: 2026-08-31
SESSION: amux
CARD: AMUX-48
SYMPTOM: Supersedes/extends "A fix that brings the fleet back up can itself
  make local cargo unsafe again" (same date, above) — that entry covered a
  manual `cargo check` getting OOM-killed once. Verifying AMUX-48's `done`
  card an hour later surfaced something worse: `amux-builder.timer`
  (enabled, polling every 60s) has been trying to build commit d7af60f5
  since it landed and failed SIX consecutive times over ~15 minutes, every
  attempt dying with a bare `Terminated` right after "Preparing worktree"
  finishes, before any `Compiling` line ever appears in the log. Host load
  climbed the whole time this was observed: 43.59 -> 58.08 (1-min, 4
  cores) — not a one-off spike, a sustained, worsening trend. The
  builder's own lock (mkdir-based, `scripts/rust-auto-build.sh`) IS working
  correctly — attempts are serialized, not overlapping — so this is not the
  builder compounding its own problem, it's the AMBIENT load (this
  session's 8 concurrent Claude processes + a desktop stack (Xvfb/x11vnc/
  openbox/chromium) that restarted mid-observation for unrelated reasons
  (see FRONT-4) + everything else on this box) leaving no room for even a
  single serialized release build to complete.
COST: `/health`'s `commit` field has been stuck at `5e5f4b24da71` through
  three real fix commits (e6d48d53, d428277a, d7af60f5) landing on top of
  it — the fleet has been running increasingly-stale code for the whole
  window, and AMUX-48's own invariants check (meant to catch OTHER
  processes dying silently) cannot itself be confirmed live because the
  binary that would contain it never finishes building. The exact
  "outcome confirmed to still hold" a `verified` gate asks for could not be
  honestly claimed for the live-deploy half of that question — recorded
  as a caveat on the card rather than papered over.
FIX: none yet. Same interim mitigation as the prior entry (offload,
  headroom-check before local cargo) doesn't cover THIS case — the builder
  is a system service, not something a session chooses to run or skip.
  A real fix needs either genuinely lowering this box's baseline occupancy
  (durable question: does this box need to run 8 concurrent Claude
  sessions plus a full desktop stack plus periodic release builds, or does
  one of those need to move), or giving the builder itself a remote-offload
  path the way this session now does manually for ad hoc verification.

---

## Typing at a lane disabled that lane's auto-pickup
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3757
SYMPTOM: Every prompt is auto-captured as a `doing` card whose desc is still literally `**Prompt:** <what was typed>`, and that card counted against the WIP-1 cap. An unanswered prompt is not work in progress — the decompose nudge exists precisely to make the lane dispose of it — but the pickup query could not tell the two apart. The exemption list already carried tripwire, watch, epic and needs:you for the same reason and had never been extended to the cards amux mints itself.
COST: The specimen is TUBES-2225, titled "Why are you stopping": Ethan's complaint about tubescience stopping was itself the card holding the WIP slot that kept it stopped. A frustrated re-prompt is the likeliest prompt to arrive at a stalled lane, so the loop closed on exactly the lanes already in trouble. This lane's own board held 11 capture shells in `doing` at once, all of them his prompts.
FIX: 7e4682f0 — a capture shell joins the WIP exemption, using the same `substr(desc,1,11)` form as the fold query in board.rs so the two cannot disagree about what a capture shell is. Reshaping the desc, which is the exit the decompose nudge already asks for, makes it count again.

## A latency card named an innocent endpoint with a verdict that was confidently backwards
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3772
SYMPTOM: A host-wide stall that RAMPS files a single-family outlier card on the scan where fewer than AMUX_OUTLIER_ROLLUP_AT (3) families have crossed the threshold. That card's verdict then says "This is not a percentile shift — it is individual requests going wrong, so look at the request, not the family", which is the exact opposite of the truth, and it names an endpoint that answered in 0.09s minutes later. The rollup that describes it correctly already exists and fires on every subsequent scan; nothing revisits the card filed at the leading edge.
COST: One lane-turn to diagnose, and the diagnosis only landed because `host_load_at_worst` was in the payload and I followed it. A reader who trusts the verdict audits innocent code. ethos.md rates a loud wrong probe worse than a silent one, and this is one: it answers, names a specific target, and is wrong.
FIX: none yet, deliberately. The obvious fix — suppress a single-family card when an open ROLLUP exists — is WRONG while a rollup card can sit parked in backlog indefinitely, because it would mute every genuine single-endpoint regression. That prerequisite is AMUX-3774 and is now fixed; this card is parked with that as its trigger. Recorded because building the wrong fix first is exactly what I did, and the order matters.

## Discarding an autofix card as a "duplicate" deletes the only thing suppressing the re-file
AREA: instruments
SEVERITY: annoys
STATUS: open
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3849
SYMPTOM: A live outage (`/api/browser/start` 502) produced FOUR cards in three hours. I hand-filed AMUX-3842 with the diagnosis, then discarded the two autofix cards as duplicates of it, twice, and a fourth arrived anyway. `open_card_for_fault` suppresses on `source_ref LIKE 'autofix:<ident>|%'` for any card not done/verified/discarded — so a HAND-FILED card carries no signature and can never suppress, and discarding the autofix ones removes the only cards that could. The two look identical on the board: same title shape, same status vocabulary, no visible difference between a card the detector will honour and one it cannot see. `discarded` not suppressing is DELIBERATE and correct (it is what lets a genuinely new occurrence file after a judged one), so every individual piece behaved as designed while the composite guaranteed a re-file loop.
COST: Three discards, four cards, and the wrong conclusion available at every step — the obvious reading is "the dedupe is broken", which is what I would have reported if I had not gone and read `fault_identity`. The detector was right and I had deleted its memory. Also self-inflicted noise on a shared board while the underlying outage sat correctly parked in `needsyou`.
FIX: none yet. Immediate workaround, applied: copy the autofix signature onto the hand-filed card's `source_ref`, which makes it suppress (verified against the LIKE). Two candidate real fixes, cheapest first: (a) `amux board discard` warns when the card carries an autofix signature AND is the last non-terminal card holding that ident — a discard that turns the detector back on should say so; (b) `board add` for a fault already carded by autofix is the wrong move entirely and the honest path is folding the diagnosis INTO the autofix card, which nothing currently suggests. The transferable shape: a card's suppressing power lives in a field nobody looks at, so two cards that read identically to a human behave oppositely to the detector.

## "The tests pass" is load-dependent on this box, so a green suite is a weaker claim than it reads
AREA: tests
SEVERITY: slows
STATUS: open
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3853
SYMPTOM: A full `cargo test -p amux-server --lib` run showed 8 failures, all in `opencode::structured`, in code nobody had touched. Re-run in isolation the same tests are 15 pass / 0 fail. The failures were build contention: those tests spawn a binary out of the shared `CARGO_TARGET_DIR` while another lane's build is rewriting it, which is the ETXTBSY family `2618b7d3` already added a retry for. The retry is not sufficient under the load this machine actually carries (50 lanes, a builder rebuilding on every commit, and any peer running clippy).
COST: I nearly reported 8 failures as a regression in a peer's area, and spent a cycle proving they were not. The larger cost is retrospective: every "1530 pass, 0 failed" I wrote on a card today rested on a run that happened not to contend, and I could not have told the difference at the time. A green suite here means "green, and nothing was building" — the second clause is invisible and nobody states it. That is the same shape as the 706ms latency number from the same day: a measurement taken on a machine whose load is the dominant variable, reported as if the load were not there.
FIX: none yet. The cheap instrument, not the cure: have the test run record whether a build was in flight (the builder's lock is already on disk at `~/.amux/rust-build.lock`) and print it beside the result, so a red suite says whether it was contended. The cure is either per-lane target dirs (rejected before, for disk) or serialising the spawn-a-binary tests behind the same lock the builder takes. Naming the instrument first because the wrong lesson from this entry is "ignore red suites", and a contention flag is what separates the two honestly.

---
## `git commit -a` in a shared checkout swept three lanes' in-flight work into one lane's commit, twice in four hours
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-30
SESSION: amux
CARD: AF-342
SYMPTOM: Mid-task on AMUX-3886 I had ~87 uncommitted lines in
 crates/amux-server/src/api/browser.rs (a `with_cause` helper plus 28 call sites).
 ts-gke committed 78009d90, "browser-reaper: add hard TTL to kill old browsers
 regardless of page state", touching the same file for an unrelated reason. All 87 of my
 lines went in with it. `git log -S with_cause --oneline` now answers with a commit about
 a TTL arm. I found out only because `git diff` on my own file came back a single hunk
 when I had made two, which is a coincidence of what I happened to check next.
COST: About 25 minutes: reconstructing what had moved, proving the sweep from
 `git log -S`, and then rebuilding a mine-only tree in a scratch worktree because the
 shared checkout by then held three lanes' in-flight edits and would not compile. The
 durable cost is the record: the fix for a browser 502 is filed under a browser-reaper
 TTL commit, and the next person to run `git log -S` or `git blame` on it gets a wrong
 answer with nothing marking it wrong. Not rewriting history over it — 172 unpushed
 commits with live lanes — so this entry and the follow-up commit body are the record.
SEVERITY-NOTE (appended same day, after the recurrence): raising this from `slows`.
 It happened AGAIN four hours later, same lane. 8a990ebd, "browser-reaper: activity arm",
 carries THREE lanes' work: my remaining AMUX-3886 change (+281 integrations/browser.rs,
 +59 api/browser.rs), amux-frustrations' entire AF-342 fix (+199 git_guard.rs, +100
 test-staged-guard-render.sh, the hook, checks.yml, their ledger entry), and ts-gke's own
 reaper arm. The second sweep landed AFTER ts-gke had read the diagnosis of the first,
 agreed with it in writing, and said they were adopting the explicit-paths guard. So this
 class does not require a careless session; it requires a lane that intends the right
 thing and reaches for a familiar verb.
 AND THE FIX FOR THIS WAS ONE OF THE THINGS SWEPT. amux-frustrations had AF-342 STAGED,
 holding the commit on a full-suite result, when someone else's commit took the index. A
 lane that stages early and verifies before committing is MORE exposed, not less, because
 its work sits in the shared index longer. That is the argument against every advisory
 guard on this path.
 ATTRIBUTION CORRECTION (same day, after ts-gke checked my evidence). I claimed above
 that both sweeps were the SAME LANE and leaned on "same Amux-Session AND same
 Amux-Conversation" as two agreeing signals. They are ONE signal. Read
 .git/hooks/prepare-commit-msg: `stamp="$AMUX_SESSION"`, then `conv` is a lookup of
 `~/.amux/sessions/$stamp.meta.json` for `cc_conversation_id`. The conversation field is
 DERIVED FROM the session field, so a wrong stamp produces a wrong conversation id
 identically and the commit reads as doubly confirmed. Everything reduces to one
 env var in whatever process ran `git commit`, and AMUX_SESSION is inherited by any
 child of a lane.
 So "two sweeps by one lane, the second after that lane agreed in writing" is NOT
 established, and I withdraw it. What survives: two sweeps happened, and the mechanism
 is `git commit -a` (established independently — my UNTRACKED test file was not taken
 while every modified TRACKED file was, which `git add -A` would not produce). The class
 argument does not need the actor to be identified, which is the useful part.
 Contrary evidence worth keeping: all three ts-gke-stamped commits carry
 `Co-Authored-By: Claude Sonnet 4.6` while that lane runs opus-5, and `Claude-Session:
 session_01Gg7LPMY45VdVgrq29tHv2A` is on 78009d90 and 2a914717 but ABSENT from 8a990ebd
 — a field no amux hook writes. None of that is conclusive (the hook's own comment
 measures Claude-Session on ~30% of commits, so absence proves nothing), and that is the
 point: the record cannot answer who committed, in either direction.
 CARDED as AMUX-3916: the stamp needs one field the committing process cannot inherit.
 MECHANISM, narrower than the first entry had it. My untracked test file was NOT taken
 while every modified TRACKED file was: that is `git commit -a`, not `git add -A`. `-a`
 stages every modified tracked file at commit time — exactly the set a shared checkout
 fills with peers' work — and it never touches the index beforehand, so it walks straight
 past AF-316's staging refusal. The guard to state is "never pass -a", not "prefer
 explicit paths".
FIX: This is AF-342 (filed by amux-frustrations ~20 minutes before 78009d90 landed)
 seen from the other end, and it CORRECTS one clause of that entry. AF-342's COST says
 "The guard correctly kept the peer's two dirty browser.rs files OUT of the commit, so
 its load-bearing half worked." On the very next commit, on one of those same two files,
 it did not: the load-bearing half is exactly what failed here. Both observations are
 real — amux-frustrations was warned and stopped, ts-gke was not — which means the
 guard's protection is not a property of the guard, it is a property of whether the
 committing session happens to read 93 lines of warning it has learned to scroll past.
 That is the argument AF-342's own SYMPTOM makes ("warnings that fire on the normal path
 are the ones people learn to scroll past, which is how the peer-hunk case gets missed"),
 now with the case attached. ts-gke's diagnosis, unprompted and worth keeping: the
 property the guard needs is "this path has no edit record from the COMMITTING session",
 not "this path was edited via shell" — heredocs are one way to be invisible, and a
 codegen step, a `git checkout` and a peer's editor are three more. Scope AF-342's fix to
 the general property.

## A trustworthy test run on a contended file now requires a private worktree, and each one costs a full dependency rebuild
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-30
SESSION: amux-frustrations
CARD: AF-336
SYMPTOM: Verifying the AF-342 fix, `cargo test -p amux-server --lib git_guard` failed to
 compile for ~35 minutes on errors entirely inside a peer's in-flight
 crates/amux-server/src/api/browser.rs (E0308 tuple arity, then an unterminated json!
 macro) while three lanes edited the tree. `cargo test` builds the TREE, so a red result
 said nothing about my change and a green one would have been equally uninformative.
 Both amux and amux-frustrations independently reached for the same workaround in the
 same hour, neither having proposed it to the other: `git worktree add --detach <tmp>
 HEAD`, apply only your own diff, test there.
COST: ~35 minutes of blocked verification on this pass, plus a full dependency rebuild
 per worktree because CARGO_TARGET_DIR keys on the workspace path, so the shared build
 cache does not carry over. The durable cost is that the sanctioned verification command
 in VERIFY.md is now untrustworthy for any contended file, with nothing in its output
 saying so: scripts/test-contended.sh reports whether a BUILD was running, which is a
 different question from whether a peer's half-saved source is in your tree. Two lanes
 converging on an unshared workaround in one hour is the signal that it is the norm.
FIX: AF-336 (per-lane worktree) ends this class rather than detecting it, and this entry
 is evidence for it rather than a new proposal. Until then the cheap half is honesty in
 the instrument: have scripts/test-contended.sh report, beside its result, whether any
 tracked source in the crate under test is dirty and attributed to another session. A
 compile failure in a file you did not touch would then read as such instead of as your
 own regression.

## SUPERSEDES the entry above: the consumer guard EXISTED and was correct — `--lib` never ran it
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-30
SESSION: amux-frustrations
CARD: AF-346
SYMPTOM: My entry above says the a99955f7 dashboard regression happened because no
 consumer-side invariant existed and that amux was adding one. Both halves are wrong, and
 amux established it by checking rather than agreeing with me.
 `tests/board_api.rs :: list_is_slim_by_default_and_serves_prose_only_on_request` already
 existed, drives the real HTTP list path, and asserts desc_head starts with the card's
 first line. Run against a99955f7 in a scratch worktree it fails in 0.16s. The guard was
 written before either of us got here, was right, and would have blocked the commit.
 It did not run because I verified with `cargo test -p amux-server --lib`, which reports
 "1625 passed" and SKIPS every `tests/*.rs` target: 47 integration files, ~339 tests.
COST: The regression itself is costed in the entry above. The cost of THIS entry is the
 wrong lesson I nearly left in the ledger: "add consumer-side tests" is useless advice
 when the consumer-side test is already written, and it would have sent the next reader
 to write a duplicate of a passing test instead of fixing the command that skipped it.
 A false mechanism filed as history is the thing archiving rules exist to prevent, and I
 was ten minutes from it.
FIX: amux put it in VERIFY.md by name — `--lib` is a partial run whose number reads like a
 total — and strengthened two assertions in that same test that were weaker than they
 looked: `desc_len.as_u64().is_some()` is TRUE of 0, so it and the log_n line beside it
 would BOTH have gone green against the blanked loader. Only desc_head had teeth. Now
 they assert `> 0`, mutation-checked, at cc3b4221. What remains open is the general shape:
 a suite-shaped command that silently covers a subset is the same instrument failure as a
 probe reporting zero when it never ran, and `--lib` is not the only such flag.

## The observed-edit record has no content hash, so "who edited this" is unfalsifiable by construction
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-31
SESSION: amux-frustrations
CARD: AMUX-3954
SYMPTOM: The staged-guard named me as a co-editor of
 crates/amux-server/src/runtime_jobs/autofix.rs. Three timestamps break the claim:
   my observed record for that path   20:41:38
   the file's actual mtime            22:06:42   <- the bytes that were committed
   the mass `cargo fmt` sweep         22:10:14   (alerts.rs, auth.rs, ~180 files)
 My record is 85 minutes BEFORE the write whose content landed, and the file is 3.5
 minutes off the fmt sweep, so it was a third, separate write. The record is
 `<ts> <session> n=<count> paths=<names>` with no hash anywhere (confirmed in the writer
 by amux), so the guard compares a TIMESTAMP WINDOW against a file that moved, and any
 write to that path inside the window inherits whoever's window it was.
COST: Two mis-attributions by one lane in a single day. This one, and earlier amux told
 ts-gke their commit had absorbed 220 lines — the trailer evidence showed the commit was
 not even ts-gke's conversation. Different signal, same shape: a name with no way to test
 it. Each costs a round trip between two lanes to disprove, and the durable cost is worse
 than the minutes: a guard that names the wrong peer teaches lanes to discount it, which
 spends the credibility it needs for the cases where it is right. On this same day the
 SAME guard correctly stopped a real sweep, so both outcomes are live.
FIX: Hash each path at observation time and compare against the staged blob — match, name
 them; differ, drop the name and say why. That turns "someone touched this path recently"
 into "someone touched THIS CONTENT", which is the claim the warning already makes in
 prose. Tracked as AMUX-3954, deliberately NOT built at the end of a long session: it is a
 change to a safety-critical guard, which is how a fix becomes the next incident.
NOTE THE THIRD OUTCOME, because neither party had a slot for it: this was not "you were
 right" or "I was wrong". The signal was REAL and pointed at the WRONG EVENT. An
 attribution system keyed on time rather than content will keep producing that verdict,
 and the AF-179 caveat is doing real work — it is why amux hedged instead of asserting —
 but a caveat cannot make an unfalsifiable signal falsifiable.

## A test cell that reads the ambient process ancestry cannot fail on the box that wrote it
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-31
SESSION: amux
CARD: AMUX-3962
SYMPTOM: `checks` red on main for the whole fleet, two consecutive runs. Failing step
 `test-commit-stamp.sh`, cells 1 and 2, `alpha='' beta=''` and `got ''`. Both cells ran
 the commit-msg hook under whatever process ancestry the test inherited and asserted on
 the `Amux-Agent` trailer, which the hook populates by walking its own parents for a
 `claude` process. On any dev box that walk finds the session running the test, so both
 cells pass. In CI there is no claude anywhere in the tree, the hook correctly omits the
 field, and both cells fail on an empty string. Reproduced locally by reparenting the
 test to init, which is what a runner looks like from inside the walk: 7 passed, 2 failed,
 same two cells, same empty values.
COST: About an hour of fleet-wide red CI, and the specific cost is that `checks` is the
 job every lane's `board done` evidence leans on, so a red there taxes work nobody
 involved was doing. Worse, it was invisible in the only place anyone was looking: two
 lanes independently ran the local suite that night and both read green (1665/0), because
 the local suite and the CI job were not running the same thing. The commits that went
 red were not the commits that broke it. The cells had NEVER been green in CI; run
 33396997200 was simply the first one to reach them, so the fleet-wide red landed on
 whoever happened to push next, four commits downstream of the author.
FIX: 232c212f. The two cells now build their own ancestry, the technique the later cells
 in the same file already used: one `claude` shim (a symlink, so ps sees a matching
 argv[0] basename), both hook runs under it, so ancestry is a test INPUT rather than a
 property of whoever launched the test. 9/9 with a claude ancestor and 9/9 reparented to
 init. Cell 2 got stronger on the way past: it asked `ps -p <pid>` for liveness, which
 cannot tell the right process from any live one. Mutating the hook to stamp `pid=1` is
 both invariant and live, and the old pair passed that completely clean; against the
 shim's known pid it fails.
THE SHAPE, which is the reusable part: a cell that reads the ambient environment measures
 the LAUNCHER, not the code. It is not merely untested in the other environment, it is
 structurally unable to fail in the one where it was written, so a local green carries no
 information about it at all. The tell is an assertion whose subject was not constructed
 by the test. That is ethos rule 7 with a location attached: "can your check actually
 fail" has to be asked about the environment as well as the logic, and the way to ask it
 is to run the file somewhere the ambient answer is absent.

---
## The staged-guard's blocked-commit remedy edits the other lane's staged work
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-31
SESSION: amux (found the technique), amux-frustrations (filed and fixed)
CARD: AF-365
SYMPTOM: When the guard BLOCKS a commit over a peer's co-edited file, its only
  suggestion was `git restore --staged <their paths>`. On a shared index that
  mutates state belonging to the other lane: their file is staged because THEY
  staged it, and unstaging is an edit to someone else's in-flight work made by a
  party who cannot see what they intended. The near-miss that exposed it: amux had
  an unstaged `checks.yml` hunk at ~line 316 while my hunk in the SAME FILE was
  already staged at ~line 181.
COST: No damage, because amux found the exit themselves and said the guard does not
  suggest it. What the obvious path would have cost is worse than plain absorption:
  committing that file would have SPLIT my change, landing my CI wiring under their
  commit message while the app.js it wires stayed uncommitted, so my own commit
  would have wired nothing. Two lanes, one file, and every documented move was wrong.
  `git add -p`, which the guard recommends two screens down for the partial-stage
  case, is also the wrong tool here: the problem is not which of YOUR hunks to take,
  it is that THEIRS are already staged.
FIX: Fixed. `git commit <your paths>` is now offered FIRST, labelled as the exit
  that touches nothing the peer owns, and the unstage remedy now says out loud that
  it edits the shared index. A cell in test_amux_staged_guard.py pins both the
  presence and the ORDER, plus the stated reason, because an unexplained ordering
  gets tidied back by the next person who thinks restore reads better first.
  The cell reads the SHIPPED hook rather than executing the branch (that text is
  inline in main() and reaching it needs a multi-session git fixture), and it says
  so rather than implying parity with the cells above it.

---
## Editing a running .sh corrupts it mid-run, and the instrument cannot report its own death
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-31
SESSION: amux (hit it and diagnosed it), amux-frustrations (owns the file, took the fix)
CARD: AF-368
SYMPTOM: `amux` ran `scripts/test-contended.sh -p amux-server` and got:
    1888 passed, 0 failed, and NO `test result: FAILED` line anywhere
    no contention verdict printed at all
    ./scripts/test-contended.sh: line 53: syntax error near unexpected token `('
    exit 2
  Line 53 was a bare `#`, and the file was `bash -n` clean throughout. Two of my
  commits to that file landed inside their run. bash reads a script INCREMENTALLY,
  by byte offset, so the file growing underneath the running shell shifted the
  offsets and bash resumed mid-token, then failed on whatever byte now sat at its
  saved position — nowhere near either edit.
COST: Near-miss on a false red. Exit 2 with zero failures reads as a broken suite,
  and amux nearly reported it as one; what stopped them was noticing that "0 failed"
  and "exit 2" cannot both be a test result. They also correctly refused to report
  their own AMUX-3718 work green off that run, because its exit status described my
  edit rather than their code. This is the THIRD cause of a red suite after the
  builder and the dirty worktree, and it is the one this script structurally cannot
  report: it dies before reaching any echo, so its verdict is not wrong, it is
  ABSENT. The instrument's blind spot is the instrument.
FIX: Fixed. The wrapper now copies itself to a temp file and `exec`s that before
  doing anything else, so an edit cannot reach a run in flight. `exec` means one
  shell and the exit status still belongs to cargo. Snapshotting is the only fix at
  the right layer, because a report cannot describe a run that stopped existing.
  GENERALISES, and this is the part worth keeping: every .sh in this repo is
  exposed, and the bash CLI ships on SAVE, so `amux` itself is the largest instance
  — a long `amux` invocation running while any lane saves that file is this exact
  hazard. Not fixed here; that is a separate card.
  A NOTE ON THE TEST, because the first one lied. I wrote a behavioural cell that
  started the wrapper, truncated the file to garbage mid-run, and asserted it still
  exited 0. It passed. It also passed with the re-exec MUTATED AWAY, because bash
  buffers a file this small in a single read and the truncation never reached the
  running shell. A control that cannot fail is worse than none, so it was deleted
  rather than relabelled. The shipped cells assert the preamble exists, execs the
  snapshot, and has NO executable statement before it — position being the property
  that matters, since a snapshot taken after other work is a snapshot of a file that
  could already have moved. Both mutations now redden exactly one cell each.

---
## A status signal with a store, a consumer and a unit test, and no producer anywhere
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-02
SESSION: amux
CARD: AMUX-4024
SYMPTOM: `subagents_live` was null for 125 of 125 lanes. AMUX-3048 shipped
  `subagent_event_post` (start/stop), a `{count, ts}` store, a reader in
  `FleetSignals::subagents_working`, an explain field, a status-history column and a
  passing unit test. No hook ever POSTed an event, so every one of those read null
  forever. The code comment deferring the count-authoritative "off" direction reads as
  a careful trade-off between two live signals; there was only ever one, because the
  other was never sent. Two more details compound it: the deferral names the producer
  as "PreToolUse:Task" and the tool is called `Agent` in current Claude Code, so the
  hook would have been inert even if someone had wired the documented name; and
  `hooks.report_hooks_wired` walks the entries that EXIST, so it structurally cannot
  fail on an event class nobody added.
COST: Two wrong lane statuses reported by Ethan in one afternoon, in opposite
  directions, both landing on the mtime fallback nobody knew was load-bearing:
  tubescience read IDLE while blocked on a background agent, mvs-pitr read WORKING
  with an AGENTS badge over an empty composer. About 40 minutes of this session spent
  designing a fix keyed on the reported count before checking whether any lane
  reported one — the answer was none, and the first fix would have been green and
  completely inert, which is the same defect a second time.
FIX: Producer wired in `scripts/hooks/hook-report.sh` (`subagent:start` / `subagent:stop`)
  and in settings.json as `PreToolUse[^(Task|Agent)$]` + `SubagentStop`; count made
  authoritative in both directions; `hooks.report_hooks_wired` extended with an
  absent-event-class arm so the next dead producer fails a check instead of reading
  as a deliberate trade-off.

## Reading the shared worktree to understand code returns a peer's draft, and the wrong decision leaves no artifact
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-09-02
SESSION: amux-frustrations
CARD: AF-336
SYMPTOM: Reported by general-canvas-apps, self-traced by mixpeek-homepage-claude. A lane
  changed a PUBLIC ARGUMENT'S SEMANTICS after reading a gate's invocation out of the
  shared worktree, which held another lane's uncommitted draft of the same job. The
  draft's line was broken. The committed line was correct and carried a comment, three
  lines from the one they quoted, that would have stopped the change.
  DISTINCT FROM THIS CARD'S OTHER ENTRY, which is the BUILD case: there, a peer's
  in-flight edit reddens your test run, which is loud and self-correcting on a rerun.
  Here the tree poisons a DECISION. Nobody pushes anything, the reader's commit is
  entirely their own work and looks correct, and the wrongness lives in a conclusion
  drawn from bytes that were nobody's committed truth.
COST: One wrong public-API semantics change, caught only because its author went back and
  traced their own reasoning. THE REAL COST IS THAT THERE IS NOTHING TO COUNT. The four
  write-side races on this card each left a diff and all four were caught — three by the
  victim running a receipt diff, one by the racing author. This class leaves no diff, no
  repair commit and no receipt, so the observed rate of one is not a measurement, it is
  the absence of an instrument. It also retires the strongest objection to AF-336: at
  four catchable races the counter-argument was "the cost is repair commits and may be
  cheaper than 125 worktrees", and a class with no artifact has no such bound.
FIX: Two halves, and only the first is shipped.
  DISCIPLINE, done: ~/.claude/CLAUDE.md's shared-checkout section covered a peer's edit
  redding your BUILD and said nothing about a peer's draft poisoning your READING. It now
  carries the distinction, the specimen, and the two commands — `git show
  origin/main:<path>` for what everyone actually runs, `git show HEAD:<path>` for what
  this checkout last committed — with general-canvas-apps' line kept because it is the
  memorable form: a worktree read is a snapshot of nobody's truth.
  ISOLATION, still needsyou on AF-336: per-lane worktrees make the read CORRECT rather
  than merely well-advised. That is the difference between a rule every lane must
  remember on every read and a property of the environment. A rule that must be
  remembered is exactly what this file exists to stop relying on.

---

## A peer asked me a blocking question I cannot answer: they are an isolated worker
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-09-02
SESSION: amux-frustrations
CARD: AF-352
SYMPTOM: The `amux` lane sent me a push-consent ask — "is all of your unpushed work in
  a state you are happy to have on origin? One line back is enough" — with two named
  answers and a stated consequence for each. I wrote the reply and `amux send amux`
  refused: "'amux' is an isolated (raw-agent) worker with the amux harness stripped. It
  is not a peer or relay target and is reachable only by the owner from the dashboard."
  THE SEND WORKS IN ONE DIRECTION ONLY, and nothing said so until I had written the
  answer. `GET /api/sessions/amux` carries `isolated: true`, so the fact is available;
  it is just not available at the moment you need it, which is when a message from them
  arrives asking for a reply. Their message carried a server-verified origin stamp,
  which reads as a working channel.
COST: a real ask blocked. They are holding a 34-commit push on an answer they cannot
  receive, and their own fallback ("wait -> I tell Ethan you are mid-something") will
  now fire on my silence rather than on my answer, which reports the wrong reason to
  Ethan. The remaining channel is a board card in their queue for what is a yes/no.
FIX: `.claude/rules/frustrations.md` already documents this class exactly — "LIVE IS NOT
  VALIDATABLE ... the session payload already carries `isolated`; read it, or discover
  it from a refused send after you have written the message". I discovered it the second
  way, having read that rule earlier the same day. That is the tell that the rule is in
  the wrong place: it asks a human to remember a lookup before writing, and the moment
  the lookup matters is the moment a message ARRIVES.
  CORRECTION, same day, after walking the sanctioned path end to end: THE DOCUMENTED
  FALLBACK ALSO FAILS, and the advice printed at the first refusal sends you to a
  mechanism that fails for the same reason. All four channels, in order, all refused:
      amux send amux              -> "not a peer or relay target"
      card in their queue         -> blocked by THEIR OWN WIP limit ("close_these_first")
      amux board progress <card>  -> "progress noted, but OWNER NOT NOTIFIED: target is
                                     an isolated (raw-agent) worker: amux automation is
                                     not delivered into it"
      only the owner, from the dashboard
  The cross-group send refusal explicitly recommends the board handoff ("use the board on
  a card owned by <them>: `amux board progress <CARD>` notifies the owner at their next
  turn"), and neither refusal mentions the other. A lane following the guidance exactly
  ends up where it started, having written the message twice.
  The cheap mechanism is at delivery, not in prose: when an isolated worker's message is
  delivered to a peer, say so in the delivery envelope — one clause, "this sender cannot
  receive replies; only the owner can reach them" — and stop naming the board path for
  isolated targets, since it does not work for them. The `isolated` flag is on the record
  being rendered in both places.
  Related: AF-352 is the entry for entries whose authors can never sign off, which is
  the same asymmetry costing something different.
## Runtime hook copies drift from HEAD silently — install.sh has no supervision
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-02
SESSION: amux
CARD: AMUX-99
SYMPTOM: GET /api/health/invariants showed hooks.report_hook_matches_committed
  and hooks.shared_guard_matches_committed both failing — runtime hook sha
  differs from the sha baked into the running binary. ~/.amux/hooks/
  git-shared-guard.py and ~/.amux/hook-report.sh were both installed 2026-08-30
  20:48 and never reinstalled since, while their source kept getting real
  commits — most notably e782b68a (AMUX-3932), a genuine guard-BYPASS fix
  ("command substitution inside a quoted argument bypassed the shared-checkout
  guard"). That fix passed every CI gate and sat in git history, never live on
  this box, because nothing re-runs install.sh's hook-install step
  automatically. AMUX-28/AMUX-29 already covered this exact invariant pair and
  are marked done with no evidence recorded on either — the drift came back
  because the underlying gap (install.sh only runs manually, unlike the Rust
  binary auto-builder / amux-builder.timer) was never closed the first time.
COST: a real security-relevant fix (a shared-checkout guard bypass) sat
  undeployed for days on a box running unsupervised agents against a shared
  checkout, with the health invariant correctly flagging it the whole time and
  nothing consuming that signal. Discovered only because this session was
  sweeping GET /api/health/invariants for other reasons.
FIX: manually re-ran install.sh's own install_hook_from_head sequence for both
  files (git show HEAD:<rel> + chmod +x + sha256 sidecar). Confirmed live:
  invariant failures dropped from 6 to 4, both hooks.* entries cleared.
  NOT fixed: the durable gap. AMUX-99 is the recurrence card and names the two
  real options (a systemd timer polling install.sh's hook block the way
  amux-builder.timer polls the Rust build, or the invariant self-healing since
  it already computes the right bytes) — a design choice, not made here.

## Claude completion notifications could precede the subagent's actual completion
AREA: provider-integration
SEVERITY: slows
STATUS: open (provider-side notification defect; amux lifecycle handling is fixed)
DATE: 2026-09-02
SESSION: amux-testing-e2e
CARD: ATE-10
SYMPTOM: Claude produced an initial subagent completion notification while that agent
  still reported waiting and its requested file did not exist; a second notification
  arrived only after the file was actually written.
COST: Treating notification prose as lifecycle truth would have marked delegated work
  complete early.
FIX: Amux does not infer lifecycle from Claude's notification text. The status fix
  consumes the provider's explicit subagent start/stop hooks and keeps notification
  content as display-only evidence. The provider-side duplicate/early notification
  remains outside this repository.

## A correct answer makes a wrong reason feel checked, and the reason is what gets generalised into a rule
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-09-03
SESSION: amux-frustrations
CARD: AF-445
SYMPTOM: Named by mixpeek-cicd, 2026-09-03, about their own near-miss, and it applies to two
  of mine from the same day. Three instances, all with the same shape: a TRUE sub-fact made a
  FALSE conclusion feel established, and in every case the conclusion was about to become a
  rule rather than a one-off answer.
    1. (mixpeek-cicd) They cleared three staged-guard notices correctly and generalised the
       reason into a proposed guard change: downgrade when provenance is `observed`, because
       observed means no recorded edit. What actually settled their three cases was different
       and per-instance — the trailer named a peer, their own commits on that path were days
       old, and they knew from memory they had not opened it. Their words, which are the
       entry: "I picked `observed` as the safety discriminator while producing nothing but
       `observed` records all night, which is a fair definition of not having checked." Every
       file they shipped that day was a heredoc write, i.e. exactly the record their rule
       would have dismissed. Three right answers, one wrong rule, aimed at a guard every lane
       reads.
    2. (mine, AF-290) The card said seven session verbs are duplicates "another route already
       expresses", and a `mutate.sh` run had PASSED — route.callers_have_routes did not fire
       when the routes were deleted. Both true. The conclusion was false: `/api/workers/{id}`
       is mounted and resolves NOTHING (0 of 12 fleet lanes, 0 workers against 129 sessions),
       so migrating would have handed the dashboard "worker not found" on every destructive
       path. The passing mutation is what made the premise feel verified; it asks whether a
       route EXISTS, not whether it ANSWERS.
    3. (mine, AF-346) The card said the slim board serializer "drops desc and log, which is
       why the response carries none". The response does carry none — true, and checkable in
       one curl. The conclusion, that hydration can stop selecting them, was false: the slim
       branch makes five derivations over those columns. The correct observation is what made
       the plan look established.
COST: none shipped, in all three, and that is the problem with counting it. Instance 1 was
  caught because the recipient of the proposal had spent the day writing heredocs and
  recognised the record; instance 2 because I probed a running server instead of reading the
  card; instance 3 because I read the serializer instead of the card's summary of it. Each
  catch was a coincidence of what the reader happened to have in hand that hour. The rate at
  which this class is CAUGHT is not evidence about the rate at which it OCCURS, and all three
  were one review-pass away from becoming a rule other people would follow.
FIX: no tooling proposed, deliberately. `mutate.sh seams` and `survey` both answer "is this
  held?"; neither can answer "is the reason for this the reason it is true?", which needs a
  second derivation rather than a second run — and instance 2 is the proof, because a
  mutation PASSED and that pass is what did the damage.
  mixpeek-cicd's sentence is the whole of it and is worth quoting rather than paraphrasing:
  the answer being right is what makes the reason feel checked. The practical form, which is
  the only part that has ever worked for me: when a correct answer is about to become a RULE,
  re-derive it from a different starting point than the one that produced it. Instance 2 took
  a live probe against a running server, instance 3 took reading the code rather than the
  card, and instance 1 took a reader with different recent history. None took more than
  minutes; all three took a DIFFERENT SOURCE, not more care with the same one.
  Logged rather than built because I do not have a mechanism and would rather say so than
  ship a checklist item that joins the prose nobody enforces.
INSTANCE 4, and it is MINE, produced inside the card for this entry within the hour. Having
  written "no mechanism proposed", I built one: group the request log by family, flag any
  family that was called and never returned 2xx. It reported ONE finding across 89 families
  and looked clean and cheap. /api/workers was not in it — the family reports 4,016 of 4,394
  succeeding, because /api/workers/{id}/<verb> is 4,006/4,368 while /api/workers/{id} itself
  is 1/17. So the detector I wrote to catch instance 2 answered CORRECTLY at the granularity
  I chose and could not have found instance 2. A pass from it would have felt like evidence
  that AF-290's premise was fine. Re-run by ROUTE SHAPE it finds the defect immediately:
  713 shapes -> 9 candidates -> 1 survives a "is it actually mounted" filter, which is
  `GET /api/workers/{id}` at 0/15. Predicate and blind spots recorded on AF-298.
INSTANCE 5, from mixpeek-cicd, applying this entry to their own work an hour after reading
  it — and it sharpens the entry's own remedy rather than repeating it. They had pinned a
  config file with an assertion that the line above a key STARTS WITH `#`. `# TODO: revisit
  this setting` satisfies it, while the comment's actual job is to stop a future editor from
  restoring pytest defaults and silently deleting 49 tests. A comment-EXISTS check wearing
  comment-ANSWERS clothes.
  THE PART THAT CHANGES HOW I WORK: they had mutation-tested it. Their mutation DELETED the
  comment, which the weak assertion already caught, so the mutation passed and told them
  nothing. Their words: "A mutation is derived from the same understanding as the assertion,
  so it inherits the same blind spot by default. Mine was not a second derivation, it was the
  first one run backwards."
  That lands directly on this session, which has treated a killed mutation as proof roughly
  twenty times today. A killed mutation proves the assertion catches THE FAILURE I IMAGINED.
  It says nothing about the failure I did not. Their tell is the cheap version and it costs
  one sentence: STATE A MUTATION THE ASSERTION SHOULD CATCH AND DOES NOT. If you cannot
  generate one, that is a fact about your imagination, not about the assertion.
  Applied immediately to instance 4's own predicate before proposing it, which produced four
  blind spots I would otherwise have shipped silently — the worst being that it keys on
  STATUS, so a route answering 200 with an error body passes it, across 1,646,523 2xx rows
  nothing inspects for that shape.
THE TAXONOMY, from mixpeek-cicd reading instances 4 and 5 back and refusing to let them be
  one thing. Three shapes, and the remedies differ, which is why separating them is worth the
  paragraph:
    NARROWER than the question. The predicate is weaker than the property, over the right
      object. Their pytest.ini: "line starts with #" against "the comment explains why not to
      change this". Remedy: state a mutation the assertion should catch and does not.
    COARSER than the question. The predicate is right, the population is a superset that
      CONTAINS its own counterexample. My /api/workers: 4,016/4,394 at family level is a true
      number that includes the 1/17 it hides. Remedy: re-key at the granularity of the
      finding. Their sentence for why this one survives review better: the number it reports
      is genuinely true.
    WRONG FIELD. The predicate is right-shaped and reads a different field than the one
      carrying the answer. Blind spot 4 above: keyed on STATUS, so a 200 with an error body
      passes, across 1,646,523 rows every one of which is genuine evidence of something you
      are not asking about. Remedy: ask which field carries the answer before asking whether
      it is held.
  ONE CLAUSE OF THEIRS IS TOO STRONG, and saying so is the same courtesy they paid me on the
  absorption wording. They wrote that no amount of second-derivation fixes the coarse case,
  "because the second derivation would also have been per-family". In fact the live probe —
  `GET /api/workers/{lane}` -> 404 across 12 lanes — is what found it, and that IS a second
  derivation from a different source. What their argument correctly establishes is narrower
  and more useful: A SECOND DERIVATION HELPS ONLY IF IT VARIES THE DIMENSION THE FIRST ONE
  COLLAPSED. Same source at a finer granularity works; a different source at the same
  granularity does not. "Re-derive from a different source" was my own remedy two paragraphs
  up and it is underspecified: the axis matters more than the source.
INSTANCE 6, mixpeek-cicd's, and it is the coarse shape on a third surface — which matters,
  because three instances in one repo would be three names for one thing. Their words:
    "npm audit reported `1 high` on the homepage lockfile. The count is accurate and names no
    package, so it cannot be routed: severity is an aggregate over advisories, and the
    decision needs the advisory. `npm audit --json` per package is the finer key, and it
    turned a number into a name. The failure mode is not a wrong count, it is a correct count
    that excludes the item, which is why nobody challenges it and why it sat."
  A CI guard, a route table and a package audit. Three surfaces that fail differently, one
  shape.
AND A DEFECT IN HOW THIS FILE IS WRITTEN, which is mine and worth more than the instance.
  mixpeek-cicd built their too-strong clause from my WRITE-UP order — family detector first,
  live probe second — when my WORK order was the reverse. Their note on it: an account of a
  finding is ordered for the reader, so treating its sequence as causal is a free way to be
  wrong about method. Every entry in this file is ordered for the reader. When the ORDER is
  load-bearing for the method — when the point is which step found the thing — say which
  order you are giving, because a reader reasoning about method from a narrative sequence is
  doing something reasonable that the narrative did not warn them about.
THE UNIFYING FORM, mixpeek-cicd's, better than my "no remedy subsumes another": each shape is
  a PROJECTION that loses a different dimension, so a remedy restoring one cannot restore the
  others. Narrower loses predicate strength, coarser loses granularity, wrong-field loses the
  field. That is also why their enumeration guard and ts-gke's denominator check are not
  ranked — projections of one corpus along axes neither reaches from the other.
  Their consequence, which is the sentence I would put at the top of this entry if entries had
  tops: "my guard passes" is never a statement about the system, only about the axis, and the
  only honest closing line is which axis somebody else is holding.
NOTE: distinct from AF-435 (checks that ran, passed and could not have failed). That one is
  about an instrument with no discriminating power. This is about an instrument that
  discriminated CORRECTLY and a human generalising the wrong invariant from the result.
  Instances 4 and 5 are the bridge between them: a check with real discriminating power, at
  the wrong granularity or over the wrong property, produces a TRUE result that supports a
  false conclusion — and a mutation drawn from the same understanding confirms it.

## An answer-only prompt still cards when its no-op tail isn't one of ten hardcoded literal strings
AREA: harness
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-03
SESSION: amux-testing-e2e
CARD: ATE-17
SYMPTOM: Yesterday's fix (53b3e952, archived from frustrations.md 2026-09-02, validated
  against the literal specimen "...? Please answer only; do not change anything.") stops
  carding THAT exact string. A same-session E2E rerun today sent the same question with a
  differently-worded but equally answer-only tail: "...? Answer only; do not change files
  or create board work." is_informational_query()'s ANSWER_ONLY_TAILS list in
  crates/amux-core/src/board.rs matches ~10 hardcoded literal tail strings, not a
  structural "no imperative here" signal; this tail isn't one of them, so the tail-check
  fails closed and the question-word branch never fires. Two cards (ATE-15, ATE-16) minted
  for two paraphrased answer-only questions in one E2E run.
COST: The exact friction the archived entry described recurred one day later under a
  paraphrase, consuming two board ids and two WIP-adjacent doing slots for pure Q&A that
  was already answered inline both times.
FIX: f999caff replaces the literal ANSWER_ONLY_TAILS list with `tail_is_answer_only()`,
  which splits the tail on the same connectors `capture_has_task_followup` already uses
  for the pre-question clause and requires no resulting clause starts a task per the
  existing `capture_clause_starts_task` verb check — reusing the mechanism already
  trusted for the rest of the function instead of an enumerable list. Pinned the ATE-17
  specimen plus a negative control (a real task stacked after an answer-only opener must
  still card); `scripts/mutate.sh` confirms the negative control can actually fail.
  NOT YET independently re-validated against the running server build — see AF-433's
  discipline for what that validation should check before this entry is archived.

## staged-guard blocks on an edit-ownership record that a plain `git diff` is enough to create
AREA: attribution
SEVERITY: blocks
STATUS: open
DATE: 2026-09-03
SESSION: amux
CARD: AMUX-4083
SYMPTOM: Two independent blocks in one hour, both false, both naming a session
  that had only READ the file.
  (1) mixpeek-oss went to commit two browser.rs paths and staged-guard refused,
  reporting that session `amux` had an edit record on both files 3 minutes
  prior. What `amux` had actually done in that window was `git diff` and
  `grep` on those paths, to describe them accurately in a message ASKING
  mixpeek-oss to commit them. No write. They cleared it with
  AMUX_VERIFIED_SOLO=1 after checking the diff content and line counts were
  identical before and after.
  (2) Fifteen minutes later the guard blocked `amux` from running
  `git checkout --theirs` on app.css and sw.js to resolve a MERGE CONFLICT,
  naming amux-homepage: "discarding a file ANOTHER SESSION HAS ALSO EDITED ...
  UNRECOVERABLE". Reconstructing the ours-side of the conflict and diffing it
  against HEAD gave 0 differing lines for sw.js, and every app.css difference
  traced to #184's own auto-merged hunks. No peer content existed in either file.
COST: About 25 minutes across two sessions, and a cross-session round trip that
  existed only to clear the first block. The second one is worse than the time:
  the refusal text says UNRECOVERABLE and instructs you to stash or ask the named
  peer, so the honest response to a false positive is to stop and ask a session
  that has nothing to do with the file. It also teaches the wrong lesson, since
  the way past it is an override flag, and a guard whose normal resolution is its
  own bypass stops being read.
FIX: Do not derive edit ownership from mtime alone. CLAUDE.md already states the
  rule the guard violates: "An owner derived from mtime is not evidence ...
  reports whoever was ACTIVE, not whoever WROTE, because every lane shares the
  cwd." Record ownership from an actual WRITE — the PostToolUse hook already sees
  Edit/Write tool calls and could stamp content identity (a hash of the file
  before and after) instead of a timestamp. AMUX-3954 is the same defect stated
  as "an observed co-edit record carries no content identity, so it names a
  session for a write it did not make"; this entry is two measured specimens of
  it, one of which blocked a peer rather than the recorder. Second, a file in
  CONFLICTED state is a distinct case the guard does not model: its content is
  git-generated, so "another session also edited it" cannot be inferred from the
  working copy at all.
CO-SIGNED: mixpeek-oss, who hit specimen (1) from the blocked side and
  independently verified it the same way ("read-only git diff/grep during
  message composition, flagged as an edit ... a signal with no way to
  distinguish read from write").

## A process killed before it can log leaves the fleet no diagnostic surface for the failure that removes the diagnostic surface

AREA: instruments
SEVERITY: wrong-conclusion
STATUS: open
DATE: 2026-09-03
SESSION: amux-frustrations
CARD: AF-458
SYMPTOM: the server is in a launchd crash loop and NOTHING in its own logs says so.
 macOS SIGKILLs it at exec for `Code Signature Invalid` / `Launch Constraint
 Violation`, so it dies before any of our code can write a shutdown line. Both
 StandardOutPath and StandardErrorPath point at ~/.amux/logs/server-rs.log, and the
 last line before each death is an ordinary WARN. The only honest record is
 ~/Library/Logs/DiagnosticReports/*.ips plus `launchctl print`, where `runs` went
 10 -> 18 -> 23 in about two minutes and `properties` reads "needs LWCR update".
COST: this is the flap the whole fleet is hitting, and it presents as five unrelated
 problems. It forced gtm-engine's send onto the unstamped fallback (see the two
 entries above), made `amux board retitle` exit 7 with no message, broke a `git
 commit` with "unable to write new_index file", and made two /api/board reads
 return empty. Each looks like its own bug. Worse, the log carries an ERROR-level
 line 24 seconds before a death — "migration VERSION COLLISION at 35" — which is
 loud, adjacent, and irrelevant: migrate.rs:636 documents it as deliberately
 non-fatal ("this reports rather than refuses ... a gate with no truthful path,
 ethos rule 3") and it appears identically on runs that stayed healthy. A wrong
 cause was one step away and I nearly filed it. Fifth AF-445-shaped near-miss in
 this session.
FIX: not actioned — the remedy touches a launchd agent and ~/Dev/CLAUDE.md requires
 explicit owner approval ("This machine runs 24/7. Do NOT restart launchd agents").
 One-shot is `launchctl bootout gui/501/com.amux.server-rs` then `bootstrap`, since
 the binary itself verifies clean on disk and it is launchd's cached Lightweight
 Code Requirement that is stale. The durable fix is the builder re-bootstrapping the
 agent after it swaps the binary; until then every deploy on this box reopens the
 window. The INSTRUMENT half is the part that belongs here: a process killed before
 it can log needs its death reported somewhere a lane already looks. /health going
 unreachable and `/api/debug/*` being unreachable at the same moment means the fleet
 has no diagnostic surface for exactly the failure that removes the diagnostic
 surface.
NOTE: gtm-engine independently confirmed this from the other end and bounded it
 (origin-stamped, 2026-09-03). They closed five cards inside a flap window trusting
 a "-> done" line, re-read all five at the FIELD, and found two gaps that were their
 own omissions rather than the crash loop. Their conclusion: "on this lane the flap
 degraded loudly every time and silently never." Every symptom seen so far is
 fail-loud (curl rc 7, empty body, refused index write, a verb exiting non-zero with
 no message); nothing yet shows a write that REPORTED success and did not land. So
 the failure mode is availability, not silent corruption, which is the difference
 between a degraded fleet and one whose records are suspect. Not a reason to leave
 it running; it is a reason not to re-verify every board write made today.
NOTE: CAUSE CORRECTED, 2026-09-03, same session. The codesign SIGKILL is real
 (crash report 160828.ips) but it is NOT what drives the climbing run counter, and
 I recommended a fix that would not have worked. Three facts I should have checked
 before recommending anything: only ONE crash report all day against 76 runs (a
 codesign kill writes one per death), the binary unchanged since 16:10 so there is
 no swap-kill-swap cycle, and `codesign --verify` clean right now. What is actually
 happening is a port race: an agent session started `AMUX_RS_PORT=8824
 amux-server-rs` by hand in a gemini-shell background job (pid 20191, parent a
 /bin/bash -c with `trap 'jobs -p > "$_bgpids_file"' EXIT`), it holds 8824, and
 launchd's managed copy cannot bind, exits cleanly with 78, and KeepAlive respawns
 it forever. Clean exit, hence no .ips. So `bootout`/`bootstrap` would have resumed
 losing the same race. The entry's INSTRUMENT argument survives intact and is if
 anything stronger: a process that exits before binding logs nothing either, both
 halves of `runs`-climbing-with-a-silent-log look identical, and I distinguished
 them only by counting crash reports, which is not a thing any lane would think to
 do. The deeper problem this exposed: the fleet's live server is an UNSUPERVISED
 background job that dies with its parent shell, while the supervisor that should
 own it is locked out of the port.

## An archived card is listed as actionable and refuses every closing action

AREA: board
SEVERITY: wrong-conclusion
STATUS: open
DATE: 2026-09-03
SESSION: gtm-engine
CARD: AF-460
SYMPTOM: a card can hold `archived: 1`, `status: backlog` and `closed_at: None` at
 once. It appears in the DEFAULT `/api/board` list, which is what the idle nudge
 reads, so it is offered as a drainable backlog card with "you have to pull from
 it". Every closing verb then refuses with `archived_task_immutable` / "task is
 archived; restore it first". The nudge says drain it; the board says you cannot.
 The asymmetry is what makes it permanent: `--trigger` DOES work on an archived
 card, so such a card is silenceable forever and closeable never.
COST: 26 days on GE-564, whose trigger sat 617h stale while it re-listed. A triage
 on 2026-08-20 chose ARCHIVE, the archive neither closed nor hid it, and nobody
 could close it afterwards. SECOND INSTANCE, and mine is the worse one: I hit the
 identical refusal on AF-224 the same day and read it as "already archived, no
 action needed" rather than as a defect. A lane that shrugs at the refusal never
 reports it, which is why one card absorbed 26 days before anyone said so.
FIX: not chosen — three candidates land in different places and it is a data-model
 call: (1) archiving sets a terminal status, (2) the default list excludes
 archived, (3) the nudge filters them. Recommending (1), because (2) and (3) leave
 a card that is simultaneously backlog and archived and merely stop showing it to
 one reader. Workaround that works today and is documented nowhere: PATCH
 archived:0, then done. Companion entry: the refusal message is correct and only
 reaches you when you ACT, never where the card is listed (AF-461).

## 19 NUL bytes made grep call a 67 MB log binary, and every `grep -o` sweep silently lost half its matches
AREA: instruments
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-frustrations
CARD: AF-481
SYMPTOM: same file, same pattern, three answers. `grep -c 'verdict=READ'` -> 17
 lines. `grep -o 'verdict=READ' | wc -l` -> 8. `grep -ao ...` -> 17. grep declares a
 file binary if it contains a single NUL byte and then SUPPRESSES match output while
 `-c` keeps counting lines, and it says nothing at all when its output goes to a
 pipe. The source was one warn: the create-path acyclicity check passes a placeholder
 self id of `"\u{0}new-card"`, chosen because no real card id can contain a NUL,
 which is correct and also true of a space. `depends_on_cycle` logs it as
 `self_id = %self_id`, so 19 NULs landed in server-rs.log from one stuck cycle
 (GE-473 -> MHC-256) retried across three days.
COST: 53% of the matches, in the reassuring direction, on the instrument this repo's
 own log-sweep doc prescribes. I nearly filed AMUX-2841's specimen count as 8 when it
 is 17, which would have understated a watch's evidence by half. Nineteen bytes in 67
 MB is enough, so no amount of the file being "mostly text" protects you. The wider
 shape is that a probe can be correct, run cleanly, exit 0 and answer about a
 different population than the one you asked about, with nothing beside the number
 saying so.
FIX: db3ff38a and accbba96. The sentinel is now `"(new card)"`; non-collision is
 unchanged, since card ids are `[A-Z]+-<digits>` and a space and parentheses are as
 impossible as a NUL was, and it survives a log. The guard asserts the PROPERTY, not
 the string: no control characters, at least one character no card id can contain,
 and non-empty so neither can pass vacuously. Two mutations fire (back to the NUL
 sentinel; a valid-id-shaped "NEW-0"). The repo CLAUDE.md now tells lanes to grep
 that file with `-a`, because AF-481 removed this source and any logged payload can
 reintroduce one.
 SELF-CORRECTION, recorded because it is the same class: I first reported 216,873
 NULs from `grep -c $'\0'`. bash cannot put a NUL in a string, so `$'\0'` is the
 EMPTY string and that command is `grep -c ''`, a line count wearing a NUL count's
 label. The real figure is 19, read from the bytes in python. Both halves of this
 entry are a probe whose argument silently became something else.

## `GET /api/board` returns a WORKING SET, and the cap is disclosed only in headers the prescribed recipe cannot see
AREA: instruments
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-frustrations
CARD: AF-480
SYMPTOM: `GET /api/board` returns 2,053 rows; `GET /api/board?all=1` returns 12,745.
 The default caps terminal rows (done/verified/discarded) to the most recently
 updated FLEET-WIDE, which across ~50 lanes can be none of yours. The cap is right
 and the server is honest about it, in RESPONSE HEADERS: `x-amux-truncated: 1`,
 `x-amux-total: 2052`, `x-amux-terminal-total: 10793`. The body is a bare JSON array
 with no envelope, and the recipe in ~/.claude/CLAUDE.md is
 `curl -sk $AMUX_URL/api/board | python3 -c "..."`, which cannot see a header.
COST: reconciling all 84 frustrations.md entries against the default listing reported
 63 cards as MISSING FROM THE BOARD. All 63 existed. That is a whole reconciliation
 pass, and the report it produced was wrong in the direction that invents work: it
 would have had me file 63 duplicate cards for entries that already had one. The
 `amux` CLI ALREADY reads those headers and prints the cap, and its own comment says
 why ("nobody reads response headers from a pipe, which is ethos rule 4's second
 layer: a tag in a store the reader never opens") — so the capability existed, was
 correct, and did not reach the path CLAUDE.md tells every lane to run. Ethos rule 1:
 a feature nobody can name is a feature nobody has.
FIX: ~/.claude/CLAUDE.md now shows both forms with their row counts, says the cap is
 header-only and that a raw curl cannot see it, and points at `amux board ls` which
 can. The "read the whole board" recipe in the task-ledger section takes `?all=1`,
 since that one is asked to be exhaustive by its own sentence. Not fixed and
 deliberately not attempted: putting the disclosure in the BODY would need an
 envelope, and every consumer of that endpoint parses a bare array.

## "commit by pathspec" protects other files, and does nothing for a file you BOTH edited
AREA: attribution
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-frustrations
CARD: AF-485
SYMPTOM: ~/.claude/CLAUDE.md prescribes, for a shared checkout, "commit by pathspec.
 `git commit <your paths>` ignores the index for everything it does not name and
 leaves their staged entries untouched." Every clause is true and the paragraph reads
 as a general guarantee against sweeping a peer. It is not one. `git commit <path>`
 takes the WORKING TREE state of that path, all of it, not your hunks — so a peer's
 UNSTAGED edits to a file you name land in your commit, under your message and your
 Amux-Session trailer, while the pathspec does exactly what it promises. The same
 paragraph pre-emptively dismisses `git add -p` ("theirs are already staged"), which
 is correct for the state it describes and wrong for this one, so the reader is
 steered away from the one tool that would have helped.
COST: self-traced, 2026-09-04. I committed 66818693 by pathspec on
 crates/amux-server/src/api/session_verbs.rs and swept amux-testing-e2e's
 uncommitted Codex composer-footer fix, its LIVE_CODEX_IDLE fixture and its
 regression test: four of six hunks, 59 of 171 added lines. I pushed it before they
 could tell me, so the remedy CLAUDE.md prescribes for an absorbed change (do not
 rewrite shared history; record the reasoning in a follow-up) is now the only one
 available. The code survived correct and tested; the REASONING did not, because my
 commit message is entirely about a diagnostic window parameter and says nothing
 about Codex footer chrome. They found it, not me, and they asked whether they could
 push a commit that was already on origin.
 The guard was honest and I misread it: the commit printed "no transcript for RUNNING
 cotenant(s) amux-codex — their edits are INVISIBLE to this verdict". It never named
 amux-testing-e2e. A guard saying it cannot see is telling you to look.
 Filed the same day I measured that 21 of 75 live entries in this file are the one
 shared-index class (AF-336). This is the twenty-second, produced by following the
 guidance written to prevent it.
FIX: the pathspec paragraph in ~/.claude/CLAUDE.md now states what pathspec does NOT
 cover, carries the measurement above, and gives the three commands to run before a
 pathspec commit on a co-edited file: `git diff -- <path>`, `git diff --cached --
 <path>`, and `git add -p -- <path>` followed by a commit with no pathspec. It also
 says why `add -p` is right here despite the dismissal three lines above it: that
 dismissal is about a peer's already-STAGED work, a different state.
 NOT FIXED and deliberately not attempted: making the staged-guard see a cotenant it
 has no transcript for. It already reports that blind spot by name, which is the
 honest behaviour; the defect was in the guidance, not the guard.

## An idle Codex prompt was labelled `UNSUBMITTED TEXT` using its model/path footer as the draft
AREA: instruments
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-36
SYMPTOM: the worker's stop hook said idle, Codex's structured rollout said idle,
 zero subagents were live, and the pane visibly ended at the dim `Ask Codex to do
 anything` placeholder. `/api/sessions/amux-testing-e2e` nevertheless returned
 `status: waiting`, `composer_stuck_since > 0`, and
 `composer_preview: gpt-5.6-solxhigh~/Dev/amux`; the dashboard rendered that
 override as `UNSUBMITTED TEXT`. The composer reader stopped on Claude's border
 and status-bar glyphs but Codex puts its model/effort/path footer directly below
 the prompt with no border, so the footer was concatenated into the input.
COST: the worker presented a false human-action state for roughly three hours and
 contradicted both of its structured state sources. A human could have pressed
 Enter to submit what the UI claimed was pending, although there was no command
 in the composer.
FIX: 66818693. `composer_state` now treats Codex's ANSI-styled middle-dot
 model/path footer as a structural boundary without naming any model or version.
 It deliberately requires the raw styling and therefore fails toward visible
 `Typed` if Codex changes its chrome, never toward a false successful send. The
 live-frame regression first reproduced `Typed("gpt-5.6-solxhigh~/Dev/amux")`,
 then passed as `Placeholder`; its control keeps real typed text pending. Live
 build `668186939734` cleared `composer_stuck_since` and `composer_preview`, and
 the browser changed from the false badge to the worker's actual idle/working
 state in real time.

## One blocked To Do hid an independent backlog from automatic draining
AREA: coordination
SEVERITY: stuck
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-37
SYMPTOM: `mvs-research` could finish its active card and then remain idle with
 MR-14 in To Do and 18 cards in backlog. MR-14 depends on MR-27, which the lane
 cannot complete, while MR-150 and other backlog work have no dependency. The
 pickup selector only tried backlog when its To Do SQL query returned zero rows;
 one blocked row therefore hid every independent backlog card even though this
 worker had the default auto-drain configuration enabled.
COST: the board advertised queued work and the worker advertised idle, but the
 driver repeatedly returned `all-candidates-refused`. Progress then depended on
 a person noticing the mismatch and moving a backlog card by hand. A tempting
 workaround would be to ignore dependencies or claim backlog directly, both of
 which would weaken the board's gates.
FIX: ATE-37 makes automatic draining a fallback after every To Do candidate has
 been evaluated and honestly refused. It still gives runnable To Do priority and
 still promotes exactly one backlog card through backlog -> todo -> doing. The
 drainable backlog query now excludes cards with open dependencies, so it cannot
 merely move the same blockage sideways. The regression reproduces an older
 blocked backlog card, a blocked To Do, and a newer runnable backlog card and
 requires the runnable card alone to be selected. All 106 board-driver tests
 pass, including WIP, needs:you, freshness, capture-shell, irreversible-action,
 dependency-promotion, and explicit auto-drain opt-out controls.

## Codex kept a live turn above its prompt shell and the dashboard called it idle
AREA: instruments
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-38
SYMPTOM: while `amux-testing-e2e` visibly showed `Waiting for background
 terminal (... esc to interrupt)` directly above `Ask Codex to do anything`,
 `/api/status-explain` set `pane.says_working=false` and the Workers UI labelled
 the session idle. The status parser treated the persistent Codex prompt/model
 shell as a newer idle boundary even though Codex paints that shell throughout
 an active turn.
COST: the worker's current board card lost its working highlight during a real
 generation. That makes the board contradict the terminal and can also let the
 driver reason from a false idle state.
FIX: ATE-38 recognizes the exact adjacent live shapes on both supported Codex
 layouts: older builds paint `Working` after the submitted prompt, while current
 builds paint `Working`, `Running`, or `Waiting for background terminal/command`
 immediately before the disabled prompt shell. A separated historical row still
 cannot override the newest prompt, and a completed `Worked for` row is an idle
 control. Adapter and end-to-end status-truth tests cover active, completed,
 stale-row, queued-message, and prompt-churn cases without matching arbitrary
 transcript prose.

## A commit naming ATE-38 attached its outputs to the newer ATE-39 card
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-39
SYMPTOM: commit `be87f031` named `(ATE-38)` in its subject, but the post-commit
 `commit-report` appended the commit activity and derived links for
 `sessions_legacy.rs` and `be87f031` to the currently-Doing ATE-39. ATE-38 had
 no durable artifact rows when the report landed. The endpoint ignored the
 explicit task id and selected the worker's most recently updated in-flight card.
COST: the board put another task's source and commit on this card and left the
 producing task without its outputs. The user had to inspect both cards, identify
 the wrong newest-card guess, and provide a corrective live specimen before the
 task record could be trusted.
FIX: ATE-39 (this commit). `commit-report` reads the full subject and changed-file
 list from the git object, attaches by an explicit body/subject task id, and
 refuses ambiguity instead of guessing newest. It stores the full SHA and every
 changed file as durable rows on that exact task and emits
 `commit_report_task_exact` / `commit_report_task_ambiguous` log markers.

## Evidence hid a real .env file and rendered the 1.93M row count as a file
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-39
SYMPTOM: TUBES-2426 evidence named `customers/tubescience/.env` but Board details
 returned no asset link. TUBES-2428 named the same file yet returned only `1.93M`,
 a decimal measurement, as a clickable missing-file asset. The parser required a
 non-empty stem before the final dot and accepted alphabetic measurement suffixes.
COST: the actual customer configuration artifact disappeared from two task records,
 while one record sent a reviewer toward a manufactured file. The user had to
 compare the two live payloads to show that the positive and negative parser arms
 were both backwards.
FIX: ATE-39 (this commit). Hidden leaf files and hidden path components are
 accepted, decimal measurement tokens are rejected, and bare/relative dotfiles
 resolve against the producing worker directory. File rows now render as semantic
 buttons; local availability and external reachability-not-measured verdicts make
 missing and unreachable assets explicit.

## Worker progress was recorded while its exact Board card stayed unclaimed
AREA: board
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-41
SYMPTOM: general-canvas-apps posted four status updates describing active work on
 GCA-153, but the card remained in To Do and `task_board_id` stayed empty. The
 status-update endpoint appended the text and artifacts without participating in
 the Board claim transition.
COST: the Board showed an actively executing worker without its current card and
 left the same card eligible for another pickup. The user had to correlate the
 worker transcript, card log, and session payload to identify the disagreement.
FIX: ATE-41 (this commit). An owned, actionable To Do/backlog card is now claimed
 in the same serialized transaction that appends the progress line and artifacts.
 Cross-worker, blocked, dependency-held, fresh-trigger, WIP-conflicting, waiting,
 and later-state updates remain informational and return a named refusal verdict;
 claimed and refused paths emit distinct sweep-visible log markers.

## An idle Codex worker borrowed a sibling's active rollout and showed WORKING
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-42
SYMPTOM: `amux` sat at the empty `Ask Codex to do anything` prompt and its own
 stop hook reported idle, but the dashboard showed WORKING on stale AMUX-4079.
 `status-explain` named `codex_rollout`; the chosen rollout actually belonged to
 active sibling `amux-testing-e2e`, which shares `/Users/ethan/Dev/amux`.
COST: the worker header and Board highlight asserted current execution where
 none existed, while two workers' transcripts and lifecycle signals were
 cross-linked solely because they used the same checkout.
FIX: ATE-42. An explicit Codex session id still wins. Before one exists, rollout
 fallback now canonicalizes the cwd and selects only the rollout born within a
 bounded window around that worker's own `last_started`; outside that window it
 refuses to guess and lets the exact terminal/provider signals decide.

## WIP-capped ready work was labelled STALLED while its holding task progressed
AREA: dashboard
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-43
SYMPTOM: TubeScience was idle at its main prompt while detached work continued
 on TUBES-2418 and TUBES-2419 correctly waited behind WIP-1. The worker header
 rendered the red `STALLED · 1 READY` chip even though `/api/board/ready` named
 TUBES-2418 as the current WIP holder.
COST: a healthy, intentionally serialized queue looked like a broken autonomy
 loop. The label hid both card identities, so the user could neither see what
 was waiting nor open the task that explained the wait.
FIX: ATE-43 (this commit). The ready frontier retains card identities and renders
 TUBES-2419 as queued behind a clickable TUBES-2418 control. Only ready work with
 zero claimable cards and no holding work keeps the stalled verdict. A one-shot
 `idle-ready-work` client beacon records which classification rendered.

## Peek and worker-card action menus drifted into different products
AREA: dashboard
SEVERITY: wrong-action
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-44
SYMPTOM: the worker card exposed 25 worker actions and configurations, while the
 peek overflow exposed only File browser and Focus mode. Worse, the peek File
 browser opened a desktop-only split pane while clicking the displayed directory
 entered the canonical full Files route for the same worker and path.
COST: the place where the user was already operating a worker hid almost every
 control, and two labels for the same file-browsing intent produced different
 session, navigation, and visible-state outcomes. A duplicated `peek-more-btn`
 id also made automation and DOM lookup choose whichever button came first.
FIX: ATE-44 (this commit). Both surfaces render one shared worker-action
 inventory; peek retains its two additional controls. All three peek file entry
 controls call one canonical full-route helper, the two overflow buttons have
 unique semantic IDs, and mismatch/file-entry verdicts reach client-debug logs.

## Claude's background-agent wait row had two conflicting status parsers
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: Primis visibly showed the provider-owned `Waiting for 1 background
 agent to finish` row and an active Explore agent, while the dashboard header
 said IDLE. `backend/adapter.rs` classified the row active, but the legacy
 session projection called a second parser that omitted it.
COST: the user had to reconcile the terminal, agent panel, session payload and
 status-explain output to establish that real work was still running.
FIX: ATE-45 shares one chrome-anchored singular/plural predicate between the
 adapter and session status path. The exact provider row overrides an idle
 parent-prompt report; quoted prose remains a negative control.

## Subagent lifecycle truth disappeared whenever the server rebuilt
AREA: hooks
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: Primis SubagentStart and SubagentStop hooks both recorded `http=000`
 during a server rebuild. The callbacks were one-shot, so the server retained
 neither the active agent identity nor its final stop after coming back.
COST: the authoritative live-agent count read zero during real work and could
 also remain positive after a lost stop; recovery depended on a later process
 reset rather than replaying the facts that had already happened.
FIX: ATE-45 gives each lifecycle edge a session/agent/event identity, persists
 it in a bounded fsynced FIFO, replays oldest-first across outages and response
 loss, and deduplicates durably in the server. Permanent 4xx poison events
 dead-letter with full identity; 000/5xx retry; any later hook wakes the queue.

## Board-drive interrupted Codex while its background terminal was running
AREA: board
SEVERITY: wrong-action
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: this ATE-45 turn visibly showed Codex's `1 background terminal
 running` provider row, but board-drive sent an Idle nudge and Codex reported
 `Conversation interrupted`. The status adapter already understood the row;
 the steering boundary trusted a fresh idle parent report without reading it.
COST: the harness interrupted its own green test run, forced the model to
 reconstruct its place, and demonstrated that dashboard truth and delivery
 safety still disagreed on the same frame.
FIX: ATE-45 makes the shared structured Codex pane state override an idle
 parent report for status, board-drive and steering. The hold clears on the
 completed `Worked for` frame, and quoted copies of the text do not match.

## Durable lifecycle replay posted valid events to the server root
AREA: hooks
SEVERITY: wrong-action
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: commit 483ff0aa queued the base `AMUX_URL`, so its drain POSTed every
 SubagentStart/Stop to `/` instead of `/api/sessions/<worker>/report`. The live
 server returned 405 and the new permanent-4xx rule immediately dead-lettered
 the valid lifecycle facts.
COST: ATE-45 was committed, deployed and moved to review with a green chaos
 suite while its central production path delivered zero lifecycle events. A
 second read-only review and live failure-log inspection were needed to catch it.
FIX: ATE-45 now constructs one canonical per-session report URL used by queued
 and immediate delivery. The fake server returns 405 for every other path, every
 captured request asserts its exact worker route, and the failure log retains
 URL, HTTP verdict and event identity.

## Submitted and pasted provider frames impersonated live background work
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: Claude's broad dingbat range included its own `❯` input glyph, so a
 prompt containing the exact waiting sentence read active. The provider-agnostic
 Codex fallback likewise accepted a pasted Codex frame inside Claude output.
COST: user-authored text could pin a truly idle worker WORKING and suppress its
 ready queue indefinitely; the original negative tests covered only unprefixed
 prose and same-line Codex quotation.
FIX: ATE-45 accepts only measured Claude spinner glyphs at column zero, excluding
 the prompt and indented pasted rows. Provider-known Codex scans keep partial-
 frame support, while provider-agnostic status requires the current exact Codex
 prompt/model-footer structure. Prompt, indented and cross-provider pastes are
 explicit negative controls and unknown variants remain sweep-visible.

## An older start could resurrect an agent whose stop arrived first
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: server lifecycle state remembered only live IDs plus 128 recent event
 IDs. A stop delivered before its older start was discarded as an orphan; once
 a start ID aged out, replaying it after the final stop made the agent live again.
COST: response reordering or a sufficiently delayed retry could leave a worker
 permanently WORKING after every child had completed, defeating both accurate
 status and automatic Board pickup.
FIX: ATE-45 stores monotonic per-agent live/terminal edges. Stop-before-start is
 a durable tombstone; older/equal resurrecting starts are rejected with named
 verdicts. Terminal entries compact to a bounded set plus a timestamp floor, so
 evicted tombstones still reject ancient replay and resets preserve generations.

## Steering's deadline overrode its live-background safety hold
AREA: messages
SEVERITY: wrong-action
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: ATE-45 mapped exact provider/background evidence to `active`, then fed
 it to the ordinary max-age rule, which deliberately turns old active messages
 into mid-turn delivery. A long agent or background terminal was still
 interruptible after `AMUX_STEER_MAX_AGE_S`.
COST: the safety fix postponed the same conversation interruption instead of
 preventing it, contradicting its own regression name and acceptance contract.
FIX: ATE-45 carries background work as a separate hard-hold fact into the
 delivery decision. No message age can bypass it; only the reported final stop
 or provider terminal frame clears the hold, while ordinary foreground turns
 retain the existing starvation deadline.

## The lifecycle drain could strand its bounded tail or lose the final wakeup
AREA: hooks
SEVERITY: stuck
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: one drain performed only 90 total loop iterations although the queue
 admitted 128 rows. At the empty boundary, a producer could enqueue and launch
 a replacement before the old drain released its nonblocking lock, so both
 exited with the final row still queued.
COST: up to 38 healthy events could remain behind an entirely healthy server,
 and the last SubagentStop could sleep until an unrelated future hook happened.
FIX: ATE-45 spends the 90-attempt budget only on retryable failures, so successes
 drain the complete bounded FIFO. The empty read releases drain ownership while
 holding the queue lock, replacements wait through the bounded handoff, and
 tests prove one process drains 128 rows plus the lock-race specimen.

## A corrupt lifecycle queue was silently replaced with an empty one
AREA: hooks
SEVERITY: data-loss
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: JSON read errors and wrong-schema JSON both became `rows=[]`; the next
 enqueue atomically overwrote the only bytes that could explain which lifecycle
 facts were lost. Malformed provider payloads also shared an empty dedupe key.
COST: a damaged queue erased its own evidence and multiple malformed but real
 callbacks collapsed into one, making the status error impossible to reconstruct.
FIX: ATE-45 atomically preserves corrupt bytes/schemas under a timestamped path
 and logs the queue, preserved path, error and verdict before recovery. Every
 malformed invocation gets a unique persisted identity; tests cover both corrupt
 forms and duplicate malformed callbacks.

## A green shared-target build embedded another worktree's dashboard
AREA: build
SEVERITY: wrong-conclusion
STATUS: open
DATE: 2026-09-04
SESSION: amux
CARD: AMUX-4142
SYMPTOM: A post-commit `scripts/safe-cargo.sh build -p amux-server` in the
 Basecoat integration worktree exited 0 and `/health` reported that worktree's
 `11c1b789` commit, but the same process served `APP_VER=0.9.804` and no
 `ui-system.js` from another worktree instead of its own `0.9.807` Basecoat
 assets. Both worktrees use the required shared `CARGO_TARGET_DIR`; Cargo
 treated the other checkout's `amux-dashboard` RustEmbed artifact as current.
COST: Seven minutes, an extra 2m32s server build, and a browser run that would
 have falsely certified the old UI if it had checked appearance without joining
 `/health.commit` to the actually served asset version.
FIX: Open as AMUX-4142. Make embedded-asset provenance part of the build
 fingerprint or have the build/deploy gate compare served APP_VER/CACHE with
 the source tree and emit a sweep-visible mismatch verdict.

## A lost Stop report left a finished Primis turn WORKING for 139 seconds
AREA: hooks
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: Root's live browser saw both Primis subagents finish and Claude return
 to its prompt at 15:22, but the worker card and input still said WORKING. The
 status explanation chose a fresh `prompt-hook` active report with zero live
 subagents; the hook log showed the missing edge exactly: `15:22:22 primis
 source=stop-hook http=000` during a server rebuild.
COST: the production UI contradicted the provider for 139 seconds and Board
 pickup remained suppressed after all work was terminal. It self-cleared only
 when the active-report trust window expired, not because the final fact landed.
FIX: ATE-45 makes main-turn state a durable singleton latest-wins queue using
 the same bounded detached drain as lifecycle events. A newer report atomically
 replaces an older pending state, so recovery cannot replay idle over a later
 active turn; successful recovery logs the state, identity, attempt and
 `replayed_state` verdict. The exact lost-Stop outage replays idle without any
 later hook and is a shipped regression cell.

## Claude's stale background-wait scrollback overruled a later completed turn
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: Primis visibly returned to its final prompt after
 `CLAUDE-POSTFIX-COMPLETE`, with zero live subagents and an idle stop report,
 but status-explain still set `provider_background_working=true` because an
 older provider-owned "Waiting for 1 background agent to finish" row remained
 in tmux scrollback.
COST: Workers stayed WORKING and turn-boundary-safe Board drive remained
 suppressed for minutes after the real work finished.
FIX: ATE-45 reads Claude's provider rows as ordered lifecycle edges: a newer
 completed-turn marker terminates every older wait, while a newer wait still
 wins. The status path logs `superseded_by_completed_turn`, and exact-frame
 adapter, detector and status-explain regressions cover the live Primis pane.

## One capture shell's cooldown hid the next non-work shell forever
AREA: board
SEVERITY: stuck
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: PRIMI-204 was explicitly classified by its prompt as "do not create or
 retain a board task", but a recent nudge for PRIMI-203 activated the lane-wide
 advance cooldown before PRIMI-204 received its own cleanup prompt. It remained
 in `doing` after the turn ended with no `decompose:PRIMI-204` event.
COST: a non-task occupied the Board indefinitely while the drive report called
 the lane healthy and no model was asked to make the keep/discard decision.
FIX: ATE-45 lets a newly captured shell rejected by the shared pickup classifier
 bypass an unrelated lane cooldown exactly once, prioritizes that exact card,
 and relies on the durable per-card `decompose:<id>` idem to close the exception.
 The bypass emits `capture_cleanup_bypassed_lane_cooldown`; a focused regression
 proves both the first nudge and duplicate suppression.

## Codex `turn_aborted` left an interrupted turn structurally active
AREA: status
SEVERITY: wrong-conclusion
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: After Codex displayed `Conversation interrupted` and returned to its
 empty prompt, both Workers surfaces stayed WORKING. Status-explain chose a
 fresh `codex_rollout` active vote even though the stop report was idle,
 `subagents_live=0`, and `provider_background_working=false`. The rollout held
 the exact missing edge: an `event_msg` whose payload type was `turn_aborted`.
COST: An already terminal turn suppressed safe Board drive and contradicted the
 provider UI until a later recognized lifecycle event replaced the stale vote.
FIX: ATE-45 treats both top-level and nested Codex abort events as durable idle
 boundaries, surfaces the chosen boundary in status-explain, and emits the
 `interrupted_turn_is_terminal` status-truth verdict. Exact rollout and pane
 regressions pin the interrupted-turn prompt frame for Codex and Ollama.

## An unsigned fake browser PID shut down every Linux CI runner process
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-44
SYMPTOM: Ten consecutive Rust check jobs ended around seven minutes with "runner
 received a shutdown signal" after commit 92044fc8 added a browser-reaper test
 seeded with PID 4294967295. The test called the real browser stop path, which
 passed that decimal string to Linux procps `kill -TERM`; procps returned success
 and treated the unsigned value as the signed process-group sentinel -1.
COST: Every descendant main run lost the workspace-test process and GitHub runner,
 blocking ATE-44 and ATE-45 verification while the 25-minute workflow timeout and
 passing test output falsely suggested external cancellation.
FIX: ATE-44 validates every browser PID at the signed OS boundary, refuses
 reserved/group values before constructing arguments or launching external
 `kill`, and emits `invalid_process_id_refused`. The original
 4294967295 fixture remains as an end-to-end regression, with boundary controls
 for PID 0, PID 1, ordinary positive PIDs, and the signed maximum.

## Multiplayer workspace switching was replayed later as offline work
AREA: cloud
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-codex
CARD: AC-416
SYMPTOM: In three simultaneous saved browser profiles, switching workspaces
 returned synthetic HTTP 202 `queued/offline`, reloaded as though it succeeded,
 and either stayed in the old workspace or changed context later when the
 outbox replayed. The god-mode account's switcher also rendered all 62 inherited
 workspaces as a giant green invitation banner, and chrome-cdp's shared
 `pages.json` made listing profile C erase the target lookup for profiles A/B.
COST: Ethan, god mode, and the Gmail participant could not be kept in one
 workspace long enough to prove cross-user board/log visibility; retries
 created delayed context switches, and the operator-facing page exposed the
 whole customer directory above the actual dashboard.
FIX: Workspace switching is now explicitly non-replayable, requires a real
 JSON acknowledgement, and reports `workspace_switch_failed` instead of
 reloading on failure. The gateway acknowledges JSON clients before entering a
 tenant container and logs `[org-switch] ... verdict=switched`. Org rows say
 `via_god_mode`, so inherited access remains in Settings without becoming an
 invite banner. chrome-cdp now scopes targets/sockets by profile or port and
 selects the requested profile from the multi-browser status array.

## Three-user cloud test saturated on minute-long workspace requests
AREA: cloud
SEVERITY: blocks
STATUS: open
DATE: 2026-09-04
SESSION: amux-codex
CARD: AC-416
SYMPTOM: The Gmail workspace stayed on "Starting your workspace" for several
 minutes while board/session reads in the other two profiles took 50-135s.
 The local 8824 control plane simultaneously repeated its known failure mode:
 TCP accepted, but TLS `/health` handshakes timed out until the watchdog or a
 manual launchd restart replaced the process.
COST: A browser-created Backlog canary existed only in Ethan's optimistic page
 state; after more than a minute the owner profile still had an empty board, so
 real cross-user observation and actor attribution could not be certified.
FIX: AC-416. The saved profiles and exact three-identity browser path now
 reproduce it without credentials, and the watchdog/server log records the TLS
 hang. Diagnose tenant wake latency and the local request-path stalls before
 claiming realtime multiplayer from a cached shell.

## A CLI negative control assumed its specimen was globally unique
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-44
SYMPTOM: Fresh descendant CI completed the full Rust workspace suite after the
 PID-safety fix, then `scripts/test-cli-launch-unbound.sh` refused to build its
 broken fixture because a second command legitimately gained the same local
 AMUX_API declaration used by cmd_start.
COST: The otherwise-valid ATE-44 descendant run stayed red after all 1,947
 amux-server library tests and every integration binary passed; the failure was
 attributed from the job log only after the old runner-shutdown defect cleared.
FIX: The negative control now counts and removes the declaration only within
 cmd_start, preserving the independent declaration in cmd_open_browser while
 still proving the broken fixture fails with an unbound variable. Its failure
 output names the scoped occurrence count.

## An async menu-reachability assertion raced the app's normal rerender
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-04
SESSION: amux-testing-e2e
CARD: ATE-44
SYMPTOM: The full Playwright run passed 300 scenarios, then the ATE-44 iOS
 Safari parity case failed because `scrollIntoViewIfNeeded` acquired the Focus
 action before a session poll rerendered the peek menu and detached that node.
COST: The six-case focused matrix had passed, but the first full descendant CI
 run stayed red after 16.7 minutes and could not satisfy ATE-44's final gate.
FIX: The parity test now scrolls and measures the final Focus action inside the
 same synchronous render snapshot. It still proves overflow, scrollability and
 viewport reachability while eliminating the cross-rerender locator lifetime.

## Staged-guard attributed this Codex task's files to two peer lanes and blocked its commit
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-09-05
SESSION: amux (Codex agent; no $AMUX_SESSION in env)
CARD: AMUX-3249
SYMPTOM: After implementing and browser-testing local multiplayer invites, the commit
  guard attributed the staged files to `amux-cloud` and `amux-frustrations` and refused
  the commit even though every staged hunk was produced by this task. The shell had an
  empty $AMUX_SESSION, but its tmux name resolved to `amux-amux` and the installed
  MR-43 prepare-commit hook already contained that fallback, so the commit stamp and
  the edit-record ownership used by the guard still disagreed.
COST: One refused commit and about 5 minutes re-reading all nine staged files by hand
  before the documented AMUX_VERIFIED_SOLO override could be used honestly.
FIX: AMUX-3249. Attribute Codex tool writes to the active agent/session, or make the
  guard distinguish absent agent edit records from affirmative peer ownership so a
  missing producer cannot be rendered as evidence that a peer authored the diff.

## Delegated worker requests held the requester's WIP instead of becoming task dependencies
AREA: scheduler
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-05
SESSION: amux
CARD: AMUX-4153
SYMPTOM: `amux board request` created a durable child task and terminal callback,
 but left the requester's active parent in `doing` with no `depends_on` edge. The
 requester therefore occupied its WIP slot and appeared idle until the peer finished.
COST: Delegation serialized work that should have run concurrently, hid independent
 ready work from the requester, and left the board without a durable record of why
 the requester was waiting.
FIX: ccb37879 atomically adds the delegated child to the parent task's dependencies,
 requeues the parent to `todo` to release WIP, and lets the existing ready frontier
 wake it when the child closes. `amux::task_dependency` now logs a verdict for every
 linked, standalone, ambiguous, invalid-parent, or cycle-refused peer request.

## Usage resets opened the clock gate after delivery had already been skipped
AREA: scheduler
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-05
SESSION: amux
CARD: AMUX-4154
SYMPTOM: A Claude worker with a known, elapsed reset remained protocol-level
 `RateLimited` until its first subsequent prompt, while the legacy lane sweep could
 roll a stale clock banner forward to the next day. The scheduler also recovered
 rate-limited workers after command delivery and planning had already consumed a
 stale snapshot, leaving the worker idle for another tick.
COST: Workers could remain visibly idle after their provider said usage was
 available, and queued work needed a manual interaction or an extra scheduler cycle
 before it continued.
FIX: dd57e1d5 makes an elapsed reported reset immediately deliverable, recovers
 protocol workers before pumping and planning, and preserves the original reset
 through stale terminal banners. `amux::usage_reset` emits `worker_recovered`,
 `delivery_released`, and `delivery_gate_open` verdicts at each release boundary.

## Settings collapsed a multi-provider fleet into one Claude quota bar
AREA: observability
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-05
SESSION: amux
CARD: AMUX-4154
SYMPTOM: The Settings usage panel exposed only Claude's two coarse percentages even
 though the built-in registry also supports Codex, Gemini, and Ollama. Codex's API
 supplies named buckets, exact reset times, durations, credits, and plan metadata,
 but none of those facts reached the operator.
COST: Operators could not tell which provider constrained a mixed fleet, how long a
 limit would last, or whether another provider was unmetered or simply unmeasured.
FIX: ecbf4daf derives the Settings rows from the full built-in provider registry,
 retains every provider-reported window and exact reset, and distinguishes unavailable
 quota APIs from unlimited local inference. 6bdf9999 also resolves Codex through the
 same login-shell path as a real worker, rather than launchd's stale-but-executable
 shim. `amux::usage_probe` logs whenever a probe succeeds or cannot report its quota.

## append-only push guard offered two causes, and the real one was a third
AREA: gates
SEVERITY: slows
STATUS: open
DATE: 2026-09-06
SESSION: amux-frustrations
CARD: AF-528
SYMPTOM: Pushing a maintainer conflict-resolution to a CONTRIBUTOR'S FORK branch (PR
  #188), the pre-push guard refused: "frustrations.md as pushed is MISSING 4 line(s)
  the remote's current copy has", then insisted I decide between RETIREMENT and STALE
  REPUBLISH before acting, warning that the wrong choice is destructive. Neither was
  true. main had UPDATED the ATE-17 entry IN PLACE (STATUS: open -> fixed, FIX
  paragraph rewritten; origin/main:frustrations.md:2342-2365) and the fork branch
  carried the older copy, so merging main FORWARD replaced their stale text with
  main's newer text. The archive cross-check cannot help: the lines did not MOVE to
  frustrations-archive.md, they were rewritten where they stood.
COST: ~10 minutes ruling out both offered causes against a refusal that says picking
  wrong is destructive. The direction was the OPPOSITE of the accusation -- I was
  publishing NEWER content and it read as a revert.
FIX: Two cheap discriminators the guard already has the inputs for. (1) If the missing
  lines are ADJACENT to CHANGED lines in the same entry block, that is an in-place
  rewrite, not a deletion -- name it as a third cause. (2) Report whether the push
  target is `origin` or a fork branch: merging main forward into a fork can only ever
  ADD to origin's history, so the stale-republish reading does not apply there at all.
  A line-set difference cannot see an EDIT any more than it can see a MOVE, which is
  the failure mode this file's own contract already names one layer up.

## Isolated workers hid confirmed owner work from the shared board
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4159
SYMPTOM: The live `amux` worker has `CC_ISOLATED=1`. Its first task prompt in this
session was recorded in `cmd_history` row 44634 as `delivery=direct`,
`submit_verdict=confirmed`, but `card_id=NULL`; there was no capture verdict in the
server log. Direct prompt capture deliberately excluded every isolated worker, and
the capture health invariant deliberately excluded the same population, so mechanism
and monitor agreed on invisible work.
COST: The user had to point at the Workers board to establish that the task ledger
was still incomplete, and the first linked implementation request had no card or
task-local evidence even though the worker had received and was executing it.
FIX: 6bce0158 removes isolation from owner-prompt capture while preserving its
harness, peer-discovery, and automation boundaries. Capture logs now include
`owner_isolated`, the health invariant evaluates isolated-owner prompts, and an exact
regression fixture proves the confirmed live prompt shape mints and links a card.

## Decomposition accepted child cards that did not say how to execute or verify them
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4161
SYMPTOM: The capture decomposition endpoint required a title, priority, dependency
 indexes, and next action, but accepted an empty description and no acceptance
 criteria. All 24 live decomposition children predate the corrected contract and
 lack acceptance criteria, yet no health invariant reported that incomplete
 population.
COST: A worker could receive a syntactically valid child without enough durable
 detail to determine scope or falsify completion; terminal evidence then depended
 on conversational context outside the board.
FIX: The endpoint now rejects vague descriptions and missing, malformed, or duplicate
 acceptance criteria atomically, and the board health sweep reports every incomplete
 live decomposition row with `measured`, `n_considered`, and per-card gap names.

## Manual claim bypassed a decomposed task's dependency graph
AREA: board
SEVERITY: corrupts
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4161
SYMPTOM: Board-drive held dependency-backed children in backlog, but the public claim
 endpoint could force a backlog child directly to doing without consulting those
 dependencies. The chaos journey reproduced the bypass by claiming plan step two
 while step one was still open.
COST: Two workers could execute an ordered plan out of sequence, consuming work whose
 prerequisite had not produced its result while the board still displayed a valid
 dependency edge.
FIX: The claim primitive now checks dependencies in the same SQLite writer transaction
 as its status compare-and-swap. A refusal returns `dependency_blocked` with the exact
 blockers and writes both a WARN verdict and a durable `claim.dependency_blocked`
 session event.

## A different decomposition retry was reported as idempotent
AREA: board
SEVERITY: corrupts
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4161
SYMPTOM: Once an epic had children, every later decomposition request returned
 `idempotent: true` without comparing the submitted plan to the committed one. A
 retried or racing caller could therefore believe its changed plan won even though
 the board retained a different child set.
COST: The API acknowledged work it did not store and gave the caller no discriminator
 for recovering after a lost response or concurrent decomposition.
FIX: Decomposition now persists a normalized plan SHA-256 on the root epic. Exact
retries return measured idempotent success; divergent retries return a measured 409
with both hashes and emit a greppable `plan_conflict` WARN verdict.

## The todo ceiling stranded a dependency successor after its predecessor closed
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4161
SYMPTOM: Live dogfooding closed AMUX-4162, but AMUX-4163 remained in backlog across
 multiple measured board-drive ticks even though its only dependency was done. The
 selector found it correctly; the shared transition engine then refused backlog to
 todo because this lane already had 122 unrelated todo cards against its 20-card
 queue ceiling, and the refusal was discarded without a log line.
COST: An accepted ordered plan could stop permanently between steps for a condition
 unrelated to that plan, while `/api/debug/board-drive` reported `promoted: 0` with
 no card or refusal reason to investigate.
FIX: Dependency-cleared promotions now carry a narrow todo-ceiling exemption while
 retaining transition, archive, and gate checks; revisit and ordinary queue additions
 remain capped. Any selected promotion still refused by the transition engine now
 emits a measured `promotion_refused` WARN naming the card and exact refusal.


## A successful trigger PATCH immediately puts parked work back in todo
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4168
SYMPTOM: The 48-hour worker audit found explicit source_ref updates retaining an older last_verified_at. GCA-157 was parked at 12:51 and auto-drained at 12:51; its new condition did not start a new parking window. The API returned success plus an advisory instead of completing the parking operation.
COST: Repeated park/drain cycles and a fleet audit to discover why the workers still looked idle over todo cards.
FIX: Record parking time in the PATCH transaction for explicit trigger writes, including reassertions and autofix diversions; preserve explicit timestamps/null. Fixed in af53a6bb (AMUX-4168). The PATCH/selector regression fails with timestamp zeroed; live AF-298, AG-39 and GCA-157 reassertions passed and emitted trigger_timestamp_repaired.

## Historical dependency prose prevents the worker from reconciling its own queue
AREA: scheduler
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4168
SYMPTOM: Six idle workers had nine todo candidates refused by the prose dependency regex. MS-1253 was blocked by its own ID; MR-21's first historical blocker outranked its newer description of the remaining work. No reconciliation prompt reached these workers.
COST: Six live worker queues stayed unclaimable while their board cards still said todo; the user had to request another fleet investigation.
FIX: Keep structured dependencies authoritative and deliver ambiguous prose as a dependency-recheck prompt, with a named WARN verdict, instead of vetoing pickup. Fixed in af53a6bb (AMUX-4168). The nine-card/six-worker regression fails when the veto returns. Scheduled live deliveries reached all six workers; all nine cards were reconciled against current dependencies or handed onward.

## Stale-WIP recovery immediately assigns the same card again
AREA: scheduler
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-06
SESSION: amux
CARD: AMUX-4168
SYMPTOM: BR-51 was reclaimed at 04:25 and picked again at 04:26, then reclaimed at 10:26 and picked again at 10:27. Selection ignores pickup.reclaimed_stale when applying its per-card cooldown, so recovery does not yield to the other queued work.
COST: Two six-hour recovery cycles left byo-ray holding the same WIP slot over eight/nine eligible todo candidates.
FIX: Apply the existing bounded per-card cooldown to the reclaim event too, and log stale_reclaim_yields_to_next_card. Fixed in af53a6bb (AMUX-4168). The regression fails without the reclaim event in the cooldown. The actual byo-ray snapshot selected BR-137 after reclaiming BR-51; its live current CI run remains in progress, so no live reclaim was forced.

## Worker message arrows did nothing on Codex output
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4178
SYMPTOM: Live amux terminal contained three Codex prompt lines but zero navigable message elements. Previous message did not move. Consecutive Claude prompts could nest, and terminal provenance was classified before worker-scoped history loaded.
COST: User could not navigate the conversation; reproduced a zero-target click on the live worker.
FIX: Recognize both prompt glyphs, balance ANSI and message blocks, classify from scoped history, land at message starts, and emit peek-message-nav beacons with landed/no-targets/target-not-visible verdicts. Desktop, mobile and WebKit: 9 passed; removing Codex detection fails the regression.

## A stale dependency cycle can hide a new task cycle
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4179
SYMPTOM: depends_on_cycle inspected only the first cycle anywhere on the board and ignored it if it did not contain the edited task. A second newly introduced cycle could therefore escape the write guard. Parent lineage had no cycle guard, and no periodic graph integrity check exposed malformed or dangling relationships.
COST: Plans could become structurally unbuildable without a reliable rejection or graph-wide diagnosis.
FIX: Edited-task reachability checks with real cycle witnesses, independent parent DAG validation, and a typed snapshot over existing primitives. board.graph_integrity runs periodically; dependency_cycle_rejected, lineage_cycle_rejected and task_graph_invalid identify failures. Removing the dependency guard makes the persisted-corruption API regression accept a cycle and fail.

## A refusal test read the live fleet's open sending policy
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4179
SYMPTOM: The complete server suite failed in the_cross_group_refusal_names_the_verb_that_works_without_an_existing_card because it expected a refusal while reading real worker configuration. Cross-group sending has been open by default since September 3.
COST: One false failure interrupted a 2,012-test library run before integration targets could execute.
FIX: Isolated AMUX_HOME with explicit opt-out and distinct groups, following the neighboring policy tests. The failure now names its explicit fixture if the gate unexpectedly opens; the focused test passes.

## Codex's worktree suffix revived the false unsubmitted-text badge
AREA: status
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: ATE-36
SYMPTOM: An empty Codex composer rendered `model · path · Main [default]`, but the footer recognizer required `path` to be the final middle-dot segment and exactly one dim separator. The live session therefore reported `composer_preview=gpt-5.6-solxhigh~/Dev/amux` and retained an UNSUBMITTED TEXT badge overnight.
COST: The owner had to ask why the worker claimed to hold text, and the status surface asserted a nonexistent pending command for more than ten hours.
FIX: Parse the stable plain model/path prefix plus any fully dim trailing footer context, preserve the typed-text control, and annotate stuck-composer WARN/event payloads with `possible_codex_footer_chrome` so future TUI drift is sweep-visible.

## Routine graph checks downloaded the complete 58 MB audit snapshot
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4179
SYMPTOM: The first live graph check read 57,937,366 bytes for 13,381 tasks because structural validation and full descriptions/evidence shared one export response.
COST: Every routine CLI check transferred and parsed 58 MB; the loopback export alone took 0.576 seconds.
FIX: Add /api/graph/board/verify using the same structural verifier as the periodic monitor; CLI summaries/checks use it, while --json retains the reproducible full audit export. Invalid checks emit task_graph_invalid with projection=validation; unmeasured reads emit task_graph_unmeasured. Regression compares both projections and proves a 2 MB task body cannot inflate the preflight response.

## A provider-failed subagent stayed live forever after its stop hook vanished
AREA: status
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: ATE-45
SYMPTOM: social-activities showed WORKING + AGENTS twelve hours after its Sonnet turn returned to the prompt. The durable lifecycle set still contained agent add9b6f920fb9ef31 because SubagentStart arrived, the agent immediately failed on HTTP 429, and no SubagentStop hook followed. Its provider-owned parent transcript already contained a newer structured task notification with status=failed, but status derivation never reconciled that terminal fact.
COST: An idle lane looked actively occupied, its board state contradicted the visible prompt, and the stale child survived reports and server restarts with no age-based bound.
FIX: Reconcile only provider-owned structured terminal task notifications newer than the stored per-agent start edge, through the same durable ordering/tombstone path as a real stop. Run it immediately at an idle report and from the periodic sweep for already-leaked rows; malformed, missing, stale, quoted, and still-live evidence fails open for the model. Emit subagent_lifecycle WARN verdict terminal_transcript_reconciled with the session, agent id, provider status, event timestamp, and resulting count.

## Scrolling a worker terminal fired empty message-navigation toasts
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: AMUX-4178
SYMPTOM: Safari repeatedly displayed “No matching messages in loaded output” while the user was scrolling a worker terminal, even though they had not asked to navigate. Live client-debug beacons recorded repeated no-targets arrow activations during the scroll interaction.
COST: Ordinary reading produced alarming, irrelevant feedback and made the new message arrows feel unreliable; when a real explicit navigation found no loaded match, it also left the user to locate and press Load earlier manually.
FIX: Arm message navigation on pointer-down and accept only a trusted keyboard activation or a non-moving pointer gesture with no intervening terminal scroll. Suppressed gestures emit peek-message-nav verdict=suppressed-scroll-gesture and never toast or move. A genuine empty navigation now pages the saved worker log once, reclassifies it, lands on a newly found message, or explains the exact terminal result.

## A global cross-group grant was silently narrowed by a worker allow-list
AREA: messages
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: AMUX-4018
SYMPTOM: Global Settings showed “Workers may message across groups” ON and the global env persisted CC_SEND_ALLOW=*, but amux-frustrations → amux-testing-e2e still entered per-message approval because the resolver returned the sender worker’s nonempty legacy allow-list and ignored the explicit global grant.
COST: The visible fleet policy contradicted enforcement, ordinary peer handoffs stalled, and neither the refusal nor worker UI identified which layer actually decided the result.
FIX: Resolve CC_SEND_ALLOW as a layered allow-list: nonempty explicit global/group/worker values compose additively, while an explicit empty more-specific value is the visible deny/reset. Enforcement, both worker config routes, the worker list, and the UI share the resulting value/source/reason. Emit cross_group_policy_refused and cross_group_policy_persisted verdicts with the effective source.

## Worker header stacked tiny message arrows and hid actions behind unexplained icons
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4188
SYMPTOM: Ethan circled the Human 0 tower, tab grid and directory link/edit icons on his phone. The navigation group measured 76px tall with 24px arrow targets; its counter secretly cycled seven message types.
COST: A second user report after the earlier scrolling fix; 30px of avoidable header height and small ambiguous tap targets remained on the primary phone surface.
FIX: One horizontal 44px-control toolbar with an explicit message-type select and on-demand Find. Search and message navigation share arrows/counters, including keyboard changes. Tabs is labeled; directory changes and copy links use named worker-menu entries. peek-toolbar-layout emits unusable-controls with measured/population/overflow/target sizes if this layout regresses. Browser checks cover desktop, 375px mobile and iOS WebKit; a negative control restores column stacking to prove the geometry check fails.

## Cached worker links opened before message state existed and never loaded scoped history
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4188
SYMPTOM: Live #peek=amux displayed Cannot access _pmSel before initialization. _peekMsgRows remained null: the initial cached-message render threw before _peekMessagesLoad reached its fetch.
COST: Live toolbar verification was blocked; direct worker links could keep Human at zero because the scoped history request never started.
FIX: Initial deep-link routing and screen restoration now run together after DOM readiness and full bundle initialization. Cached worker/history fixtures prove a real scoped request and human classification. Global action/script failures now emit client-action-error beacons as well as the existing toast. Live layout signals also exposed a zoom-coordinate false positive; geometry validation now compares layout sizes to layout thresholds and records rendered height separately.


## A completed worker-menu action left a listener that swallowed the next opening click
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4188
SYMPTOM: Physical mobile testing copied the directory link successfully, but the next Worker actions click immediately closed the menu. Its old document click listener survived because menu items stop propagation.
COST: Live verification of the new named directory actions failed; people needed an unexplained second click after using an action.
FIX: Closing the menu now retires both the pending dismissal timer and the document listener, including action and toggle paths. Browser regression tests repeatedly open Change directory, cancel and reopen the menu. An opening lost before paint emits worker-action-menu verdict=open-lost with measured and the menu-item population.

## An empty composer control was recorded as a confirmed message and failed silently
AREA: browser
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: ATE-75
SYMPTOM: After an interrupted turn, the dashboard accepted `continue`, then recorded a second message.sent event with chars=0 even though the empty suggestion probe found nothing. Its fallback Enter returned no visible effect verdict, and the worker had to be stopped and resumed before the composer/control state was trustworthy again.
COST: The live acceptance turn was interrupted, a no-op entered the durable audit trail as a confirmed send, and recovery required a worker stop/start.
FIX: Classify a missing suggestion as submission=no_effect, omit it from send history/last_send, and emit a session.control_noop event plus composer-control WARN. Raw key responses now name effect=unverified, and the dashboard awaits the fallback and displays that verdict or an explicit failure.

## A terminal card kept an intermediate status after completion
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: ATE-84
SYMPTOM: ATE-75 reached Done with a 12:05 validation note still shown as Latest status; Refresh asked the worker and did not surface the current terminal outcome.
COST: Reviewers could not tell whether the completed work had actually deployed or passed live acceptance, and the visible worker-action summary omitted the final evidence.
FIX: The save_patched durable write choke point now atomically records a provider-independent final summary in last_result and a STATUS (board) history line, including outcome, recorded actions, tests/deployment/live evidence, and linked assets. Terminal Refresh rehydrates the authoritative board detail instead of requesting provider text. Legacy terminal rows are repaired on their next durable board write, and terminal_summary_recorded is logged for sweeps.

## Local multiplayer passed localhost while its Tailscale invite links were unusable
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: Codex local Tailscale multiplayer
CARD: ATE-79
SYMPTOM: The checked-in local multiplayer Playwright test passed in three browser
 projects, but it ran only on localhost. A real browser negotiated HTTP/2 on the
 Tailscale TLS listener, where authority and scheme live on the request URI rather
 than the HTTP/1.1 Host header, and Team generated an http://localhost invite link
 that no second tailnet node could open.
COST: The original green E2E result gave the wrong release verdict and about 45
 minutes went to repeating the flow at the real Tailscale origin before the protocol
 difference became visible.
FIX: ATE-79 reads authority and scheme from either HTTP version, pins the HTTP/2
 request shape in the route test, and logs `origin_fallback` when neither source is
 present.

## Local multiplayer called a member revoked before testing a reload
AREA: tests
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: Codex local Tailscale multiplayer
CARD: ATE-79
SYMPTOM: The revocation E2E asserted that the already-open member page received 401
 after deletion and stopped. The HttpOnly cookie remained in the browser; reloading
 made its DB lookup fail, classified the request as an ordinary public shell, and
 injected the owner bearer into JavaScript. The member was therefore promoted to
 owner by the first recovery action a user would try.
COST: A revocable-invite feature carried a privilege-escalation path despite a green
 browser test, and the missing lifecycle edge added about an hour of independent
 cookie, reload, mutation, and request-log checks.
FIX: ATE-79 distinguishes absent, verified, and revoked member cookies before owner
 bootstrap, adds the reload assertion to all three Playwright projects, and emits
 `revoked_member_bootstrap_withheld` when the blocked transition is attempted.

## The PWA cache discarded a valid remote-owner bootstrap credential
AREA: auth
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: Codex local Tailscale multiplayer
CARD: ATE-79
SYMPTOM: A remote owner opened the dashboard with the correct `?_token`, but the
 service worker canonicalized every root navigation to cached `/` before the server
 could see the query. The UI loaded, reported Polling, and every Team mutation failed
 unauthorized, while direct HTTP with the same credential succeeded.
COST: Two fresh browser origins and two server rebuilds were needed to separate a
 server-auth failure from a cached-shell failure; without the browser check the new
 secure remote-owner boundary would have shipped with no usable recovery path.
FIX: ATE-79 exchanges the one-time URL bearer for a derived HttpOnly owner-session
cookie, removes it from the address bar, bypasses the canonical shell cache for the
exchange request, and pins the app/service-worker version seam.

## A stale card detail routed a terminal Refresh to the provider
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-testing-e2e
CARD: ATE-84
SYMPTOM: ATE-75 was Done in the durable board, but a stale client reopened it with In Progress selected. Its old Refresh/status-request path then reached the worker, and the resulting status update replaced the generated Final outcome summary.
COST: The UI made a terminal card look active and allowed provider prose to overwrite the durable completion record, so Refresh could not restore the evidence reviewers needed.
FIX: Card-detail hydration now applies the authoritative GET status to the selected controls, and Refresh GETs before deciding whether a provider request is permitted. Terminal status-request calls are refused and logged without delivery; terminal status updates and stale last_result PATCHes preserve Final outcome while appending evidence. Emit terminal_status_request_preserved and terminal_status_update_preserved markers, with Rust provider-shape and desktop/mobile/iOS Playwright regressions.

## Message jumps mixed zoomed screen coordinates with unzoomed scroll offsets
AREA: browser
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4192
SYMPTOM: Multi-worker live checks found 80% jumps hundreds or thousands of pixels from the selected message. At 100%, the fixed 12px landing put the first line beneath the terminal's floating controls. Search also matched serialized HTML, so entities and ANSI-split phrases could not be found, and a working worker's refresh could replace the selected search result.
COST: The preceding live check at one normal zoom did not cover these cases; Ethan requested cross-worker verification again. The baseline reproduced unreadable landings on amux-frustrations and mixpeek-general.
FIX: Convert rendered geometry to layout coordinates, reserve the actual floating-control inset, reveal nested horizontal table matches, and publish zoom/inset/scroll-error/visibility in navigation beacons. Search rendered text across inline spans as one result. Pin selected search/message nodes while newer output is buffered. Regression cases cover long/wrapped text, symbols, Unicode paths, ANSI spans, wide tables, start/end wrapping and real refreshes across desktop, 375px and WebKit.


## Saved Codex composer hints became false message-navigation targets
AREA: browser
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4192
SYMPTOM: The six-worker sweep found four copies of Ask Codex to do anything in amux's saved output being treated as messages. The composer exclusion only applied at the very end of the entire log, so historical frames escaped it.
COST: Geometrically correct jumps still landed on terminal input hints instead of submitted messages.
FIX: Detect the adjacent model footer for each unclassified block, including historical frames; authoritative submitted-message history still wins. A browser test covers old composer frames followed by later output and the same literal text recorded as a real human message.


## Dependency completion released code at Done and returned to the child task
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4193
SYMPTOM: Code dependencies became runnable at Done before verification. The completion callback was linked to the producer child instead of the original requester task and lacked its next action. Claims and backlog promotion disagreed on missing and discarded dependencies.
COST: Ethan requested explicit end-to-end proof that verified dependency completion actually wakes and resumes the original worker. Existing unit fixtures encoded the earlier, weaker completion boundary.
FIX: Use the existing type-specific verification boundary for claims, promotion and durable callbacks. Derive the original return task from depends_on, retain graph edges, link the producer-origin message to the original task, include remaining blockers and next action, and record both task histories. Log dependency_waiting_for_verification, dependency_callback_linked and withheld or unmeasured delivery cases.

## A bottom-clamped message jump was mistaken for resuming live output
AREA: browser
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4192
SYMPTOM: The deployed multi-worker sweep lost the selected message on amux-testing-e2e when a jump hit the bottom. The async scroll listener unlocked live updates even though refreshPeek itself preserved selected targets.
COST: One live navigation failure survived synchronous refresh tests; a working worker replaced the selected message after a geometrically correct jump.
FIX: The scroll listener now preserves the navigation lock for selected messages and search results at the finite scroll boundary. Regression tests wait for actual scroll events before refreshing and cover both trailing-output and end-of-output targets. Existing navigation beacons expose missing targets and scroll errors.

## Concurrent Bash observations are treated as file ownership
AREA: attribution
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: mixpeek-ops-server
CARD: MOS-33
SYMPTOM: A concurrent reader was named owner of three ops research files after their mtimes changed during its Bash command. The production classifier reproduces a foreign block with provenance observed while its explanation asserts a transcript write. A later observation can also replace an existing recorded writer.
COST: The reader had to disown files it never edited; publication required an ownership check and this repair.
FIX: Keep mtime observations in a separate, counted advisory. They cannot name an owner or replace recorded edits; preserve recorded-writer and blind-cotenant protection. Regression: concurrent_reader_observations_cannot_claim_the_ops_research_files.
VERIFIED: dd416c753b24 is running (build eee97f2b86189c02). The same five staged paths changed from three observed-only foreign blocks to zero foreign owners, with three advisory paths/four observer records retained; the MOS-33 log records that denominator. Final source passes 70 guard tests, six real hook-main controls, existing protection checks, clippy and cargo check.

---
## Fleet read as stopped while its original tmux server still held 62 sessions
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4203
SYMPTOM: At 17:27:23 EDT fleet captures began timing out; at 17:29:49 the socket refused connections. A new tmux server created at 17:30:07 replaced the default socket while its original owner remained alive with 62 sessions. /api/debug/tmux measured only the replacement (7 sessions at first inspection), and invariants classified the original workers as stopped. Kernel socket owners and server identity were absent from both instruments.
COST: 30 minutes with most of the fleet inaccessible before diagnosis; original sessions had to be recovered via a separately preserved socket and 57 non-archived workers reconciled with the replacement fleet. The initiating stall cannot be proven from retained logs.
FIX: AMUX-4203 adds independent socket-owner evidence, persistent stall process/stack samples, an invariant WARN, and guarded tmux creation. The initial stall remains unproven; do not read recovery as proof of its cause.

---
## Worker startup joined the environment cleanup and agent launch into one shell command
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4203
SYMPTOM: During fleet recovery, handoff-consumer-0907 displayed `unset ANTHROPIC_API_KEYclaude --dangerously-skip-permissions ...`; bash rejected the agent flags as unset identifiers. Two other workers remained at shell prompts after accepted starts. `type_line` used separate tmux clients for literal input and Enter, discarded errors, and proceeded to the next command under capture load.
COST: Three individual launch retries, plus manual verification that all 57 non-archived workers had live agents rather than merely an accepted start response.
FIX: Submit each literal line and Enter together in one tmux command queue. WARN with shell_line_submission_failed when tmux does not confirm it, without recording shell command contents. A private-socket regression test verifies two complete shell commands reach the shell.

---
## One broadcast prompt minted 105 capture cards across 56 lanes: the capture dedupe window is 45s, the delivery it guards waits for a turn boundary
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-568
SYMPTOM: The 5:59 PM recovery broadcast produced 56 identical cmd_history rows (one per lane, 503 chars, enqueued 18:09-18:10) and 105 board cards. 46 of 56 lanes hold more than one card for that single prompt and two hold three. Every duplicate was minted by the steering-delivery path (session_verbs.rs:10893, "ledger: auto-captured board card from STEERING-delivered prompt (AMUX-3148)"), whose id encodes the ENQUEUE time: AF-567 is `id=steer-1788818960752`, enqueued 18:09:20 and minted 18:26:15. Its "already carded" guard is `cmd_history ... AND card_id IS NOT NULL AND ts > now - AMUX_CAPTURE_DEDUP_WINDOW_S` with a 45s default, so at 17 minutes it saw nothing and minted again. The duplicates then have no cmd_history row of their own, because the linking UPDATE looks for the most recent UNCARDED row for that text and there is none.
COST: 49 junk cards across 46 lanes, each needing a manual disposition by whoever owns it. gainz is parked on a permission prompt clearing GAINZ-52/53 with `curl -X DELETE`, which also destroys the audit trail `amux board discard` keeps. My own lane spent the recovery turn deciding what two identical `doing` cards meant before any work started.
FIX: 271167a3. A 45s window is sized for a transport retry and is being asked to guard a path whose entire purpose is to WAIT for a turn boundary, which took 17 to 25 minutes for every duplicate here. Dedupe against `issues` instead, the table the mint itself writes, with no time box while the card is non-terminal: refuse the mint when the session already holds a non-terminal `source='capture'` card whose desc is byte-identical. Log the refusal with the surviving card id so a wrongly suppressed distinct prompt is findable rather than silent.
NOTE: An earlier draft of this entry blamed a second mint path in orchestrator/runtime.rs. That was wrong and is recorded on AF-568. The orchestrator path stamps `source='orchestrator'`; all 104 sourced cards here read `capture`, which is what sent me back to the log.

---
## The junk classifier brands any card that MENTIONS the capture marker, so a card cannot describe the capture mechanism
AREA: notices
SEVERITY: annoys
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-569
SYMPTOM: AF-568, the card ABOUT duplicate capture cards, was nudged "captured chat prompt, not a unit of work" while it sat in `doing` with a shipped fix and a 4000-char write-up. It is not a capture by any structural test: source='agent', creator='amux-frustrations', desc begins "MEASURED, 2026-09-07", no `capture: session prompt` marker in its log. The brand at board_drive.rs:880 was `desc.contains("capture: session prompt")`, and AF-568's desc contains that string once, inside backticks, in the sentence explaining the bug. Measured: contains=True, starts_with=False, folds=0.
COST: A live card with a shipped fix was told it was not a unit of work and offered discard/retitle/decompose. Small in minutes, and it is the shape that matters: the classifier cannot tell a MENTION from a USE, so any card documenting the capture mechanism is misfiled as its output. It also fired BEFORE the STRUCTURE VETO at :907, which returns "not junk" on 2+ ALLCAPS section heads; AF-568 had three, so a substring match short-circuited the evidence that existed to prevent exactly this.
FIX: 271167a3's follow-up. Anchor it to the intent the code already states three lines above ("a card literally defined as the capture marker"): `desc.trim_start().starts_with(...)`. Every real capture is still caught by the anchored PROMPT check below, on the `**Prompt:** ` prefix session_verbs.rs actually mints. Line 880's positive case had NO test before this, which is why changing it broke nothing: the existing tests around it cover the PROMPT check and the log-only case. Two new cells, mutation-checked in both directions.

---
## A gate refusal printed "correct the TYPE" while its own field in the same response said retyping would not change it
AREA: gates
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-571
SYMPTOM: `amux board verified AF-513` returned, in one response, `how_to_ack.gate_source` saying "this gate comes from the GROUP scope (amux), not from the item type - retyping will NOT change it", and then a closing line reading "If a criterion does not apply because the card is mistyped (item_type=investigation), correct the TYPE rather than acking something untrue", followed by the `amux board type` command. The reader acts on the closing line: it is last, it is prose rather than a nested JSON field, and it is the only one carrying a runnable command. Following it costs a retype and leaves the card mistyped, because a group-scoped gate resolves identically for every type.
COST: More than the wasted command. That closing line is what makes an author believe the gate has an honest exit, and for this gate it does not. An author who retypes and sees the same four criteria return has been sent to the one remedy that provably cannot work, which pushes them toward acking something untrue - exactly what the line exists to prevent. It sits over a real population: 324 of group amux's 770 done cards are types that cannot satisfy criterion 1 at all (AF-570).
FIX: The condition was already computed and transmitted. The server emits `how_to_ack.gate_source` exactly when the gate came from a non-type scope. The CLI now prints the retype advice only when that field is ABSENT, and when present prints what would actually change it: satisfy the criteria, hand the card to the lane the criterion is about, or change the gate at its own scope. Verified on the installed CLI in both directions (group-scoped gate gets the new text, type-scoped gate is byte-identical to before), and mutation-checked on a COPY rather than in place, because ~50 lanes execute this file continuously: disabling the condition makes the group-scoped refusal print the retype advice again.

---
## Adding `set -e` to a harness made it abort silently on an idle host, and the fix could not fail on the box that shipped it
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-564
SYMPTOM: main's `checks` workflow went red at 327c470e (18:44) after e1032a2a (16:38) was green, and stayed red for six consecutive commits. The failing step logged `test-build-provenance: 9 passed, 0 failed` and then `##[error]Process completed with exit code 1` with nothing in between: no verdict line, no failure, no output at all from the next harness. Cause is my own 39ac1877, whose entire diff to that file is `set -uo pipefail` -> `set -euo pipefail`. The harness reads `_real_builds="$( { pgrep -x rustc; pgrep -x cargo; } 2>/dev/null | tr -d '[:space:]')"`; on an idle host both pgreps exit 1, pipefail carries that through the substitution into the assignment, and set -e aborts before the first echo. Confirmed in isolation: with set -e and no match, exit 1 and no output; without it, `REACHED, x=[]` and exit 0.
COST: main red for six commits across ~6 hours, which held amux-cloud's cloud-freshness track behind red CI and made every lane's push gate meaningless. The card filed against it named the wrong range (c6876cf1..8a70187, all of whose runs are green), so anyone bisecting from the card would have searched commits that were never involved.
FIX: `|| true` on that assignment, because the decision reads the OUTPUT and never the status, which is the exact distinction AF-560's own convention names and which I got wrong here. Plus a new host-independent cell (n) running the same construct with names that can never match. Cell (n) is the load-bearing half: every existing cell that could have caught this lives inside the branch that only runs when nothing is building, so on this box that whole population is unreachable and the suite reported PASS through the entire outage. THE REAL LESSON: a harness whose precondition is "nothing else is running" can never fail on a machine that always has a builder running, so the environment that ships the change is the one environment guaranteed not to test it.

---
## The group verified gate demands "functionality change is live" from card types that produce no functionality change
AREA: gates
SEVERITY: slows
STATUS: open
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-570
SYMPTOM: The `verified` gate for group `amux` resolves from the GROUP scope, so it is the same four criteria for every item type, and the first is "Functionality change is live and exercised, not just merged". Measured across group amux's done cards: 324 of 770 (42%) are types that cannot have a functionality change at all (investigation 169, doc 48, ops 32, blocker 24, chore 21, epic 9, escalation 6, decision 5, watch 5, research 5). AF-513 is a complete, correct investigation whose finding is that GET /api/board omits desc; it shipped no code, so criterion 1 has no true answer and criterion 4 ("no regression in what it touched") has no referent. amux-gtm independently hit the same wall from a different population: AG-39's output is a provisioned trial, imported seed data and a 6/6 chain run.
COST: Finished work cannot leave `done` truthfully, so it either sits there forever or someone acks a falsehood. 324 cards in group amux alone. The board's documented escape (correct the TYPE) is inert here because a group-scoped gate overrides the type default, and until AF-571 the refusal recommended it anyway.
FIX: Shipped the MECHANISM, not the policy: an opt-in `@additive` marker on a scoped gate, so a gate can UNION with the type default instead of replacing it. Measured no-op at ship time — 6 scoped gates configured fleet-wide, 0 carry the marker. amux-cloud found the trap that made this necessary: precedence in effective_gate_trail is first-tier-wins with REPLACEMENT, so simply editing the group gate down to the two peer criteria would have silently removed "Confirmed working in prod" and the other three from every `code` card while passing a naive "is AF-513 unblocked now" check. The group:amux OPT-IN is the policy half and is NOT shipped: amux-gtm and amux-cloud agreed, amux-homepage has not replied, and `amux` is isolated and cannot be asked. STATUS stays open until the opt-in lands, because the friction is live until then.

---
## A bare `git config` inside a linked worktree rewrites the SHARED repo config, and our own push-gate advice creates the exposure
AREA: gates
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-577
SYMPTOM: A linked worktree SHARES .git/config with the main checkout, so any `git config` run inside one, without --worktree, rewrites the file every sibling worktree and the main checkout read. No GIT_DIR, no inherited environment, no hook. Reproduced here in a fresh repo: BEFORE main user.email=<real>, core.bare=false; after `(cd linked && git config user.email t@t.t && git config core.bare true)`, main reads t@t.t and core.bare=true. On the mixpeek checkout this took every work-tree operation down for every lane for ~30 minutes (MG-1648) and authored 9 commits on their origin/main as `t <t@t.t>`. Found by mixpeek-general (132a2b8a) after they retracted their own GIT_DIR hypothesis, which mixpeek-cicd had falsified.
COST: amux's config is intact, so this is exposure rather than damage: 22 registered worktrees on the shared checkout and `extensions.worktreeConfig` UNSET, which means none of them can hold private config and all 22 can only write shared. Our own CLAUDE.md:185 tells every lane to run `git worktree add --detach /tmp/push-check main` before pushing, which is why 17 of the 22 are push-check-shaped. The rule that protects a push gate manufactures this exposure. mixpeek-general's counter-case is the sharper argument for a guard over a doc fix: mixpeek/CLAUDE.md:399 says the OPPOSITE ("do NOT create git worktrees"), and they still accumulated 8, so a doc only binds the lanes that read it.
FIX: a `_worktree_config_verdict` check in scripts/git-hooks/git-shared-guard.py, the PreToolUse hook every lane's Bash already passes through. It refuses a `git config` WRITE when the cwd resolves to a linked worktree, and names --global / --system / --worktree / the main checkout as exits while explicitly ruling OUT --local (in a linked worktree "local" IS the shared file, so recommending it would name the defect as its own cure). 14 new cells, ALL 179 PASS. THE PLACEMENT IS THE WHOLE FIX: the first draft sat after the AMUX_SHARED_CHECKOUTS scope gate and returned 0 for all eight probe cells including `core.bare true`, because a linked worktree is never inside the checkout it belongs to. An inert guard that reads as installed is worse than none. The test now passes a deliberately-unrelated shared_root so it cannot drift back behind that gate.

---
## Bounded fleet probes manufactured timeouts by waiting before reading their output
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-07
SESSION: amux
CARD: AMUX-4203
SYMPTOM: Both synchronous fleet-probe runners polled child exit before draining stdout/stderr. A child writing 262,144 bytes filled the pipe and was killed on its three-second deadline; both regression tests failed against the shipped functions. The capture runner justified this with “30 lines”, which is not a byte bound. Its timeout WARN and diagnostic note blamed an unresponsive tmux without recording bytes read or whether the child had already exited.
COST: The original fleet incident produced 511 capture timeout warnings during 17:27–17:29, but the instrument could not distinguish an upstream stall from its own unread pipe. The investigation had to reproduce the runner separately before its timeout verdict could be trusted. Current captures were below pipe capacity, so this entry does not claim the pipe defect initiated that incident.
FIX: Drain both pipes nonblockingly while polling the child, with the deadline covering continuously producing children and descendants holding a pipe after the child exits. Preserve byte counts, PID, elapsed time and child-exit versus pipe-EOF phase in WARN logs and /api/debug/tmux, including when the diagnostic's own fleet query fails. Trigger bounded independent tmux/host evidence collection on the first timeout, at most once per minute; retain host load and processes ranked by CPU/RSS without process arguments. Regression fixtures test large stdout and stderr, real hangs, continuous output, inherited pipes and successful output preservation.

---
## MEMORY.md is rebuilt from the server's sources, so the pointer the memory instruction tells you to write is deleted on the next compose
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-07
SESSION: amux-frustrations
CARD: AF-578
SYMPTOM: Routed up by ts-gke (TG-3374) as index truncation: their MEMORY.md exceeded the read ceiling, so 9 entries never loaded, with an auto-generated fleet roster eating 6.1 KB (22%) of a capped file. All confirmed. The cause underneath is worse. Measured: 44 memory files on disk in the amux project dir, MEMORY.md indexing 5 of them, and the server source ~/.amux/memory/amux-frustrations.md carrying ~25 pointers with ZERO overlap with those 5. Two indexes, not two views of one list. session_verbs.rs composes MEMORY.md as a full rebuild from its own sources and fs::write's it wholesale, so anything a session appended directly is destroyed on the next compose. ts-gke measured the same shape at mixpeek scale: 1,080 memory files against 125 index pointers.
COST: The memory FILES survive, so every write returns success and every .md persists; only the index line that makes them findable dies. 39 of 44 in this lane were already content with no pointer, which is a failure with no symptom until someone counts. Three of mine were at risk including one written in the session that found this. The direct cause is a contradiction between two instructions: the session prompt says "add a one-line pointer in MEMORY.md", and session_verbs.rs:9365 says the durable path is ~/.amux/memory/<worker>.md. A session following its own instructions writes the volatile half.
FIX: `preserved_agent_pointers` turns the destructive rebuild into a merge: any `- [Title](file.md)` line in the existing MEMORY.md whose target file EXISTS and which the sources do not already carry is re-emitted, placed BEFORE the roster so the roster stays last (ts-gke's option 3, free). A pointer whose file is gone is NOT preserved, because the memory rules tell sessions to delete memories that turn out to be wrong and resurrecting one would make deletion impossible. Per ts-gke, the composer now WARNS with what it dropped and could not carry, split into deleted-target and unrecognised-line counts: a destructive rebuild that cannot name what it removed is the same class as a guard that reads green while skipped. KNOWN LIMIT, named in the code because ts-gke found it: lane memory sources are not all pointer-shaped (ts-gke.md is headings and prose with zero pointer lines), so for a lane in that style this preserves nothing and the warn is the only signal.

## A working Codex lane inherited its sibling's idle turn, and the alert blamed a discarded hook
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux
CARD: AMUX-4220
SYMPTOM: mvs-research produced 26 retained status.agrees_with_pane failures from 04:59 to 05:23 EDT. Its actual Codex process held the research rollout open, but nearest-start association selected mvs-pitr's completed rollout for both workers. The failure evidence named a 30-hour-old stop-hook even though status-explain said report.applied=false and decided_by=codex_rollout.
COST: Roughly 20 minutes tracing the deciding signal through metadata, rollout boundaries, and kernel open-file ownership; the original alert omitted the evidence needed to distinguish a broken hook from a sibling transcript.
FIX: AMUX-4220 refuses ambiguous startup associations and missing explicit claims, excludes subagent candidates, logs resolution failures with filenames, and retains the actual derivation in invariant evidence. Existing codex_session_id metadata provides the durable recovery using kernel-proven ownership. Regression tests cover the two-worker startup collision, candidates beyond the previous scan cap, and the provider-boundary grace period. Full findings: docs/incidents/2026-09-08-codex-rollout-cross-attribution.md.

---
## Overlap lineage elected the reporting peer in its log while the durable owner stayed unchanged
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: OVL-15a87ecd9d24b055 elected ATE-114/handoff-producer-0907, but ATE-115's self-report appended "elected owner [ATE-115]" to both cards. GET kept the real owner but hid the resolution note, resolving actor and callback list. A second concern name was labeled scope-split even while its resolution remained pending.
COST: Live acceptance could not be signed off from contradictory ownership evidence; two new regression tests failed on the deployed source (5 passed, 2 failed).
FIX: Derive lineage from the elected row inside its writer transaction and name the reporter separately. Expose resolution provenance, both callbacks and self-report state; only an explicit resolution can report scope-split. An ordinary retry appends one correction to legacy lineage without rewriting history. board_overlap_lineage_recorded and board_overlap_lineage_refreshed expose the result in amux logs.

---
## Overlap callback tests passed alone by reading real worker identities
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: The overlap suite passed alone but its containing board module failed 54/2: both HTTP fixtures expected queued/200 and received retryable/202 with no-env-file. They used real handoff worker names without isolated session fixtures; other board tests correctly changed AMUX_HOME under the shared test lock.
COST: The earlier focused green overstated callback coverage and required a containing-suite investigation before deployment.
FIX: Both HTTP fixtures now own a guarded temporary AMUX_HOME and persisted test worker identities. The vanished-worker test first asserts retryable/no-env-file, then restores its fixture identity and proves one durable queued callback on retry. The production board_overlap_callback_retryable marker already names the exact refusal the old fixtures concealed.

---
## Deployment overlap preflight stopped offline disk-cleanup diagnostics
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: Checks CI for 9e4de512 stopped after provenance 9/0 with no disk-clear result. The disk-only builder path queried the live overlap deployment permit for the checkout's worker trailer, then exited before cleanup on CI without a server. Locally the same test passed 24/0 by reaching the fleet server. A missing grep match then hid the builder's refusal under set -e.
COST: Exact-commit CI failed after live acceptance passed, requiring an offline reproduction and another deployment before ATE-93 could close.
FIX: Diagnostic-only builder modes log OVERLAP GUARD NOT APPLICABLE and skip the network permit because they cannot install. Real deployment still logs OVERLAP GUARD UNMEASURED and refuses adoption. Tests exercise both offline modes and the real refusal on one worker-attributed commit, pin disk tests to an unreachable endpoint, and print the missing-cleanup failure rather than exiting silently. Activation authority and launcher suites now run in Checks CI.

---
## Rename integration tests built a stale hand-written issues schema that production had already migrated
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: Fresh Rust CI failed all 3 rename_migrates_reviewer cases with
  `issues.requested_by: no such column`. The test manually declared five issues
  columns while the production rename path had correctly grown to migrate more
  relationship fields, so the fixture—not the implementation—failed on the clean
  runner after focused overlap and board suites were green.
COST: Exact-commit CI required a separate log download and another code/test/deploy
  cycle; the red looked like a rename regression until the missing fixture column
  was isolated.
FIX: Build the rename fixture through migrate::test_memdb_pub so every production
  schema migration is present, then insert only the rows the scenario needs. The
  clean focused target passes 3/3 and future schema growth can no longer drift this
  fixture silently.

---
## Callback e2e still expected the child link that original-task resumption removed
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: worker-request-callback.spec.ts expected the terminal callback's hard
  card_id and a second card-detail message button on the completed child, while
  the deployed callback contract correctly attaches that message to the original
  requester task and keeps only the request source on the child.
COST: Two three-browser working-tree runs failed 3/3 after the backend and card
  detail suites were green; the first failure looked like a missing callback and
  the second like missing UI lineage until the retained temp database exposed the
  exact cmd_history rows.
FIX: Assert the board request on the child and the callback on the original parent,
  with the child id retained in callback text. The child UI expects one source
  message, then proves card→message→card across desktop, 375px Chromium, and iPhone
  WebKit; the focused matrix passes 3/3 and emits message-card-nav verdicts.

---
## Overlap routes worked but the canonical route census called them unrouted
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: The full integration gate found `/api/board/overlap`, its coordination
  lookup, and its deployment-permit endpoint mounted and covered by handler tests,
  but absent from `ROUTE_TABLE`. `/api/debug/routes` and downstream request-log
  sweeps therefore could not distinguish those working endpoints from dead routes.
COST: The first otherwise-green full ATE-93 server run failed after more than six
  minutes and prevented clippy and deployment from starting.
FIX: Register all three overlap paths with the exact methods mounted by the board
  router. The bidirectional route-table integration guard now covers the family,
  and request-log normalization exposes overlap traffic to the existing diagnostic
  sweeps instead of silently classifying it through a generic card route.

---
## Overlap timestamps shipped without units in the invariant registry
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: The full server gate found all seven numeric `*_at` columns introduced
  by overlap coordination absent from `TIMESTAMP_COLUMNS`. Their writers use
  seconds, but the schema and runtime invariant had no durable unit declaration.
COST: A second broad gate reached its final integration targets before failing,
  preventing the required clippy and deployment chain from starting.
FIX: Declare callback updates, coordination create/update/resolve stamps, member
  create/last-seen stamps, and merged-reference creation as seconds beside the
  existing board timestamps. The exhaustive timestamp-unit guard now makes any
  future schema drift fail with the exact missing table and column.

---
## Clean CI turned one missing provider prerequisite into 34 unrelated pointer timeouts
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: Exact-SHA Rust CI passed 354 browser scenarios, then 34 mobile/WebKit
  terminal cases waited 30 seconds each because the clean test home correctly
  displayed `#no-apikey-banner` over their controls. Two independent stale
  fixtures also failed: default-model restoration called `selectOption` on the
  shipped text input, and Working-now created four Doing rows without one causal
  `task.claimed` identity.
COST: The required ATE-93 CI gate ran 23 minutes before reporting 37 failures,
  and one absent external prerequisite looked like dozens of terminal regressions.
FIX: The broad harness now announces and writes an obvious non-secret provider
  test value into every isolated project's `server.env`, the durable surface
  `/api/identity` actually recognizes (it intentionally ignores process-injected
  keys), keeping unrelated specs in their configured-install prerequisite.
  The settings test restores the text input through fill+blur, and Working-now
  creates its exact owner through the real claim endpoint. Its UI-only runtime
  activation preserves that exact server-produced identity instead of expecting
  a stopped fixture to project as live. Both API-key scenarios restore the known
  harness baseline explicitly because a persisted empty value shadows the
  process fallback; the slow-refresh scenario also reads the masked value back
  as its cleanup postcondition, so a future regression fails at the writer
  instead of poisoning later specs. Missing-key behavior remains product
  behavior; it is no longer accidental global state for tests whose acceptance
  has nothing to do with provider setup.

---
## One repeated timestamp subtraction made the exact-SHA Rust gate nondeterministic
AREA: gates
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-08
SESSION: amux-testing-e2e
CARD: ATE-93
SYMPTOM: The guarded workspace test passed 2,064 tests and failed only
  `stored_observations_reach_the_actual_guard_without_naming_the_reader`: its
  separately evaluated expected timestamp differed from the stored value by one
  f64 ULP (1788859526.403303 versus 1788859526.4033027).
COST: Exact-SHA Rust CI ran for over four minutes and blocked ATE-93's terminal
  gate even though the production observation round trip was correct.
FIX: Compute the fixture timestamp once and use that same binary value for the
  report and strict round-trip assertion. The failure now self-announces the
  JSON/SQLite contract instead of conflating codegen rounding with persistence.

---
## The activation authority rebuilds on a probe that TIMED OUT, and the rebuild is what makes probes time out
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-09-08
SESSION: amux-frustrations
CARD: AMUX-4225
SYMPTOM: Both `--max-time` curls in `scripts/rust-auto-build.sh` (2 of 2) convert a
  FAILED probe into an adoption verdict. `stamp_matches_live_image` does
  `live=$(live_server_commit) || return 1` on a `--max-time 4` health call, and
  `overlap_deploy_permitted` does `curl --max-time 8 ... || { OVERLAP GUARD
  UNMEASURED; return 1; }`. Neither can say "I could not measure"; a timeout is
  spent as a negative answer, and the correction is a full `--release` rebuild.
  That rebuild (rustc measured at 596% CPU, plus a 1268-file worktree checkout and
  a 14 GB target clear) is a large contributor to the load that makes the next
  probe time out. Measured on this box at 10:38 EDT: load average 163.55 with
  `available_parallelism` 28, `/api/health` p95 4731 ms over the trailing hour
  against a 2 ms baseline, and TLS handshake timeouts on the watchdog.
  The abbreviated-SHA theory is WRONG and worth recording as dead so nobody
  re-derives it: `stamp_matches_live_image` compares with a PREFIX pattern
  (`case "$built_sha" in "$live"*`), so 12-char `a604412b4923` against the 40-char
  stamp matches correctly. The reason a604412b logged STAMP DRIFT while NAMING a
  matching commit is that the deciding probe failed and line 195 then RE-PROBED
  successfully to compose the message. The log line prints evidence against the
  verdict it announces, from a different call than the one that decided.
COST: 10 STAMP DRIFT verdicts and 3 OVERLAP GUARD refusals today, each costing a
  redundant release build of an already-installed revision; watchdog /health
  timeouts at 10:03, 10:14, 10:15, 10:26 and 10:28; `/api/sessions` returning zero
  bytes after 20s at 10:34; TubeScience independently blocked registering handoffs
  on a 20s localhost read timeout. Two lanes spent the morning diagnosing a loop
  whose trigger is its own remedy.
FIX: Not mine to write. `amux` (isolated) has the fix already written and
  UNCOMMITTED in this shared checkout: `crates/amux-server/src/activation.rs`
  (hash-checked install identity, `same_revision` skip), `runtime_jobs/poll_watch.rs`
  (WARN naming a job that holds a runtime thread), and an `ACTIVATION AWAITING
  ADOPTION` skip_rebuild path in the builder. Not live: `origin/main`'s builder has
  0 occurrences of `ACTIVATION AWAITING ADOPTION`, the worktree has 1, and the
  launchd authority runs committed bytes from `~/.amux/activation-source`.
  Deliberately left untouched, per the shared-checkout rule. `amux` is an isolated
  raw-agent worker: my send was refused, so only the owner can ask it to commit.

---
## A health timeout rebuilt the same revision and a later successful curl was logged as drift
AREA: runtime
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-08
SESSION: amux
CARD: AMUX-4225
SYMPTOM: Already-stamped a604412b rebuilt at 10:12:56 and self-adopted at
  10:16:06 while health, sessions and handoff calls stalled. The drift line
  printed the matching 12-character SHA, concealing earlier failed probes.
COST: One confirmed redundant release build took 3m01s; another entered 18GB
  cache cleanup. Missing measurement provenance sent diagnosis toward SHA
  abbreviation, which the original comparator already accepted.
FIX: One captured identity decision, unmeasured deferral, hash-checked install
  receipts, same-revision adoption suppression, bounded off-runtime health
  reads, and per-job slow-poll attribution. Native sample had no stacks; the
  original unannounced stop remains unproven. RCA:
  docs/incidents/2026-09-08-health-stalls-build-feedback.md.

---
## Every owner follow-up created another Doing claim and replaced the runtime's current card
AREA: board
SEVERITY: blocks
STATUS: fixed
DATE: 2026-09-08
SESSION: amux
CARD: AMUX-4228
SYMPTOM: During AMUX-4225 recovery, four delivered owner follow-ups each
  minted Doing and task.claimed. Runtime correctly reported five conflicting
  claims; asking to fix the conflict itself added another claim.
COST: Repeated manual reconciliation. Moving the extra cards to Todo was
  refused by the lane's existing WIP cap, so the requests were preserved in
  triggered Backlog; no unrelated work was closed to manufacture room.
FIX: Capture checks the active Doing population in the writer transaction.
  Follow-ups remain durable, already-delivered cards in triggered Backlog and
  emit task.captured, preserving the active task.claimed identity. Logs name
  capture_pending_active_claim, the existing card and population. Focused
  capture tests: 21 passed, including distinct prompt preservation, one active
  claim, a pending delivery receipt, and retry deduplication.
