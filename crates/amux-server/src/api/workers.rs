//! Worker API: CRUD + start/stop/peek (RR-0034; parts of RR-0035 rename/alias
//! and RR-0040 group/env/permissions changes; Invariants 13, 17, 37, 43).
//!
//! Mounted at `/api/workers` inside the `protected` router (api/mod.rs), so
//! every handler sits behind bearer auth; the legacy `/api/sessions/*` paths
//! reach the same handlers through `aliases::alias_layer` (RR-0018a) and are
//! equally protected because the rewrite happens outside auth.
//!
//! Every mutation goes through `Store::write_async` and reports honestly:
//! `applied: false` with no rev/version bump on no-ops (Invariant 37), a
//! `ConfigChangeResult` naming the apply mode and any session swap on config
//! changes (Invariant 43), and `PendingEvent`s for every real change so
//! SSE/delta-sync consumers see it (Invariant 35).

use super::aliases::{alias_fields, FieldStyle};
use super::AppState;
use crate::db::queries::{self, SessionRow, WorkerRow};
use crate::db::{PendingEvent, WriteOutcome};
use amux_core::ids::{GroupId, SessionId, WorkerId};
use amux_core::provider::{ProviderCapabilities, ProviderId};
use amux_core::revision::{EntityType, MutationKind};
use amux_core::search::PagedResponse;
use amux_core::session::{backend_ref, BackendId, ExitReason};
use amux_core::worker::{
    apply_config, ConfigChangeResult, Worker, WorkerCapabilities, WorkerConfig, WorkerState,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_workers).post(create_worker))
        .route(
            "/{id}",
            get(get_worker).patch(patch_worker).delete(delete_worker),
        )
        .route("/{id}/start", post(start_worker))
        .route("/{id}/stop", post(stop_worker))
        .route("/{id}/peek", get(peek_worker))
}

// ---- shared helpers -----------------------------------------------------

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": e.to_string() }),
    )
}

fn not_found(key: &str) -> Response {
    err(
        StatusCode::NOT_FOUND,
        json!({ "error": "worker not found", "key": key }),
    )
}

/// Provider capabilities lookup. There is NO provider registry yet (RR-0007
/// adapters land it), so this returns the core's conservative all-false
/// default for every provider: a model change classifies as SessionRestart
/// rather than falsely promising a hot switch the runtime cannot perform
/// (better to over-restart than to claim `next_turn` and never apply).
/// Replace with the real registry when provider adapters exist.
fn provider_caps(_provider: &str) -> ProviderCapabilities {
    ProviderCapabilities::default()
}

