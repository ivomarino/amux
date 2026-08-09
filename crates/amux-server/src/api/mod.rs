//! axum router assembly (RR-0021, Invariant 13).
//!
//! Route groups land phase by phase. Every route added here must also appear
//! in the OpenAPI spec (RR checklist) and — for legacy-compat paths — in the
//! alias registry (RR-0018a).

pub mod alerts;
pub mod branding;
pub mod stats;
pub mod usage;
pub mod aliases;
pub mod auth;
pub mod board;
pub mod criteria;
pub mod browser;
pub mod calendar;
pub mod dictation;
pub mod email;
pub mod file_viewer;
pub mod files;
pub mod fs;
pub mod gmail_auth;
pub mod groups;
pub mod health;
pub mod history;
pub mod journal;
pub mod map;
pub mod memories;
pub mod metrics;
pub mod messages;
pub mod org;
pub mod prefs;
pub mod py_proxy;
pub mod schedules;
pub mod sessions_legacy;
pub mod settings;
pub mod skills;
pub mod sse;
pub mod static_files;
pub mod sync;
pub mod torrents;
pub mod upload;
pub mod verify;
pub mod workers;
pub mod workers_deadletters;

use crate::db::SharedStore;
use axum::Router;
use std::time::Instant;

/// Shared application state for handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: SharedStore,
    pub started: Instant,
    /// Content hash of the running binary, the `build` discriminator that
    /// the Python CLAUDE.md workflow leans on: "did the server actually
    /// change under me?" must be answerable from /health.
    pub build_hash: String,
    /// Bearer token; None disables auth (tests, first-run).
    pub auth_token: Option<String>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/sync", axum::routing::get(sync::delta_sync))
        .route("/api/events", axum::routing::get(sse::events))
        .nest("/api/board", board::routes())
        // Dead-letter routes merge into the workers nest (RR-0068): same
        // /api/workers prefix, second nest at one path is an axum conflict.
        .nest(
            "/api/workers",
            workers::routes().merge(workers_deadletters::routes()),
        )
        .nest("/api/memories", memories::routes())
        .nest("/api/messages", messages::routes())
        .nest("/api/schedules", schedules::routes())
        .nest("/api/verify", verify::routes())
        .nest("/api/prefs", prefs::routes())
        .nest("/api/criteria", criteria::routes())
        .nest("/api/metrics", metrics::routes())
        .nest("/api/usage", usage::routes())
        .nest("/api/alert", alerts::routes())
        .route("/api/stats/daily", axum::routing::get(stats::daily))
        .route("/api/branding", axum::routing::get(branding::get_branding)
            .post(branding::post_branding).delete(branding::delete_branding))
        // base64 icons: the handler's own 5MB check must answer (Python's
        // 400), not axum's 2MB default 413.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .route("/api/branding/asset/{fname}", axum::routing::get(branding::serve_asset))
        .nest("/api/email", email::routes())
        .nest("/api/cal-events", calendar::routes())
        // Legacy SHAPE (not just path): the SPA renders this array (RR-0075).
        .route("/api/sessions", axum::routing::get(sessions_legacy::list_sessions_legacy))
        // Identity is config-introspection over server.env + ~/.claude.json
        // this server already reads — NATIVE, not a fleet capability
        // (AMUX-2597 addendum: the SPA's _initIdentity boot call 404'd here).
        .route("/api/identity", axum::routing::get(identity))
        // EVERY python-proxied mount below derives from py_proxy's
        // PROXIED_FAMILIES table (AMUX-2597: the rust/python boundary is a
        // registry, not scattered mounts). Session verbs ride this merge;
        // remaining Module rows carry their declared in-module hops.
        // Enumerable at GET /api/debug/boundary; matrix:
        // docs/rust-migration/server-boundary.md.
        .merge(py_proxy::family_routes())
        .nest("/api/browser", browser::routes())
        // File VIEWER family — NATIVE (AMUX-2598): payload + raw range
        // streaming + vtt + ffmpeg prepare/transcode with durable job state
        // (api/file_viewer.rs; was PROXIED_FAMILIES' /api/file namespace row).
        .nest("/api/file", file_viewer::routes())
        // Ebook library index (calibre metadata.db / opf scan) — top-level
        // path in python, so top-level here.
        .route("/api/library", axum::routing::any(file_viewer::library))
        .nest("/api/files", files::routes())
        // /api/fs/* is the SPA's Files contract (multipart upload + dir
        // field, open/mkdir/rename/read/list/search/delete on ABSOLUTE
        // paths) — a different contract from /api/files above. NATIVE port
        // of the python handlers (api/fs.rs), same wire shapes.
        .nest("/api/fs", fs::routes())
        // The SPA Files BROWSER's listing + dir autocomplete — native, same
        // module (top-level paths in python, so top-level here).
        .route("/api/ls", axum::routing::any(fs::ls))
        .route("/api/autocomplete/dir", axum::routing::any(fs::autocomplete_dir))
        // Chunked upload: /api/upload/start, /api/upload/:id/chunk/:n,
        // /api/upload/:id/finish — native Rust (no Python proxy).
        .nest("/api/upload", upload::routes())
        .nest("/api/uploads", upload::serve_routes())
        // Groups/tags (AMUX-2594/AMUX-2597): NATIVE — the list derives from
        // CC_TAGS env files, config from the shared group_config table
        // (api/groups.rs). Both spellings, matching python's alias.
        .nest("/api/groups", groups::routes())
        .nest("/api/tags", groups::tags_routes())
        .nest("/api/journal", journal::routes())
        // Skills / slash-commands / map: the SPA tabs' data (AMUX-2586 #6).
        .nest("/api/skills", skills::routes())
        .nest("/api/slash-commands", skills::slash_routes())
        .nest("/api/map", map::routes())
        .nest("/api/history", history::routes())
        .nest("/api/settings", settings::routes())
        .nest("/api/push", crate::push::routes())
        .nest("/api/dictation", dictation::routes())
        // Transcription lives at the TOP-LEVEL /api/dictate (python parity);
        // the dictation module owns it and answers NATIVELY (AMUX-2598:
        // whisper worker + gemini fallback in this process). Body limit off:
        // audio runs to 25MB raw / ~33MB base64, and the handler's own 413
        // must answer, not axum's 2MB default.
        .route(
            "/api/dictate",
            axum::routing::post(dictation::dictate)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .nest("/api/torrents", torrents::routes())
        .nest("/api/org", org::routes())
        // Absolute-path routes (merged, not nested): the gmail callback
        // below is public, and a nest wildcard at /api/gmail would shadow it.
        .merge(gmail_auth::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    let app = Router::new()
        // Public: the PWA shell + health must load before auth happens.
        .route("/health", axum::routing::get(health::health))
        .route("/api/debug/tmux", axum::routing::get(health::debug_tmux))
        // The rust/python ownership registry, readable where a debugging
        // session already looks (ethos rule 4): which families answer
        // natively, which proxy to python and why, and the cutover exit for
        // each. Route names only — nothing secret, so public like its
        // debug sibling above.
        .route("/api/debug/boundary", axum::routing::get(py_proxy::boundary))
        // Public: calendar fetchers (Google/Apple) cannot send bearer tokens.
        .route("/api/calendar.ics", axum::routing::get(calendar::ics_feed))
        // Dynamic manifest: PWA name/color follow the branding prefs (the
        // static file is the fallback inside the handler).
        .route("/manifest.json", axum::routing::get(branding::manifest))
        // Public: Google's OAuth redirect carries no bearer token. Python
        // admits it via the localhost auth bypass; require_bearer has no
        // such bypass, so the callback must sit outside it (single-use
        // server-minted state is the guard).
        .merge(gmail_auth::callback_routes())
        .merge(static_files::routes())
        .merge(protected)
        .with_state(state);

    // Legacy route aliases (RR-0018a): /api/sessions/* rewrites to
    // /api/workers/* BEFORE routing, so the rewrite must wrap the finished
    // router. Auth is inside the wrapper — legacy paths are exactly as
    // protected as canonical ones.
    aliases::alias_layer(app)
}

// ---------------------------------------------------------------------------
// GET /api/identity — cloud user info + auth-config introspection, NATIVE
// (python source: amux-server.py:73349-73385; placeholder set py:77268-77283).
// The SPA's `_initIdentity` calls this at boot (app.js:568); it 404'd on the
// rust origin. Lives here rather than a module because it is a single
// handler over config this server already loads.
// ---------------------------------------------------------------------------

/// Python's `_PLACEHOLDER_API_KEYS` (py:77268), verbatim: obvious
/// template/dummy values that must not count as "a key is configured".
const PLACEHOLDER_API_KEYS: &[&str] = &[
    "changeme", "change-me", "change_me",
    "your-api-key", "your_api_key", "your-key", "yourkey",
    "your_key_here", "your-key-here", "your-api-key-here",
    "placeholder", "dummy", "example", "sample",
    "test", "test-key", "testkey",
    "replace-me", "replace_me",
    "xxx", "xxxx",
    "sk-ant-xxx", "sk-ant-your-key", "sk-ant-example", "sk-ant-placeholder",
];

/// Python truthiness for the `oauthAccount` value in ~/.claude.json
/// (`bool(json.loads(...).get("oauthAccount"))`): {} and "" are falsy.
fn py_truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

async fn identity(headers: axum::http::HeaderMap) -> axum::Json<serde_json::Value> {
    let email = headers
        .get("x-amux-user-email")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // has_api_key counts ONLY server.env (python's comment: never
    // Docker-injected process env), and only non-placeholder values.
    let home = std::env::var("AMUX_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
    });
    let mut has_key_in_env = false;
    let mut base = String::new();
    let mut tok = String::new();
    if let Ok(env_text) = std::fs::read_to_string(home.join("server.env")) {
        for l in env_text.lines() {
            let l = l.trim();
            if let Some(val) = l.strip_prefix("ANTHROPIC_API_KEY=") {
                let v = val.trim().trim_matches('"').trim_matches('\'').to_lowercase();
                if !v.is_empty() && !PLACEHOLDER_API_KEYS.contains(&v.as_str()) {
                    has_key_in_env = true;
                }
            } else if let Some(val) = l.strip_prefix("ANTHROPIC_BASE_URL=") {
                base = val.trim().to_string();
            } else if let Some(val) = l.strip_prefix("ANTHROPIC_AUTH_TOKEN=") {
                tok = val.trim().to_string();
            }
        }
    }
    // A managed upstream (amux cloud's Anthropic proxy) is fully configured
    // auth — never prompt such a workspace for a key (python's comment).
    let has_proxy = !base.is_empty() && !tok.is_empty();
    let has_oauth = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".claude.json"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| py_truthy(&v["oauthAccount"]))
        .unwrap_or(false);
    // key_valid/key_error: python serves a CACHED background validation
    // (_api_key_status). This server runs no validator, so it answers what
    // python answers before its first validation — null/"" — rather than
    // inventing a verdict (Invariant 20: never invent state).
    axum::Json(serde_json::json!({
        "email": email,
        "is_cloud": !email.is_empty(),
        "has_api_key": has_key_in_env || has_oauth || has_proxy,
        "has_oauth": has_oauth,
        "managed_upstream": has_proxy,
        "key_valid": serde_json::Value::Null,
        "key_error": "",
    }))
}
