//! axum router assembly (RR-0021, Invariant 13).
//!
//! Route groups land phase by phase. Every route added here must also appear
//! in the OpenAPI spec (RR checklist) and — for legacy-compat paths — in the
//! alias registry (RR-0018a).

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
pub mod memories;
pub mod metrics;
pub mod messages;
pub mod org;
pub mod prefs;
pub mod schedules;
pub mod sessions_legacy;
pub mod settings;
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
        .nest("/api/email", email::routes())
        .nest("/api/cal-events", calendar::routes())
        // Legacy SHAPE (not just path): the SPA renders this array (RR-0075).
        .route("/api/sessions", axum::routing::get(sessions_legacy::list_sessions_legacy))
        .nest("/api/browser", browser::routes())
        .nest("/api/files", files::routes())
        .nest("/api/journal", journal::routes())
        .nest("/api/history", history::routes())
        .nest("/api/settings", settings::routes())
        .nest("/api/push", crate::push::routes())
        .nest("/api/dictation", dictation::routes())
        // Python serves transcription at the TOP-LEVEL /api/dictate (the
        // dictation module owns it; here it is an honest 501).
        .route("/api/dictate", axum::routing::post(dictation::dictate))
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
