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

## staged-guard named a co-editing session that never edited the file — ownership inferred from API traffic
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

## Acking a peer's card with a desc PATCH silently destroys their write-up
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

## Discarding a spurious autofix card refiles it, so doing the right thing loops
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

## A graft-push checkout read as DIVERGED on every path, withholding the safe restore
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-24
SESSION: mixpeek-frustrations (reported), amux (fixed)
CARD: AMUX-3599
SYMPTOM: The idle commit-nudge filed dirty append-only files as DIVERGED — "commits in BOTH
  directions, neither single-arm remedy is safe" — on a checkout where the local commits were a
  REPLAY of content already upstream. DIVERGED forbids both remedies, so the reader is left with
  a union-merge they do not need and the safe `git checkout origin/main -- <file>` is withheld.
  The classifier asked `git log origin/main..HEAD -- <path>`, which counts commits BY SHA, and a
  commit already upstream under a different sha sits in that range permanently. On a graft-push
  checkout that is EVERY path.
COST: The wrong verdict on the exact file class the nudge singles out by name — the append-only
  ledgers, where the union-merge directive is printed. A reader following it does more work than
  needed and, worse, learns that the nudge's verdicts are unreliable on their checkout, which is
  the expensive direction: the next DIVERGED that IS real gets read as more of the same. Nobody
  lost data; the reported cost is a wrong prescription plus the turn spent establishing it.
FIX: d55b7a63 — content set-difference instead of sha arithmetic, since sha identity is what a
  replay destroys. The remedy overwrites the WORKTREE, so restore-safety is exactly "does the
  worktree hold lines origin does not"; zero means nothing here can be lost. One-sided by design:
  it only ever downgrades diverged->stale, only on a readable pair AND an empty difference, so
  any error leaves DIVERGED standing.
NOTE: This is the SECOND defect in this cell in four days and they point opposite ways. The cell
  was ADDED on 2026-08-20 because the two-bucket classifier filed a genuinely-diverged path STALE
  and the prescribed restore disarmed a data-loss push guard. This entry is the same cell now
  over-firing. Both are the same underlying error — reading commit identity as content identity —
  and it produced a false negative first, then a false positive, which is why "be more careful
  with the direction test" would not have caught either. The durable form is that a classifier
  prescribing a DESTRUCTIVE remedy has to be gated on what the remedy actually destroys, not on
  a proxy for it. Also worth recording: the fix logs the downgrade, because STALE-because-
  downgraded and STALE-outright were otherwise byte-identical in the log, which is the one-output-
  two-states shape on the arm that prescribes the destructive remedy.

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
## Mutation testing's obvious harness is a whole-file write, which reverts a peer mid-edit
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

## Every amux-launched Chrome opens with the yellow "unsupported command-line flag" infobar
AREA: browser
SEVERITY: annoys
STATUS: fixed
DATE: 2026-08-24
SESSION: mixpeek-research
CARD: MR-38
SYMPTOM: Ethan's screenshot at 15:40: "You are using an unsupported command-line flag:
  --ignore-certificate-errors-spki-list=... Stability and security will suffer." across the top
  of every window the amux browser opens. The SPKI pin is on Chrome's kBadFlags list
  (chrome/browser/ui/startup/bad_flags_prompt.cc:107), so the bar has been on every launch since
  the pin shipped. Nothing in amux could see it: it is browser chrome, not page content, and no
  verb screenshots that, so the only detector was a human looking at the window.
COST: every human-facing browser session since the pin shipped read as broken or unsafe to the
  person looking at it, until Ethan screenshotted it. About 90 minutes across two lanes to land,
  most of it the shared-checkout dance (the peer's whole-file write dropped two of three edits
  once; see the entry above at "Mutation testing's obvious harness is a whole-file write").
FIX: 9f4e6971. --test-type on the launch line: chromium infobar_utils.cc:173 returns before
  ShowBadFlagsPrompt for a test-harness launch (ChromeDriver passes it on every session);
  --enable-automation would also work but adds its own "controlled by automated test software"
  bar. Flags extracted into chrome_launch_args() and launch_args_tests pins "bad flag =>
  --test-type" with a control that the pin is really present; mutation-checked red without the
  flag. NOT confirmed on screen from this lane: screencapture is refused for a tmux shell (no
  Screen Recording grant), so the visual check is Ethan's next launch. Already-running Chromes
  keep the bar until relaunched.

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

## session-freshness reported a stale shadowing CLI and prescribed the `cp` that rebuilds it
AREA: instrumentation
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-24
SESSION: amux
CARD: AMUX-3687
SYMPTOM: The freshness hook's Axis-2 shadow detector fired correctly on
  /usr/local/bin/amux and then offered `cp "$REPO/amux" "$cand"` as the remedy. A copy
  silences the warning and leaves a copy, which is stale again the next time anyone edits
  ./amux — so the prescribed fix reconstructs the exact condition being reported. It is
  also how the specimen got there: ~/.local/bin/amux has been a SYMLINK since install.sh
  created it, and /usr/local/bin/amux was the copy, so the one file that could drift was
  the one the remedy would recreate. What was actually sitting there was an Aug-6
  227-line stub knowing two verbs (send, board) and defaulting AMUX_URL to
  https://localhost:8822, the retired port (AMUX-3046). A lane resolving it gets
  connection-refused on every call, and help-and-exit-0 on `url` or `alert`.
