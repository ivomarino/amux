//! `/api/health/invariants` + `/api/debug/invariants` (AMUX-2622).
//!
//! Two surfaces with different jobs, deliberately not one:
//!
//! * **health** — a rollup a human or a probe reads in one glance, carrying
//!   evidence FRESHNESS. "184/184 pass" is worthless without "checked 4s ago";
//!   a monitor that died an hour ago also reports 184/184.
//! * **debug** — live incidents with their full evidence and the raw latest
//!   evaluation per invariant, for the person actually diagnosing.

use super::AppState;
use crate::invariants::{monitor, rollup, store, Status};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/health/invariants", axum::routing::get(health))
        .route("/api/debug/invariants", axum::routing::get(debug))
}

/// GET /api/health/invariants — rollup + counts + freshness.
///
/// Evaluates FRESH rather than replaying the last stored pass: a health
/// endpoint that can only report the previous poll cannot answer "is it broken
/// right now", which is the only question worth asking it.
async fn health(State(state): State<AppState>) -> Response {
    let results = monitor::evaluate_all(&state).await;
    let conf = rollup(&results);
    let (mut pass, mut fail, mut unknown) = (0, 0, 0);
    for r in &results {
        match r.status {
            Status::Pass => pass += 1,
            Status::Fail => fail += 1,
            Status::Unknown => unknown += 1,
            Status::Skipped => {}
        }
    }
    let live = store::live_incidents(&state.store).unwrap_or_default();
    // 200 even when unhealthy: this endpoint's job is to REPORT, and a non-2xx
    // makes generic uptime tooling retry/alert on the reporter rather than read
    // the report. The verdict is in the body, where it can be precise.
    Json(json!({
        "confidence": conf.as_str(),
        "checks": {"pass": pass, "fail": fail, "unknown": unknown, "total": results.len()},
        "live_incidents": live.len(),
        // Named explicitly so a caller cannot mistake "no failures" for
        // "everything verified" when probes could not run.
        "note": if unknown > 0 {
            "some probes could not reach a verdict — unknown is not pass"
        } else { "" },
        "failures": results.iter().filter(|r| r.status == Status::Fail).map(|r| json!({
            "invariant_id": r.invariant_id, "entity": r.entity_key,
            "expected": r.expected, "observed": r.observed,
        })).collect::<Vec<_>>(),
        "unknowns": results.iter().filter(|r| r.status == Status::Unknown).map(|r| json!({
            "invariant_id": r.invariant_id, "why": r.observed,
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// GET /api/debug/invariants — live incidents + last evaluation per invariant.
async fn debug(State(state): State<AppState>) -> Response {
    let incidents = store::live_incidents(&state.store).unwrap_or_default();
    let latest = store::latest_per_invariant(&state.store).unwrap_or_default();
    // A check whose newest row is old has STOPPED RUNNING, which is a different
    // and more alarming fact than its last verdict. Surfaced as an explicit
    // list rather than left for the reader to compute from timestamps.
    let stale: Vec<_> = latest
        .iter()
        .filter(|l| l["age_s"].as_f64().unwrap_or(0.0) > 300.0)
        .cloned()
        .collect();
    Json(json!({
        "live_incidents": incidents,
        "latest_per_invariant": latest,
        "stale_checks": stale,
        "notes": {
            "dedupe": "one incident per (invariant_id, entity); occurrences counts repeats",
            "stale_checks": "no evaluation in >300s — the check itself may be dead, which \
                             is worse than a failing check because silence reads as health",
            "unknown": "a probe that could not reach a verdict. NOT a pass.",
        }
    }))
    .into_response()
}
