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

  CONTESTED 2026-08-21 by the author (amux-cloud). The 08-15 guard overhaul
  (d5c575e / b9dbf70 / 26adbc6) may well cover the incident shape (wholesale git add,
  has_unstaged_changes=False), but nobody has re-run the throwaway-repo specimen against
  it, and the entry's own FIX-NOTE records that shape as validated STILL SILENT. Held open
  on the honest basis that a plausible fix is not an exercised one. amux-cloud volunteered
  to re-run the specimen; the entry goes when that runs, not before.

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

  CONTESTED 2026-08-21 by the author (amux-cloud). No commit in history references AC-354
  except the docs commit a21ad4d, so the card closing is not evidence of a fix — this is
  the "card closed on a different thing" shape the validation pass was watching for. A
  bare REPL worker is not cheap to exercise, so the entry stays until someone names the
  fix sha.

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
UPDATE 2026-08-27 (amux, prompted by amux-frustrations): THE FIX ABOVE IS THE SECOND ANSWER,
  NOT THE FIRST, and I am revising what this entry asks for. They found that their own
  third instance was filed against a harness that was ALREADY isolated: e2e/serve-head.sh has
  built from committed HEAD in a detached worktree since 7624877a (2026-08-11), and
  `git log -S 'crate::worker::WorkerId' --all` finds nothing, so the import that killed their
  run was never committed and cannot have reached a build of HEAD. They diagnosed "a peer
  mid-edit" and the record cannot establish that it was one — because serve-head.sh announced
  its source only `if [ -n "$dirty" ]`, so a clean-tree run printed nothing and all three
  source paths looked identical. Fixed in eeccbbc1: one SOURCE line per path.
  THE SAME GAP IS IN THE ad-hoc `cargo test` PATH THIS ENTRY IS ABOUT. A failing run does not
  say what tree it compiled, and on this checkout the answer is always "the working tree,
  including every peer's uncommitted edits". So the cheap first answer to "is this mine" is
  a SOURCE line plus a count of dirty files that are not yours — not blame analysis. Checked
  before writing this: nothing wraps `cargo test` for that today (scripts/test-tree-clean.sh
  is about RESIDUE, whether a command left the checkout as it found it, which is a different
  question). Build the SOURCE line first; the cargo-blame wrapper is worth having only for
  what the source line cannot resolve.
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
## A main lane with no $AMUX_SESSION in its env is invisible to the staged-guard's edit records
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-24
SESSION: mixpeek-research
CARD: MR-43
SYMPTOM: This lane runs in tmux session `amux-mixpeek-research` (amux-launched), yet
  $AMUX_SESSION is empty in its shell. In one task that meant: `amux board add` would have
  created an unattributed card, the prepare-commit-msg trailer would have been empty, and the
  staged-guard's cross-session check said "you have no edit record on this path in the last
  360m" for a file this lane had edited three times in the previous ten minutes, because the
  PostToolUse edit-record hook reports under the same empty variable. Its verdict then named
  the peer as the sole editor and blocked the commit. The three subagent entries earlier in this
  file (SESSION: "... no $AMUX_SESSION in env") are the same shape one level down.
COST: two refused commits and about 5 minutes, plus a guard verdict that was wrong about who
  edited the file; every CLI call needed AMUX_SESSION exported by hand from the tmux name.
FIX: derive the session from the tmux session name (`tmux display-message -p '#S'`, strip the
  `amux-` prefix) in the edit-record hook and the CLI when the variable is empty, and say in
  the guard verdict when that fallback was used. Plus a WARN in the lane-launch path when a lane
  starts without the variable, so /api/logs/analyze can count these instead of a human noticing.

## The drift-detector protecting mixpeek's git guard is itself blind to staleness
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-24
SESSION: mixpeek-research
CARD: MR-44
SYMPTOM: Landing MR-43 (tmux-derived $AMUX_SESSION fallback) required running
  `install-hooks.sh --all` to propagate the fix. It reported mixpeek's
  `.githooks/amux-staged-guard` as "diverges from canonical but carries every
  canonical feature — left untouched", the correct, safe verdict for a
  deliberate local merge. It is not one: mixpeek's copy is GUARD_VERSION = 4
  against a canonical of 9, missing ~215 lines including AF-127 outcome
  reporting and the AF-195 index/worktree divergence check. The staleness
  check greps the canonical's single `guard-features` token (AMUX-2946) as a
  bare substring anywhere in the target file; mixpeek's v4 copy happens to
  contain that literal string at line 75 in an unrelated comment about retired
  ports, so the check reads "feature present" when the actual AMUX-2946
  feature never landed there. This is the exact MG-1485 dark-guard shape the
  mechanism exists to catch, undetected by the mechanism itself, in the one
  checkout that matters most for daily commits.