COST: 18 days undetected, and the detection that finally landed pointed at a remedy that
  would have reset the clock. Not measurable in minutes for me (the hook named the file
  and I checked it), but any lane whose PATH ordered /usr/local/bin first was talking to
  a dead port for those 18 days with no error a session would recognise as a stale CLI.
FIX: `ln -sfn`, in both branches of the axis, with the reason stated inline so it does not
  get "simplified" back to a cp. b0a0c6b7. Live shadow reconciled the same way; both PATH
  entries now resolve to the checkout and the axis is silent.
  The generalisable half: a detector that names a remedy owes the same scrutiny to the
  REMEDY as to the check. This one could fail, fired correctly, and still closed the loop
  back onto itself.

## "CDP never answered within 30s" printed with `DevTools listening on <that port>` in its own message
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

## The nudge that tells you to discard a card names no command that does it
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
## A detector's query failure was swallowed, so the whole detector had no coverage
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

## SUPERSEDES the entry above: browser state's cap was silent, and my diagnosis of it was wrong
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

## `force` claimed to log the judgment and logged an empty string, 41 times out of 41
AREA: attribution
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3723
SYMPTOM: Every force audit line on this board reads `force by <who>: a->b reason=` with nothing after the `=`. 41 lines, 41 blank — never once populated. The board contract advertises force as "bypass (judgment stays with you; logged)", and ts-gke's 2026-08-03 fix made attribution mandatory precisely so the ledger would name the party holding the judgment. It named them and recorded no judgment.
COST: Found while auditing how the autofix backlog was actually closed, and it made that audit undecidable for 25 cards: bulk-discarded in one minute, attributed, with nothing recorded about why. Reconstructing intent meant reading desc diffs card by card. The one escape hatch from the entire gate system was the one action whose trace could not answer the only question anyone asks of it.
FIX: f013ba5b. Neither obvious suspect was guilty, which is why it survived: `amux board --force` has always REFUSED to run without a reason, and the server has always written a supplied reason to the log (an existing test asserts it, and passed throughout). The CLI validated the reason and then sent it as `desc_append` instead of `reason` — 9 of the 41 cards carry a good "[FORCED] <why>" in their desc beside a ledger line that says nothing. A test on either side of the seam and none ON it. Now: the CLI sends both, the server refuses a blank reason from any caller with a 400 that names the sanctioned command, and a `force_without_reason` tracing marker makes the next off-path caller visible (a bare 400 here groups with every other board-PATCH 400 in /api/logs/analyze).

---
## amux lanes answer from an 8th-generation summary; a raw terminal answers from primary sources
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
AREA: instruments
SEVERITY: blocks
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3756
SYMPTOM: `derive_status` applies a real trust model to a stored self-report — previous life, `stale_active`, trust window — and publishes `applied:false` for one it refuses. `steer_lane_at_boundary`, the gate on auto-pickup, board nudges and steering delivery, read the SAME row and asked only `state == "idle"`. So a lane whose Stop hook never fired kept a stuck `active` report, its dashboard badge correctly read IDLE (`decided_by: activity_fallback`), and the drive loop skipped it as `mid-turn` forever. The two halves of amux disagreed about the same fact, and the correct half was the one nobody acted on.
COST: 4 of 52 running lanes held out of the work loop, every one with `auto_pickup: true` and eligible cards waiting: creative-dna 61.4h, ai-video-editor 59.5h, mixpeek-autopilot 6.4h, primer 1.0h. Self-perpetuating, because only a turn writes a new report and only a human starts a turn on a lane the loop refuses to touch — so the sole exit was Ethan typing at it, which is exactly what he reported ("why do i need to push @tubescience to continue"), and doing so destroyed the evidence. The `mid-turn` skip reason read identically for a lane genuinely generating and one deadlocked for two and a half days.
FIX: 7e4682f0 — `report_applies()` is the one predicate, called by the badge and the gate; `lane_report()` is the one read, replacing two unjudged copies. A refused report WARNs `stuck_self_report` once per lane per report ts, and the board-drive trace's `mid-turn` detail now names the report's state, age and verdict.

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

## A gate that reads the real filesystem from inside a pure board function turns three unrelated tests red on every host
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
AREA: tokens
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3759
SYMPTOM: `pickup_prompt` built the card's `desc + log` and then wrote `.chars().take(500)`. The lane received an ID and a 500-character stub, and spent tool calls reading back text the function had in hand one line earlier. Measured over 11,117 turns across 67 lane transcripts: an auto-pickup turn takes a MEDIAN OF 22 TOOL STEPS where a human-prompted turn takes 3, at a median resident context of 308,059 tokens per model call (p90 738k, max 966k). The cap saves ~1k tokens of steering text and costs ~308k per avoidable fetch — the wrong resource by three orders of magnitude. Silent, too: a truncated excerpt was indistinguishable from a short card.
COST: On the live queue it truncated 86% of todo cards (median definition 1,933 chars, p90 6,658) and discarded 108,820 characters of card definition. 43.8% of fleet turns and 49.7% of input tokens are amux-initiated, so this rides the largest single class of spend. Ethan noticed by feel — "theres also way too much tokens used for some reason in between tasks" — because no instrument reported steps-per-turn by what started the turn.
FIX: ade006c2 — `AMUX_PICKUP_EXCERPT_CHARS`, default 4000, config rather than a constant because this is D4 in the ethos ledger. A cut excerpt now says it was cut and names the read.

