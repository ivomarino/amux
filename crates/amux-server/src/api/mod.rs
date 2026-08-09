//! axum router assembly (RR-0021, Invariant 13).
//!
//! Route groups land phase by phase. Every route added here must also appear
//! in the OpenAPI spec (RR checklist) and — for legacy-compat paths — in the
//! alias registry (RR-0018a).

pub mod aliases;
pub mod auth;
pub mod board;
pub mod health;
pub mod schedules;
pub mod sse;
pub mod static_files;
pub mod sync;
pub mod workers;

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
        .nest("/api/workers", workers::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    let app = Router::new()
        // Public: the PWA shell + health must load before auth happens.
        .route("/health", axum::routing::get(health::health))
        .merge(static_files::routes())
        .merge(protected)
        .with_state(state);

    // Legacy route aliases (RR-0018a): /api/sessions/* rewrites to
    // /api/workers/* BEFORE routing, so the rewrite must wrap the finished
    // router. Auth is inside the wrapper — legacy paths are exactly as
    // protected as canonical ones.
    aliases::alias_layer(app)
}
