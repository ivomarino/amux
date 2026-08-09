//! CRM API (RR-0090): CRUD over the LIVE `crm_contacts` / `crm_tags` /
//! `crm_interactions` tables, route- and field-compatible with the Python
//! `/api/crm/*` handlers so the dashboard works unchanged.
//!
//! Python parity decisions, recorded so they are not "fixed" later:
//! - Contact ids are `PPL-N` from the SHARED `issue_counters` table
//!   (`_next_issue_id("PPL")`); interaction ids are
//!   `secrets.token_urlsafe(10)`-shaped (10 random bytes, base64url, no
//!   padding).
//! - The list endpoint returns the EXACT Python projection (id..phone +
//!   computed `last_date`/`next_followup`/`next_followup_note` + `tags`),
//!   ordered contacted-first by oldest `last_date` — never-contacted sink
//!   to the end. It does NOT include `notes`/`created`/`updated`.
//! - PATCH/DELETE answer `{"ok": true}` without existence checks (Python
//!   does not 404 there); tags-only PATCHes do not bump `updated` (Python's
//!   `if fields:` guard).
//! - The external CRM mirror (`_crm_sync_external` / Lightfield) is NOT
//!   ported; when `LIGHTFIELD_API_KEY` is configured the
//!   IntegrationRegistry says so (`crm_sync: unavailable`, RR-0073) instead
//!   of writes silently not mirroring.

use super::calendar::query_rows_json;
use super::AppState;
use crate::db::board_store::next_issue_id;
use crate::db::{PendingEvent, WriteOutcome};
use amux_core::revision::{EntityType, MutationKind};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/contacts", get(list_contacts).post(create_contact))
        .route(
            "/contacts/{id}",
            get(get_contact).patch(patch_contact).delete(delete_contact),
        )
        .route("/contacts/{id}/interactions", axum::routing::post(add_interaction))
        .route(
            "/interactions/{id}",
            axum::routing::patch(patch_interaction).delete(delete_interaction),
        )
        .route("/followups", get(followups))
        // Python's trailing `return self._json({"error": "not found"}, 404)`
        // for anything else under /api/crm/.
        .fallback(|| async {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response()
        })
}

// ---- shared helpers -------------------------------------------------------

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() }))
}

fn ev(entity: &str, id: &str, mutation: MutationKind) -> PendingEvent {
    PendingEvent {
        entity_type: EntityType::Other(entity.into()),
        entity_id: id.to_string(),
        mutation,
    }
}

