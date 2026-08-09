//! Board API (RR-0049 routes + 409 gate contract + force audit; RR-0055
//! archive/restore; the list shape RR-0053's auto-capture will write into).
//!
//! Mounted at `/api/board` inside the `protected` router (api/mod.rs). This
//! is the STRANGLER-FIG surface: it serves the same `issues` rows the Python
//! server serves, in the same shapes the Python dashboard/CLI already parse —
//! a bare JSON array from the list, the `gate not acknowledged` 409 body the
//! CLI's `--checked` flow is built around, `X-Amux-Truncated` headers on the
//! capped list. Interop mappings live in `db::board_store`.
//!
//! Every status change routes through core's `apply_transition` — one state
//! machine, one code path (Invariant 3); nothing here hand-rolls a status
//! write. Gate refusals carry core's `WhyBlocked` list alongside the Python
//! keys, force bypasses are audited into the card's own log (ethos rule 6:
//! the Python board claimed force-is-logged while nothing logged it), and
//! no-op PATCHes report `applied: false` with `rev` unmoved (Invariant 37).

use super::AppState;
use crate::db::board_store::{self as bs, ArchivedFilter, IssueRow};
use crate::db::{PendingEvent, WriteOutcome};
use amux_core::board::{
    apply_transition, why_blocked, BoardTransition, TaskStatus, TransitionError,
};
use amux_core::events::Actor;
use amux_core::revision::{EntityType, MutationKind};
use amux_core::verification::{Evidence, EvidenceKind, EvidenceSource};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_board).post(create_item))
        .route("/{id}", get(get_item).patch(patch_item))
        .route("/{id}/archive", post(archive_item))
        .route("/{id}/restore", post(restore_item))
}

// ---- shared helpers ------------------------------------------------------

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": e.to_string() }),
    )
}

fn not_found(id: &str) -> Response {
    err(
        StatusCode::NOT_FOUND,
        json!({ "error": "item not found", "id": id }),
    )
}

fn no_write() -> WriteOutcome {
    WriteOutcome {
        applied: false,
        events: Vec::new(),
    }
}

fn ev(id: &str, mutation: MutationKind) -> PendingEvent {
    PendingEvent {
        entity_type: EntityType::Task,
        entity_id: id.to_string(),
        mutation,
    }
}

fn finish<T>(
    slot: &Mutex<Option<T>>,
    outcome: T,
    write: WriteOutcome,
) -> rusqlite::Result<WriteOutcome> {
    *slot.lock().expect("outcome slot poisoned") = Some(outcome);
    Ok(write)
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Local HH:MM, matching Python's `time.strftime("%H:%M")` log stamps.
fn hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// The verified caller identity from `X-Amux-Session` (AMUX-1768: provenance
/// is the header, never body text). Returns (core actor, log display name).
/// No worker registry lookup exists yet, so a named caller maps to
/// `Actor::System{component: <name>}` — honest about being unverified-as-a-
/// Worker while still carrying the name into every audit line.
fn actor_from_headers(headers: &HeaderMap) -> (Actor, String) {
    match headers
        .get("x-amux-session")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => (
            Actor::System {
                component: name.to_string(),
            },
            name.to_string(),
        ),
        None => (
            Actor::System {
                component: "api-anonymous".into(),
            },
            "api-anonymous".into(),
        ),
    }
}

// ---- body shapes ---------------------------------------------------------

/// Full detail body: everything, full `desc`, full `log` (L1: the full desc
/// is never in a LIST payload; it is always here).
fn detail_body(row: &IssueRow) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "desc": row.desc,
        "status": row.status,
        "session": row.session,
        "shepherd": row.shepherd,
        "type": row.item_type,
        "creator": row.creator,
        "due": row.due,
        "due_time": row.due_time,
        "created": row.created,
        "updated": row.updated,
        "owner_type": row.owner_type,
        "pinned": row.pinned,
        "pos": row.pos,
        "archived": row.archived,
        "depends_on": row.depends_on,
        "reviewer": row.reviewer,
        "log": row.log,
        "source_ref": row.source_ref,
        "last_verified_at": row.last_verified_at,
        "rev": row.rev,
        "gate": row.gate_criteria(),
        "tags": row.tags,
        "version": row.version,
    })
}

/// List body (L1, ported from Python `_board_slim_desc` + `_board_project`):
/// desc truncated to its first line (max 200 chars) with `desc_truncated`
/// set when it was cut; `log` never ships in a list — `log_n` (line count)
/// stands in. `slim` additionally drops desc and adds `desc_len`.
fn list_body(row: &IssueRow, slim: bool) -> Value {
    let mut v = detail_body(row);
    let obj = v.as_object_mut().expect("detail_body is an object");
    obj.remove("log");
    let log_n = row
        .log
        .as_deref()
        .map(|l| l.lines().filter(|x| !x.trim().is_empty()).count())
        .unwrap_or(0);
    obj.insert("log_n".into(), json!(log_n));
    if slim {
        obj.remove("desc");
        obj.insert("desc_len".into(), json!(row.desc.chars().count()));
    } else if row.desc.chars().count() > 200 {
        let first: String = row
            .desc
            .split('\n')
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        obj.insert("desc".into(), json!(first));
        obj.insert("desc_truncated".into(), json!(true));
    }
    v
}

