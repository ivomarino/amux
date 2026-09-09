//! Shadow reconciliation and explicit local last-known-green promotion.

use super::AppState;
use crate::reconciliation::GateCommand;
use amux_core::revision::{EntityType, MutationKind};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/reconciliations", axum::routing::post(run))
        .route("/reconciliations/{id}", axum::routing::get(get))
        .route(
            "/reconciliations/{id}/promote",
            axum::routing::post(promote),
        )
}

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-amux-session")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("api-anonymous")
        .to_string()
}

fn internal(error: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": error.to_string()})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunInput {
    repo_path: PathBuf,
    candidate_sha: String,
    gates: Vec<GateCommand>,
}

async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunInput>,
) -> Response {
    if actor(&headers) == "api-anonymous" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "reconciliation requires an identified actor"})),
        )
            .into_response();
    }
    let id = format!("recon_{}", ulid::Ulid::new().to_string().to_lowercase());
    let repo_path = body.repo_path;
    let repo_for_run = repo_path.clone();
    let candidate = body.candidate_sha;
    let gates = body.gates;
    let started_at = Utc::now();
    let run = match tokio::task::spawn_blocking(move || {
        crate::reconciliation::run_candidate(&repo_for_run, &candidate, &gates)
    })
    .await
    {
        Ok(Ok(run)) => run,
        Ok(Err(error)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": error.to_string()})),
            )
                .into_response()
        }
        Err(error) => return internal(error),
    };
    let completed_at = Utc::now();
    let stored = json!({
        "id": id,
        "repo_path": repo_path,
        "candidate_sha": run.candidate_sha,
        "status": run.status,
        "gate_results": run.gates,
        "failure_reason": run.failure_reason,
        "started_at": started_at,
        "completed_at": completed_at,
        "promoted_at": Value::Null,
        "promoted_by": Value::Null
    });
    let row = stored.clone();
    let row_id = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            conn.execute(
                "INSERT INTO _amux_reconciliations
                 (id,repo_path,candidate_sha,status,gate_results,failure_reason,started_at,completed_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![
                    row_id,
                    row["repo_path"].as_str().unwrap_or_default(),
                    row["candidate_sha"].as_str().unwrap_or_default(),
                    row["status"].as_str().unwrap_or_default(),
                    row["gate_results"].to_string(),
                    row["failure_reason"].as_str(),
                    row["started_at"].as_str().unwrap_or_default(),
                    row["completed_at"].as_str().unwrap_or_default(),
                ],
            )?;
            if row["status"] == "green" {
                conn.execute(
                    "INSERT INTO _amux_green_snapshots
                     (repo_path,candidate_sha,reconciliation_id,verified_at)
                     VALUES(?1,?2,?3,?4)
                     ON CONFLICT(repo_path) DO UPDATE SET candidate_sha=?2,reconciliation_id=?3,
                       verified_at=?4,promoted_at=NULL,promoted_by=NULL",
                    rusqlite::params![
                        row["repo_path"].as_str().unwrap_or_default(),
                        row["candidate_sha"].as_str().unwrap_or_default(),
                        row_id,
                        row["completed_at"].as_str().unwrap_or_default(),
                    ],
                )?;
            }
            for gate in row["gate_results"].as_array().into_iter().flatten() {
                crate::db::throughput_store::record_metric(
                    conn,
                    "build_wait",
                    "reconciliation",
                    gate["duration_ms"].as_u64(),
                    None,
                    gate["label"].as_str(),
                    completed_at,
                )?;
            }
            let _ = crate::db::throughput_store::evaluate_wip(conn, completed_at)?;
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![crate::db::PendingEvent {
                    entity_type: EntityType::Other("reconciliation".into()),
                    entity_id: row_id,
                    mutation: MutationKind::Created,
                    payload: Some(row),
                }],
            })
        })
        .await;
    match write {
        Ok(_) => (StatusCode::CREATED, Json(stored)).into_response(),
        Err(error) => internal(error),
    }
}