COST: not measured directly — the cost is whatever the missing ~5 versions of
  protection would have caught and did not (AF-195's index/worktree check in
  particular: mixpeek is a shared checkout where that class of bug already
  happened once, per its own header).
FIX: two separate fixes. (1) Upgrade mixpeek/.githooks/amux-staged-guard and
  prepare-commit-msg from v4 to v9 — a real merge, commit in that repo. (2)
  Make the drift-token check itself resistant to this: require the token
  match to come from a comment-anchored form, or compare GUARD_VERSION
  numerically in addition to/instead of grepping tokens. Otherwise the next
  stale copy hides the same way. Neither started; MR-44.

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
## The nudge that tells you to union-merge cannot tell you how to do it safely
AREA: notices
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-25
SESSION: mixpeek-frustrations (hit it), amux (fixed it)
CARD: AMUX-3718
SYMPTOM: A DIVERGED FRUSTRATIONS.md nudge said `MERGE the two versions (for
  append-only files, union-merge per .claude/rules/frustrations.md)` and stopped
  there. Two things were wrong at once. The cited path exists in ~/Dev/amux and
  NOT in ~/Dev/mixpeek, where the reader was, because commit_nudge is server
  code that fires into every lane's OWN checkout. And the safe procedure it was
  pointing at could never have arrived anyway: `build()` defines commit_worthy
  as the dirty paths that are NOT stale/diverged/revived, and the archive-check
  note was emitted from inside `commit_worthy_body`, which receives exactly that
  set. So a DIVERGED append-only file was structurally excluded from the only
  code that emits the archive check. The one state that prescribes a union-merge
  was the one state that could not be told how to perform it.
COST: A near-miss on real data. The lane followed the destructive half verbatim,
  which would have resurrected an entry closed on a 692/692 prod measurement and
  double-inserted a content twin already on origin under a different subject. It
  also cost a second lane a wrongly-filed card, since from ~/Dev/mixpeek the
  only visible symptom is "this file does not exist" and the citation looks like
  the whole bug. Both readings were reasonable and both were incomplete.
FIX: 972b44a4. Hoisted the note to `build()` over the full dirty set so it
  travels with every arm, deleted the citation because the procedure is already
  inline, and rewrote the note's unit test to go through `build()` — it had been
  calling `commit_worthy_body` directly and was green for the entire time the
  note was unreachable (ethos rule 7 / AF-161: a check pinning the wrong layer
  is exactly as green as one pinning the right layer). Second fix per the
  two-fixes rule: `missing_archive_check()` now WARNs on the ACTUAL delivered
  bytes before `steer_enqueue`, so the next regression announces itself in
  server-rs.log instead of arriving as another near-miss.
  The general shape worth remembering: a citation is only as good as the reader's
  checkout, and the dangerous half of an instruction must never be the half that
  travels while the safety half is behind a link.

## `amux` died at load with a bash syntax error — every subcommand, every session, at once
AREA: cli
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-26
SESSION: amux (hit and reported by gtm-media-assets)
CARD: AMUX-3722
SYMPTOM: `amux send <peer> --stdin` -> `/Users/ethan/.local/bin/amux: line 1906: syntax error near unexpected token `;;'`. Identical on --file and on every `amux board` subcommand: a parse error at LOAD, so nothing input-dependent. Reporter also observed that ~/.local/bin/amux stats as 26 bytes with a Feb 18 mtime, which cannot be a script with a line 1906 — that is the SYMLINK's own stat, and the target is what changed.
COST: The whole fleet lost the CLI until a peer noticed and reported it over the HTTP API. Unquantifiable session-minutes across ~50 lanes, and the reporter spent time chasing an mtime that could never have explained it. Worse than the outage: a CLI that cannot parse cannot print its own help, so the tool could not tell anyone how to work around the tool — a session that had only ever used `amux send` had no path left to discover POST /api/sessions/<n>/send exists, and would read it as "amux is down" rather than "the CLI is down".
FIX: 5ecec79c. Two gates at the two boundaries. `.claude/check-and-commit.sh` runs `bash -n` on any edit to `amux` — that is the one that fires in TIME, because ~/.local/bin/amux is a symlink into the working tree and for the bash CLI the deploy boundary is the SAVE, not the commit: no install, no builder cycle, no CI in between. `tests/cli_syntax_guard.rs` is the backstop for an edit made without the hook. Both mutation-verified against the real specimen. The repo already ran `node --check` on dashboard JS on every save; the CLI, which ships faster and breaks wider, had nothing.

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