/// The serde tag of a WorkerState ("stopped", "starting", ...) for
/// StatusChanged events and error bodies.
fn state_tag(state: &WorkerState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.get("state").and_then(|s| s.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

/// The canonical API body for a worker. Field aliasing (RR-0018a) is applied
/// by callers via `alias_fields`.
fn worker_body(row: &WorkerRow) -> Value {
    json!({
        "id": row.id,
        "display_name": row.display_name,
        "name_aliases": row.name_aliases,
        "cwd": row.cwd,
        "provider": row.provider,
        "model": row.model,
        "backend": row.backend,
        "environment": row.environment,
        "permissions": row.permissions,
        "group": row.group_id,
        "state": serde_json::to_value(&row.state).unwrap_or_else(|_| json!({"state": "stopped"})),
        "version": row.version,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

fn ev(entity_type: EntityType, id: &str, mutation: MutationKind) -> PendingEvent {
    PendingEvent {
        entity_type,
        entity_id: id.to_string(),
        mutation,
    }
}

fn no_write() -> WriteOutcome {
    WriteOutcome {
        applied: false,
        events: Vec::new(),
    }
}

/// Park a handler-level outcome in the slot and return the write outcome —
/// the only way data leaves a `Store::write` closure, since the closure's
/// return type is fixed by the writer loop.
fn finish<T>(
    slot: &Mutex<Option<T>>,
    outcome: T,
    write: WriteOutcome,
) -> rusqlite::Result<WriteOutcome> {
    *slot.lock().expect("outcome slot poisoned") = Some(outcome);
    Ok(write)
}

/// Map a non-SQL corruption (e.g. an unparseable id in a row we minted) into
/// the closure's error type so it surfaces as a 500 instead of a panic.
fn corrupt(e: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// Resolve `display_name` vs the legacy `name` spelling (RR-0018a request
/// rule): either is accepted; both-present-and-different is a 400 naming
/// both values — never a silently picked winner (Invariant 37).
fn resolve_name_fields(
    display_name: Option<String>,
    name: Option<String>,
) -> Result<Option<String>, Box<Response>> {
    match (display_name, name) {
        (Some(a), Some(b)) if a != b => Err(Box::new(err(
            StatusCode::BAD_REQUEST,
            json!({
                "error": "display_name and name (legacy alias) carry different values",
                "display_name": a,
                "name": b,
            }),
        ))),
        (a, b) => Ok(a.or(b)),
    }
}

// ---- GET /api/workers ---------------------------------------------------

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_limit() -> u64 {
    200
}

/// List workers, PagedResponse-shaped (Invariant 40: `total`/`truncated`
/// announce what a page omits instead of silently capping).
pub async fn list_workers(
    State(state): State<AppState>,
    Query(p): Query<ListParams>,
) -> Response {
    let offset = p.offset;
    let limit = p.limit.clamp(1, 1000);
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(queries::list_workers(&conn, offset, limit)?)
    })
    .await;
    let (rows, total) = match joined {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e),
    };
    let items: Vec<Value> = rows.iter().map(worker_body).collect();
    let page = match PagedResponse::new(items, total, offset, limit) {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    match serde_json::to_value(&page) {
        Ok(v) => Json(alias_fields(v, FieldStyle::default())).into_response(),
        Err(e) => internal(e),
    }
}

// ---- POST /api/workers --------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)] // Invariant 37: unknown fields are rejected, not dropped
pub struct CreateWorkerBody {
    #[serde(default)]
    pub display_name: Option<String>,
    /// Legacy spelling (the Python dashboard says "name"); RR-0018a request
    /// aliasing rules apply.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub environment: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
    #[serde(default)]
    pub group: Option<String>,
}

pub async fn create_worker(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkerBody>,
) -> Response {
    let display_name = match resolve_name_fields(body.display_name, body.name) {
        Ok(Some(n)) if !n.trim().is_empty() => n,
        Ok(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                json!({ "error": "display_name is required" }),
            )
        }
        Err(resp) => return *resp,
    };
    let group = match &body.group {
        Some(g) => match GroupId::parse(g) {
            Ok(id) => Some(id),
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": e.to_string(), "group": g }),
                )
            }
        },
        None => None,
    };
    let config = WorkerConfig {
        display_name,
        name_aliases: Vec::new(),
        cwd: body.cwd.unwrap_or_default(),
        provider: ProviderId::new(body.provider.unwrap_or_else(|| "claude".into())),
        model: body.model,
        backend: body.backend.map(BackendId::from).unwrap_or_default(),
        environment: body.environment.unwrap_or_default(),
        permissions: body.permissions.unwrap_or_default(),
        group,
    };

    // RR-0034 create contract: server-minted ULID id, version 0, Stopped.
    let id = WorkerId::from_ulid(ulid::Ulid::new());
    let now = chrono::Utc::now().to_rfc3339();
    let row = WorkerRow::new(&id, &config, &now);

    let row_for_write = row.clone();
    let write = state
        .store
        .write_async(move |conn| {
            queries::insert_worker(conn, &row_for_write)?;
            Ok(WriteOutcome {
                applied: true,
                events: vec![ev(EntityType::Worker, &row_for_write.id, MutationKind::Created)],
            })
        })
        .await;
    match write {
        Ok(reply) => {
            let mut v = alias_fields(worker_body(&row), FieldStyle::default());
            v["rev"] = json!(reply.rev.0);
            (StatusCode::CREATED, Json(v)).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- GET /api/workers/{id} ----------------------------------------------

/// Detail by id, display_name, or alias (Invariant 17 — see
/// `queries::get_worker` for the resolution order).
pub async fn get_worker(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    let store = state.store.clone();
    let k = key.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(queries::get_worker(&conn, &k)?)
    })
    .await;
    match joined {
        Ok(Ok(Some(row))) => {
            Json(alias_fields(worker_body(&row), FieldStyle::default())).into_response()
        }
        Ok(Ok(None)) => not_found(&key),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---- PATCH /api/workers/{id} --------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)] // Invariant 37
pub struct PatchWorkerBody {
    #[serde(default)]
    pub display_name: Option<String>,
    /// Legacy spelling of display_name (RR-0018a).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// NOTE: absent = unchanged. Clearing a model back to provider-default
    /// (`"model": null`) is indistinguishable from absent in this shape and
    /// therefore not supported yet — a double-Option lands with the full
    /// RR-0037 model-change work.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub environment: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
    #[serde(default)]
    pub group: Option<String>,
    /// Optimistic concurrency (Invariant 35): when present, the write only
    /// applies if the entity is still at this version; otherwise 409.
    #[serde(default)]
    pub expect_version: Option<u64>,
}

enum PatchOutcome {
    NotFound,
    Conflict { current_version: u64 },
    Noop { body: Value },
    Applied { body: Value, change: ConfigChangeResult },
}

pub async fn patch_worker(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<PatchWorkerBody>,
) -> Response {
    let display_name = match resolve_name_fields(body.display_name.clone(), body.name.clone()) {
        Ok(n) => n,
        Err(resp) => return *resp,
    };
    // Parse the group id OUTSIDE the write closure so a bad request is a
    // clean 400 before anything touches the writer thread.
    let group: Option<GroupId> = match &body.group {
        Some(g) => match GroupId::parse(g) {
            Ok(id) => Some(id),
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": e.to_string(), "group": g }),
                )
            }
        },
        None => None,
    };

    let slot: Arc<Mutex<Option<PatchOutcome>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let key_w = key.clone();

    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = queries::get_worker(conn, &key_w)? else {
                return finish(&slot_w, PatchOutcome::NotFound, no_write());
            };
            // Conflict outranks no-op: a stale caller should learn their view
            // of the world is old, even if the change they wanted is moot.
            if let Some(expect) = body.expect_version {
                if expect != row.version {
                    return finish(
                        &slot_w,
                        PatchOutcome::Conflict { current_version: row.version },
                        no_write(),
                    );
                }
            }

            let old_cfg = row.config();
            let mut new_cfg = old_cfg.clone();
            if let Some(v) = display_name {
                new_cfg.display_name = v;
            }
            if let Some(v) = body.cwd {
                new_cfg.cwd = v;
            }
            if let Some(v) = body.provider {
                new_cfg.provider = ProviderId::new(v);
            }
            if let Some(v) = body.model {
                new_cfg.model = Some(v);
            }
            if let Some(v) = body.backend {
                new_cfg.backend = BackendId::from(v);
            }
            if let Some(v) = body.environment {
                new_cfg.environment = v;
            }
            if let Some(v) = body.permissions {
                new_cfg.permissions = v;
            }
            if let Some(g) = group {
                new_cfg.group = Some(g);
            }
            // Invariant 17: a rename leaves the old name behind as an alias,
            // so `@old-name` written yesterday still resolves tomorrow.
            if new_cfg.display_name != old_cfg.display_name {
                new_cfg.name_aliases.retain(|a| a != &new_cfg.display_name);
                if !new_cfg.name_aliases.contains(&old_cfg.display_name) {
                    new_cfg.name_aliases.push(old_cfg.display_name.clone());
                }
            }

            if new_cfg == old_cfg {
                // Invariant 37: a no-op says so — no version bump, no rev
                // bump, no events.
                return finish(
                    &slot_w,
                    PatchOutcome::Noop { body: worker_body(&row) },
                    no_write(),
                );
            }

            let worker_id = WorkerId::parse(&row.id).map_err(corrupt)?;
            let live = queries::live_session_for(conn, &row.id)?;
            let current_session = live.as_ref().and_then(|s| SessionId::parse(&s.id).ok());

            // Classification + application via core (`apply_config` calls
            // `classify_config_change` and escalates to the strongest mode).
            let mut core_worker =
                Worker::new(worker_id.clone(), old_cfg, WorkerCapabilities::default());
            core_worker.state = row.state.clone();
            core_worker.version = row.version;
            let now = chrono::Utc::now();
            let (updated, change) = apply_config(
                core_worker,
                new_cfg.clone(),
                &provider_caps(new_cfg.provider.as_str()),
                now,
                current_session,
                || SessionId::from_ulid(ulid::Ulid::new()),
            );
            let now_s = now.to_rfc3339();

            let n = queries::update_worker_config(conn, &row.id, &new_cfg, row.version, &now_s)?;
            if n == 0 {
                // Unreachable under the single-writer (we read the version in
                // this same transaction), kept because the guarded UPDATE is
                // the real optimistic check when these queries compose
                // elsewhere.
                return finish(
                    &slot_w,
                    PatchOutcome::Conflict { current_version: row.version },
                    no_write(),
                );
            }

            let mut events = Vec::new();
            if updated.state != row.state {
                queries::update_worker_state(conn, &row.id, &updated.state, &now_s)?;
                events.push(ev(
                    EntityType::Worker,
                    &row.id,
                    MutationKind::StatusChanged {
                        from: state_tag(&row.state),
                        to: state_tag(&updated.state),
                    },
                ));
            } else {
                events.push(ev(EntityType::Worker, &row.id, MutationKind::Updated));
            }

            // Session replacement bookkeeping (Invariant 43). The actual
            // process swap arrives with the orchestrator (RR-0041); the
            // durable record — old session ended as Replaced, new session
            // row Starting — is written here so the swap is one auditable
            // event, never an unpaired death and birth.
            if change.session_replaced {
                if let (Some(old_ses), Some(new_ses)) = (&change.old_session, &change.new_session)
                {
                    queries::end_session(conn, old_ses.as_str(), &ExitReason::Replaced, &now_s)?;
                    events.push(ev(EntityType::Session, old_ses.as_str(), MutationKind::Updated));
                    queries::insert_session(
                        conn,
                        &SessionRow {
                            id: new_ses.as_str().to_string(),
                            worker_id: row.id.clone(),
                            backend: new_cfg.backend.as_str().to_string(),
                            backend_ref: backend_ref(&worker_id),
                            pid: None,
                            started_at: now_s.clone(),
                            ended_at: None,
                            exit_reason: None,
                        },
                    )?;
                    events.push(ev(EntityType::Session, new_ses.as_str(), MutationKind::Created));
                }
            }

            let mut new_row = row.clone();
            new_row.set_config(&new_cfg);
            new_row.version = row.version + 1;
            new_row.state = updated.state.clone();
            new_row.updated_at = now_s;
            finish(
                &slot_w,
                PatchOutcome::Applied { body: worker_body(&new_row), change },
                WriteOutcome { applied: true, events },
            )
        })
        .await;

    let reply = match write {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let outcome = slot.lock().expect("outcome slot poisoned").take();
    match outcome {
        None => internal("patch produced no outcome"),
        Some(PatchOutcome::NotFound) => not_found(&key),
        Some(PatchOutcome::Conflict { current_version }) => err(
            StatusCode::CONFLICT,
            json!({ "error": "version conflict", "current_version": current_version }),
        ),
        Some(PatchOutcome::Noop { body }) => {
            let mut v = alias_fields(body, FieldStyle::default());
            v["applied"] = json!(false);
            v["change"] = Value::Null;
            Json(v).into_response()
        }
        Some(PatchOutcome::Applied { body, change }) => {
            let mut v = alias_fields(body, FieldStyle::default());
            v["applied"] = json!(true);
            v["change"] = serde_json::to_value(&change).unwrap_or(Value::Null);
            v["rev"] = json!(reply.rev.0);
            Json(v).into_response()
        }
    }
}

