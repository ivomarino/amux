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
pub mod files;
pub mod gmail_auth;
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
        .route("/api/sessions/{name}", axum::routing::any(py_proxy::proxy_session_verb))
        .route("/api/sessions/{name}/{*verb}", axum::routing::any(py_proxy::proxy_session_verb))
        .nest("/api/browser", browser::routes())
        .nest("/api/files", files::routes())
        // /api/fs/* is the SPA's Files contract (multipart upload + dir
        // field, open/mkdir/rename/read/list/search/delete on ABSOLUTE
        // paths) — a different contract from /api/files above, so it
        // proxies to the Python owner rather than aliasing (py_proxy.rs).
        .nest("/api/fs", py_proxy::passthrough_routes())
        // Chunked upload: /api/upload/start, /api/upload/:id/chunk/:n,
        // /api/upload/:id/finish. Python owns the chunked protocol and the
        // uploads directory; the SPA's peek drag-and-drop hits these.
        .nest("/api/upload", py_proxy::passthrough_routes())
        .nest("/api/uploads", py_proxy::passthrough_routes())
        // Groups/tags (AMUX-2594): the SPA's group picker reads /api/groups
        // ({"groups":[{name,workers,...}]}); Python owns the fleet and its
        // own groups->tags aliasing (amux-server.py:65345, config paths
        // exempt), so BOTH spellings proxy wholesale pre-cutover.
        .nest("/api/groups", py_proxy::passthrough_routes())
        .nest("/api/tags", py_proxy::passthrough_routes())
        .nest("/api/journal", journal::routes())
        // Skills / slash-commands / map: the SPA tabs' data (AMUX-2586 #6).
        .nest("/api/skills", skills::routes())
        .nest("/api/slash-commands", skills::slash_routes())
        .nest("/api/map", map::routes())
        .nest("/api/history", history::routes())
        .nest("/api/settings", settings::routes())
        .nest("/api/push", crate::push::routes())
        .nest("/api/dictation", dictation::routes())
        // Python serves transcription at the TOP-LEVEL /api/dictate (the
        // dictation module owns it; it proxies to the Python engine). Body
        // limit off: audio runs to 25MB raw / ~33MB base64, and Python's own
        // 413 must answer, not axum's 2MB default.
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
