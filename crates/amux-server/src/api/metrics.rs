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
        .route("/replay", axum::routing::get(replay))
}

/// GET /api/metrics/replay — audit replay (RR-0111a): fold the event journal
/// to HEAD and compare against the live tables. Divergences come back NAMED
/// (entity + fields + both values), horizon entities are reported instead of
/// fabricated, and every list cap announces itself in the body.
async fn replay(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(crate::db::replay::verify_replay(&conn)?)
    })
    .await;
    let report = match joined {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    match serde_json::to_value(&report) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
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
    // journal every consumer shares. STORAGE FORMAT, resolved 2026-08-09
    // after two sessions fixed the same mismatch in opposite directions:
    // the writer (db/mod.rs apply_write) now stores the BARE tag
    // ("fleet_progress"); the `{"kind":"other","data":name}` object form
    // was an accident of a no-op trim_matches, and while it was the format,
    // every `entity_type = '<tag>'` filter (this fn, the RR-0044b dedupe,
    // window_stats) silently matched nothing — a check that could not fail,
    // ethos rule 7. Rows written before the fix still carry the object
    // form, so this reads BOTH.
    let last = |name: &str| -> Option<String> {
        let legacy = serde_json::to_string(&amux_core::revision::EntityType::Other(name.into()))
            .unwrap_or_default();
        conn.query_row(
            "SELECT entity_id FROM _amux_state_events WHERE entity_type IN (?1, ?2)
             ORDER BY rev DESC LIMIT 1",
            [name, legacy.as_str()],
            |r| r.get(0),
        )
        .ok()
    };
    let parse = |s: Option<String>| {
        s.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    };

    // Per-provider fleet state (RR-0044b): the dashboard's "Exhausted,
    // resets in 2h 14m, 14 workers parked" card. Derived from the SAME
    // worker rows by the SAME core function the runtime uses to park
    // workers — this view cannot disagree with the mechanism it describes
    // (ethos rule 1).
    let providers: serde_json::Value = {
        use amux_core::provider_fleet::{derive, ProviderState, DEFAULT_RESUME_STAGGER_SECS};
        let stagger = std::env::var("AMUX_RS_RESUME_STAGGER_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_RESUME_STAGGER_SECS);
        match crate::orchestrator::runtime::hydrate_workers(&conn) {
            Ok(workers) => derive(&workers, chrono::Utc::now(), stagger)
                .into_iter()
                .map(|(pid, p)| {
                    let (state, reset_at) = match &p.state {
                        ProviderState::Available => ("available", None),
                        ProviderState::QuotaExhausted { reset_at, .. } => {
                            ("quota_exhausted", reset_at.map(|r| r.to_rfc3339()))
                        }
                        ProviderState::Unknown => ("unknown", None),
                    };
                    (
                        pid.as_str().to_string(),
                        json!({
                            "state": state,
                            "reset_at": reset_at,
                            "workers_parked": p.affected_workers.len(),
                            "workers_total": p.workers.len(),
                        }),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>()
                .into(),
            // A provider view that cannot be read must say so, never
            // report an empty (healthy-looking) fleet (ethos rule 7).
            Err(e) => json!({ "error": e.to_string() }),
        }
    };

    Json(json!({
        "last_heartbeat": parse(last("fleet_progress")),
        "last_fleet_state_change": parse(last("fleet_state")),
        "last_exhaustion_action": parse(last("exhaustion")),
        "providers": providers,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use crate::api::{router, AppState};
    use crate::db::{SharedStore, Store, WriteOutcome};
    use amux_core::worker::{WorkerConfig, WorkerState};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn app() -> (axum::Router, SharedStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("amux-test.db")).unwrap());
        let state = AppState {
            store: store.clone(),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        (router(state), store, dir)
    }

    fn seed_worker(store: &SharedStore, n: u128, provider: &str, state: WorkerState) {
        let id = amux_core::ids::WorkerId::from_ulid(ulid::Ulid::from_parts(
            1_700_000_000_000,
            n,
        ));
        let provider = provider.to_string();
        store
            .write(move |conn| {
                let row = crate::db::queries::WorkerRow::new(
                    &id,
                    &WorkerConfig {
                        display_name: format!("w{n}"),
                        name_aliases: vec![],
                        cwd: "/tmp".into(),
                        provider: amux_core::provider::ProviderId(provider.clone()),
                        model: None,
                        backend: amux_core::session::BackendId::herdr(),
                        environment: Default::default(),
                        permissions: vec![],
                        group: None,
                    },
                    "2026-01-01T00:00:00Z",
                );
                crate::db::queries::insert_worker(conn, &row)?;
                crate::db::queries::update_worker_state(
                    conn,
                    id.as_str(),
                    &state,
                    "2026-01-01T00:00:00Z",
                )?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
    }

    /// RR-0044b metrics shape: /api/metrics/fleet reports per-provider
    /// {state, reset_at, workers_parked, workers_total}, from the same
    /// derivation the runtime parks with.
    #[tokio::test]
    async fn fleet_metrics_report_per_provider_state() {
        let (app, store, _dir) = app();
        let reset = Utc::now() + chrono::Duration::hours(2);
        seed_worker(&store, 31, "claude", WorkerState::RateLimited { reset_at: Some(reset) });
        seed_worker(&store, 32, "claude", WorkerState::Idle { since: Utc::now() });
        seed_worker(&store, 33, "codex", WorkerState::Idle { since: Utc::now() });

        let res = app
            .clone()
            .oneshot(Request::builder().uri("/api/metrics/fleet").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let claude = &v["providers"]["claude"];
        assert_eq!(claude["state"], "quota_exhausted", "{v}");
        assert_eq!(claude["reset_at"], reset.to_rfc3339(), "{v}");
        assert_eq!(claude["workers_parked"], 1, "{v}");
        assert_eq!(claude["workers_total"], 2, "{v}");

        let codex = &v["providers"]["codex"];
        assert_eq!(codex["state"], "available", "{v}");
        assert_eq!(codex["reset_at"], serde_json::Value::Null, "{v}");
        assert_eq!(codex["workers_parked"], 0, "{v}");
        assert_eq!(codex["workers_total"], 1, "{v}");
    }

    /// The `last()` lookups must find what the runtime writes (the stored
    /// entity_type is serde-encoded JSON, not the bare name — this test
    /// fails against the bare-name query that shipped originally).
    #[tokio::test]
    async fn fleet_metrics_surface_the_last_heartbeat() {
        let (app, store, _dir) = app();
        let rt = crate::orchestrator::runtime::Runtime {
            store: store.clone(),
            backends: vec![],
            tick_secs: 3,
            heartbeat_every: 1,
            breaker: amux_core::circuit::FleetCircuitBreaker {
                window_budget_tokens: u64::MAX,
                window_secs: 3600,
                min_progress_per_window: 0,
                max_failures_per_window: 1000,
            },
            fleet_state: std::sync::Mutex::new(amux_core::circuit::FleetState::Normal),
            protocol: None,
            pickup_unowned: false,
            resume_stagger_secs: 5,
        };
        rt.tick_once(true).await.unwrap(); // heartbeat tick

        let res = app
            .clone()
            .oneshot(Request::builder().uri("/api/metrics/fleet").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            v["last_heartbeat"].is_object(),
            "heartbeat published by the runtime must be readable here: {v}"
        );
    }
}