// ---- start / stop / delete ----------------------------------------------

enum StepOutcome {
    NotFound,
    /// The requested transition is illegal from the current state -> 409.
    Refused { error: &'static str, state: String },
    Applied { body: Value },
    /// Already in the requested state -> honest no-op (Invariant 37).
    Noop { body: Value },
}

/// POST /api/workers/{id}/start — 202 Accepted. Writes the durable record of
/// the start (worker -> Starting, a live session row, events); the actual
/// process spawn is the orchestrator's job (RR-0041) and lands there — this
/// endpoint accepts the request, it does not pretend the process exists.
pub async fn start_worker(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    let slot: Arc<Mutex<Option<StepOutcome>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let key_w = key.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = queries::get_worker(conn, &key_w)? else {
                return finish(&slot_w, StepOutcome::NotFound, no_write());
            };
            if !matches!(row.state, WorkerState::Stopped) {
                return finish(
                    &slot_w,
                    StepOutcome::Refused {
                        error: "worker is not stopped",
                        state: state_tag(&row.state),
                    },
                    no_write(),
                );
            }
            let worker_id = WorkerId::parse(&row.id).map_err(corrupt)?;
            let ses_id = SessionId::from_ulid(ulid::Ulid::new());
            let now_s = chrono::Utc::now().to_rfc3339();
            queries::insert_session(
                conn,
                &SessionRow {
                    id: ses_id.as_str().to_string(),
                    worker_id: row.id.clone(),
                    backend: row.backend.clone(),
                    backend_ref: backend_ref(&worker_id),
                    pid: None,
                    started_at: now_s.clone(),
                    ended_at: None,
                    exit_reason: None,
                },
            )?;
            queries::update_worker_state(conn, &row.id, &WorkerState::Starting, &now_s)?;
            let events = vec![
                ev(
                    EntityType::Worker,
                    &row.id,
                    MutationKind::StatusChanged { from: "stopped".into(), to: "starting".into() },
                ),
                ev(EntityType::Session, ses_id.as_str(), MutationKind::Created),
            ];
            finish(
                &slot_w,
                StepOutcome::Applied {
                    body: json!({
                        "session": "starting",
                        "session_id": ses_id.as_str(),
                        "worker_id": row.id,
                        "state": "starting",
                    }),
                },
                WriteOutcome { applied: true, events },
            )
        })
        .await;
    step_response(write, slot, &key, StatusCode::ACCEPTED)
}

