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

/// GET /api/debug/tmux — runs the exact fleet-discovery command from INSIDE
/// this process and reports argv, exit status, output sizes, and the env
/// that determines the socket. Exists because the live launchd instance
/// served running=0 for the whole fleet while the same binary in a login
/// shell served 49, and no log line could say why (ethos rule 4: the
/// instrument must express the discriminator, from the consumer's vantage).
pub async fn debug_tmux() -> axum::Json<serde_json::Value> {
    let out = std::process::Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_activity}\t#{session_created}",
        ])
        .output();
    let which = std::process::Command::new("which").arg("tmux").output();
    axum::Json(match out {
        Ok(o) => serde_json::json!({
            "spawn": "ok",
            "exit": o.status.to_string(),
            "stdout_bytes": o.stdout.len(),
            "stdout_lines": String::from_utf8_lossy(&o.stdout).lines().count(),
            "stdout_head": String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or(""),
            "stderr": String::from_utf8_lossy(&o.stderr).trim(),
            "which_tmux": which.ok().map(|w| String::from_utf8_lossy(&w.stdout).trim().to_string()),
            "env_path": std::env::var("PATH").unwrap_or_default(),
            "env_tmux_tmpdir": std::env::var("TMUX_TMPDIR").ok(),
            "env_tmpdir": std::env::var("TMPDIR").ok(),
            "env_tmux": std::env::var("TMUX").ok(),
            "cwd": std::env::current_dir().ok().map(|p| p.display().to_string()),
        }),
        Err(e) => serde_json::json!({ "spawn": "failed", "error": e.to_string() }),
    })
}