## A "slim" payload that omits a column can still SHIP DERIVATIONS of it, and the loader layer cannot see that
AREA: instruments
SEVERITY: blocks
STATUS: open
DATE: 2026-08-30
SESSION: amux-frustrations
CARD: AF-346
SYMPTOM: I made `/api/board`'s slim path stop SELECTing desc+log, having verified the slim
 response carries neither key. It shipped (a99955f7) and blanked every card preview on the
 fleet dashboard: desc_len>0 = 0, desc_head!="" = 0, log_n>0 = 0, folded_n>0 = 0 across
 2,047 cards, and needsyou_note gone, so cards waiting on a human stopped showing their
 question. `list_body` derives five values from `row.desc`/`row.log` BY REFERENCE and ships
 those instead of the prose. Reverted at b1227af0, restored and verified.
COST: A live user-visible regression on the owner's own dashboard, caught by a peer
 measuring the deployed build rather than by any test. Roughly 20 minutes of blank previews
 fleet-wide, plus a near-miss on a double revert that would have re-applied it. My full test
 suite was green: the cell I wrote asserted the slim hydrate returns empty prose and the full
 one returns it (both true), and the two PRE-EXISTING equivalence tests key on desc+log,
 which the slim body omits by design, so they compared two payloads that both correctly
 omitted the fields and asserted nothing about the derived ones.
FIX: The structural half is a naming problem the code cannot express: "slim" describes the
 PAYLOAD, and every reader takes it as a claim about the LOADER. AMUX-2840 was this same bug
 one layer up ("silently blanked both in the dashboard"), its warning is three lines above
 the line I changed, and I read past it - so a comment is demonstrably not sufficient here.
 What would have caught it is a consumer-side invariant rather than a mechanism-side one: a
 slim row's desc_len must equal the real desc length, and its desc_head the real first line.
 amux is adding that cell with the revert. Generally: when a payload drops a column, the test
 that matters asserts on what the payload DERIVES from it, not on the column's absence.

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
## test-contended.sh rules out the builder, so its green verdict reads as "therefore your bug"
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-31
SESSION: amux-frustrations (diagnosed), amux (paid the cost)
CARD: AF-356
SYMPTOM: `amux` ran the suite and got 1 failure, `gate_table_matches_python`, in a
  module they had not touched. It passed in isolation. test-contended.sh printed its
  verdict: "the auto-builder was NOT rebuilding during this run, so the shared binary
  was stable under it. A failure here is NOT build contention." They concluded ETXTBSY
  anyway, reported it as their weakest evidence line, and asked me to form my own view.
  The true cause was neither: MY uncommitted edit to board_store.rs was sitting in the
  shared worktree while their test ran. I was adding `ItemType::Decision` (AF-323), and
  that test asserted `default_gates_for("decision", Done) == default_gates_for("code",
  Done)` because `decision` was its stand-in for an unknown type. My half-finished
  change made the assertion false in a file they never opened.
COST: A wrong root cause carried into a verification request as its stated weakest
  line, and ~20 minutes of mine to refute it. The deeper cost is that the wrapper's
  verdict is load-bearing in the other direction: it exists to stop a red being read
  as a regression, and by ruling out the ONE cause it can see, it makes the remaining
  space look like "your bug" when a shared checkout has a third option. Both failure
  modes look identical from inside a test run: a red in a module you did not touch,
  green on rerun. The wrapper cannot tell them apart and does not say so.