/// POST /api/workers/{id}/stop — worker -> Stopped; the live session row (if
/// any) is ended as Killed. Stopping a stopped worker is an honest no-op.
pub async fn stop_worker(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    let slot: Arc<Mutex<Option<StepOutcome>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let key_w = key.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = queries::get_worker(conn, &key_w)? else {
                return finish(&slot_w, StepOutcome::NotFound, no_write());
            };
            let live = queries::live_session_for(conn, &row.id)?;
            if matches!(row.state, WorkerState::Stopped) && live.is_none() {
                return finish(
                    &slot_w,
                    StepOutcome::Noop {
                        body: json!({ "applied": false, "state": "stopped", "worker_id": row.id }),
                    },
                    no_write(),
                );
            }
            let now_s = chrono::Utc::now().to_rfc3339();
            let mut events = Vec::new();
            if let Some(ses) = live {
                queries::end_session(conn, &ses.id, &ExitReason::Killed, &now_s)?;
                events.push(ev(EntityType::Session, &ses.id, MutationKind::Updated));
            }
            if !matches!(row.state, WorkerState::Stopped) {
                queries::update_worker_state(conn, &row.id, &WorkerState::Stopped, &now_s)?;
                events.push(ev(
                    EntityType::Worker,
                    &row.id,
                    MutationKind::StatusChanged {
                        from: state_tag(&row.state),
                        to: "stopped".into(),
                    },
                ));
            }
            finish(
                &slot_w,
                StepOutcome::Applied {
                    body: json!({ "applied": true, "state": "stopped", "worker_id": row.id }),
                },
                WriteOutcome { applied: true, events },
            )
        })
        .await;
    step_response(write, slot, &key, StatusCode::OK)
}