async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let conn = match state.store.read() {
        Ok(conn) => conn,
        Err(error) => return internal(error),
    };
    let row: rusqlite::Result<Value> = conn.query_row(
        "SELECT id,repo_path,candidate_sha,status,gate_results,failure_reason,started_at,completed_at,
                promoted_at,promoted_by FROM _amux_reconciliations WHERE id=?1",
        [&id],
        |r| {
            let gates: String = r.get(4)?;
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "repo_path": r.get::<_, String>(1)?,
                "candidate_sha": r.get::<_, String>(2)?,
                "status": r.get::<_, String>(3)?,
                "gate_results": serde_json::from_str::<Value>(&gates).unwrap_or(Value::Null),
                "failure_reason": r.get::<_, Option<String>>(5)?,
                "started_at": r.get::<_, String>(6)?,
                "completed_at": r.get::<_, String>(7)?,
                "promoted_at": r.get::<_, Option<String>>(8)?,
                "promoted_by": r.get::<_, Option<String>>(9)?
            }))
        },
    );
    match row {
        Ok(row) => Json(json!({"measured": true, "n_considered": 1, "item": row})).into_response(),
        Err(rusqlite::Error::QueryReturnedNoRows) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no reconciliation"})),
        )
            .into_response(),
        Err(error) => internal(error),
    }
}

async fn promote(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if actor(&headers) == "api-anonymous" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "promotion requires an identified human actor"})),
        )
            .into_response();
    }
    let (repo, sha, status): (String, String, String) = {
        let conn = match state.store.read() {
            Ok(conn) => conn,
            Err(error) => return internal(error),
        };
        match conn.query_row(
            "SELECT repo_path,candidate_sha,status FROM _amux_reconciliations WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ) {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "no reconciliation"})),
                )
                    .into_response()
            }
            Err(error) => return internal(error),
        }
    };
    if status != "green" {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "only a green reconciliation can be promoted"})),
        )
            .into_response();
    }
    let repo_for_ref = PathBuf::from(&repo);
    let sha_for_ref = sha.clone();
    let confirmed = match tokio::task::spawn_blocking(move || {
        crate::reconciliation::promote_local_ref(&repo_for_ref, &sha_for_ref)
    })
    .await
    {
        Ok(Ok(confirmed)) => confirmed,
        Ok(Err(error)) => return internal(error),
        Err(error) => return internal(error),
    };
    let promoted_by = actor(&headers);
    let at = Utc::now().to_rfc3339();
    let id_for_write = id.clone();
    let repo_for_write = repo.clone();
    let by_for_write = promoted_by.clone();
    let at_for_write = at.clone();
    match state
        .store
        .write_async(move |conn| {
            conn.execute(
                "UPDATE _amux_reconciliations SET promoted_at=?2,promoted_by=?3 WHERE id=?1",
                rusqlite::params![id_for_write, at_for_write, by_for_write],
            )?;
            conn.execute(
                "UPDATE _amux_green_snapshots SET promoted_at=?2,promoted_by=?3
                 WHERE repo_path=?1 AND reconciliation_id=?4",
                rusqlite::params![repo_for_write, at_for_write, by_for_write, id_for_write],
            )?;
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![crate::db::PendingEvent {
                    entity_type: EntityType::Other("green_snapshot".into()),
                    entity_id: id_for_write,
                    mutation: MutationKind::Updated,
                    payload: None,
                }],
            })
        })
        .await
    {
        Ok(_) => Json(json!({
            "reconciliation_id": id,
            "candidate_sha": sha,
            "local_ref": "refs/heads/amux/last-known-green",
            "confirmed_sha": confirmed,
            "promoted_at": at,
            "promoted_by": promoted_by,
            "pushed": false
        }))
        .into_response(),
        Err(error) => internal(error),
    }
}
