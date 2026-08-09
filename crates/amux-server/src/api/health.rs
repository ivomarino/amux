//! `/health` (RR-0021): build hash, uptime, store status, global revision.
//!
//! The Python workflow's hard-won rule — "bracket any measurement with
//! /health's build" — carries over verbatim: `build` is a content hash of
//! the running binary, so a restart with the same code and a restart with
//! different code are distinguishable (ethos rule 4).

use super::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    pub status: &'static str,
    pub build: String,
    pub uptime_s: u64,
    pub rev: Option<u64>,
    pub store: &'static str,
    pub pid: u32,
    pub server: &'static str,
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    // A store that cannot answer the revision query is degraded — surface
    // that instead of a green lie (ethos rule 7: a check must be able to
    // fail).
    let (rev, store, code) = match state.store.current_rev() {
        Ok(rev) => (Some(rev.0), "ok", StatusCode::OK),
        Err(_) => (None, "hung", StatusCode::SERVICE_UNAVAILABLE),
    };
    (
        code,
        Json(Health {
            status: if store == "ok" { "ok" } else { "degraded" },
            build: state.build_hash.clone(),
            uptime_s: state.started.elapsed().as_secs(),
            rev,
            store,
            pid: std::process::id(),
            server: "amux-rust",
        }),
    )
}