FIX: The wrapper already answers "was the builder rebuilding". Have it also answer
  "was the tree dirty, and in which files" — `git status --porcelain` before and
  after, printed beside the verdict, naming the paths and their sessions from the
  observed-edit records the staged-guard already keeps. Then the verdict line becomes
  two facts instead of one, and the missing clause stops being invisible. Ethos rule 4:
  the output that can read "clean" must publish whether it measured the thing at all,
  and this one measures one of two causes while its sentence implies both.
FIXED 2026-08-31 in the commit naming AF-356. The wrapper now captures the dirty set
  BEFORE and AFTER the run and prints it beside the verdict, on BOTH arms. The
  before/after pair is the strongest form: a tree that CHANGED under the compile is
  the case a single snapshot cannot see. It deliberately does NOT attribute an owner,
  because mtime attribution has been wrong on this checkout repeatedly (AF-179,
  AMUX-3662) and a confident wrong owner is worse than a named file with none. Clean
  is STATED rather than left silent, since a silent probe and a clean tree are
  otherwise the same output. scripts/test-contended-worktree.sh, 7 cells, wired into
  CI; the mutation that writes it the obvious wrong way (clause inside the clean arm
  only) fails exactly (c2) and (d).

---
## A peer's `git add` swept a migration DELETION into an SEO commit
AREA: attribution
SEVERITY: slows
STATUS: open
DATE: 2026-08-31
SESSION: amux-frustrations
CARD: AF-357
SYMPTOM: `crates/amux-server/migrations/0035_reclaim_skipped_hits_repair.sql` was a
  leftover duplicate from a merge renumber, and it was reddening the suite fleet-wide
  via `every_migration_file_on_disk_is_registered`. It was removed in `26c45798`,
  whose subject is "feat(seo): parallel-feature-dev rewrite, new splitting-work guide,
  changelog 2026-08-30/31". An SEO commit deleted a migration file. Confirmed with
  `git log --diff-filter=D -- <path>`.
COST: The deletion was correct and needed doing, so nothing broke. The cost is
  forensic and it is deferred: `git log` on that path names a commit whose message
  cannot account for it, so the next person asking "when did this migration go, and
  why" finds an answer that explains nothing. I can also date it precisely, which is
  the part nobody could have recovered later: my first full suite run went red on
  exactly this and my rerun of the same test minutes later went green with no change
  from me, so the deletion landed INSIDE a 181-second test run. Anyone chasing that as
  a flake would have found nothing, for as long as they looked.
FIX: This is AF-316's class (one checkout, N lanes, one index) in the DELETE
  direction, which the existing entries do not cover: they are all about a peer's work
  being ADDED to your commit. A removal is worse to trace, because the file is not
  there to notice. The per-lane worktree in AF-336 ends it. Short of that, the
  staged-guard's co-edit notice counts insertions and deletions per path but does not
  flag a staged DELETION of a file the committer never touched, which is the cheap
  version of this specific catch.

---
## A decision card carried two mechanisms, so the half needing no decision waited 13 days
AREA: board
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-31
SESSION: amux-frustrations
CARD: AF-359
SYMPTOM: AF-98 sat in `needsyou` from 2026-08-18 titled "Cross-group send blocks the
  frustrations validation program", asking Ethan for "one-line config, or a deliberate
  no". It described TWO different refusals as one problem. Reaching `desktop` is
  refused by the isolated-raw-agent rule, which only the owner can work around and is
  genuinely his call. Reaching `mixpeek-frustrations` and `mixpeek-research` is
  refused by the ordinary intra-group rule, whose refusal text names its own remedy in
  the body: "set CC_SEND_ALLOW on amux-frustrations". Both symptoms print as "send
  refused" and only the refusal BODIES distinguish them, so from the card's own text
  the two are indistinguishable.
COST: 13 days of the frustrations program blocked on three entries that needed no
  decision from anyone. I set CC_SEND_ALLOW="ops,new-features" on this lane's own env,
  scoped rather than `*`, and all three routed immediately with no restart. The
  mechanism had been printing the fix in every refusal for those 13 days. The deeper
  cost is the shape: a card is ONE unit of work, something that can be honestly done or
  not done, and this one could not be. No single answer finished it, so it parked in
  the queue where the answerable half was invisible behind the unanswerable half.