/// DELETE /api/workers/{id} — soft delete, only from Stopped (a running
/// worker must be stopped first; 409 otherwise). The row survives for the
/// audit/session history hanging off it; it just stops resolving.
pub async fn delete_worker(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    let slot: Arc<Mutex<Option<StepOutcome>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let key_w = key.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = queries::get_worker(conn, &key_w)? else {
                return finish(&slot_w, StepOutcome::NotFound, no_write());
            };
            if !matches!(row.state, WorkerState::Stopped) {
                return finish(
                    &slot_w,
                    StepOutcome::Refused {
                        error: "worker is not stopped; stop it before deleting",
                        state: state_tag(&row.state),
                    },
                    no_write(),
                );
            }
            let now_s = chrono::Utc::now().to_rfc3339();
            let n = queries::soft_delete_worker(conn, &row.id, &now_s)?;
            if n == 0 {
                // Raced with another delete inside the same writer queue:
                // already gone, report absence rather than a fresh change.
                return finish(&slot_w, StepOutcome::NotFound, no_write());
            }
            finish(
                &slot_w,
                StepOutcome::Applied { body: json!({ "deleted": true, "id": row.id }) },
                WriteOutcome {
                    applied: true,
                    events: vec![ev(EntityType::Worker, &row.id, MutationKind::Deleted)],
                },
            )
        })
        .await;
    step_response(write, slot, &key, StatusCode::OK)
}