// ---- GET /api/board ------------------------------------------------------

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub archived: Option<String>,
    #[serde(default)]
    pub done_limit: Option<i64>,
    #[serde(default)]
    pub slim: Option<String>,
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("true") | Some("yes"))
}

/// Bare JSON ARRAY (the Python dashboard parses exactly that shape). The
/// terminal cap ALWAYS announces itself via the header quartet the Python
/// server emits (`X-Amux-Done-Limit`/`-Truncated`/`-Terminal-Total`/
/// `-Terminal-Returned`) — a silent cap manufactured wrong absence claims
/// twice in one week (AC-291, AC-301), so the two counts come from
/// `cap_terminal` itself, never re-derived from list lengths.
pub async fn list_board(State(state): State<AppState>, Query(p): Query<ListParams>) -> Response {
    let split = |s: &Option<String>| -> Vec<String> {
        s.as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    };
    let status_f = split(&p.status);
    let session_f = split(&p.session);
    let archived = match p.archived.as_deref().map(str::trim) {
        Some("all") | Some("") => ArchivedFilter::All,
        Some(v) if truthy(Some(v)) => ArchivedFilter::ArchivedOnly,
        // Default 0: active cards only.
        _ => ArchivedFilter::ActiveOnly,
    };
    let done_limit = p.done_limit.unwrap_or(100).max(0);
    let slim = truthy(p.slim.as_deref());

    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(bs::list_issues(&conn, &status_f, &session_f, archived)?)
    })
    .await;
    let rows = match joined {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e),
    };
    let (kept, term_total, term_kept) = bs::cap_terminal(rows, done_limit);
    let items: Vec<Value> = kept.iter().map(|r| list_body(r, slim)).collect();

    let mut headers = HeaderMap::new();
    let put = |h: &mut HeaderMap, k: &'static str, v: String| {
        if let Ok(val) = v.parse() {
            h.insert(k, val);
        }
    };
    put(&mut headers, "x-amux-done-limit", done_limit.to_string());
    put(
        &mut headers,
        "x-amux-truncated",
        if term_total > term_kept { "1" } else { "0" }.to_string(),
    );
    put(&mut headers, "x-amux-terminal-total", term_total.to_string());
    put(
        &mut headers,
        "x-amux-terminal-returned",
        term_kept.to_string(),
    );
    (StatusCode::OK, headers, Json(Value::Array(items))).into_response()
}

// ---- request-value helpers (bodies are raw maps: the Python dashboard
// PATCHes whole item objects, so deny_unknown_fields would break the UI;
// unknown keys are collected and REPORTED as `ignored_fields` instead of
// silently dropped — the narrower truth Invariant 37 actually needs) -------

fn body_str(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// A nullable string field: `None` = absent, `Some(None)` = explicit null
/// (clear it), `Some(Some(s))` = set.
fn body_opt_str(map: &Map<String, Value>, key: &str) -> Option<Option<String>> {
    match map.get(key) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(v.as_str().map(str::to_string)),
    }
}

/// tags/depends_on style list: array of strings; a bare string is coerced to
/// a one-element list (SP-539: iterating a str exploded it into one tag per
/// character — 200, no error, silently corrupted card).
fn body_str_list(v: &Value) -> Result<Vec<String>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(if s.trim().is_empty() {
            Vec::new()
        } else {
            vec![s.clone()]
        }),
        Value::Array(a) => {
            let mut out = Vec::new();
            for x in a {
                match x.as_str() {
                    Some(s) if !s.trim().is_empty() => out.push(s.trim().to_string()),
                    Some(_) => {}
                    None => return Err("must be a list of strings".into()),
                }
            }
            Ok(out)
        }
        _ => Err("must be a list of strings".into()),
    }
}

fn unknown_type_response(t: &str) -> Response {
    err(
        StatusCode::BAD_REQUEST,
        json!({
            "error": format!("unknown type {t:?}"),
            "valid_types": bs::KNOWN_TYPES,
            "why": "The gate is DERIVED from type. An unknown type would silently fall back \
                    to the strictest (code) gate, which non-code work cannot satisfy without \
                    asserting a merge that never happened.",
        }),
    )
}

fn cycle_response(cycle: &[String]) -> Response {
    err(
        StatusCode::BAD_REQUEST,
        json!({
            "error": format!("circular depends_on: {}", cycle.join(" -> ")),
            "cycle": cycle,
        }),
    )
}

const VALID_STATUSES: [&str; 11] = [
    "backlog",
    "todo",
    "doing",
    "review",
    "needsyou",
    "blocked",
    "done",
    "verified",
    "discarded",
    "armed",
    "quarantined",
];

// ---- POST /api/board -----------------------------------------------------

