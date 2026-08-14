# amux

- whenever you fix a bug do 2 things: 1) fix it durably at the root cause and 2) make it so that the bug would have surfaced in the amux logs so that a log sweep would've caught it, you may need to replicate the bug to confirm it appears in the logs. 

A **Rust workspace** (`crates/amux-core`, `amux-server`, `amux-cli`, `amux-dashboard`)
serving a static SPA. **The address is 8824** (`AMUX_RS_PORT`, what `./install.sh`
sets) — use it everywhere.

8822 is the RETIRED port and its compatibility bind is **gone** (Ethan dropped it
2026-08-11: "no more 8822 just rust", `crates/amux-server/src/lib.rs`).
`curl -sk https://localhost:8824/health` answers; `:8822/health` no longer does.
Do NOT re-add the bind to fix a symptom — lib.rs records why, and names what
replaced it (`endpoint.json` self-heal for hooks, canonical-port launch for new
lanes). `tests/legacy_port_guard.rs` fails the build if the address reappears.

The catch this leaves: lanes started before the cutover still carry
`AMUX_URL=https://localhost:8822` in their PROCESS env, which a live process cannot
rotate, so their raw `curl $AMUX_URL/...` fails at connect and reads as "server
down" (this is AMUX-3046, and it will bite you if THIS session is one of them).
Two remedies: use **`$(amux url)`** in place of `$AMUX_URL` in any recipe — it
reads the server-written `~/.amux/endpoint.json`, self-heals past a stale port, and
survives the next port move too (the `amux` CLI itself is already unaffected); and
**restart** a stranded lane to clear its env. The strand self-announces — the CLI
warns once per session, the server logs an hourly WARN naming stranded sessions,
and `GET /api/debug/legacy-port` enumerates them (`stranded_count`,
`stranded_sessions`, plus the recycle-vs-resolve decision).

The Python predecessor (`amux-server.py`, single file, inline dashboard) was **deleted
at commit 792ce1f** (2026-08-09). Git history has it. Nothing here should assume it
exists — if you find an instruction that does, that instruction is stale and fixing it
is in scope.

## The ethos — read before building anything

**The harness gets better as the models get better. Get out of the model's way.**

Every new feature or enhancement gets gut-checked against
[`.claude/rules/ethos.md`](.claude/rules/ethos.md) before you build it and again before
you call it done. Eight questions, each one there because it was violated in this repo
and cost something real: capability that never reached a session, model calls spent on
string manipulation, gates that could not be satisfied honestly, instruments that could
not express the discriminator, automation that accumulated instead of deciding, an audit
trail that was claimed but not implemented, checks that could not fail, and an agent
deciding something that was the user's to decide.

The question underneath all of them: when the next model is meaningfully better than
this one, does this feature get better with it, or does it become the ceiling?

The bottom of that file is a **Known deviations** ledger — live places where amux
still fights the ethos, each with a status and an exit condition. Check it before
touching state detection, auto-answers, helper-model calls, observation caps, or
auto-compact; move the deviation toward its exit rather than deepening it.

## Build on the primitives — never reinvent or abstract them

**The primitives are: board, workers, schedulers, filesystem, groups, memories,
environment, messages.** Ethan's conviction on this set is high and it governs every
new feature: *"I'd encourage us to just build on top of them… I don't suggest
introducing a new feature in order to effectively re-invent, or abstract the
primitives."*

Improving the UX of a primitive, or the integration between two of them, is always in
scope and is most of the good work available. Adding a ninth thing that sits above
them and re-expresses them is not.

**The test, when a capability is requested: name it in terms of the primitives.** A
"chief of staff" is not a subsystem to build — it is a configured environment, backend
and frontend: *these tabs, these board gates, this group, these workers, these
memories, these environment variables (the 3p APIs it can reach).* Ethan's own worked
example: "on receipt of a command from the user, coordinate with workers within this
group to do xyz" is **a gate on a group**, not a new coordination engine. If the
request decomposes cleanly into primitives that already exist, the work is
configuration and UX — which is the work.

Two shapes to reject:

- **A wrapper that re-expresses a primitive under a new name.** If it stores units of
  work, it is the board. If it runs something later, it is a scheduler. If it holds
  per-scope config, it is environment or memory. If it moves text between lanes, it is
  messages. A second spelling of an existing primitive doubles the surface that must
  be kept in step forever — see D6 in `ethos.md` for what a single duplicated seam
  already costs.
- **An abstraction layer over several primitives.** This one fails the compounding
  question directly: a better model can compose primitives it can see, but it cannot
  see past a layer that has already decided how they compose. The abstraction becomes
  the ceiling at exactly the moment the model gets good enough to have done better.

A genuinely absent primitive is legitimately new, but the bar is that **no
composition of the existing eight expresses it** — not that the composition is
awkward. Awkward composition is a UX defect *in* the primitives, and fixing it there
is worth more than routing around it, because every other composition gets the fix
too. When you are unsure which case you are in, say so in the commit message and name
the primitives you considered.

## You are dogfooding — fix it at the root

You run *inside* amux. Every rough edge you hit while working is a rough edge a user
hits, and you are the one person positioned to see it from the inside. So when
something in amux gets in your way — a peek that hides output, a board mutation that
bounces silently, a browser profile that saves to one place and loads from another, an
error message that sends you chasing the wrong cause — **treat it as a product defect
and fix it at its root**, not as an obstacle to route around in your own task.

- Do not paper over it with a workaround in your script, a manual step, or a note to
  yourself. If you needed the workaround, so will a user who has no idea it exists.
- Fix the source, not the symptom: the generator rather than the generated file, the
  API rather than the one caller, the instrument that could not express the failure
  rather than the one conclusion it misled you into.
- Instrumentation counts as product. If a failure was hard to diagnose because output
  was truncated, logs went to `/dev/null`, or a check could not fail, that is itself the
  bug worth fixing — the next person debugging it is a user.
- Then leave the comment explaining *why*, so the fix does not get undone by someone who
  only sees the shape of the code.

The bar: after you are done, could someone hit the same problem again? If yes, you fixed
your task, not the platform.

## EVERY BUG FIX IS TWO FIXES (Ethan, 2026-08-11 — standing, no exceptions)

When you fix a bug in amux, you owe **both** of these, and the second is not optional:

1. **Fix it durably, at the root cause.** Not the caller, not the symptom, not your own
   workaround — the thing that generated it. This is the rule above.
2. **Make the bug SURFACE IN THE AMUX LOGS.** After your fix, the *next* instance of that
   bug — or of anything in its class — must be visible in what amux already records, so a
   sweep or an autofix loop can find it without a human noticing first.

The second is what turns a fix into a class kill. A root-cause fix stops one bug; a fix
plus a signal stops the next one, in a lane nobody is watching, on a night nobody is
looking. Autofix cannot act on something that leaves no trace.

**The test, and it is concrete:** *would the bug I just fixed have shown up in
`GET /api/logs/analyze`, `/api/debug/*`, the structured request log, or a job's own
report — WITHOUT me going and looking for it?* If the honest answer is no, you are not
finished. Add the counter, the verdict field, the WARN line, or the trace that makes the
next occurrence self-announcing.

**Worked example, 2026-08-11.** A worker reported the same message delivered twice.
Establishing it had NOT been required grepping a 1.6MB pane log, normalising ANSI, and
deduping redraw captures by hand — because a pipe-pane log re-captures a visible line on
every repaint, so counting occurrences there is meaningless. The store said one row; the
pane said "53 occurrences" and meant one delivery. Nothing in amux could answer "was this
delivered once or twice" directly. THAT is the bug worth fixing, whatever the original
report turns out to be: `send_dedup` and `cmd_history.delivery` exist, and no view joins
them into an answer.

The shape to avoid: fixing the reported thing, and leaving the next occurrence just as
invisible as this one was.

**MANDATORY: log the friction, not just the fix — [`frustrations.md`](frustrations.md).**
**Any issue you experience with amux gets an entry, whether or not you fixed it, and
whether or not it blocked you.** A command that reports success and does nothing, a
notice that names the wrong session, a gate you cannot satisfy honestly, a probe that
cannot express the answer, a peek that hides output, two components disagreeing about
the same fact — append an entry. The format is fixed so it greps
(`grep '^STATUS: open'`, `grep '^AREA: attribution'`), and the full rule for what
counts is [`.claude/rules/frustrations.md`](.claude/rules/frustrations.md).

