//! /api/metrics + /api/debug/fleet (Phase 9, Invariants 34/40).
//!
//! Queue depth is a health signal (Invariant 34): a growing command queue
//! or dead-letter count is the fleet telling you delivery is failing, and
//! it must be readable where people already look (ethos rule 4) — one
//! endpoint, no log spelunking.

use super::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(metrics))
        .route("/fleet", axum::routing::get(fleet))
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

async fn metrics(State(state): State<AppState>) -> Response {
    let conn = match state.store.read() {
        Ok(c) => c,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    };
    // -1 = "query failed", never silently 0: a metric that cannot be read
    // must not report an empty queue (ethos rule 7).
    let body = json!({
        "rev": state.store.current_rev().map(|r| r.0).unwrap_or(0),
        "uptime_s": state.started.elapsed().as_secs(),
        "workers": {
            "total": count(&conn, "SELECT COUNT(*) FROM _amux_workers WHERE json_extract(state,'$.deleted_at') IS NULL"),
            "live_sessions": count(&conn, "SELECT COUNT(*) FROM _amux_sessions WHERE ended_at IS NULL"),
        },
        "queues": {
            "commands_queued": count(&conn, "SELECT COUNT(*) FROM _amux_commands WHERE state LIKE '%queued%'"),
            "commands_in_flight": count(&conn, "SELECT COUNT(*) FROM _amux_commands WHERE state LIKE '%dispatched%' OR state LIKE '%delivered%'"),
            "dead_letters": count(&conn, "SELECT COUNT(*) FROM _amux_commands WHERE state LIKE '%dead_lettered%'"),
            "messages_undelivered": count(&conn, "SELECT COUNT(*) FROM _amux_messages WHERE delivery LIKE '%queued%'"),
        },
        "board": {
            "open": count(&conn, "SELECT COUNT(*) FROM issues WHERE deleted IS NULL AND COALESCE(archived,0)=0 AND status NOT IN ('done','verified','discarded')"),
            "quarantined": count(&conn, "SELECT COUNT(*) FROM issues WHERE deleted IS NULL AND status = 'quarantined'"),
        },
        "leases": {
            "live": count(&conn, "SELECT COUNT(*) FROM _amux_leases"),
        },
        "turns_recorded": count(&conn, "SELECT COUNT(*) FROM _amux_turns"),
        "events_journal": count(&conn, "SELECT COUNT(*) FROM _amux_state_events"),
    });
    Json(body).into_response()
}

async fn fleet(State(state): State<AppState>) -> Response {
    let conn = match state.store.read() {
        Ok(c) => c,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    };
    // The last published heartbeat + fleet-state events, straight from the
    // journal every consumer shares.
    let last = |etype: &str| -> Option<String> {
        conn.query_row(
            "SELECT entity_id FROM _amux_state_events WHERE entity_type = ?1
             ORDER BY rev DESC LIMIT 1",
            [etype],
            |r| r.get(0),
        )
        .ok()
    };
    let parse = |s: Option<String>| {
        s.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    };
    Json(json!({
        "last_heartbeat": parse(last("fleet_progress")),
        "last_fleet_state_change": parse(last("fleet_state")),
        "last_exhaustion_action": parse(last("exhaustion")),
    }))
    .into_response()
}