pub async fn create_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(map) = body.as_object().cloned() else {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": "body must be a JSON object" }),
        );
    };
    let title = body_str(&map, "title").unwrap_or_default().trim().to_string();
    if title.is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "missing title" }));
    }

    // MO-3038: when the body OMITS `session` and the verified header is
    // present, the card is for the sender's own lane. An EXPLICIT value —
    // including explicit "" / null for a deliberately unassigned card — is
    // always respected.
    let (_, hdr_name) = actor_from_headers(&headers);
    let hdr_session = if hdr_name == "api-anonymous" {
        String::new()
    } else {
        hdr_name.clone()
    };
    let session = if map.contains_key("session") {
        body_str(&map, "session").unwrap_or_default().trim().to_string()
    } else {
        hdr_session.chars().take(64).collect()
    };

    let status_in = body_str(&map, "status").unwrap_or_else(|| "todo".into());
    let Some(status) = bs::parse_status(&status_in) else {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("unknown status {status_in:?}"), "valid_statuses": VALID_STATUSES }),
        );
    };
    let status_raw = bs::db_status_spelling(status).to_string();

    let item_type = body_str(&map, "type")
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "code".into());
    if !bs::KNOWN_TYPES.contains(&item_type.as_str()) {
        return unknown_type_response(&item_type);
    }

    let depends_on = match map.get("depends_on") {
        None => Vec::new(),
        Some(v) => match body_str_list(v) {
            Ok(l) => l,
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": format!("depends_on {e}") }),
                )
            }
        },
    };
    let tags = match map.get("tags") {
        None => Vec::new(),
        Some(v) => match body_str_list(v) {
            Ok(l) => l,
            Err(e) => {
                return err(StatusCode::BAD_REQUEST, json!({ "error": format!("tags {e}") }))
            }
        },
    };
    let gate = match map.get("gate") {
        None => Vec::new(),
        Some(v) => body_str_list(v).unwrap_or_default(),
    };

    // Creator attribution (AMUX-1812): the body value is a self-reported
    // CLAIM; the verified header wins, and a disagreement is recorded.
    let claimed = body_str(&map, "creator").unwrap_or_default().trim().to_string();
    let creator = match (&hdr_session.is_empty(), claimed.is_empty()) {
        (false, false) if hdr_session != claimed => format!("{hdr_session} (claimed {claimed})"),
        (false, _) => hdr_session.clone(),
        (true, false) => claimed,
        (true, true) => String::new(),
    };

    let owner_type = match body_str(&map, "owner_type").as_deref() {
        Some("human") => "human".to_string(),
        Some("agent") => "agent".to_string(),
        Some(_) => "human".to_string(),
        None => if session.is_empty() { "human" } else { "agent" }.to_string(),
    };

    let known_keys = [
        "title", "desc", "status", "session", "type", "depends_on", "tags", "creator",
        "reviewer", "shepherd", "gate", "owner_type", "due", "due_time",
    ];
    let ignored: Vec<String> = map
        .keys()
        .filter(|k| !known_keys.contains(&k.as_str()))
        .cloned()
        .collect();

    let new = bs::NewIssue {
        title,
        desc: body_str(&map, "desc").unwrap_or_default(),
        status: status_raw,
        session: Some(session).filter(|s| !s.is_empty()),
        item_type,
        creator,
        owner_type,
        due: body_str(&map, "due").filter(|s| !s.trim().is_empty()),
        due_time: body_str(&map, "due_time").filter(|s| !s.trim().is_empty()),
        reviewer: body_str(&map, "reviewer").filter(|s| !s.trim().is_empty()),
        shepherd: body_str(&map, "shepherd").filter(|s| !s.trim().is_empty()),
        gate,
        depends_on,
        tags,
    };

    enum Out {
        Cycle(Vec<String>),
        Created(Box<IssueRow>),
    }
    let slot: Arc<Mutex<Option<Out>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let write = state
        .store
        .write_async(move |conn| {
            // Acyclicity is validated INSIDE the write so no interleaved
            // create can slip a cycle between check and insert. The new id
            // does not exist yet, so a placeholder self id is fine — only
            // edges out of it are being added.
            if !new.depends_on.is_empty() {
                if let Some(cycle) = bs::depends_on_cycle(conn, "\u{0}new-card", &new.depends_on)? {
                    return finish(&slot_w, Out::Cycle(cycle), no_write());
                }
            }
            let row = bs::create_issue(conn, &new, now_secs())?;
            let id = row.id.clone();
            finish(
                &slot_w,
                Out::Created(Box::new(row)),
                WriteOutcome {
                    applied: true,
                    events: vec![ev(&id, MutationKind::Created)],
                },
            )
        })
        .await;
    let reply = match write {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let outcome = slot.lock().expect("outcome slot poisoned").take();
    match outcome {
        None => internal("create produced no outcome"),
        Some(Out::Cycle(cycle)) => cycle_response(&cycle),
        Some(Out::Created(row)) => {
            let mut v = detail_body(&row);
            v["rev"] = json!(row.rev);
            v["global_rev"] = json!(reply.rev.0);
            if !ignored.is_empty() {
                v["ignored_fields"] = json!(ignored);
            }
            (StatusCode::CREATED, Json(v)).into_response()
        }
    }
}

// ---- GET /api/board/{id} -------------------------------------------------