FIX: Fixed for this instance by splitting it: AF-98 is narrowed to the isolated-worker
  half only, and the cross-group half is closed. The general form is the 451-folds rule
  applied to `needsyou` specifically: before parking a card on a human, ask whether
  EVERY part of it needs them, because the parts that do not will wait exactly as long
  as the parts that do. A cheap version worth building: when a card enters `needsyou`,
  have the typed `--ask` gate (AF-318) refuse an ask whose own body names a remedy the
  ASKER can apply. That is detectable — this refusal literally contained an imperative
  addressed to the sender.

---
## Retiring an entry is two writes, and only the second one can fail
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-31
SESSION: amux-frustrations
CARD: AF-362
SYMPTOM: `scripts/frustrations-archive.py` does two things per entry: it MOVES the
  text from frustrations.md into frustrations-archive.md (a local file write, which
  always succeeds) and it CARRIES the SYMPTOM and COST onto the entry's card over
  HTTP. Only the second can fail, and it did, twice in one batch:
    AMUX-3887: NOT carried (curl exit 7, 0 bytes)
    AMUX-3723: NOT carried (curl exit 7, 0 bytes)
  `/api/health` answered normally moments later with `uptime_s: 11`. The server was
  not down, it was MID-RESTART: this box rebuilds and swaps the binary on every
  commit, and I had just committed twice.
COST: Two half-retired entries. The text was gone from frustrations.md and the cards
  it pointed at never received the symptom or cost, which is the whole reason AF-38's
  rule carries them: the card is where someone hitting the friction again looks. The
  archive move is NOT rolled back on a failed carry, so nothing self-heals and nothing
  re-attempts. I only caught it because the tool prints `NOT carried` honestly, which
  is AF-150's lesson working; a version that inferred success from an empty stdout
  would have reported two clean retirements. The exposure scales with batch size and
  I archived 20 entries in one run tonight, so this was luck rather than a near miss.
FIX: Fixed. The carry now retries three times with 2s between attempts, which covers
  a binary swap, and still reports `NOT carried` when the server is genuinely down
  rather than blocking the archive (a retry that ended in a false success would be
  worse than none). Cell (r) in scripts/test-frustrations-archive.sh pins it on
  elapsed time against a closed port, a floor and never a ceiling; cell (r2) is the
  control that the failure is still reported. Removing the sleep makes (r) fail.
  The general shape is worth keeping: an operation that is one verb to the user but
  two writes underneath needs to say which half it completed, and this one did, which
  is the only reason there was anything to fix rather than to discover later.

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
## A card create records the claimed creator and nothing about the request
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-31
SESSION: amux-frustrations
CARD: AF-366
SYMPTOM: Auto-pickup handed this lane `AF-364 [ts-gke] tenant-deploy engine skipped
  on 75ad074eab7c` to work, alongside `AF-363 Test card from tubescience`. Both are
  stamped `creator: amux-frustrations` and `session: amux-frustrations`, created 8
  seconds apart with empty descs. I did not create either. Nothing anywhere can say
  who did:
    - a successful POST /api/board logs NOTHING (`grep -c 'POST /api/board'` over
      today's server-rs.log returns 0), while the board READ path logs `caller_ua`
      and `caller_session` on its truncation WARN;
    - the `_amux_state_events` payload for the create stores the resulting
      `creator` field, which is the value in question, not the request's origin.
  The `X-Amux-Session` header is caller-supplied and unverified, which is fine for a
  local fleet, so the stamp is only ever as good as the caller's honesty or config.
  Verified by control: a create with NO session header leaves `creator` empty and
  takes the `AMUX-` prefix, so these two DID carry this lane's header.
COST: A lane was handed another team's deploy card by an automated loop, and the
  misrouting is unfixable at the root because the origin is unrecoverable. I could
  route AF-364 to ts-gke and discard the probe, but I cannot tell whoever did it,
  and neither can anyone auditing later. The read path being instrumented while the
  write path is not is backwards: a read is recoverable by reading again, a write is
  not.
FIX: Fixed. The create success path now emits `board card created` at INFO with
  `card`, `caller_session`, `caller_ua`, `stamped_creator` and `owner_session`, so a
  header that disagrees with the caller, or a create from an unexpected user-agent,
  is greppable. It reuses `truncation_caller` rather than re-deriving the pair, so
  the honest fallbacks ("(none)", "(unattributed)") stay identical to the read path
  instead of one site growing a silent blank. Not claimed as fixed: nothing verifies
  the header, and this does not change that. It makes a wrong stamp VISIBLE, which
  is the part that was missing.
