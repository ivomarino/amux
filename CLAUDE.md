# amux

- Whenever you fix a bug: 1) fix it at the root cause, 2) make it surface in amux logs so a sweep would catch it.

Rust workspace (`crates/amux-core`, `amux-server`, `amux-cli`, `amux-dashboard`)
serving a static SPA on **port 8824**. Use `$AMUX_URL` or `$(amux url)`.

8822 is retired. `tests/legacy_port_guard.rs` fails the build if it reappears.
If your `$AMUX_URL` still says 8822, use `$(amux url)` (reads `~/.amux/endpoint.json`).

The Python server was deleted at `792ce1f`. Do not resurrect it.

## Ethos

Gut-check every feature against `.claude/rules/ethos.md` (8 rules). The core question:
when the next model is better, does this feature get better with it, or become the ceiling?

## Primitives (do not reinvent)

board, workers, schedulers, filesystem, groups, memories, environment, messages.
If a request decomposes into these, the work is configuration and UX. Do not add a
ninth thing that re-expresses them.

## Dogfooding + two-fix rule

You run inside amux. Fix rough edges at the root, not with workarounds. Every bug fix
owes two things: the fix, and a log signal so the next instance self-announces
(counter, WARN line, verdict field). Log friction to `frustrations.md` (format in
`.claude/rules/frustrations.md`).

## Structure

- `crates/amux-server` -- axum server: `src/api/`, `src/db/`, `migrations/`, `src/runtime_jobs/`
- `crates/amux-dashboard` -- SPA: `static/` (`index.html`, `app.js`, `app.css`, `sw.js`)
- `crates/amux-core` / `crates/amux-cli` -- shared types; Rust CLI
- `amux` -- bash CLI. This file IS the fleet's CLI: `~/.local/bin/amux` is a
  symlink pointing HERE, not the other way round. Live on save, not on commit —
  and so also live on `git checkout`, `stash`, or a branch switch, which swap it
  for all lanes with no save involved.
- `e2e/` -- Playwright; `crates/amux-server/tests/` -- integration tests
- `cloud/` -- cloud.amux.io (read `cloud/README.md` first)

## Hooks

Tool-event hooks need a `matcher` (regex). `"*"` is invalid; use `".*"`.
Verify a hook by what it WROTE, not by the settings file.

## Workflow

- **No auto-pull.** Shared checkout; the freshness hook reports staleness, the human decides.
- **Commit after every completed task.** Committing deploys locally (builder adopts within ~60s).
- **Bash CLI ships on SAVE** (symlink). `check-and-commit.sh` runs `bash -n` on every save.
  Server ships on COMMIT via the auto-builder.
- **`CARGO_TARGET_DIR=~/.amux/rust-build-target`** -- one shared build dir, never per-session.
- **Bracket measurements with `/health`'s `build`** -- the builder swaps the binary on any commit.
  Use `commit` for "did source change?", `build` for "same process image?".
- **Pre-fix specimen: `<your-sha>^`**, never `HEAD~1` (shared checkout).
- **Client JS: bump `APP_VER` (app.js) + `CACHE` (sw.js) together.**
- Syntax gates: `cargo check --workspace`. Before push: `cargo clippy --workspace --all-targets -- -D warnings`.
  Tests: `cargo test -p amux-server`.

## Observability

Use the server's diagnostic endpoints before writing a grep:

| Endpoint | Use |
|---|---|
| `GET /api/logs/analyze?since_h=24` | Error groups with verdicts |
| `GET /api/logs/stats?since_h=24` | Traffic/latency rollup |
| `GET /api/debug/routes` | Route table as JSON |
| `GET /api/health/invariants` | Failing invariants (passing ones only visible in `/api/debug/invariants`) |
| `GET /api/debug/sse?since_h=24` | Is the realtime backbone carrying the fleet, or has it dropped clients onto polling? `live_connections` + `opened_total` (per-PROCESS: the builder restarts this binary on every commit and all SSE connections die with it) joined with `stale_reconnects`, the client-side beacon fired at the 18s zombie trigger. Neither half answers alone — from the server a reconnect looks like a laptop lid; only the client knows it declared the stream stale. A 0 shortly after a deploy is a ramp-up, not a verdict; `live_connections` is the discriminator. |
| `GET /api/debug/tmux` | Fleet discovery from inside the server |

Raw logs: `~/.amux/logs/server-rs.log`

## Deploy

**Before `git push origin main`:**
```bash
git fetch origin
git rev-list --count origin/main..main
git log --format="%h [%(trailers:key=Amux-Session,valueonly,separator=)] %s" origin/main..main
```
If foreign commits exist, ask their author before pushing.

When user says "deploy": `git add` + `git commit` + verify above + `git push origin main`.

A fix in `git log` is not live until `/health`'s `commit` matches.

## Single-codebase rule

Server code is identical for local and cloud. No `if IS_CLOUD` branches.
Differences driven by env vars/headers from the gateway, not build flags.

## Server config

`~/.amux/server.env` -- persistent env vars, loaded at startup as setdefault.
Credential VALUES live here only (repo is public). Inventory: `docs/credentials.md`.
After editing: `launchctl kickstart -k gui/$(id -u)/com.amux.server-rs`.

## iCal sync