pub async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let store = state.store.clone();
    let key = id.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store.read()?;
        Ok(bs::get_issue(&conn, &key)?)
    })
    .await;
    match joined {
        Ok(Ok(Some(row))) => {
            // Weak ETag for read-modify-write callers (AMUX-1711 parity).
            let mut headers = HeaderMap::new();
            if let Ok(v) = format!("W/\"{}-{}\"", row.id, row.rev).parse() {
                headers.insert("etag", v);
            }
            (StatusCode::OK, headers, Json(detail_body(&row))).into_response()
        }
        Ok(Ok(None)) => not_found(&id),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---- PATCH /api/board/{id} -----------------------------------------------

/// Keys PATCH writes. Everything else lands in `ignored_fields` (reported,
/// never silently dropped — AC-263).
const PATCH_WRITABLE: [&str; 16] = [
    "title", "desc", "status", "session", "type", "depends_on", "tags", "reviewer", "shepherd",
    "due", "due_time", "owner_type", "pinned", "pos", "gate", "source_ref",
];
/// Control keys: consumed by the PATCH protocol itself, never "ignored".
const PATCH_CONTROL: [&str; 5] = ["expect_rev", "gate_ack", "gate_checked", "force", "reason"];

enum PatchOut {
    NotFound,
    /// Any pre-write refusal (400/409) with its exact body.
    Refused(StatusCode, Value),
    /// Invariant 37: nothing changed; `rev` unmoved.
    Noop { body: Value, ignored: Vec<String> },
    Applied { body: Value, ignored: Vec<String> },
}

/// Map a (from, to) pair onto the core transition vocabulary. `None` means
/// no named transition exists — the caller falls back to the gate-checked
/// generic move (the Python board allows any->any, so refusing unmapped
/// pairs outright would break live CLI flows like todo->done).
fn named_transition(
    from: TaskStatus,
    to: TaskStatus,
    evidence: Vec<Evidence>,
    reason: String,
) -> Option<BoardTransition> {
    use TaskStatus as S;
    Some(match (from, to) {
        (S::Backlog, S::Todo) => BoardTransition::Queue,
        (S::Todo, S::Backlog) => BoardTransition::Park,
        (S::Todo, S::Doing) => BoardTransition::Start,
        (S::Doing, S::Todo) => BoardTransition::Release,
        (S::Doing, S::Review) => BoardTransition::Submit,
        (S::Review, S::Done) => BoardTransition::Approve { evidence },
        (S::Review, S::Doing) => BoardTransition::Reject { reason },
        (S::Doing, S::Done) => BoardTransition::Complete { evidence },
        (S::Done, S::Verified) => BoardTransition::Verify {
            criteria: vec![],
            evidence,
        },
        (S::Done, S::Doing) => BoardTransition::VerificationFailed { reason },
        (S::Doing, S::NeedsYou) => BoardTransition::RequestInput { question: reason },
        (S::NeedsYou, S::Doing) => BoardTransition::Resume,
        (S::Todo | S::Doing, S::Blocked) => BoardTransition::Block { reason },
        (S::Blocked, S::Todo) => BoardTransition::Unblock,
        (S::Todo | S::Backlog, S::Armed) => BoardTransition::Arm,
        (S::Armed, S::Todo) => BoardTransition::Fire { reason },
        (_, S::Discarded) => BoardTransition::Discard { reason },
        (_, S::Quarantined) => BoardTransition::Quarantine { reason },
        _ => return None,
    })
}

/// Ack evidence: one `ModelTranscript` artifact per criterion, provenance
/// `SelfReported` (an ack IS self-reported — never inflate it to
/// Independent). This is what `satisfied_by` matches against the
/// `ModelJudgment` verifiers in `bs::core_gates`.
fn ack_evidence(actor: &str, criteria: &[String], via: &str) -> Vec<Evidence> {
    let now = chrono::Utc::now();
    criteria
        .iter()
        .map(|c| Evidence {
            kind: EvidenceKind::ModelTranscript,
            description: format!("acknowledged by {actor} via {via}: {c}"),
            artifact: None,
            produced_at: now,
            source: EvidenceSource::SelfReported,
        })
        .collect()
}

/// The Python-compatible gate 409 (the CLI parses `error`, `gate`,
/// `item_type`, `attempted_status`, `valid_types` — grep amux-server.py
/// "gate not acknowledged"). Core's serialized refusal rides along under
/// `why_blocked`/`kind`: it cannot be merged flat because core spells the
/// list `blocked` while the Python contract's `blocked` is the boolean the
/// CLI-side incident (orch MO-2952) made load-bearing.
fn gate_409(
    row: &IssueRow,
    eff_gate: &[String],
    target_raw: &str,
    wb: &[amux_core::board::WhyBlocked],
) -> Value {
    let checked_args = eff_gate
        .iter()
        .map(|g| format!("{:?}", g))
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "error": "gate not acknowledged",
        "ok": false,
        "blocked": true,
        "gate": eff_gate,
        "attempted_status": target_raw,
        "item": row.id,
        "item_type": row.item_type,
        "how_to_ack": {
            "gate_ack": true,
            "or_gate_checked": eff_gate,
            "contract": "GET /api/board/contract",
            "wrong_type?": "If this item has no code, set its type (escalation/blocker/investigation/ops/research/chore/doc) — the gate is DERIVED from the type. Never ack a merge that did not happen.",
        },
        "cli": format!("amux board {target_raw} {} --checked {checked_args}", row.id),
        "valid_types": bs::KNOWN_TYPES,
        "kind": "gate_blocked",
        "why_blocked": wb,
    })
}