fn contact_tags(conn: &Connection, cid: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT tag FROM crm_tags WHERE contact_id = ?1")?;
    let rows = stmt.query_map([cid], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Body string field with Python's `body.get(k, "")` default.
fn body_str(body: &Value, k: &str) -> String {
    body.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// `secrets.token_urlsafe(10)` shape: 10 random bytes, base64url, no
/// padding. The randomness is ULID's 80-bit random field — exactly 10
/// bytes, already CSPRNG-backed, and the crate is a workspace dep.
fn interaction_id() -> String {
    let bytes = ulid::Ulid::new().0.to_be_bytes();
    crate::integrations::email::base64url_nopad(&bytes[6..16])
}

// ---- GET /api/crm/contacts ------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub q: Option<String>,
}

pub async fn list_contacts(State(state): State<AppState>, Query(p): Query<ListParams>) -> Response {
    let q = p.q.unwrap_or_default().trim().to_string();
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Value>> {
        let conn = store.read()?;
        // Python's projection + ordering, verbatim: contacted contacts first
        // (oldest touch first), never-contacted at the end.
        let mut sql = String::from(
            "SELECT c.id,c.name,c.company,c.role,c.email,c.linkedin,c.twitter,c.phone,\
             (SELECT date FROM crm_interactions WHERE contact_id=c.id ORDER BY date DESC LIMIT 1) AS last_date,\
             (SELECT follow_up_date FROM crm_interactions WHERE contact_id=c.id AND follow_up_date IS NOT NULL ORDER BY follow_up_date ASC LIMIT 1) AS next_followup,\
             (SELECT follow_up_note FROM crm_interactions WHERE contact_id=c.id AND follow_up_date IS NOT NULL ORDER BY follow_up_date ASC LIMIT 1) AS next_followup_note \
             FROM crm_contacts c WHERE c.deleted IS NULL",
        );
        let like = format!("%{q}%");
        let params: Vec<&dyn rusqlite::types::ToSql> = if q.is_empty() {
            vec![]
        } else {
            sql.push_str(" AND (c.name LIKE ? OR c.company LIKE ? OR c.role LIKE ?)");
            vec![&like, &like, &like]
        };
        sql.push_str(
            " ORDER BY CASE WHEN last_date IS NULL THEN 0 ELSE 1 END DESC, last_date ASC",
        );
        let mut contacts = query_rows_json(&conn, &sql, &params)?;
        for c in &mut contacts {
            let cid = c.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            c["tags"] = json!(contact_tags(&conn, &cid)?);
        }
        Ok(contacts)
    })
    .await;
    match joined {
        Ok(Ok(rows)) => Json(Value::Array(rows)).into_response(),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---- POST /api/crm/contacts -----------------------------------------------

pub async fn create_contact(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let name = body_str(&body, "name").trim().to_string();
    if name.is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "name required" }));
    }
    let tags: Vec<String> = body
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let fields: Vec<String> = ["company", "role", "email", "linkedin", "twitter", "phone", "notes"]
        .iter()
        .map(|k| body_str(&body, k))
        .collect();

    let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let cid = next_issue_id(conn, "PPL")?;
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT INTO crm_contacts (id,name,company,role,email,linkedin,twitter,phone,notes,created,updated) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    cid, name, fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
                    fields[6], now, now
                ],
            )?;
            for tag in &tags {
                // Python swallows tag-insert failures (dupes) — keep that.
                let _ = conn.execute(
                    "INSERT INTO crm_tags (contact_id,tag) VALUES (?1,?2)",
                    rusqlite::params![cid, tag],
                );
            }
            let events = vec![ev("crm_contact", &cid, MutationKind::Created)];
            *slot_w.lock().expect("slot") = Some(cid);
            Ok(WriteOutcome { applied: true, events })
        })
        .await;
    match write {
        Ok(_) => {
            let cid = slot.lock().expect("slot").take().unwrap_or_default();
            (StatusCode::CREATED, Json(json!({ "id": cid, "ok": true }))).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- GET /api/crm/contacts/{id} -------------------------------------------

pub async fn get_contact(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Value>> {
        let conn = store.read()?;
        let Some(mut c) = query_rows_json(
            &conn,
            "SELECT * FROM crm_contacts WHERE id = ?1 AND deleted IS NULL",
            &[&id],
        )?
        .pop() else {
            return Ok(None);
        };
        c["tags"] = json!(contact_tags(&conn, &id)?);
        c["interactions"] = Value::Array(query_rows_json(
            &conn,
            "SELECT * FROM crm_interactions WHERE contact_id = ?1 ORDER BY date DESC, created DESC",
            &[&id],
        )?);
        Ok(Some(c))
    })
    .await;
    match joined {
        Ok(Ok(Some(c))) => Json(c).into_response(),
        Ok(Ok(None)) => err(StatusCode::NOT_FOUND, json!({ "error": "not found" })),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---- PATCH /api/crm/contacts/{id} -----------------------------------------

const CONTACT_FIELDS: [&str; 8] =
    ["name", "company", "role", "email", "linkedin", "twitter", "phone", "notes"];

pub async fn patch_contact(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let fields: Vec<(String, Value)> = CONTACT_FIELDS
        .iter()
        .filter_map(|k| body.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();
    let tags: Option<Vec<String>> = body.get("tags").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let now = chrono::Utc::now().timestamp();
            let mut touched = false;
            if !fields.is_empty() {
                let set_cl: Vec<String> = fields.iter().map(|(k, _)| format!("{k}=?")).collect();
                let mut params: Vec<rusqlite::types::Value> = fields
                    .iter()
                    .map(|(_, v)| match v {
                        Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                        Value::Number(n) if n.is_i64() => {
                            rusqlite::types::Value::Integer(n.as_i64().unwrap_or(0))
                        }
                        Value::Number(n) => rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0)),
                        Value::Null => rusqlite::types::Value::Null,
                        other => rusqlite::types::Value::Text(other.to_string()),
                    })
                    .collect();
                params.push(rusqlite::types::Value::Integer(now));
                params.push(rusqlite::types::Value::Text(id_w.clone()));
                let n = conn.execute(
                    &format!(
                        "UPDATE crm_contacts SET {}, updated=? WHERE id=?",
                        set_cl.join(", ")
                    ),
                    rusqlite::params_from_iter(params),
                )?;
                touched |= n > 0;
            }
            if let Some(tags) = &tags {
                conn.execute("DELETE FROM crm_tags WHERE contact_id=?1", [&id_w])?;
                for tag in tags {
                    let _ = conn.execute(
                        "INSERT INTO crm_tags (contact_id,tag) VALUES (?1,?2)",
                        rusqlite::params![id_w, tag],
                    );
                }
                touched = true;
            }
            let events = if touched {
                vec![ev("crm_contact", &id_w, MutationKind::Updated)]
            } else {
                vec![]
            };
            Ok(WriteOutcome { applied: touched, events })
        })
        .await;
    match write {
        // Python answers {"ok": true} whether or not the row existed.
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

// ---- DELETE /api/crm/contacts/{id} ----------------------------------------

pub async fn delete_contact(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let now = chrono::Utc::now().timestamp();
            let n = conn.execute(
                "UPDATE crm_contacts SET deleted=?1 WHERE id=?2",
                rusqlite::params![now, id_w],
            )?;
            let events = if n > 0 {
                vec![ev("crm_contact", &id_w, MutationKind::Deleted)]
            } else {
                vec![]
            };
            Ok(WriteOutcome { applied: n > 0, events })
        })
        .await;
    match write {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

// ---- POST /api/crm/contacts/{id}/interactions -----------------------------

pub async fn add_interaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let today = chrono::Local::now().date_naive().to_string();
    let date = {
        let d = body_str(&body, "date");
        if d.is_empty() { today } else { d }
    };
    let itype = {
        let t = body_str(&body, "type");
        if t.is_empty() { "other".to_string() } else { t }
    };
    let notes = body_str(&body, "notes");
    // Python: `body.get("follow_up_date") or None` — falsy becomes NULL.
    let follow_up_date =
        body.get("follow_up_date").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let follow_up_note = body_str(&body, "follow_up_note");

    let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let ix_id = interaction_id();
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT INTO crm_interactions (id,contact_id,date,type,notes,follow_up_date,follow_up_note,created,updated) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                rusqlite::params![ix_id, id_w, date, itype, notes, follow_up_date, follow_up_note, now, now],
            )?;
            conn.execute(
                "UPDATE crm_contacts SET updated=?1 WHERE id=?2",
                rusqlite::params![now, id_w],
            )?;
            let events = vec![ev("crm_interaction", &ix_id, MutationKind::Created)];
            *slot_w.lock().expect("slot") = Some(ix_id);
            Ok(WriteOutcome { applied: true, events })
        })
        .await;
    match write {
        Ok(_) => {
            let ix_id = slot.lock().expect("slot").take().unwrap_or_default();
            (StatusCode::CREATED, Json(json!({ "id": ix_id, "ok": true }))).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- PATCH/DELETE /api/crm/interactions/{id} ------------------------------

const INTERACTION_FIELDS: [&str; 5] =
    ["date", "type", "notes", "follow_up_date", "follow_up_note"];

pub async fn patch_interaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let fields: Vec<(String, Value)> = INTERACTION_FIELDS
        .iter()
        .filter_map(|k| body.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            if fields.is_empty() {
                // Python skips the UPDATE entirely and still answers ok.
                return Ok(WriteOutcome { applied: false, events: vec![] });
            }
            let now = chrono::Utc::now().timestamp();
            let set_cl: Vec<String> = fields.iter().map(|(k, _)| format!("{k}=?")).collect();
            let mut params: Vec<rusqlite::types::Value> = fields
                .iter()
                .map(|(_, v)| match v {
                    Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                    Value::Null => rusqlite::types::Value::Null,
                    Value::Number(n) if n.is_i64() => {
                        rusqlite::types::Value::Integer(n.as_i64().unwrap_or(0))
                    }
                    Value::Number(n) => rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0)),
                    other => rusqlite::types::Value::Text(other.to_string()),
                })
                .collect();
            params.push(rusqlite::types::Value::Integer(now));
            params.push(rusqlite::types::Value::Text(id_w.clone()));
            let n = conn.execute(
                &format!(
                    "UPDATE crm_interactions SET {}, updated=? WHERE id=?",
                    set_cl.join(", ")
                ),
                rusqlite::params_from_iter(params),
            )?;
            let events = if n > 0 {
                vec![ev("crm_interaction", &id_w, MutationKind::Updated)]
            } else {
                vec![]
            };
            Ok(WriteOutcome { applied: n > 0, events })
        })
        .await;
    match write {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

pub async fn delete_interaction(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id_w = id.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let n = conn.execute("DELETE FROM crm_interactions WHERE id=?1", [&id_w])?;
            let events = if n > 0 {
                vec![ev("crm_interaction", &id_w, MutationKind::Deleted)]
            } else {
                vec![]
            };
            Ok(WriteOutcome { applied: n > 0, events })
        })
        .await;
    match write {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

// ---- GET /api/crm/followups -----------------------------------------------

pub async fn followups(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Value>> {
        let conn = store.read()?;
        Ok(query_rows_json(
            &conn,
            "SELECT c.id,c.name,c.company,i.follow_up_date,i.follow_up_note \
             FROM crm_interactions i JOIN crm_contacts c ON c.id=i.contact_id \
             WHERE i.follow_up_date IS NOT NULL AND c.deleted IS NULL \
             ORDER BY i.follow_up_date ASC",
            &[],
        )?)
    })
    .await;
    match joined {
        Ok(Ok(rows)) => Json(Value::Array(rows)).into_response(),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---------------------------------------------------------------------------
// Tests — temp-DB stores; router nested directly.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Store;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("crm-api-test.db")).unwrap();
        let state = AppState {
            store: Arc::new(store),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        let router = Router::new().nest("/api/crm", routes()).with_state(state);
        (router, dir)
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let b = Request::builder().method(method).uri(path);
        let req = match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, v)
    }

    #[tokio::test]
    async fn create_mints_ppl_id_and_requires_name() {
        let (app, _dir) = app();
        let (st, res) = send(
            &app,
            "POST",
            "/api/crm/contacts",
            Some(json!({ "name": "Jane Doe", "company": "Acme", "tags": ["prospect", " "] })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{res}");
        assert_eq!(res["id"], json!("PPL-1"));
        assert_eq!(res["ok"], json!(true));

        let (st, e) = send(&app, "POST", "/api/crm/contacts", Some(json!({ "company": "x" }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(e["error"], json!("name required"));
    }

    #[tokio::test]
    async fn python_shaped_row_round_trips_column_by_column() {
        let (app, dir) = app();
        // Raw rows exactly as the Python server writes them.
        {
            let conn = rusqlite::Connection::open(dir.path().join("crm-api-test.db")).unwrap();
            conn.execute(
                "INSERT INTO crm_contacts (id,name,company,role,email,linkedin,twitter,phone,notes,created,updated) \
                 VALUES ('PPL-42','Mark Howard','Lucihub','CEO','mhoward@lucihub.com','','','','met at NAB',1753000000,1753000001)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO crm_tags (contact_id,tag) VALUES ('PPL-42','prospect')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO crm_interactions (id,contact_id,date,type,notes,follow_up_date,follow_up_note,created,updated) \
                 VALUES ('tokAbc123XyzQ','PPL-42','2026-08-01','call','pilot discussion','2026-08-15','send recap',1753000002,1753000002)",
                [],
            ).unwrap();
        }
        let (st, c) = send(&app, "GET", "/api/crm/contacts/PPL-42", None).await;
        assert_eq!(st, StatusCode::OK, "{c}");
        // Every stored column, byte-identical values.
        assert_eq!(c["id"], json!("PPL-42"));
        assert_eq!(c["name"], json!("Mark Howard"));
        assert_eq!(c["company"], json!("Lucihub"));
        assert_eq!(c["role"], json!("CEO"));
        assert_eq!(c["email"], json!("mhoward@lucihub.com"));
        assert_eq!(c["linkedin"], json!(""));
        assert_eq!(c["twitter"], json!(""));
        assert_eq!(c["phone"], json!(""));
        assert_eq!(c["notes"], json!("met at NAB"));
        assert_eq!(c["created"], json!(1753000000));
        assert_eq!(c["updated"], json!(1753000001));
        assert_eq!(c["deleted"], Value::Null);
        assert_eq!(c["tags"], json!(["prospect"]));
        let ix = &c["interactions"].as_array().unwrap()[0];
        assert_eq!(ix["id"], json!("tokAbc123XyzQ"));
        assert_eq!(ix["contact_id"], json!("PPL-42"));
        assert_eq!(ix["date"], json!("2026-08-01"));
        assert_eq!(ix["type"], json!("call"));
        assert_eq!(ix["notes"], json!("pilot discussion"));
        assert_eq!(ix["follow_up_date"], json!("2026-08-15"));
        assert_eq!(ix["follow_up_note"], json!("send recap"));

        // PATCH round-trip against the Python-shaped row.
        let (st, r) = send(
            &app,
            "PATCH",
            "/api/crm/contacts/PPL-42",
            Some(json!({ "role": "CEO & Founder", "tags": ["prospect", "pilot"] })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, c2) = send(&app, "GET", "/api/crm/contacts/PPL-42", None).await;
        assert_eq!(c2["role"], json!("CEO & Founder"));
        assert_eq!(c2["name"], json!("Mark Howard")); // untouched
        let mut tags: Vec<String> = c2["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect();
        tags.sort();
        assert_eq!(tags, vec!["pilot", "prospect"]);
    }

    #[tokio::test]
    async fn list_projection_matches_python_with_followup_columns() {
        let (app, _dir) = app();
        for name in ["Alpha", "Beta"] {
            let (st, _) =
                send(&app, "POST", "/api/crm/contacts", Some(json!({ "name": name }))).await;
            assert_eq!(st, StatusCode::CREATED);
        }
        // Alpha gets an interaction with a follow-up; Beta stays untouched.
        let (st, _) = send(
            &app,
            "POST",
            "/api/crm/contacts/PPL-1/interactions",
            Some(json!({ "date": "2026-08-01", "type": "call", "notes": "n",
                         "follow_up_date": "2026-08-20", "follow_up_note": "ping" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);

        let (st, list) = send(&app, "GET", "/api/crm/contacts", None).await;
        assert_eq!(st, StatusCode::OK);
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // Python ordering: contacted first (oldest last_date first),
        // never-contacted at the end.
        assert_eq!(arr[0]["id"], json!("PPL-1"));
        assert_eq!(arr[0]["last_date"], json!("2026-08-01"));
        assert_eq!(arr[0]["next_followup"], json!("2026-08-20"));
        assert_eq!(arr[0]["next_followup_note"], json!("ping"));
        assert_eq!(arr[1]["id"], json!("PPL-2"));
        assert_eq!(arr[1]["last_date"], Value::Null);
        // The projection does NOT leak notes/created/updated (Python parity).
        assert!(arr[0].get("notes").is_none());
        assert!(arr[0].get("created").is_none());
        assert!(arr[0]["tags"].is_array());

        // q filter matches name/company/role.
        let (_, filtered) = send(&app, "GET", "/api/crm/contacts?q=Alph", None).await;
        assert_eq!(filtered.as_array().unwrap().len(), 1);
        let (_, none) = send(&app, "GET", "/api/crm/contacts?q=zzz", None).await;
        assert_eq!(none.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn interaction_lifecycle_and_followups() {
        let (app, _dir) = app();
        let (_, _) = send(&app, "POST", "/api/crm/contacts", Some(json!({ "name": "C" }))).await;
        let (st, ix) = send(
            &app,
            "POST",
            "/api/crm/contacts/PPL-1/interactions",
            Some(json!({ "notes": "intro", "follow_up_date": "2026-09-01", "follow_up_note": "f" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        let ix_id = ix["id"].as_str().unwrap().to_string();
        // token_urlsafe(10) shape: 14 urlsafe chars, no padding.
        assert_eq!(ix_id.len(), 14, "{ix_id}");
        assert!(ix_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));

        // Defaults: date=today, type=other.
        let (_, c) = send(&app, "GET", "/api/crm/contacts/PPL-1", None).await;
        let stored = &c["interactions"].as_array().unwrap()[0];
        assert_eq!(stored["type"], json!("other"));
        assert_eq!(
            stored["date"],
            json!(chrono::Local::now().date_naive().to_string())
        );

        let (_, fu) = send(&app, "GET", "/api/crm/followups", None).await;
        let fua = fu.as_array().unwrap();
        assert_eq!(fua.len(), 1);
        assert_eq!(fua[0]["id"], json!("PPL-1"));
        assert_eq!(fua[0]["name"], json!("C"));
        assert_eq!(fua[0]["follow_up_date"], json!("2026-09-01"));
        assert_eq!(fua[0]["follow_up_note"], json!("f"));

        // PATCH the interaction.
        let (st, r) = send(
            &app,
            "PATCH",
            &format!("/api/crm/interactions/{ix_id}"),
            Some(json!({ "type": "email", "follow_up_date": null })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, c2) = send(&app, "GET", "/api/crm/contacts/PPL-1", None).await;
        let stored = &c2["interactions"].as_array().unwrap()[0];
        assert_eq!(stored["type"], json!("email"));
        assert_eq!(stored["follow_up_date"], Value::Null);

        // DELETE the interaction (hard delete, Python parity).
        let (st, r) = send(&app, "DELETE", &format!("/api/crm/interactions/{ix_id}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, c3) = send(&app, "GET", "/api/crm/contacts/PPL-1", None).await;
        assert_eq!(c3["interactions"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn delete_contact_is_soft_and_hides_everywhere() {
        let (app, dir) = app();
        let (_, _) = send(&app, "POST", "/api/crm/contacts", Some(json!({ "name": "Gone" }))).await;
        let (st, r) = send(&app, "DELETE", "/api/crm/contacts/PPL-1", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (st, _) = send(&app, "GET", "/api/crm/contacts/PPL-1", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        let (_, list) = send(&app, "GET", "/api/crm/contacts", None).await;
        assert_eq!(list.as_array().unwrap().len(), 0);
        // Soft: the row is still in the table (user data is never bulk-lost).
        let conn = rusqlite::Connection::open(dir.path().join("crm-api-test.db")).unwrap();
        let deleted: Option<i64> = conn
            .query_row("SELECT deleted FROM crm_contacts WHERE id='PPL-1'", [], |r| r.get(0))
            .unwrap();
        assert!(deleted.is_some());
    }

    #[tokio::test]
    async fn unmatched_crm_path_is_pythons_404() {
        let (app, _dir) = app();
        let (st, e) = send(&app, "GET", "/api/crm/bogus", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(e["error"], json!("not found"));
    }
}