## "Is this badge accurate" is unanswerable by the time the screenshot arrives
AREA: instruments
SEVERITY: slows
STATUS: open
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3761
SYMPTOM: `derive_status_explain` is computed fresh per request and never persisted, and `session_events` records no lane status rows at all (verified against the live DB: zero for gtm-research across the whole window in question). So `status-explain` answers "which rule decided this lane is WORKING right now", while the question anyone actually asks is "why WAS it WORKING when I looked" — and a screenshot always arrives minutes later, by which time the lane has taken another turn and the evidence is gone.
COST: Ethan sent a screenshot of gtm-research reading WORKING + AGENTS over a pane whose visible text was the agent saying it had no task queued, and asked whether that was accurate. It reads `idle` now, correctly and for a good reason, and which rule fired 31 minutes earlier cannot be recovered. AMUX-3434 built status-explain specifically so a wrong badge would not cost a screenshot investigation; it still does, one layer up.
FIX: none yet. Record a `session.status_decided` event on CHANGE of status or `decided_by`, and return recent history from status-explain. The natural home is the ScanLoop, and a write-on-change into a 2.2GB SQLite from a 15s loop over 52 lanes needs its row rate measured before it ships.

## Fixing a mechanism made its own nudge text false, and the false nudge went to the lane that wrote the fix
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
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-26
SESSION: amux
CARD: AMUX-3774
SYMPTOM: autofix files one card per fault, and only OPEN cards suppress — deliberately, so a judged-and-discarded card lets the next occurrence through. But `backlog` is open. AMUX-3651 sat parked in backlog from 08-24, so every server-wide stall since was correctly detected, correctly deduped, and filed nowhere. The suppression reason also asserted "Its count is what moves; a second card would carry no new information" while the code pushes a report row and `continue`s, never touching the card — so the one signal it pointed at did not exist.
COST: Two days of a whole detector class dark, including a live six-family stall. `filed: []` on the tick reads identically for "nothing is wrong" and "everything is muted", which is this repo's most-reinvented bug. Found only because I was chasing an unrelated duplicate card and opened the suppression list; nothing would have surfaced it otherwise, and the card that muted the class looked like an ordinary parked backlog item.
FIX: 8b55d0bf — the false claim deleted (ethos rule 6: implement it or delete it), the suppressing card's staleness printed WHETHER OR NOT it is alarming, an explicit note that suppressing does not bump the card, and an `autofix_mute` WARN past AMUX_AUTOFIX_MUTE_WARN_DAYS. Verified live: AMUX-3651, stale_days=2.03. The better fix — actually bumping the count — is named on the card and deliberately left for its own change, because it is a write on every scan against the live board.

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

## An empty commit reported success, attached itself to a card, and closed it
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
AREA: instruments
SEVERITY: slows
STATUS: fixed
DATE: 2026-08-28
SESSION: amux
CARD: AMUX-3829
SYMPTOM: The browser idle-reaper held its "first seen empty" clock in a process-global map. The builder installs a new binary and the server self-adopts on EVERY commit, so the 3600s window restarted whenever anyone in the fleet committed. Measured that day: 22 builds between 06:33 and 16:26, median gap 16.7 minutes, and only TWO gaps of 60 minutes or more. Against a one-hour window that is a reaper which on a working day can almost never fire. Throughout, `/api/system-jobs` reported `spawned: true, ticks: N, status: ok`, and every word of that was TRUE — the loop was running perfectly. The job's health describes the LOOP; the defect was in state the loop carries. From outside, a reaper that can never fire and one about to fire were byte-identical.
COST: A card shipped claiming "the 18-hour zombie that prompted this cannot recur" when it could, and it stayed that way until I went looking during a verification pass. Nothing in the system was going to surface it: there is no signal anywhere for "this job is alive and cannot succeed". The generalisable trap is that a runtime job's registered health answers "is the loop running", which is a different question from "can this job do its work", and the two come apart exactly when the work depends on state that does not survive a restart — on a machine that restarts its server on every commit, that is most stateful jobs.
FIX: 5a8c85ab — the clock moves to `~/.amux/browser-idle.json`, rewritten whole each tick so stopped profiles drop out. The countdown is published on `/api/browser/status` as `idle_s` (null when not empty, which is a different fact from zero) so the invisible state becomes observable, and the release log carries `pre_boot_s`, how much of the window predates this process: a non-zero value there IS the restart-survival working, and the in-memory version could only ever print 0. The transferable question, which I would now ask of any registered job: if this process restarted right now, would the job lose progress, and would anything say so?