Events only (not schedules/board). `GET /api/calendar.ics` locally. S3 key is random
and lives only in `server.env`. Never commit the actual URL (repo is public).

## Browser Automation

Use `/chrome-cdp`: `node skills/chrome-cdp/scripts/cdp.mjs <list|snap|shot|click|type|eval|nav> <target>`.

## Encrypted Secrets Management

**Status:** ✅ Fully operational with age (X25519) encryption.

Central encrypted secrets store for passwords, API keys, OAuth credentials, and webhooks. All workers and handlers can access decrypted secrets via AppState or environment variables.

**Architecture:**
1. **At rest:** `secrets/amux-secrets.yaml` encrypted with age (X25519)
2. **At startup:** Server decrypts once, caches in-memory via `Arc<Mutex>`
3. **In memory:** `SecretStore` provides dot-separated path lookup
4. **Via API:** `/api/secrets/*` endpoints (GET, POST)
5. **Legacy code:** Secrets loaded into environment variables (`EXTERNAL_*`, etc.)

**Setup (one-time):**
```bash
# Generate age key (if not exists)
mkdir -p ~/.config/sops/age
age-keygen -o ~/.config/sops/age/keys.txt

# Encrypt initial secrets with age
age -r <public-key-from-above> secrets/amux-secrets.yaml.plaintext > secrets/amux-secrets.yaml

# Key is NOT committed; only encrypted .yaml file is
echo "secrets/amux-secrets.yaml" >> .gitignore
```

**API Endpoints:**
```bash
# List all secret paths (no values)
curl -sk https://localhost:8824/api/secrets

# Get specific secret
curl -sk https://localhost:8824/api/secrets/external_services.openai.api_key

# Get schema structure (all values redacted)
curl -sk https://localhost:8824/api/secrets/inspect

# Update secret (re-encrypts)
curl -sk -X POST -H 'Content-Type: application/json' \
  -d '{"value":"new-value"}' \
  https://localhost:8824/api/secrets/external_services.openai.api_key
```

**Environment Variables Populated:**
Secrets are automatically loaded into env vars for legacy code:
- `EXTERNAL_SERVICES_OPENAI_API_KEY` = `external_services.openai.api_key`
- `OAUTH_GOOGLE_CLIENT_ID` = `oauth.google.client_id`
- And all others under `external_services`, `oauth`, `databases`, `webhooks`, `api_keys`

**Graceful Degradation:**
If secrets cannot be loaded, server logs a warning and continues (allows operation without credentials for development).

**Security Notes:**
- Age encryption (X25519) at rest
- Age key stored at `~/.config/sops/age/keys.txt` (never committed)
- Decrypted only once at startup
- Cached in memory only
- Environment variable leakage risk — prefer API access for sensitive operations
- API endpoints require authentication (checked against `state.auth_token`)

**Phase 5: Web UI Dashboard** ✅
- Secrets management interface at `/ui/secrets`
- Create, read, update secrets via web UI
- Search and filter functionality
- Modal dialogs for editing
- Dark mode support

**Phase 6: MCP Integration** ✅
- Claude agents can request secrets via MCP
- `REQUEST_SECRET` tool for specific paths
- `LIST_SECRETS` and `INSPECT_SCHEMA` tools
- Secure request logging
- Ready for rate limiting

**Phase 7: GitHub OAuth Connector** ✅
- Full OAuth 2.0 flow with GitHub
- Credentials stored in encrypted secrets
- GitHub issue/PR sync to amux board
- Webhook integration ready
- Setup: See `docs/github-setup.md`

## GitHub Connector (Phase 7)

Integrates GitHub OAuth using the secrets infrastructure.

**Setup:**
```bash
# 1. Create OAuth app at https://github.com/settings/developers/new
# 2. Add credentials to ~/.amux/server.env or secrets store:
EXTERNAL_SERVICES_GITHUB_CLIENT_ID=Ov23li...
EXTERNAL_SERVICES_GITHUB_CLIENT_SECRET=...

# 3. Test connection
curl -sk https://localhost:8824/api/github/status
```

**Endpoints:**
- `GET /api/github/status` — Check connection
- `GET /api/github/auth/start` — Begin OAuth flow
- `GET /api/github/auth/callback` — Handle OAuth redirect

All credentials are encrypted using the secrets infrastructure (Phases 1-4).

## Secret Metadata Store (Phase 3 — Under Development)

Tracks purpose, ownership, and rotation schedule for encrypted secrets.
Endpoints: `GET /api/secrets/manifest`, `GET /api/secrets/{path}/metadata`, 
`POST /api/secrets/{path}/metadata`

Response includes pre-populated Google OAuth metadata with rotation tracking.

### Known Build Issue: Binary Permissions

The auto-builder (`rust-auto-build.sh`) may install the binary without execute
permissions, causing "exec failed — Permission denied" during self-adoption.

**Fix:**
```bash
# Check permissions (should show -rwx------)
ls -l ~/.local/bin/amux-server-rs

# If not executable:
chmod +x ~/.local/bin/amux-server-rs

# Restart server
pkill -9 amux-server-rs
~/.local/bin/amux-server-rs &

# Verify (server listens on 8824)
curl -sk https://localhost:8824/health
```

**Note:** Port 8824 is the canonical server port (8822 is retired).
The `endpoint.json` self-heal handles port discovery for scripts using `$(amux url)`.