pub async fn patch_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(map) = body.as_object().cloned() else {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": "body must be a JSON object" }),
        );
    };
    let (actor, actor_name) = actor_from_headers(&headers);
    let force_actor = if actor_name == "api-anonymous" {
        "anonymous".to_string()
    } else {
        actor_name.clone()
    };

    let slot: Arc<Mutex<Option<PatchOut>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let id_w = id.clone();

    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = bs::get_issue(conn, &id_w)? else {
                return finish(&slot_w, PatchOut::NotFound, no_write());
            };

            // Optimistic concurrency: expect_rev checks the PYTHON counter.
            // Conflict outranks everything — a stale caller must learn their
            // view is old before any other verdict.
            if let Some(exp) = map.get("expect_rev").and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            }) {
                if exp != row.rev {
                    return finish(
                        &slot_w,
                        PatchOut::Refused(
                            StatusCode::CONFLICT,
                            json!({
                                "error": "rev conflict",
                                "current_rev": row.rev,
                                "expected": exp,
                                "item": detail_body(&row),
                                "hint": "re-read, re-apply your change to the current item, retry with the new rev",
                            }),
                        ),
                        no_write(),
                    );
                }
            }

            let ignored: Vec<String> = map
                .keys()
                .filter(|k| {
                    !PATCH_WRITABLE.contains(&k.as_str()) && !PATCH_CONTROL.contains(&k.as_str())
                })
                .cloned()
                .collect();

            // ---- stage non-status field changes onto a working copy ------
            // (staged BEFORE the gate check so a PATCH changing type and
            // status together gates on the NEW type — the Python handler's
            // own rule.)
            let mut next = row.clone();
            let mut changed: Vec<String> = Vec::new();
            let mut tags_change: Option<Vec<String>> = None;

            if let Some(t) = body_str(&map, "title") {
                if t != next.title {
                    next.title = t;
                    changed.push("title".into());
                }
            }
            if let Some(d) = body_str(&map, "desc") {
                if d != next.desc {
                    next.desc = d;
                    changed.push("desc".into());
                }
            }
            // Nullable string columns: explicit null/"" clears, absent leaves.
            let set_opt =
                |key: &str, field: &mut Option<String>, changed: &mut Vec<String>| {
                    if let Some(v) = body_opt_str(&map, key) {
                        let v = v.filter(|s| !s.trim().is_empty());
                        if *field != v {
                            *field = v;
                            changed.push(key.into());
                        }
                    }
                };
            set_opt("session", &mut next.session, &mut changed);
            set_opt("reviewer", &mut next.reviewer, &mut changed);
            set_opt("shepherd", &mut next.shepherd, &mut changed);
            set_opt("due", &mut next.due, &mut changed);
            set_opt("due_time", &mut next.due_time, &mut changed);
            set_opt("source_ref", &mut next.source_ref, &mut changed);
            if let Some(ot) = body_str(&map, "owner_type") {
                let ot = if ot == "agent" { "agent" } else { "human" }.to_string();
                if ot != next.owner_type {
                    next.owner_type = ot;
                    changed.push("owner_type".into());
                }
            }
            if let Some(p) = map.get("pinned") {
                let p = match p {
                    Value::Bool(b) => i64::from(*b),
                    v => v.as_i64().unwrap_or(0),
                };
                if p != next.pinned {
                    next.pinned = p;
                    changed.push("pinned".into());
                }
            }
            if let Some(p) = map.get("pos").and_then(|v| v.as_f64()) {
                if (p - next.pos).abs() > f64::EPSILON {
                    next.pos = p;
                    changed.push("pos".into());
                }
            }
            if let Some(t) = body_str(&map, "type") {
                let t = t.trim().to_lowercase();
                if !t.is_empty() {
                    if !bs::KNOWN_TYPES.contains(&t.as_str()) {
                        // Reject at the door: an unknown type silently
                        // inherits the code gate non-code work cannot satisfy.
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::BAD_REQUEST,
                                json!({
                                    "error": format!("unknown type {t:?}"),
                                    "valid_types": bs::KNOWN_TYPES,
                                    "why": "The gate is DERIVED from type. An unknown type would silently fall back to the strictest (code) gate, which non-code work cannot satisfy without asserting a merge that never happened.",
                                }),
                            ),
                            no_write(),
                        );
                    }
                    if t != next.item_type {
                        next.item_type = t;
                        changed.push("type".into());
                    }
                }
            }
            if let Some(v) = map.get("gate") {
                let list = match body_str_list(v) {
                    Ok(l) => l,
                    Err(e) => {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::BAD_REQUEST,
                                json!({ "error": format!("gate {e}") }),
                            ),
                            no_write(),
                        )
                    }
                };
                let new_gate = if list.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&list).unwrap_or_default())
                };
                if next.gate_criteria() != list {
                    next.gate = new_gate;
                    changed.push("gate".into());
                }
            }
            if let Some(v) = map.get("depends_on") {
                let deps = match body_str_list(v) {
                    Ok(l) => l,
                    Err(e) => {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::BAD_REQUEST,
                                json!({ "error": format!("depends_on {e}") }),
                            ),
                            no_write(),
                        )
                    }
                };
                if deps != next.depends_on {
                    if let Some(cycle) = bs::depends_on_cycle(conn, &row.id, &deps)? {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::BAD_REQUEST,
                                json!({
                                    "error": format!("circular depends_on: {}", cycle.join(" -> ")),
                                    "cycle": cycle,
                                }),
                            ),
                            no_write(),
                        );
                    }
                    next.depends_on = deps;
                    changed.push("depends_on".into());
                }
            }
            if let Some(v) = map.get("tags") {
                let tags = match body_str_list(v) {
                    Ok(l) => l,
                    Err(e) => {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::BAD_REQUEST,
                                json!({ "error": format!("tags {e}") }),
                            ),
                            no_write(),
                        )
                    }
                };
                let mut a = tags.clone();
                let mut b = next.tags.clone();
                a.sort();
                b.sort();
                if a != b {
                    next.tags = tags.clone();
                    tags_change = Some(tags);
                    changed.push("tags".into());
                }
            }

            // ---- status transition through the core state machine --------
            let mut status_event: Option<(String, String)> = None;
            if let Some(target_in) = body_str(&map, "status") {
                let Some(target) = bs::parse_status(&target_in) else {
                    return finish(
                        &slot_w,
                        PatchOut::Refused(
                            StatusCode::BAD_REQUEST,
                            json!({
                                "error": format!("unknown status {target_in:?}"),
                                "valid_statuses": VALID_STATUSES,
                            }),
                        ),
                        no_write(),
                    );
                };
                let from = bs::parse_status(&next.status);
                if from != Some(target) {
                    let Some(from) = from else {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::CONFLICT,
                                json!({
                                    "error": format!(
                                        "current status {:?} is not in the shared vocabulary; \
                                         fix it via the Python board first",
                                        next.status
                                    ),
                                }),
                            ),
                            no_write(),
                        );
                    };
                    let Some(task) = next.to_task() else {
                        return finish(
                            &slot_w,
                            PatchOut::Refused(
                                StatusCode::CONFLICT,
                                json!({ "error": "row cannot be mapped to a core task" }),
                            ),
                            no_write(),
                        );
                    };
                    let force = map.get("force").and_then(Value::as_bool).unwrap_or(false);
                    let reason = body_str(&map, "reason").unwrap_or_default();
                    // RR-0048d (Invariant 50): leaving todo requires authored
                    // acceptance criteria — enforcement opt-in during
                    // coexistence (AMUX_RS_REQUIRE_CRITERIA=1); force bypasses
                    // WITH its audit line like every other gate.
                    if task.status == TaskStatus::Todo
                        && target != TaskStatus::Todo
                        && !target.is_terminal()
                        && !force
                    {
                        match crate::api::criteria::todo_exit_permitted(conn, &next.id) {
                            Ok(Ok(())) => {}
                            Ok(Err(msg)) => {
                                return finish(
                                    &slot_w,
                                    PatchOut::Refused(
                                        StatusCode::CONFLICT,
                                        json!({
                                            "error": "acceptance criteria required",
                                            "ok": false,
                                            "blocked": true,
                                            "item": next.id,
                                            "detail": msg,
                                        }),
                                    ),
                                    no_write(),
                                );
                            }
                            Err(e) => {
                                return finish(
                                    &slot_w,
                                    PatchOut::Refused(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        json!({ "error": e.to_string() }),
                                    ),
                                    no_write(),
                                );
                            }
                        }
                    }
                    let eff_gate = bs::effective_gate(&next, target);
                    let gates = bs::core_gates(&eff_gate, target);
                    let target_raw = bs::status_to_db(target, &next.status);

                    // Gate acknowledgement (AMUX-1719: gate_checked must
                    // MATCH the effective gate — every criterion present).
                    let mut evidence: Vec<Evidence> = Vec::new();
                    let mut ack_via: Option<String> = None;
                    if !eff_gate.is_empty() && !force {
                        let gc = map.get("gate_checked").and_then(Value::as_array).map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|s| s.trim().to_string())
                                .collect::<Vec<_>>()
                        });
                        if let Some(gc) = &gc {
                            let missing: Vec<&String> =
                                eff_gate.iter().filter(|c| !gc.contains(c)).collect();
                            if !missing.is_empty() {
                                return finish(
                                    &slot_w,
                                    PatchOut::Refused(
                                        StatusCode::CONFLICT,
                                        json!({
                                            "error": "gate_checked does not match the gate",
                                            "ok": false,
                                            "blocked": true,
                                            "gate": eff_gate,
                                            "missing": missing,
                                            "you_sent": gc,
                                            "attempted_status": target_raw,
                                            "item": row.id,
                                            "item_type": next.item_type,
                                            "how_to_ack": {
                                                "gate_checked": eff_gate,
                                                "or_gate_ack": true,
                                                "or_force": "true (explicit bypass; logged)",
                                                "contract": "GET /api/board/contract",
                                                "wrong_type?": "If these criteria don't fit the work, the TYPE is wrong — fix the type, not the truth.",
                                            },
                                        }),
                                    ),
                                    no_write(),
                                );
                            }
                            ack_via = Some(format!("gate_checked ({}/{})", gc.len(), eff_gate.len()));
                        } else if map.get("gate_ack").and_then(Value::as_bool).unwrap_or(false) {
                            ack_via = Some("gate_ack".into());
                        }
                        match &ack_via {
                            Some(via) => {
                                evidence = ack_evidence(&actor_name, &eff_gate, via);
                            }
                            None => {
                                let wb = why_blocked(&task, target, &gates, &[]);
                                return finish(
                                    &slot_w,
                                    PatchOut::Refused(
                                        StatusCode::CONFLICT,
                                        gate_409(&next, &eff_gate, &target_raw, &wb),
                                    ),
                                    no_write(),
                                );
                            }
                        }
                    }

                    // Discharge the gate HERE, with core's OWN predicate
                    // (`why_blocked` is the same function `apply_transition`'s
                    // gate_check runs — the view shares the predicate of the
                    // mechanism). It must happen at this boundary because the
                    // ack protocol is the API's: half the named transitions
                    // (Start, Resume, Queue, ...) carry no evidence slot, so
                    // handing `gates` to `apply_transition` would refuse an
                    // ack that was just verified criterion-by-criterion. The
                    // transition below therefore runs with the gate already
                    // discharged (empty gate slice), evidence recorded in the
                    // card log. `force` skips this check but never the audit.
                    if !force {
                        let wb = why_blocked(&task, target, &gates, &evidence);
                        if !wb.is_empty() {
                            return finish(
                                &slot_w,
                                PatchOut::Refused(
                                    StatusCode::CONFLICT,
                                    gate_409(&next, &eff_gate, &target_raw, &wb),
                                ),
                                no_write(),
                            );
                        }
                    }

                    let now = chrono::Utc::now();
                    let tx = if force {
                        BoardTransition::Force {
                            status: target,
                            reason: reason.clone(),
                        }
                    } else {
                        // No named transition (e.g. todo->done, which the
                        // Python board serves constantly) applies through
                        // core as an attributed direct set — the gate was
                        // discharged above, one code path for the write.
                        named_transition(from, target, evidence.clone(), reason.clone())
                            .unwrap_or_else(|| BoardTransition::Force {
                                status: target,
                                reason: format!(
                                    "direct status set via PATCH (no named {} -> {} transition)",
                                    bs::db_status_spelling(from),
                                    bs::db_status_spelling(target)
                                ),
                            })
                    };

                    match apply_transition(&task, tx, &actor, &[], now) {
                        Ok(updated) => {
                            let from_raw = next.status.clone();
                            let stamp = hhmm();
                            if let Some(via) = &ack_via {
                                next.log = Some(bs::append_log(
                                    next.log.as_deref(),
                                    &stamp,
                                    &format!(
                                        "{actor_name}: gate satisfied via {via} for {target_raw}"
                                    ),
                                ));
                            }
                            let line = if force {
                                // The audited bypass (ethos rule 6): the force
                                // MUST leave a trace, on the card itself.
                                format!(
                                    "force by {force_actor}: {from_raw}->{target_raw} reason={reason}"
                                )
                            } else {
                                format!("{actor_name}: {from_raw} -> {target_raw}")
                            };
                            next.log = Some(bs::append_log(next.log.as_deref(), &stamp, &line));
                            next.status = target_raw.clone();
                            next.version = i64::try_from(updated.version).unwrap_or(next.version + 1);
                            status_event = Some((from_raw, target_raw));
                            changed.push("status".into());
                        }
                        Err(TransitionError::NoOp) => { /* nothing to do */ }
                        Err(TransitionError::GateBlocked { blocked }) => {
                            return finish(
                                &slot_w,
                                PatchOut::Refused(
                                    StatusCode::CONFLICT,
                                    gate_409(&next, &eff_gate, &target_raw, &blocked),
                                ),
                                no_write(),
                            );
                        }
                        Err(e) => {
                            // InvalidTransition / NotArmable / Archived...:
                            // the serialized core error IS the body, plus the
                            // Python-style flags so no reader mistakes a
                            // refusal for success.
                            let mut body = serde_json::to_value(&e)
                                .unwrap_or_else(|_| json!({"kind": "transition_error"}));
                            body["error"] = json!(e.to_string());
                            body["ok"] = json!(false);
                            body["blocked"] = json!(true);
                            body["attempted_status"] = json!(target_raw);
                            body["item"] = json!(row.id);
                            return finish(
                                &slot_w,
                                PatchOut::Refused(StatusCode::CONFLICT, body),
                                no_write(),
                            );
                        }
                    }
                }
            }

            if changed.is_empty() {
                // Invariant 37: nothing changed -> applied:false, rev/version
                // untouched, unknown keys named.
                return finish(
                    &slot_w,
                    PatchOut::Noop {
                        body: detail_body(&row),
                        ignored,
                    },
                    no_write(),
                );
            }

            // Writes bump rev (the Python counter) AND version (the Rust one).
            next.rev = row.rev + 1;
            if !changed.contains(&"status".to_string()) {
                next.version = row.version + 1;
            }
            next.updated = now_secs();
            bs::save_patched(conn, &next)?;
            if let Some(tags) = &tags_change {
                bs::set_tags(conn, &next.id, tags, next.updated)?;
            }
            let mutation = match &status_event {
                Some((f, t)) => MutationKind::StatusChanged {
                    from: f.clone(),
                    to: t.clone(),
                },
                None => MutationKind::Updated,
            };
            let id = next.id.clone();
            finish(
                &slot_w,
                PatchOut::Applied {
                    body: detail_body(&next),
                    ignored,
                },
                WriteOutcome {
                    applied: true,
                    events: vec![ev(&id, mutation)],
                },
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
        Some(PatchOut::NotFound) => not_found(&id),
        Some(PatchOut::Refused(status, body)) => err(status, body),
        Some(PatchOut::Noop { mut body, ignored }) => {
            body["applied"] = json!(false);
            if !ignored.is_empty() {
                body["ignored_fields"] = json!(ignored);
                body["ignored_note"] = json!(
                    "these keys are not writable via PATCH and were NOT applied; \
                     the rest of this response reflects the card as stored"
                );
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Some(PatchOut::Applied { mut body, ignored }) => {
            body["applied"] = json!(true);
            body["global_rev"] = json!(reply.rev.0);
            if !ignored.is_empty() {
                body["ignored_fields"] = json!(ignored);
                body["ignored_note"] = json!(
                    "these keys are not writable via PATCH and were NOT applied; \
                     the rest of this response reflects the card as stored"
                );
            }
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

// ---- POST /api/board/{id}/archive + /restore (RR-0055) -------------------

async fn archive_restore(
    state: AppState,
    id: String,
    headers: HeaderMap,
    body: Option<Value>,
    restore: bool,
) -> Response {
    let (actor, actor_name) = actor_from_headers(&headers);
    let reason = body
        .as_ref()
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    enum Out {
        NotFound,
        Refused(Value),
        Noop(Value),
        Applied(Value),
    }
    let slot: Arc<Mutex<Option<Out>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let Some(row) = bs::get_issue(conn, &id_w)? else {
                return finish(&slot_w, Out::NotFound, no_write());
            };
            let Some(task) = row.to_task() else {
                return finish(
                    &slot_w,
                    Out::Refused(json!({
                        "error": format!(
                            "current status {:?} is not in the shared vocabulary; \
                             fix it via the Python board first",
                            row.status
                        ),
                    })),
                    no_write(),
                );
            };
            let tx = if restore {
                BoardTransition::Restore {
                    reason: reason.clone(),
                }
            } else {
                BoardTransition::Archive {
                    reason: reason.clone(),
                }
            };
            match apply_transition(&task, tx, &actor, &[], chrono::Utc::now()) {
                Ok(updated) => {
                    let mut next = row.clone();
                    next.archived = i64::from(updated.archived);
                    let verb = if restore { "restored" } else { "archived" };
                    let line = if reason.is_empty() {
                        format!("{actor_name}: {verb}")
                    } else {
                        format!("{actor_name}: {verb} — {reason}")
                    };
                    next.log = Some(bs::append_log(next.log.as_deref(), &hhmm(), &line));
                    next.rev = row.rev + 1;
                    next.version = i64::try_from(updated.version).unwrap_or(row.version + 1);
                    next.updated = now_secs();
                    bs::save_patched(conn, &next)?;
                    let id = next.id.clone();
                    finish(
                        &slot_w,
                        Out::Applied(detail_body(&next)),
                        WriteOutcome {
                            applied: true,
                            events: vec![ev(&id, MutationKind::Updated)],
                        },
                    )
                }
                // Already in the requested archive state: honest no-op,
                // rev unmoved (Invariant 37).
                Err(TransitionError::NoOp) => finish(&slot_w, Out::Noop(detail_body(&row)), no_write()),
                Err(e) => finish(
                    &slot_w,
                    Out::Refused(json!({ "error": e.to_string() })),
                    no_write(),
                ),
            }
        })
        .await;
    let reply = match write {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let outcome = slot.lock().expect("outcome slot poisoned").take();
    match outcome {
        None => internal("archive/restore produced no outcome"),
        Some(Out::NotFound) => not_found(&id),
        Some(Out::Refused(body)) => err(StatusCode::CONFLICT, body),
        Some(Out::Noop(mut body)) => {
            body["applied"] = json!(false);
            (StatusCode::OK, Json(body)).into_response()
        }
        Some(Out::Applied(mut body)) => {
            body["applied"] = json!(true);
            body["global_rev"] = json!(reply.rev.0);
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

pub async fn archive_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    archive_restore(state, id, headers, body.map(|Json(v)| v), false).await
}

pub async fn restore_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    archive_restore(state, id, headers, body.map(|Json(v)| v), true).await
}
