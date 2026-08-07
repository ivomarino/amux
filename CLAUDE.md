# amux

Single-file project: everything lives in `amux-server.py` (Python server + inline HTML/CSS/JS dashboard).

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

- `amux-server.py` — the server + dashboard (single file)
- `mcp.json` — centralized MCP server config (shared by local and cloud)
- `cloud/` — GCP VM provisioning (Terraform + setup script)

## Workflow

- **Staleness announces itself; nothing auto-pulls.** The `SessionStart` hook
  (`.claude/session-freshness.sh`) fetches and reports two things a session cannot see
  on its own: how far this checkout is behind `origin/main` (naming `amux-server.py` /
  `amux` / `CLAUDE.md` when they are in the diff, because those are the conflicts you
  are about to hit), and whether the INSTALLED CLI and the RUNNING server still match
  this checkout. It is silent when everything is current.

  **Do not "improve" it into a scheduled pull.** This is a shared checkout, and the
  Deploy section below records a peer's `git pull --rebase` replaying another session's
  unpushed commit onto origin. A background job that rewrites the working tree can
  destroy in-flight work belonging to a session that is not even running. The hook
  reports; the human decides. Set `AMUX_SKIP_FRESHNESS=1` to silence it.

  Both axes are there because both bit on 2026-08-05: the checkout was ~110 commits
  behind (a fix was written that upstream already had), and the installed CLI was a
  Jul-31 copy whose missing verb printed help and exited 0, silently swallowing three
  status requests.
- **Commit after every completed task.** When you finish a piece of work (bug fix, feature, refactor), immediately `git add amux-server.py && git commit` with a concise message. Don't batch multiple tasks into one commit.
- The server auto-restarts on file save (watches its own mtime) — but **it watches the
  file it is RUNNING, which is `~/.local/bin/amux-server.py`, not your repo checkout.**
  Editing `~/amux/amux-server.py` alone changes nothing that is live: `ps` the process
  and you will see it running out of `~/.local/bin`. To see a change on
  `https://localhost:8822`, install it, then confirm it actually took:
  ```bash
  # NOTE: ~/.local/bin/amux-server.py is a SYMLINK to this checkout, so there is
  # nothing to copy — `cp` refuses with "are identical". The label is
  # com.amux.serVER; `com.amux.serve` fails with "Could not find service", which
  # looks like a restart you did not get. Both verified 2026-08-06.
  launchctl kickstart -k gui/$(id -u)/com.amux.server
  curl -sk https://localhost:8822/ | grep -c '<a line unique to your change>'
  ```
  Verify with a string your edit INTRODUCED, not one that already existed — grepping a
  common idiom returns a happy non-zero count against the old build and tells you
  nothing (this cost a wrong "it's live" call on AMUX-4).
- Always verify Python syntax after edits: `python3 -c "import ast; ast.parse(open('amux-server.py').read())"`
- **Client JS changes need `APP_VER` and the sw.js `CACHE` bumped together**, or a
  browser holding the cached script never receives the fix.
- Always verify Python syntax after edits: `python3 -c "import ast; ast.parse(open('amux-server.py').read())"`

## Deploy

⚠ **BEFORE `git push origin main`: check what you are shipping that is not yours.** This is a
SHARED checkout — other sessions commit here — and `.github/workflows/deploy-cloud.yml` triggers on
`push: branches:[main], paths:['amux-server.py']` (as does `cloud-image.yml`). **So any push of
main deploys EVERY unpushed commit touching that file, including other sessions' work that nobody
reviewed at that moment.** Always run first:

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

When the user says **"deploy"**, run the full pipeline:
1. `git add` changed files (typically `amux-server.py`)
2. `git commit` with a concise message
3. Run the two checks above
4. `git push origin main`

## Single-codebase rule (CRITICAL)

**`amux-server.py` is identical for both local (OSS) and cloud deployments — no exceptions.**

- Never add cloud-only or OSS-only code branches (no `if IS_CLOUD`, no `if os.environ.get('CLOUD')`).
- Features that differ between environments must be driven by headers/env vars injected by the gateway (e.g., `X-Amux-User-Email`) or by presence/absence of configuration, not by build-time flags.
- `cloud/docker/amux-server.py` must never be committed — it is auto-generated during deploy. It is in `.gitignore`.

## Server config — `~/.amux/server.env`

Persistent env vars for the server. Loaded at startup via `os.environ.setdefault` so process-level env always wins. Survives `os.execv` auto-restarts.

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

After creating/editing server.env, `touch amux-server.py` to trigger a reload.

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

**Google caches ICS feeds by URL, hard.** There is no reliable way to force a refresh — Google refetches on its own cadence (hours). If you edit the feed's *content shape* and need Google to see it now, publish to a NEW random key and re-subscribe; the old URL keeps serving Google's stale cache. `AMUX_S3_KEY` is read at startup via `os.environ.setdefault`, and execv reloads inherit the env, so changing it needs a real restart: `launchctl kickstart -k gui/$(id -u)/com.amux.serve`.

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

Claude Code, the amux server, and Chrome all run on the same desktop machine. Use `https://localhost:8822` for amux dashboard URLs.