The entry is owed even when you fixed it in the same breath. You know the fix; the
file is how the *pattern* becomes visible to everyone else, and a fix you made
silently teaches nobody that the subsystem keeps producing that shape. The only
things that do not belong there are your own mistakes with no amux involvement and
one-off environment noise.

This exists because a single frustration is a complaint and a cluster is an argument.
No one entry proves a subsystem needs rebuilding; three entries sharing an `AREA` do,
and that pattern is invisible unless the entries are counted. Link a card on every one —
a frustration without a `CARD:` is something to grumble about, with one it is work
somebody can pick up.

## Structure

Do not resurrect the Python server (see the top of this file) or add anything that
depends on it.

- `crates/amux-server` — the server (axum): API in `src/api/`, SQLite layer in
  `src/db/`, SQL migrations in `migrations/`, orchestration in `src/runtime_jobs/`
- `crates/amux-dashboard` — the SPA, served from `static/` (`index.html`, `app.js`,
  `app.css`, `sw.js`)
- `crates/amux-core` / `crates/amux-cli` — shared types; the Rust CLI (`amux-rs`)
- `amux` — the legacy **bash** CLI the fleet still runs (`amux send/board/alert`);
  it is an HTTP client of the server, not part of it
- `e2e/` — Playwright suites; `crates/amux-server/tests/` — integration tests
  (`boundary_golden.rs` + `tests/fixtures/` are recorded Python-contract goldens —
  the contract memory; they compare against recordings, not a live server)
- `install.sh` — one-command install (build + `~/.local/bin` + launchd agents)
- `mcp.json` — centralized MCP server config (shared by local and cloud)
- `cloud/` — cloud.amux.io: the workspace image (`docker/Dockerfile`, a rust
  multi-stage build of `crates/`), the python auth/orchestration gateway
  (`gateway/`), litestream replication, and the seed + e2e scripts. Shipped by
  `.github/workflows/deploy-cloud.yml`, which **cannot build a python image** —
  a guard job asserts it. Read `cloud/README.md` before changing anything there:
  it names what degrades in the container and the two gateway↔server contract
  gaps (`/api/observability`, `/api/share/<token>/info`) that are still open.

## Claude Code hook entries: matchers are REGEXES, and tool events require one

Writing a hook into `~/.claude/settings.json` has two traps that both produce an entry
that LOOKS right in the file and never runs:

- **Tool events (`PostToolUse`/`PreToolUse`) need a `matcher`; lifecycle events
  (`Stop`/`UserPromptSubmit`) do not.** An entry without one is ignored. Every
  pre-existing tool entry in that file carries a matcher — if yours is the only one
  that does not, that is the tell.
- **The matcher is a REGEX.** `"*"` is not a valid one (`nothing to repeat at
  position 0`); use `".*"`. Existing entries look like `"Write|Edit|MultiEdit"` and
  `"Bash"`.

Both were shipped in sequence on AMUX-2538 and both were inert, across three
verification runs, while the JSON read as correct each time.

**Verify a hook by what it WROTE, not by the settings file.** The heartbeat there was
caught only because the report records WHICH source last wrote it, so `tool-hook`
showing zero across 79 samples was visible; a status field alone would have looked
correct throughout. If a hook is not firing, have it write a marker file as well as
its real action — that distinguishes "never ran" from "ran and its command failed or
its env was missing", which testing the endpoint and the settings entry cannot.

## Workflow

