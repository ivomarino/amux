# Server boundary: what Rust owns, what Python owns (AMUX-2597)

Two servers run on this machine pre-cutover: **Python** (`amux-server.py`, :8822)
owns the live fleet; **Rust** (`crates/amux-server`, :8824) serves the same SPA and
answers natively where it can. This file is the ownership matrix. The code twin is
the registry in `crates/amux-server/src/api/py_proxy.rs` (`PROXIED_FAMILIES` /
`NATIVE_FAMILIES`) — mounts derive from it, `tests/proxy_composition.rs`
cross-checks it against the routes mod.rs actually mounts, and it is served live at
**`GET /api/debug/boundary`**. Every proxied response carries
`x-amux-answered-by: python-proxy`; no header means Rust answered.

Rules:
- A family proxies only via a `PROXIED_FAMILIES` row. No ad-hoc proxy mounts.
- Cutover of a family = delete its row, mount a native router, update this table.
- Both servers share `~/.amux` (SQLite DB, session env files, uploads dir); SHARED-DB
  notes below say which store a native family reads.

## Matrix

| /api family | Owner | Notes |
|---|---|---|
| `/api/board`, `/api/workers`, `/api/sync`, `/api/events` | RUST-NATIVE | shared DB (issues/workers/events) |
| `/api/sessions` (bare list) | RUST-NATIVE | python-SHAPE array derived from env files + tmux + persisted reports |
| `/api/sessions/{name}`, `/api/sessions/{name}/{verb}` | PYTHON-OWNED (proxied) | fleet lifecycle: peek/send/start/stop/config/YOLO rewrite env files + restart live sessions. Exit: AgentRuntime seam (#47/#48). Rust-managed `wrk_` names get a 501 pointer, never a silent proxy |
| `/api/fs/*` | RUST-NATIVE (this change) | SPA Files surface: mkdir/open/upload/rename/read/search/list/delete. Ported guards: `_is_path_allowed` py:93-121, `_is_dangerous_write` py:670-698. Pure filesystem, no DB |
| `/api/ls`, `/api/autocomplete/dir` | RUST-NATIVE (this change) | SPA Files browser listing + dir autocomplete (api/fs.rs). Pure filesystem |
| `/api/file` (+`/raw` `/prepare` `/transcode`), `/api/library` | PYTHON-OWNED (proxied) | file VIEWER + media pipeline: range/keepalive streaming, ffmpeg prepare/transcode with in-process job state (`_MEDIA_PREP_JOBS`), ebook→HTML, image inline caps. Exit: port viewer + raw streaming once media job state is durable |
| `/api/groups`, `/api/tags` | RUST-NATIVE (this change) | group list derived from CC_TAGS env files (trimmed split, blocked sessions excluded, tag-isolation scoping); per-group config in shared-DB `group_config` (schema py:10346). Both spellings, matching python's alias (py:65345) |
| `/api/upload`, `/api/uploads` | RUST-NATIVE | chunked upload protocol + serving `~/.amux/uploads` (api/upload.rs) |
| `/api/identity` | RUST-NATIVE | cloud user + auth-config introspection (X-Amux-User-Email, server.env key/proxy presence, ~/.claude.json oauth) — py:73349. `key_valid`/`key_error` answer null/"" (python's pre-validation state): this server runs no key validator and does not invent one |
| `/api/browser` | SPLIT | start/status/stop/profiles native; driver verbs (screenshot/action/state/agent/…) proxied — the playwright/model engine runs in the python process. Mount: inside `api::browser::routes()` |
| `/api/dictate`, `/api/dictation/config` | PYTHON-OWNED (proxied) | whisper/Gemini engines load in the python process; `/config` must describe the engine `/api/dictate` uses. Mount: inside `api::dictation::routes()`. History/dict CRUD (`/api/dictation/*`) is native |
| everything else mounted in `api/mod.rs` (memories, messages, schedules, prefs, email, cal-events, branding, skills, map, journal, history, settings, push, torrents, org, gmail, metrics, usage, alerts, stats, debug, health, calendar.ics) | RUST-NATIVE | see `NATIVE_FAMILIES` notes per family |

## Contract subtleties worth knowing (all fixture- or live-verified 2026-08-09)

- **/api/fs wrong-method = 404, not 405.** Python routes on (method, path) pairs and
  falls through to its generic `{"error": "not found"}`. The native port preserves it.
- **/api/fs/read "binary" = "not valid UTF-8"**, nothing else: NUL bytes come back as
  text; a valid UTF-8 file truncated mid-codepoint by `max_bytes` comes back base64.
- **/api/fs/upload never clobbers by default** (`name_1.ext` suffixing); multipart
  `overwrite=1` opts in. Dangerous names (launch agents, shell rc, `.plist`…) get a
  per-file `{"error": "refused: could execute code"}` entry, not a request failure.
- **/api/ls vs /api/fs/list are different contracts**: ls filters dotfiles unless
  `hidden=1`, OMITS stat-failing entries (fs/list reports `"unreadable"`), dirs carry
  `size: null`, missing path is 400 "not a directory", no 1000-entry cap.
- **/api/fs/search returns 400 only for a missing query**; a bad root is a 200 with
  `error: "access denied or not a directory"`. Zero-result responses carry a `note`
  naming every filter that could be responsible. The rust port also honors
  `AMUX_SEARCH_RG`, which Python's error text promises but its code never read
  (py:21056 — implemented rather than inherited as a false claim).
- **group config PATCH: absent key RESETS the column** ("" / `[]` / 0) — partial
  updates must resend the full object. The upsert's COALESCE arms are DEAD CODE on
  both origins: an explicit JSON null becomes SQL NULL, which fails the NOT NULL
  constraints *before* conflict resolution → 500. Verified against Python's exact
  schema+SQL in sqlite, not assumed from reading the code.
- **Tag isolation on /api/groups**: an `X-Amux-Worker`/`X-Amux-Session` caller sees
  same-tag groups only; unknown or untagged callers see only themselves (usually an
  empty list). No header (dashboard) = unscoped.
- The worker page's Files-tab search is `/api/fs/search` with the worker's CC_DIR as
  `path` — one engine for both surfaces (AMUX-2420). The workers-LIST search box is
  client-side; it consumes no endpoint.

## Verification

- Golden: `tests/boundary_golden.rs` replays recorded live-Python responses
  (`tests/fixtures/boundary/live_recorded.json`, GET-only capture) against the native
  handlers — hermetic, runs in CI.
- Live oracle: `tests/boundary_live_oracle.rs` (`--ignored`) runs identical GETs
  against :8822 and the native router; 2026-08-09 run: **29 paths agreed, 0 diffs**
  (fs list/read/search, ls, autocomplete, groups both spellings, all 14 live group
  configs, 3 scoped callers), `/health build` bracket held.
- Composition: `tests/proxy_composition.rs` pins proxied families to the proxy
  (honest 502 on dead python), native families to no-proxy-stamp, and the registry
  to the mounts.