fn step_response(
    write: anyhow::Result<crate::db::WriteReply>,
    slot: Arc<Mutex<Option<StepOutcome>>>,
    key: &str,
    applied_status: StatusCode,
) -> Response {
    let reply = match write {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let outcome = slot.lock().expect("outcome slot poisoned").take();
    match outcome {
        None => internal("mutation produced no outcome"),
        Some(StepOutcome::NotFound) => not_found(key),
        Some(StepOutcome::Refused { error, state }) => err(
            StatusCode::CONFLICT,
            json!({ "error": error, "state": state }),
        ),
        Some(StepOutcome::Noop { body }) => (StatusCode::OK, Json(body)).into_response(),
        Some(StepOutcome::Applied { mut body }) => {
            body["rev"] = json!(reply.rev.0);
            (applied_status, Json(body)).into_response()
        }
    }
}

// ---- GET /api/workers/{id}/peek -----------------------------------------

/// Peek needs a live terminal backend to read from; that lands with the
/// backend adapters. Until then this is an honest 501 naming what delivers
/// it — not a fake empty payload a caller would mistake for "no output"
/// (the exact misread the Python peek's viewport bug taught us to fear).
pub async fn peek_worker(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    let store = state.store.clone();
    let k = key.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(queries::get_worker(&conn, &k)?)
    })
    .await;
    match joined {
        Ok(Ok(Some(row))) => err(
            StatusCode::NOT_IMPLEMENTED,
            json!({
                "error": "peek is not implemented yet",
                "lands_with": "RR-0031/RR-0032 — terminal backend adapters (Phase 1)",
                "worker_id": row.id,
            }),
        ),
        Ok(Ok(None)) => not_found(&key),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{router, AppState};
    use crate::db::Store;
    use axum::body::Body;
    use axum::http::{header, HeaderMap, Request, StatusCode};
    use tower::ServiceExt;

    fn app_with_token(token: Option<String>) -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("amux-test.db")).unwrap();
        let state = AppState {
            store: std::sync::Arc::new(store),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: token,
        };
        (router(state), dir)
    }

    fn app() -> (axum::Router, tempfile::TempDir) {
        app_with_token(None)
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        send_with(app, method, path, body, &[]).await
    }

    async fn send_with(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Value) {
        let mut b = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let req = match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, headers, v)
    }

    async fn create(app: &axum::Router, name: &str) -> String {
        let (st, _, body) = send(
            app,
            "POST",
            "/api/workers",
            Some(json!({ "display_name": name, "cwd": "/tmp/w" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "create failed: {body}");
        body["id"].as_str().unwrap().to_string()
    }

    async fn health_rev(app: &axum::Router) -> u64 {
        let (st, _, body) = send(app, "GET", "/health", None).await;
        assert_eq!(st, StatusCode::OK);
        body["rev"].as_u64().unwrap()
    }

    // ---- RR-0034 test list ----------------------------------------------

    #[tokio::test]
    async fn create_get_rename_then_alias_resolves() {
        let (app, _dir) = app();
        let (st, _, created) = send(
            &app,
            "POST",
            "/api/workers",
            Some(json!({ "display_name": "backend", "cwd": "/tmp/x" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        let id = created["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("wrk_"));
        assert_eq!(created["version"], json!(0));
        assert_eq!(created["state"]["state"], json!("stopped"));

        let (st, _, by_id) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(by_id["id"].as_str().unwrap(), id);
        let (st, _, by_name) = send(&app, "GET", "/api/workers/backend", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(by_name["id"].as_str().unwrap(), id);

        // Rename (RR-0035): Immediate, version bumps, old name becomes alias.
        let (st, _, patched) = send(
            &app,
            "PATCH",
            &format!("/api/workers/{id}"),
            Some(json!({ "display_name": "rust-backend", "expect_version": 0 })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{patched}");
        assert_eq!(patched["applied"], json!(true));
        assert_eq!(patched["version"], json!(1));
        assert_eq!(patched["change"]["mode"], json!("immediate"));
        assert_eq!(patched["change"]["session_replaced"], json!(false));
        assert!(patched["name_aliases"]
            .as_array()
            .unwrap()
            .contains(&json!("backend")));

        // Invariant 17: the OLD name still resolves, to the SAME id.
        let (st, _, by_alias) = send(&app, "GET", "/api/workers/backend", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(by_alias["id"].as_str().unwrap(), id);
        assert_eq!(by_alias["display_name"], json!("rust-backend"));
    }

    #[tokio::test]
    async fn stale_expect_version_is_409_and_applies_nothing() {
        let (app, _dir) = app();
        let id = create(&app, "w").await;
        // Move to version 1.
        let (st, _, _) = send(
            &app,
            "PATCH",
            &format!("/api/workers/{id}"),
            Some(json!({ "display_name": "w2", "expect_version": 0 })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);

        // Stale write against version 0.
        let (st, _, body) = send(
            &app,
            "PATCH",
            &format!("/api/workers/{id}"),
            Some(json!({ "cwd": "/stale/view", "expect_version": 0 })),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(body["error"], json!("version conflict"));
        assert_eq!(body["current_version"], json!(1));

        // Nothing changed.
        let (_, _, back) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(back["cwd"], json!("/tmp/w"));
        assert_eq!(back["version"], json!(1));
    }

    #[tokio::test]
    async fn noop_patch_reports_unapplied_and_bumps_nothing() {
        let (app, _dir) = app();
        let id = create(&app, "w").await;
        let rev_before = health_rev(&app).await;

        // Same values -> no-op (Invariant 37).
        let (st, _, body) = send(
            &app,
            "PATCH",
            &format!("/api/workers/{id}"),
            Some(json!({ "display_name": "w", "cwd": "/tmp/w" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["applied"], json!(false));
        assert_eq!(body["change"], Value::Null);
        assert_eq!(body["version"], json!(0)); // NOT bumped
        assert!(body.get("rev").is_none()); // a no-op carries no new rev

        // Read back: entity version AND global rev both unmoved.
        let (_, _, back) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(back["version"], json!(0));
        assert_eq!(health_rev(&app).await, rev_before);
    }

    #[tokio::test]
    async fn start_stop_delete_lifecycle() {
        let (app, _dir) = app();
        let id = create(&app, "w").await;

        // Start: 202, durable Starting record.
        let (st, _, body) = send(&app, "POST", &format!("/api/workers/{id}/start"), None).await;
        assert_eq!(st, StatusCode::ACCEPTED);
        assert_eq!(body["session"], json!("starting"));
        assert!(body["session_id"].as_str().unwrap().starts_with("ses_"));
        let (_, _, back) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(back["state"]["state"], json!("starting"));

        // Delete while running: 409.
        let (st, _, body) = send(&app, "DELETE", &format!("/api/workers/{id}"), None).await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(body["state"], json!("starting"));
        // Double-start: 409 too.
        let (st, _, _) = send(&app, "POST", &format!("/api/workers/{id}/start"), None).await;
        assert_eq!(st, StatusCode::CONFLICT);

        // Stop: applied, back to stopped, live session ended.
        let (st, _, body) = send(&app, "POST", &format!("/api/workers/{id}/stop"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["applied"], json!(true));
        let (_, _, back) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(back["state"]["state"], json!("stopped"));

        // Stopping again: honest no-op (Invariant 37).
        let (st, _, body) = send(&app, "POST", &format!("/api/workers/{id}/stop"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["applied"], json!(false));

        // Delete now succeeds (soft), and the worker stops resolving.
        let (st, _, body) = send(&app, "DELETE", &format!("/api/workers/{id}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body["deleted"], json!(true));
        let (st, _, _) = send(&app, "GET", &format!("/api/workers/{id}"), None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        let (_, _, list) = send(&app, "GET", "/api/workers", None).await;
        assert_eq!(list["total"], json!(0));
    }

    #[tokio::test]
    async fn cwd_change_with_live_session_replaces_it_atomically() {
        // Invariant 43: cwd is process-level -> SessionRestart; with a live
        // session the ONE result carries both ids and the DB shows the old
        // session ended as Replaced and the new one live.
        let (app, dir) = app();
        let id = create(&app, "w").await;
        let (st, _, started) = send(&app, "POST", &format!("/api/workers/{id}/start"), None).await;
        assert_eq!(st, StatusCode::ACCEPTED);
        let first_ses = started["session_id"].as_str().unwrap().to_string();

        let (st, _, body) = send(
            &app,
            "PATCH",
            &format!("/api/workers/{id}"),
            Some(json!({ "cwd": "/somewhere/else", "expect_version": 0 })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["applied"], json!(true));
        assert_eq!(body["change"]["mode"], json!("session_restart"));
        assert_eq!(body["change"]["session_replaced"], json!(true));
        let old_ses = body["change"]["old_session"].as_str().unwrap().to_string();
        let new_ses = body["change"]["new_session"].as_str().unwrap().to_string();
        assert_eq!(old_ses, first_ses);
        assert_ne!(old_ses, new_ses);

        // Verify the durable record directly against the store's DB file.
        let conn = rusqlite::Connection::open(dir.path().join("amux-test.db")).unwrap();
        let live = queries::live_session_for(&conn, &id).unwrap().unwrap();
        assert_eq!(live.id, new_ses);
        assert_eq!(
            queries::live_session_for(&conn, &id).unwrap().unwrap().ended_at,
            None
        );
        let reason: String = conn
            .query_row(
                "SELECT exit_reason FROM _amux_sessions WHERE id = ?1",
                rusqlite::params![old_ses],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<ExitReason>(&reason).unwrap(),
            ExitReason::Replaced
        );
    }

    // ---- RR-0018a wiring -------------------------------------------------

    #[tokio::test]
    async fn legacy_sessions_route_serves_workers_with_deprecated_header() {
        let (app, _dir) = app();
        let id = create(&app, "w").await;

        let (st, headers, canonical) = send(&app, "GET", "/api/workers", None).await;
        assert_eq!(st, StatusCode::OK);
        assert!(headers.get("deprecated").is_none());
        assert_eq!(canonical["total"], json!(1));
        assert_eq!(canonical["truncated"], json!(false)); // PagedResponse shape

        // Bare /api/sessions now serves the PYTHON SHAPE (bare array from
        // the dedicated handler, no Deprecated header) — the SPA's
        // fetchSessions throws on anything else (browser-golden finding #3).
        let (st, headers, legacy) = send(&app, "GET", "/api/sessions", None).await;
        assert_eq!(st, StatusCode::OK);
        assert!(headers.get("deprecated").is_none());
        let arr = legacy.as_array().expect("bare array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], json!("w"));
        assert!(arr[0]["status"].is_string());

        // Per-session verbs now PROXY to the Python fleet owner; a
        // rust-managed worker on the legacy path gets the modern pointer,
        // never a silent Python 404.
        let (st, _headers, detail) = send(&app, "GET", "/api/sessions/w", None).await;
        assert_eq!(st, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(detail["hint"], json!("/api/workers/w"));
        // The modern path serves the detail.
        let (st, _h, detail) = send(&app, "GET", "/api/workers/w", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(detail["id"].as_str().unwrap(), id);
    }

    #[tokio::test]
    async fn conflicting_name_fields_are_400_and_legacy_name_is_accepted() {
        let (app, _dir) = app();
        // Both spellings, different values: 400 naming both (never a silent
        // winner — Invariant 37 via RR-0018a).
        let (st, _, body) = send(
            &app,
            "POST",
            "/api/workers",
            Some(json!({ "display_name": "a", "name": "b" })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(body["display_name"], json!("a"));
        assert_eq!(body["name"], json!("b"));

        // The legacy spelling alone works.
        let (st, _, body) = send(
            &app,
            "POST",
            "/api/workers",
            Some(json!({ "name": "legacy-created" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        assert_eq!(body["display_name"], json!("legacy-created"));
    }

    // ---- auth + peek ------------------------------------------------------

    #[tokio::test]
    async fn worker_routes_sit_behind_auth_including_legacy_paths() {
        let (app, _dir) = app_with_token(Some("sekrit".into()));
        let (st, _, _) = send(&app, "GET", "/api/workers", None).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        // The legacy alias must not be an auth bypass.
        let (st, _, _) = send(&app, "GET", "/api/sessions", None).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        let (st, _, _) = send_with(
            &app,
            "GET",
            "/api/workers",
            None,
            &[("authorization", "Bearer sekrit")],
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }

    #[tokio::test]
    async fn peek_is_honestly_unimplemented() {
        let (app, _dir) = app();
        let id = create(&app, "w").await;
        let (st, _, body) = send(&app, "GET", &format!("/api/workers/{id}/peek"), None).await;
        assert_eq!(st, StatusCode::NOT_IMPLEMENTED);
        assert!(body["lands_with"].as_str().unwrap().contains("RR-0031"));
        // Unknown worker is still a 404, not a 501.
        let (st, _, _) = send(&app, "GET", "/api/workers/ghost/peek", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
}