- **Staleness announces itself; nothing auto-pulls.** The `SessionStart` hook
  (`.claude/session-freshness.sh`) fetches and reports two things a session cannot see
  on its own: how far this checkout is behind `origin/main` (naming `crates/` /
  `amux` / `CLAUDE.md` when they are in the diff, because those are the conflicts you
  are about to hit), and whether the INSTALLED CLI still matches this checkout. It is
  silent when everything is current.

  **Do not "improve" it into a scheduled pull.** This is a shared checkout, and the
  Deploy section below records a peer's `git pull --rebase` replaying another session's
  unpushed commit onto origin. A background job that rewrites the working tree can
  destroy in-flight work belonging to a session that is not even running. The hook
  reports; the human decides. Set `AMUX_SKIP_FRESHNESS=1` to silence it.

  Both axes are there because both bit on 2026-08-05: the checkout was ~110 commits
  behind (a fix was written that upstream already had), and the installed CLI was a
  Jul-31 copy whose missing verb printed help and exited 0, silently swallowing three
  status requests.
- **Commit after every completed task.** When you finish a piece of work (bug fix, feature, refactor), immediately `git add` the files you changed and `git commit` with a concise message. Don't batch multiple tasks into one commit. Committing is also what DEPLOYS locally (next bullet).
- **Editing the working tree changes nothing that is live — COMMITTED source is what
  ships.** `com.amux.server-rs-builder` (`scripts/rust-auto-build.sh`, every 60s)
  rebuilds when the last commit touching `crates/`/`Cargo.*` moves, installs
  `~/.local/bin/amux-server-rs`, and the running server self-adopts (exits for launchd
  to relaunch). 8822 is retired and no longer bound (see the top of this file) — if
  your `$AMUX_URL` still names it, use `$(amux url)`. To see a change live, commit it,
  then confirm it took:
  ```bash
  # wait for the builder cycle (or force it: bash scripts/rust-auto-build.sh)
  curl -sk https://localhost:8824/health   # `build` hash must move, `server":"amux-rust"`
  curl -sk https://localhost:8824/ | grep -c '<a line unique to your change>'
  ```
  Verify with a string your edit INTRODUCED, not one that already existed — grepping a
  common idiom returns a happy non-zero count against the old build and tells you
  nothing (this cost a wrong "it's live" call on AMUX-4).
- Syntax/type gates after edits: `cargo check --workspace` (and `cargo clippy
  --workspace --all-targets -- -D warnings` before pushing — CI denies warnings).
  Tests: `cargo test -p amux-server`.
- **ONE shared build dir: `CARGO_TARGET_DIR=~/.amux/rust-build-target`. Never a
  per-session scratch dir.** This line used to say the opposite ("use a scratch
  `CARGO_TARGET_DIR` so parallel sessions don't thrash one lock") and that
  instruction filled the disk: a debug target tree is 10-15GB, ~37 of them
  accumulated under `/private/tmp/amux-*target`, and on 2026-08-10 the volume hit
  741MB free with a 50-session fleet running and writes failing with ENOSPC.
  The lock it was avoiding costs almost nothing — measured on this machine, an
  incremental rebuild is 1.48s alone and two concurrent ones finish in 1.65s
  (1.11x), because cargo's build lock makes the second builder WAIT and then find
  the work already done. Every session builds the same checkout, so a shared
  target dir is a warm cache they hand each other, not a conflict: per-session
  dirs paid 15GB *and* a full rebuild each to avoid a second of waiting.
  Set it once in your shell rather than per command:
  ```bash
  export CARGO_TARGET_DIR=~/.amux/rust-build-target
  ```
  If you find a stale `/private/tmp/*target` dir with no live build in it
  (`lsof +D <dir>` empty and mtime hours old), delete it — it is somebody's
  abandoned 15GB.
- **Bracket any timing/availability measurement with `/health`'s `build`.** The builder
  swaps the running binary whenever ANYONE's commit lands — on this shared checkout that
  is routinely not you, with your own tree clean and nothing in your session hinting the
  binary changed underneath you. A restart's symptoms (timeouts, HTTP 000, wild
  latencies) are indistinguishable from the thing you are measuring being slow, so the
  wrong conclusion arrives already corroborated by repeat runs. Read `build` before and
  after and assert it did not move:
  ```bash
  B0=$(curl -sk $AMUX_URL/health | python3 -c 'import json,sys;print(json.load(sys.stdin)["build"])')
  # ... take the measurement ...
  B1=$(curl -sk $AMUX_URL/health | python3 -c 'import json,sys;print(json.load(sys.stdin)["build"])')
  [ "$B0" = "$B1" ] || echo "INVALID: build moved $B0 -> $B1 — you measured two different servers"
  ```
  `build` is a content hash of the running binary, so it discriminates a code change, not
  merely a bounce (`pid`/`uptime_s` catch the bounce). On 2026-08-08 this cost a session
  a published wrong conclusion: a filtered-board hang (AMUX-2562) was measured, blamed on
  a restart, and "disproved" by a re-measurement that had silently run against the FIXED
  build. The instrument was already there and nobody was routed to it.
- **For a pre-fix specimen, use `<your-sha>^` — NEVER `HEAD~1`.** Every fix here is
  supposed to be checked against the code it fixed (ethos rule 7), so this recipe gets
  reached for constantly, and on a shared checkout it is wrong the moment another lane
  commits between your commit and your check:
  ```bash
  git show "$(git log -1 --format=%H --author-date-order --grep='<your subject>')^:<path/to/file>"
  git show 523df63^:crates/amux-server/src/lib.rs   # or just: the parent of YOUR sha, read off git log
  ```
  On 2026-08-09 `HEAD~1` was another lane's commit landed seconds earlier, so the "pre-fix"
  specimen WAS the fix. The probe reported the new behaviour already present and concluded
  the regression test was vacuous — a false verdict whose natural remedy is to rewrite a
  correctly-discriminating test into a worse one. This is the loud-wrong probe, not the
  silent one: it answers, and its answer looks exactly like the failure rule 7 warns about,
  so it corroborates itself. The tell is cheap — `git log -3` and check whose sha is where.
- **Client JS changes need `APP_VER` (`crates/amux-dashboard/static/app.js`) and the
  `CACHE` version (`crates/amux-dashboard/static/sw.js`) bumped together**, or a
  browser holding the cached script never receives the fix.

## Observability — ask the server, do not grep for it

Every `/api` request is recorded in a structured request log, and the server will
answer diagnostic questions directly. **Reach for these before writing a grep.** The
lesson is this repo's own, from 2026-08-09: diagnosing a 405 by hand — grepping
`mod.rs` and every module to work out which methods were mounted where — cost roughly
10x what one call to `/api/logs/analyze` cost, because the endpoint had already
computed the answer and stated the verdict in a sentence.

| Endpoint | Answers |
|---|---|
| `GET /api/logs/analyze?since_h=24` | Every error (status ≥ 400) grouped by (status, method, family, normalized target). For 404/405 it annotates `routed_methods` and `nearest_routes` and writes a plain `verdicts` sentence per group — including the two cells people get wrong: a 405 at a path with NO route is the GET-only SPA catch-all answering a non-GET, and a 405 whose method IS routed means the rows predate the current build (re-run before filing). |
| `GET /api/logs/stats?since_h=24` | Per-family traffic/latency/error rollup, `slow_outliers`, and `percentile_method` named in the response so nobody has to guess how p95 was computed. |
| `GET /api/debug/routes` | The whole route table as JSON — "is X routed, with which methods" is a GET, not a grep across modules. |
| `GET /api/debug/boundary` | Ownership per API family — of the families it TRACKS (62 today). `proxied: []` means nothing hops to Python, which since 792ce1f deleted the Python server is true **by construction**: it cannot report incompleteness, so it is not proof the server answers everything. Measured 2026-08-11: 68 paths the SPA calls are absent from it and 404 live (`/api/sql`, `/api/proxies`, `/api/mcp`, `/api/reports`, `/api/terminal`, …). For "does every caller have a route", use the invariant below. |
| `GET /api/health/invariants` | The check that CAN fail on the above: `route.callers_have_routes` enumerates SPA/CLI call sites against the mounted table and names each miss (404 vs "route exists but allows only \[POST\]"). Read `failures` and match on `invariant_id` — there is no `results` key, and rows carry `invariant_id`, not `id`. |
| `GET /api/debug/tmux` | Runs fleet discovery from INSIDE the server process and reports argv, exit status, output sizes, and the env that picks the socket. It exists because the launchd instance once served `running=0` for the whole fleet while the same binary in a login shell served 49, and no log line could say why. |

Raw server tracing is at `~/.amux/logs/server-rs.log`; the dashboard's Logs tab is the
same request log with a UI. If a failure was hard to diagnose because no endpoint could
express it, that missing instrument is the bug worth fixing (ethos rule 4).

## Deploy

⚠ **BEFORE `git push origin main`: check what you are shipping that is not yours.** This is a
SHARED checkout — other sessions commit here — and CI (`rust.yml`) plus the site deploy
(`pages.yml`) run against whatever a push carries. **Any push of main ships EVERY unpushed
commit, including other sessions' work that nobody reviewed at that moment** — and locally the
builder deploys every COMMIT the moment it lands, pushed or not. Always run first:

```bash
git fetch origin                                # refresh — the recipe is wrong without this
git rev-list --count origin/main..main          # how many commits am I about to ship?
git log --format="%h [%(trailers:key=Amux-Session,valueonly,separator=)] %s" origin/main..main
# ↑ "whose are they?" — %an is shared by every session on this machine, so --oneline
#   cannot discriminate. The Amux-Session trailer, stamped by prepare-commit-msg, is
#   the real discriminator. [] = untrailered commit, treat as foreign.
```

A commit already upstream under a **different SHA** (cherry-pick, rebase, replay) sits in
`origin/main..main` permanently. Before asking a peer about a seemingly foreign commit,
check whether it shares a patch-id with something already on origin:

```bash
# Compare patch-ids: if upstream has the same patch, the local commit is a duplicate, not foreign
git log --format="%H" origin/main..main | git patch-id --stable | while read pid sha; do
  if git log --format="%H" origin/main | git patch-id --stable | grep -q "^$pid "; then
    echo "DUPLICATE (already upstream): $sha"
  fi
done
```

If commits you did not write are listed **and are not upstream duplicates**, ask their author
before pushing. "My change is small" is not the question; the question is what rides along
with it.

**The mirror case is just as real: "my commits are not pushed" does NOT mean they are inert.** On a
shared checkout they are staged to ship under someone else's push, at a time you do not choose. A
session's unpushed commit was replayed onto origin by a peer's `git pull --rebase` on 2026-08-03 —
a third party's push shipped a commit its author never pushed. You control neither *when* your work
ships nor *whether* it does.

**And a peer's commit can silently REVERT your uncommitted work.** On 2026-08-09 a peer's
`git commit -a`-shaped commit swept another agent's staged file *deletions* into HEAD while
several files that agent had rewritten in the working tree reverted to their old content — the
rewrite of THIS file was lost that way and had to be redone. Re-read a shared file immediately
before you edit it, and after a peer commits, `git diff HEAD` the files you were mid-way through:
your edit is not safe merely because you made it.

When the user says **"deploy"**, run the full pipeline:
1. `git add` changed files
2. `git commit` with a concise message (the local builder adopts it within ~60s)
3. Run the two checks above
4. `git push origin main`

## Single-codebase rule (CRITICAL)

**The server code is identical for both local (OSS) and cloud deployments — no exceptions.**

- Never add cloud-only or OSS-only code branches (no `if IS_CLOUD`, no `CLOUD` env build flags).
- Features that differ between environments must be driven by headers/env vars injected by the gateway (e.g., `X-Amux-User-Email`) or by presence/absence of configuration, not by build-time flags.
- The cloud image is the SAME binary built from `crates/`. Everything it needs
  that a laptop does not lives in `cloud/docker/Dockerfile` as env or as a
  binary on PATH — `IS_SANDBOX=1` (Claude Code refuses `--dangerously-skip-permissions`
  as root, and only the deployment knows it is an isolated single-tenant
  container), `AMUX_RS_PORT` (8822 in the cloud image — the container's own
  choice, not the retired local address; the server now derives the `AMUX_URL` it
  injects into lanes from this var rather than a literal), and a chromium shim
  carrying the flags a
  display-less container needs. If you find yourself wanting `if container`, the
  answer is one of those three shapes.
- `cloud/docker/amux-server.py` is a dead artifact of the retired python image.
  Nothing builds or reads it any more; it stays in `.gitignore` so a stray copy
  can never be committed.

## Server config — `~/.amux/server.env`

Persistent env vars for the server. Loaded at startup as setdefault (process-level env always wins) — see `crates/amux-server/src/config.rs`.

**Credential VALUES live here and only here — never in this repo (it is PUBLIC), never in
a board card, never in a prompt. [`docs/credentials.md`](docs/credentials.md) is the
committed inventory of what exists: NAMES and purpose only.** Read it before concluding
you lack a credential and asking a human for one. On 2026-08-06 a session asked Ethan to
create a Clerk account and to decide about handing over an `sk_live_` key while
`E2E_CLERK_SECRET_KEY` and a full `AMUX_QA_*` god-mode login were already in this file —
the same file that session had been reading `E2E_COOKIE_SECRET` out of all day. Grepping
one key never reveals the other 34, so a credential nobody can enumerate is one nobody
has.

Example `~/.amux/server.env`:
```
AMUX_S3_BUCKET=ethan-personal
AMUX_S3_KEY=amux/cal-<random-token>.ics
AMUX_S3_REGION=us-east-2
```

After creating/editing server.env, restart the server to pick it up:
`launchctl kickstart -k gui/$(id -u)/com.amux.server-rs`.

## iCal / Google Calendar sync

The iCal feed exports amux **calendar events** only (`cal_events` table) — NOT
schedules or board issues. Those are in-app-only calendar layers. Timed events are
emitted as UTC (`DTSTART:...Z`) so Google shows the correct local time; all-day use
`VALUE=DATE`.
- Local: `GET /api/calendar.ics`
- Public S3 (for Google/Apple Calendar subscriptions): set `AMUX_S3_BUCKET` in `server.env`

S3 bucket config (on `ethan-personal`):
- Public access block: `BlockPublicAcls=true, IgnorePublicAcls=true, BlockPublicPolicy=false, RestrictPublicBuckets=false`
- Bucket policy grants public `s3:GetObject` on `arn:aws:s3:::ethan-personal/amux/*.ics` (widened from the single `calendar.ics` key so cache-busting keys work). Bucket LISTING is denied (403), so a random key is not discoverable.
- Current key: a **random token** (`amux/cal-<32hex>.ics`) that lives ONLY in `~/.amux/server.env` — **NEVER commit the actual key/URL to this repo: the repo is public, and a committed feed URL is how the old guessable key leaked.** Read it with `grep AMUX_S3_KEY ~/.amux/server.env`; the dashboard's Subscribe button shows the full URL.

**Google caches ICS feeds by URL, hard.** There is no reliable way to force a refresh — Google refetches on its own cadence (hours). If you edit the feed's *content shape* and need Google to see it now, publish to a NEW random key and re-subscribe; the old URL keeps serving Google's stale cache. `AMUX_S3_KEY` is read at startup (setdefault), so changing it needs a real restart: `launchctl kickstart -k gui/$(id -u)/com.amux.server-rs`.

The feed auto-uploads to S3 on every event write. The dashboard's calendar subscription button shows the S3 URL directly when configured.

## Browser Automation

Use `/chrome-cdp` for browser tasks. It connects directly to the user's live Chrome via CDP — real tabs, real cookies, no fresh browser.

```bash
node skills/chrome-cdp/scripts/cdp.mjs list           # list open tabs
node skills/chrome-cdp/scripts/cdp.mjs snap <target>   # accessibility tree
node skills/chrome-cdp/scripts/cdp.mjs shot <target>   # screenshot
node skills/chrome-cdp/scripts/cdp.mjs click <target> <selector>
node skills/chrome-cdp/scripts/cdp.mjs type <target> <text>
node skills/chrome-cdp/scripts/cdp.mjs eval <target> <js>
node skills/chrome-cdp/scripts/cdp.mjs nav <target> <url>
```

Requires Chrome remote debugging enabled (`chrome://inspect/#remote-debugging`) and Node.js 22+.

Claude Code, the amux server, and Chrome all run on the same desktop machine. Use `https://localhost:8824` for amux dashboard URLs (not 8822 — that address is retired).
