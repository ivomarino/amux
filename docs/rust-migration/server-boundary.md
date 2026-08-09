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
| `/api/sessions/{name}`, `/api/sessions/{name}/{verb}` | RUST-NATIVE (AMUX-2598) | full per-name verb family (peek/send/config/start/stop/resize/duplicate/clone/steer/share/tracked-files/git/log/transcript(s)/tasks/stats/memory/report/commit-report/…) answered from the fleet substrate: `~/.amux/sessions/*.env` + `*.meta.json`, `~/.amux/logs`, tmux `amux-<n>` via the L2 target helpers, herdr CLI, shared-DB steering_queue/steering_history/share_tokens/session_events/cmd_history/send_dedup/prefs. Config PATCH ports the restart choreography (provider/model/effort/yolo swap → conv-id stash + stop + start; dir change → hard restart). Rust-managed `wrk_` names still get the 501 pointer. Named gaps in `api/session_verbs.rs` module doc: no autotask labelling, no _verify_submitted JSONL gate, no boot board-digest, no commit-hook/trust/memory-compose side effects, no commit-report sweep notice, env-explain/memory-explain = 501, iTerm2 = 501 |
| `/api/fs/*` | RUST-NATIVE (this change) | SPA Files surface: mkdir/open/upload/rename/read/search/list/delete. Ported guards: `_is_path_allowed` py:93-121, `_is_dangerous_write` py:670-698. Pure filesystem, no DB |
| `/api/ls`, `/api/autocomplete/dir` | RUST-NATIVE (this change) | SPA Files browser listing + dir autocomplete (api/fs.rs). Pure filesystem |
| `/api/file` (+`/raw` `/vtt` `/prepare` `/transcode`), `/api/library` | RUST-NATIVE (AMUX-2598) | file VIEWER + media pipeline (`api/file_viewer.rs`): viewer payload (image inline-vs-stream cap `AMUX_IMG_INLINE_MAX`, pdf/video/audio cards, binary sniff, text/csv char truncation), PUT write-back, SRT→VTT, raw **Range** streaming (206/Content-Range/ETag-304; keep-alive is hyper's default — the semantic python hand-rolled in `_media_keepalive`), ffmpeg prepare/transcode with **durable job state** in shared-DB `_amux_media_jobs` (migration 0009; python's in-process `_MEDIA_PREP_JOBS` orphaned jobs on restart — a stale-heartbeat 'running' row or a 'done' row with a pruned file now restarts honestly). Cache key = python's exact sha1 derivation, so ~/.amux/media-cache survives cutover. ffmpeg/ffprobe found by ABSOLUTE path first (launchd has no shell PATH). **Deviation:** ebook→HTML (EPUB/FB2/CBZ/MOBI/AZW render) answers an honest **501** naming the missing capability (python-stdlib zip/XML/PalmDOC decoding has no rust port); non-renderable/oversize ebooks get python's download card. `/api/library` is fully native (calibre metadata.db read-only+immutable, opf sidecar scan via regex-grade extraction) |
| `/api/groups`, `/api/tags` | RUST-NATIVE (this change) | group list derived from CC_TAGS env files (trimmed split, blocked sessions excluded, tag-isolation scoping); per-group config in shared-DB `group_config` (schema py:10346). Both spellings, matching python's alias (py:65345) |
| `/api/upload`, `/api/uploads` | RUST-NATIVE | chunked upload protocol + serving `~/.amux/uploads` (api/upload.rs) |
| `/api/identity` | RUST-NATIVE | cloud user + auth-config introspection (X-Amux-User-Email, server.env key/proxy presence, ~/.claude.json oauth) — py:73349. `key_valid`/`key_error` answer null/"" (python's pre-validation state): this server runs no key validator and does not invent one |
| `/api/browser` | RUST-NATIVE (AMUX-2598) | Full family: launch/status/stop/profiles + driver verbs (navigate/screenshot(+`/screenshot/file` serving the bytes)/state/action/inspect/search/sessions/pw-profiles/save-profile/DELETE profile) over a native CDP **websocket** client (`integrations/browser.rs`) against the Chrome the server spawned. **Invariant (owner directive): browser automation always executes on the server machine; the dashboard is a remote viewer** — the CDP client refuses non-loopback endpoints, and screenshots are served through the API for remote clients (Python returns only a server filesystem `path`; its dashboard bridges via `/api/file/raw` — the native response carries both `path` and `serve`). `/agent` (Python's server-side model loop) answers an honest **501**: the session's own model drives the native verbs instead (ethos D1/D3). Deviation: `DELETE /profile/{name}` refuses dirs outside `playwright-auth/` (Python would rmtree inside the real Chrome user-data-dir) |
| `/api/dictate`, `/api/dictation/*` | RUST-NATIVE (AMUX-2598) | full dictation family, engine included: warm openai-whisper worker subprocess (same inline worker script as python, `~/.cache/whisper/<AMUX_WHISPER_MODEL>.pt` presence-detected, interpreter found by ABSOLUTE path — launchd has no shell PATH), Gemini `generateContent` fallback + AI-edit (`AMUX_DICTATION_MODEL`, BYO pref `dictation_gemini_key` beats `GOOGLE_API_KEY`), and the deterministic session-name recovery pass with a difflib-exact `SequenceMatcher.ratio` port. `/config` GET is byte-compatible with python's `json.dumps` output. No engine present = python's honest 503 naming what to install |
| everything else mounted in `api/mod.rs` (memories, messages, schedules, prefs, email, cal-events, branding, skills, map, journal, history, settings, push, torrents, org, gmail, metrics, usage, alerts, stats, debug, health, calendar.ics) | RUST-NATIVE | see `NATIVE_FAMILIES` notes per family |

## Contract subtleties worth knowing (all fixture- or live-verified 2026-08-09)

- **Session verbs, live-oracle verified (AMUX-2598)**: 16/17 read verbs match Python's
  top-level key set + value types exactly against a live fleet session
  (peek, peek?live=1, stats, meta, info, git?detail=1, tasks, instructions,
  tracked-files, commit-guard, dirty, log/info, transcripts, memory, steer,
  steer?history=1). The 17th — bare `GET /api/sessions/{name}` — serves "the SAME
  record the list endpoint serves" (python contract, py:74892), so it inherits the
  pre-existing `/api/sessions` LIST projection's known deltas (rust-extra
  managed_by/model/steering_queue; task_time/tokens/worktree typed differently) —
  fix those in sessions_legacy's projection, not per-verb.
- **Rename is convergent, not one-shot** (owner addendum): rename-to-self is a no-op
  (rev unmoved); a retry after a partial failure is admitted addressing the OLD name
  and completes the remainder; journaled to session_events
  (`session.rename.started`/`session.renamed` with per-step outcomes); migrates
  share_tokens, cmd_history and the prefs session_reports key that Python orphans.

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
- **/api/file/raw Range parsing is python's, not the RFC's**: `bytes=(\d*)-(\d*)`
  anchored at start, absent groups default to 0/EOF — so a suffix range
  (`bytes=-500`) or an unparsable Range answers 206 over the whole file, exactly as
  python does. One unavoidable divergence: start-past-end produces
  `Content-Length: 0` natively where python emits a negative Content-Length (hyper
  cannot express that); only malformed clients ever see it.
- **A 'done' media-prep job whose cached file was pruned restarts instead of
  answering ready** — durable rows outlive the files they point at, which python's
  in-memory dict never had to consider. Same for 'running' rows whose heartbeat
  (`updated_at`, ticked on every ffmpeg progress line) goes >60s stale.

## Verification

- Golden: `tests/boundary_golden.rs` replays recorded live-Python responses
  (`tests/fixtures/boundary/live_recorded.json`, GET-only capture) against the native
  handlers — hermetic, runs in CI.
- Live oracle: `tests/boundary_live_oracle.rs` (`--ignored`) runs identical GETs
  against :8822 and the native router; 2026-08-09 run: **29 paths agreed, 0 diffs**
  (fs list/read/search, ls, autocomplete, groups both spellings, all 14 live group
  configs, 3 scoped callers), `/health build` bracket held.
- File-viewer live oracle: `tests/file_viewer_live_oracle.rs` (`--ignored`,
  GET-only) diffs /api/file (9 payload kinds + 5 error shapes), /api/file/raw
  (full + bounded range on a text fixture, 3 range shapes on a real media file
  under ~/Dev with exact 206 header comparison), /api/file/vtt and /api/library
  (opf fixture, empty dir, 2 error shapes); 2026-08-09 run: **24 request pairs
  agreed, 0 diffs**, `/health build` bracket held. prepare/transcode are
  excluded on purpose (they spawn ffmpeg work on the live server — not
  read-only); their behavior is pinned by hermetic lavfi-fixture tests in
  `api/file_viewer.rs` instead.
- Composition: `tests/proxy_composition.rs` pins proxied families to the proxy
  (honest 502 on dead python), native families to no-proxy-stamp, and the registry
  to the mounts.
