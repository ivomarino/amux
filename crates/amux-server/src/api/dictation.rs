//! Dictation API (SPA long-tail port): `/api/dictation/*` + `POST
//! /api/dictate` over the LIVE `dictation_history` / `dictation_dict`
//! tables (0001_baseline.sql), route- and field-compatible with the Python
//! handlers.
//!
//! Parity decisions, recorded so they are not "fixed" later:
//! - `POST /api/dictate` (audio -> text) and the AI-edit half of
//!   `POST /history/{id}/edit` call a local Whisper subprocess / Gemini
//!   `generateContent` in Python. Neither engine exists in this server, so
//!   both answer an HONEST 501 naming the missing capability (ethos rule 7:
//!   never a fake transcription). The pure-DB half of the surface — history
//!   list/count/delete, undo-AI-edit, dict CRUD, config — is fully served.
//! - `GET /history` `count=` mode mirrors Python's `parse_qs` truthiness:
//!   the mode triggers on a NON-EMPTY `count` value (`?count=` is dropped by
//!   parse_qs and does not trigger it).
//! - `total_words` sums the WHOLE table even when `session=` filters items —
//!   that is what Python computes, so the SPA's total badge matches.
//! - Dict rows honor `UNIQUE(word, correct)`: a duplicate POST answers
//!   `{"ok": true, "already": true}` (200), exactly Python's
//!   IntegrityError branch.
//! - `PATCH /dict/{id}` overwrites BOTH columns with `body.get(k) or ""`
//!   defaults — a partial PATCH blanks the missing field, as in Python.
//! - Config GET reports `local: false` always: whisper weights on disk are
//!   irrelevant to a server with no whisper runner (an honest instrument
//!   beats a flattering one). The Gemini key check (BYO pref, then
//!   GOOGLE_API_KEY) is ported so `configured`/`source` agree with Python.
//! - Non-numeric `{id}` path segments fall to Python's trailing
//!   `{"error": "dictation route not found"}` 404, not an axum 400.

use super::calendar::query_rows_json;
use super::AppState;
use crate::db::{PendingEvent, WriteOutcome};
use amux_core::revision::{EntityType, MutationKind};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/history", get(history))
        .route("/history/{id}", axum::routing::delete(delete_history))
        .route("/history/{id}/edit", axum::routing::post(edit_history))
        .route("/dict", get(list_dict).post(add_dict))
        .route("/dict/{id}", axum::routing::patch(patch_dict).delete(delete_dict))
        .route("/config", get(get_config).post(set_config))
        // Python's trailing `return self._json({"error": "dictation route
        // not found"}, 404)` for anything else under /api/dictation.
        .fallback(|| async { route_not_found() })
}

// ---- shared helpers -------------------------------------------------------

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() }))
}

fn route_not_found() -> Response {
    err(StatusCode::NOT_FOUND, json!({ "error": "dictation route not found" }))
}

fn ev(entity: &str, id: &str, mutation: MutationKind) -> PendingEvent {
    PendingEvent {
        entity_type: EntityType::Other(entity.into()),
        entity_id: id.to_string(),
        mutation,
    }
}

/// Python truthiness for `body.get("undo")`.
fn is_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Python `s[:n]` truncates by CHARACTERS, not bytes.
fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python `re.match(r"\d+")` route ids: non-numeric falls to the module 404.
fn parse_id(id: &str) -> Option<i64> {
    id.parse::<i64>().ok()
}

/// The server-env half of Python `_dictation_key()`.
/// Tests may pin this so assertions do not depend on the ambient
/// environment (an env-dependent test is a check that cannot reliably fail).
fn google_env_key() -> String {
    #[cfg(test)]
    if let Some(v) = tests::ENV_KEY_OVERRIDE.lock().expect("env override").clone() {
        return v;
    }
    std::env::var("GOOGLE_API_KEY").unwrap_or_default().trim().to_string()
}

/// Python `_DICTATION_MODEL`.
fn dictation_model() -> String {
    std::env::var("AMUX_DICTATION_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".into())
}

/// Python `_dictation_key()` -> (key, source). BYO pref wins over env.
fn dictation_key(conn: &rusqlite::Connection) -> (String, &'static str) {
    let byo: String = conn
        .query_row("SELECT value FROM prefs WHERE key='dictation_gemini_key'", [], |r| r.get(0))
        .map(|v: String| v.trim().to_string())
        .unwrap_or_default();
    if !byo.is_empty() {
        return (byo, "byo");
    }
    (google_env_key(), "server")
}

// ---- POST /api/dictate ----------------------------------------------------

/// Transcription itself is not implemented here — Python spawns a local
/// Whisper subprocess (preferred) and falls back to Gemini
/// `generateContent`. Pretending to transcribe would be a fake instrument;
/// this names exactly what is missing and where the working path is.
pub async fn dictate() -> Response {
    err(
        StatusCode::NOT_IMPLEMENTED,
        json!({
            "error": "transcription not available: this server implements neither the \
                      local Whisper subprocess nor the Gemini generateContent fallback \
                      that POST /api/dictate needs — use the Python amux-server for \
                      transcription; /api/dictation history, dict and config routes \
                      are fully served here"
        }),
    )
}

// ---- GET /api/dictation/history -------------------------------------------

#[derive(serde::Deserialize)]
pub struct HistoryParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    count: Option<String>,
}

const HISTORY_COLS: &str = "id, session, ts, text, raw_text, prev_text, ai_edited, words, dur_ms";

pub async fn history(State(state): State<AppState>, Query(p): Query<HistoryParams>) -> Response {
    let sess = p.session.unwrap_or_default();
    // Python: min(int(limit or 200), 500) — no lower clamp (kept as-is).
    let limit = p.limit.as_deref().and_then(|s| s.parse::<i64>().ok()).unwrap_or(200).min(500);
    let count_mode = p.count.as_deref().is_some_and(|c| !c.is_empty());
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let conn = store.read()?;
        if count_mode {
            let n: i64 = if sess.is_empty() {
                conn.query_row("SELECT COUNT(*) FROM dictation_history", [], |r| r.get(0))?
            } else {
                conn.query_row(
                    "SELECT COUNT(*) FROM dictation_history WHERE session=?1",
                    [&sess],
                    |r| r.get(0),
                )?
            };
            return Ok(json!({ "count": n }));
        }
        let items = if sess.is_empty() {
            query_rows_json(
                &conn,
                &format!(
                    "SELECT {HISTORY_COLS} FROM dictation_history ORDER BY ts DESC LIMIT ?1"
                ),
                &[&limit],
            )?
        } else {
            query_rows_json(
                &conn,
                &format!(
                    "SELECT {HISTORY_COLS} FROM dictation_history WHERE session=?1 \
                     ORDER BY ts DESC LIMIT ?2"
                ),
                &[&sess, &limit],
            )?
        };
        // Whole-table total even when session-filtered (Python parity).
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(words),0) FROM dictation_history",
            [],
            |r| r.get(0),
        )?;
        Ok(json!({ "items": items, "total_words": total }))
    })
    .await;
    match joined {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => internal(e),
        Err(e) => internal(e),
    }
}

// ---- DELETE /api/dictation/history/{id} -----------------------------------

pub async fn delete_history(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(rid) = parse_id(&id) else {
        return route_not_found();
    };
    let write = state
        .store
        .write_async(move |conn| {
            let n = conn.execute("DELETE FROM dictation_history WHERE id=?1", [rid])?;
            let events = if n > 0 {
                vec![ev("dictation_history", &rid.to_string(), MutationKind::Deleted)]
            } else {
                vec![]
            };
            Ok(WriteOutcome { applied: n > 0, events })
        })
        .await;
    match write {
        // Python answers ok whether or not the row existed.
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

// ---- POST /api/dictation/history/{id}/edit --------------------------------

pub async fn edit_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(rid) = parse_id(&id) else {
        return route_not_found();
    };
    // One read: the row + the BYO key (Python fetches both before deciding).
    let store = state.store.clone();
    let looked = tokio::task::spawn_blocking(
        move || -> anyhow::Result<Option<(String, String, String)>> {
            let conn = store.read()?;
            let row = conn
                .query_row(
                    "SELECT text, raw_text, prev_text FROM dictation_history WHERE id=?1",
                    [rid],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            Ok(row)
        },
    )
    .await;
    let (_text, raw_text, prev_text) = match looked {
        Ok(Ok(Some(row))) => row,
        Ok(Ok(None)) => return err(StatusCode::NOT_FOUND, json!({ "error": "not found" })),
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e),
    };

    if is_truthy(body.get("undo")) {
        // Pure-DB undo: `prev_text or raw_text` (Python falsy-string chain).
        let prev = if prev_text.is_empty() { raw_text } else { prev_text };
        let prev_w = prev.clone();
        let write = state
            .store
            .write_async(move |conn| {
                let n = conn.execute(
                    "UPDATE dictation_history SET text=?1, ai_edited=0, prev_text='' WHERE id=?2",
                    rusqlite::params![prev_w, rid],
                )?;
                let events = if n > 0 {
                    vec![ev("dictation_history", &rid.to_string(), MutationKind::Updated)]
                } else {
                    vec![]
                };
                Ok(WriteOutcome { applied: n > 0, events })
            })
            .await;
        return match write {
            Ok(_) => Json(json!({ "ok": true, "text": prev, "ai_edited": 0 })).into_response(),
            Err(e) => internal(e),
        };
    }

    // AI-edit path. Python: no key -> 503; with a key it calls Gemini. The
    // Gemini call is the capability this server lacks, so the with-key
    // answer is an honest 501, never a fabricated edit.
    let store = state.store.clone();
    let keyed = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, &'static str)> {
        let conn = store.read()?;
        Ok(dictation_key(&conn))
    })
    .await;
    let key = match keyed {
        Ok(Ok((k, _src))) => k,
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e),
    };
    if key.is_empty() {
        // Python's exact message + status.
        return err(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "no Gemini key configured" }));
    }
    err(
        StatusCode::NOT_IMPLEMENTED,
        json!({
            "error": "AI edit not available: this server does not implement the Gemini \
                      generateContent call the edit instruction needs — {\"undo\": true} \
                      works here, or use the Python amux-server for AI edits"
        }),
    )
}

// ---- GET/POST /api/dictation/dict -----------------------------------------

pub async fn list_dict(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Value>> {
        let conn = store.read()?;
        Ok(query_rows_json(
            &conn,
            "SELECT id, word, correct, created FROM dictation_dict ORDER BY created DESC",
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

/// Insert outcome carried out of the write closure.
enum DictInsert {
    Created(i64),
    Already,
}

pub async fn add_dict(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let word = truncate_chars(
        body.get("word").and_then(Value::as_str).unwrap_or("").trim(),
        120,
    );
    let correct = truncate_chars(
        body.get("correct").and_then(Value::as_str).unwrap_or("").trim(),
        120,
    );
    if word.is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "word required" }));
    }
    let slot: Arc<Mutex<Option<DictInsert>>> = Arc::new(Mutex::new(None));
    let slot_w = slot.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let created = chrono::Utc::now().timestamp();
            match conn.execute(
                "INSERT INTO dictation_dict (word, correct, created) VALUES (?1,?2,?3)",
                rusqlite::params![word, correct, created],
            ) {
                Ok(_) => {
                    let id = conn.last_insert_rowid();
                    *slot_w.lock().expect("slot") = Some(DictInsert::Created(id));
                    Ok(WriteOutcome {
                        applied: true,
                        events: vec![ev("dictation_dict", &id.to_string(), MutationKind::Created)],
                    })
                }
                // UNIQUE(word, correct) — Python's IntegrityError branch.
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    *slot_w.lock().expect("slot") = Some(DictInsert::Already);
                    Ok(WriteOutcome { applied: false, events: vec![] })
                }
                Err(other) => Err(other),
            }
        })
        .await;
    match write {
        Ok(_) => match slot.lock().expect("slot").take() {
            Some(DictInsert::Created(id)) => {
                (StatusCode::CREATED, Json(json!({ "ok": true, "id": id }))).into_response()
            }
            _ => Json(json!({ "ok": true, "already": true })).into_response(),
        },
        Err(e) => internal(e),
    }
}

// ---- PATCH/DELETE /api/dictation/dict/{id} --------------------------------

pub async fn patch_dict(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(did) = parse_id(&id) else {
        return route_not_found();
    };
    // Python sets BOTH columns from `(body.get(k) or "")` — a missing field
    // blanks the column. Kept identical.
    let word = truncate_chars(body.get("word").and_then(Value::as_str).unwrap_or("").trim(), 120);
    let correct =
        truncate_chars(body.get("correct").and_then(Value::as_str).unwrap_or("").trim(), 120);
    let write = state
        .store
        .write_async(move |conn| {
            let n = conn.execute(
                "UPDATE dictation_dict SET word=?1, correct=?2 WHERE id=?3",
                rusqlite::params![word, correct, did],
            )?;
            let events = if n > 0 {
                vec![ev("dictation_dict", &did.to_string(), MutationKind::Updated)]
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

pub async fn delete_dict(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(did) = parse_id(&id) else {
        return route_not_found();
    };
    let write = state
        .store
        .write_async(move |conn| {
            let n = conn.execute("DELETE FROM dictation_dict WHERE id=?1", [did])?;
            let events = if n > 0 {
                vec![ev("dictation_dict", &did.to_string(), MutationKind::Deleted)]
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

// ---- GET/POST /api/dictation/config ---------------------------------------

pub async fn get_config(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, &'static str)> {
        let conn = store.read()?;
        Ok(dictation_key(&conn))
    })
    .await;
    let (key, src) = match joined {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return internal(e),
        Err(e) => return internal(e),
    };
    let configured = !key.is_empty();
    // `local` is false unconditionally: this server has no whisper runner,
    // so reporting weights-on-disk as "local: true" would promise an engine
    // /api/dictate cannot deliver (the 501 above is the same fact).
    Json(json!({
        "configured": configured,
        "source": if configured { src } else { "none" },
        "model": dictation_model(),
        "local": false,
        "local_model": "",
        "engine": "gemini",
    }))
    .into_response()
}

pub async fn set_config(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let k = body.get("key").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let k_w = k.clone();
    let write = state
        .store
        .write_async(move |conn| {
            if k_w.is_empty() {
                conn.execute("DELETE FROM prefs WHERE key='dictation_gemini_key'", [])?;
            } else {
                conn.execute(
                    "INSERT INTO prefs (key, value) VALUES ('dictation_gemini_key', ?1) \
                     ON CONFLICT(key) DO UPDATE SET value=?1",
                    [&k_w],
                )?;
            }
            Ok(WriteOutcome {
                applied: true,
                events: vec![ev("pref", "dictation_gemini_key", MutationKind::Updated)],
            })
        })
        .await;
    if let Err(e) = write {
        return internal(e);
    }
    // Python re-runs _dictation_key(): BYO (just written) wins, else env.
    let (configured, src) = if !k.is_empty() {
        (true, "byo")
    } else {
        let env = google_env_key();
        (!env.is_empty(), "server")
    };
    Json(json!({
        "ok": true,
        "configured": configured,
        "source": if configured { src } else { "none" },
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Tests — temp-DB stores, Python-shaped rows; no network, no live files.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::Store;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Pin the "server env" Gemini key so assertions cannot be flipped by
    /// the ambient GOOGLE_API_KEY of whoever runs the tests.
    pub(crate) static ENV_KEY_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
    /// Serializes the tests that set the override (async-aware: the guard
    /// is held across awaits).
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn app() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("dictation-api-test.db")).unwrap();
        let state = AppState {
            store: Arc::new(store),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        let router = Router::new()
            .nest("/api/dictation", routes())
            .route("/api/dictate", axum::routing::post(dictate))
            .with_state(state);
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

    /// Rows exactly as the Python server INSERTs them.
    fn seed_python_rows(db: &std::path::Path) {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO dictation_history (session, ts, text, raw_text, words, dur_ms) \
             VALUES ('orch', 1754700000000, 'send the board update', 'send the board update', 4, 2100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dictation_history (session, ts, text, raw_text, prev_text, ai_edited, words, dur_ms) \
             VALUES ('', 1754700001000, 'Cleaner text.', 'cleaner text raw', 'cleaner text before', 1, 2, 900)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dictation_dict (word, correct, created) VALUES ('a mux', 'amux', 1754000000)",
            [],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn python_shaped_history_round_trip() {
        let (app, dir) = app();
        seed_python_rows(&dir.path().join("dictation-api-test.db"));

        let (st, v) = send(&app, "GET", "/api/dictation/history", None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        // ts DESC: the ai-edited row first; every Python column present.
        assert_eq!(items[0]["ts"], json!(1754700001000i64));
        assert_eq!(items[0]["text"], json!("Cleaner text."));
        assert_eq!(items[0]["raw_text"], json!("cleaner text raw"));
        assert_eq!(items[0]["prev_text"], json!("cleaner text before"));
        assert_eq!(items[0]["ai_edited"], json!(1));
        assert_eq!(items[0]["words"], json!(2));
        assert_eq!(items[0]["dur_ms"], json!(900));
        assert_eq!(items[1]["session"], json!("orch"));
        assert_eq!(v["total_words"], json!(6));

        // session filter: items narrow, total stays whole-table (parity).
        let (_, f) = send(&app, "GET", "/api/dictation/history?session=orch", None).await;
        assert_eq!(f["items"].as_array().unwrap().len(), 1);
        assert_eq!(f["total_words"], json!(6));

        // count mode: one integer, no transcript payload.
        let (_, c) = send(&app, "GET", "/api/dictation/history?count=1", None).await;
        assert_eq!(c, json!({ "count": 2 }));
        let (_, c2) = send(&app, "GET", "/api/dictation/history?count=1&session=orch", None).await;
        assert_eq!(c2, json!({ "count": 1 }));

        // limit applies.
        let (_, l) = send(&app, "GET", "/api/dictation/history?limit=1", None).await;
        assert_eq!(l["items"].as_array().unwrap().len(), 1);

        // DELETE — ok, row gone.
        let id = items[1]["id"].as_i64().unwrap();
        let (st, r) = send(&app, "DELETE", &format!("/api/dictation/history/{id}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, after) = send(&app, "GET", "/api/dictation/history?count=1", None).await;
        assert_eq!(after["count"], json!(1));
    }

    #[tokio::test]
    async fn undo_reverts_to_prev_text_then_raw_text() {
        let (app, dir) = app();
        seed_python_rows(&dir.path().join("dictation-api-test.db"));
        let (_, v) = send(&app, "GET", "/api/dictation/history", None).await;
        let edited_id = v["items"][0]["id"].as_i64().unwrap();

        let (st, r) = send(
            &app,
            "POST",
            &format!("/api/dictation/history/{edited_id}/edit"),
            Some(json!({ "undo": true })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{r}");
        assert_eq!(r, json!({ "ok": true, "text": "cleaner text before", "ai_edited": 0 }));
        let (_, v2) = send(&app, "GET", "/api/dictation/history", None).await;
        assert_eq!(v2["items"][0]["text"], json!("cleaner text before"));
        assert_eq!(v2["items"][0]["ai_edited"], json!(0));
        assert_eq!(v2["items"][0]["prev_text"], json!(""));

        // Second undo: prev_text now '' -> falls back to raw_text (Python's
        // `or` chain).
        let (_, r2) = send(
            &app,
            "POST",
            &format!("/api/dictation/history/{edited_id}/edit"),
            Some(json!({ "undo": 1 })),
        )
        .await;
        assert_eq!(r2["text"], json!("cleaner text raw"));

        // Missing row -> Python's 404.
        let (st, e) = send(
            &app,
            "POST",
            "/api/dictation/history/999999/edit",
            Some(json!({ "undo": true })),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(e["error"], json!("not found"));
    }

    #[tokio::test]
    async fn ai_edit_is_503_without_key_and_honest_501_with_one() {
        let _lock = ENV_LOCK.lock().await;
        *ENV_KEY_OVERRIDE.lock().unwrap() = Some(String::new()); // no server env key
        let (app, dir) = app();
        seed_python_rows(&dir.path().join("dictation-api-test.db"));
        let (_, v) = send(&app, "GET", "/api/dictation/history", None).await;
        let id = v["items"][0]["id"].as_i64().unwrap();

        // No key anywhere -> Python's exact 503.
        let (st, e) = send(
            &app,
            "POST",
            &format!("/api/dictation/history/{id}/edit"),
            Some(json!({ "instruction": "tighten" })),
        )
        .await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{e}");
        assert_eq!(e["error"], json!("no Gemini key configured"));

        // BYO key present -> the capability is the gap, not the key: 501
        // naming the missing Gemini call (never a fabricated edit).
        let (_, _) = send(
            &app,
            "POST",
            "/api/dictation/config",
            Some(json!({ "key": "PLACEHOLDER_BYO_KEY" })),
        )
        .await;
        let (st, e) = send(
            &app,
            "POST",
            &format!("/api/dictation/history/{id}/edit"),
            Some(json!({ "instruction": "tighten" })),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_IMPLEMENTED, "{e}");
        assert!(e["error"].as_str().unwrap().contains("Gemini generateContent"), "{e}");
        *ENV_KEY_OVERRIDE.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn dict_crud_matches_python_including_unique_conflict() {
        let (app, dir) = app();
        seed_python_rows(&dir.path().join("dictation-api-test.db"));

        // Python-shaped row reads back column-exact.
        let (st, list) = send(&app, "GET", "/api/dictation/dict", None).await;
        assert_eq!(st, StatusCode::OK);
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["word"], json!("a mux"));
        assert_eq!(arr[0]["correct"], json!("amux"));
        assert_eq!(arr[0]["created"], json!(1754000000));

        // Create -> 201 {ok, id}.
        let (st, r) = send(
            &app,
            "POST",
            "/api/dictation/dict",
            Some(json!({ "word": "  cloud f l a r e ", "correct": "Cloudflare" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{r}");
        assert_eq!(r["ok"], json!(true));
        let new_id = r["id"].as_i64().unwrap();

        // Duplicate (word, correct) -> Python's IntegrityError answer, 200.
        let (st, dup) = send(
            &app,
            "POST",
            "/api/dictation/dict",
            Some(json!({ "word": "cloud f l a r e", "correct": "Cloudflare" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(dup, json!({ "ok": true, "already": true }));

        // word required.
        let (st, e) = send(&app, "POST", "/api/dictation/dict", Some(json!({ "correct": "x" }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(e["error"], json!("word required"));

        // PATCH overwrites both columns (missing field blanks — parity).
        let (st, r) = send(
            &app,
            "PATCH",
            &format!("/api/dictation/dict/{new_id}"),
            Some(json!({ "word": "cf" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, list2) = send(&app, "GET", "/api/dictation/dict", None).await;
        let patched = list2
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"].as_i64() == Some(new_id))
            .unwrap()
            .clone();
        assert_eq!(patched["word"], json!("cf"));
        assert_eq!(patched["correct"], json!(""));

        // DELETE.
        let (st, r) = send(&app, "DELETE", &format!("/api/dictation/dict/{new_id}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["ok"], json!(true));
        let (_, list3) = send(&app, "GET", "/api/dictation/dict", None).await;
        assert_eq!(list3.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn config_reports_key_presence_and_never_the_key() {
        let _lock = ENV_LOCK.lock().await;
        *ENV_KEY_OVERRIDE.lock().unwrap() = Some(String::new());
        let (app, _dir) = app();

        let (st, c) = send(&app, "GET", "/api/dictation/config", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(c["configured"], json!(false));
        assert_eq!(c["source"], json!("none"));
        assert_eq!(c["model"], json!("gemini-2.5-flash"));
        assert_eq!(c["local"], json!(false));
        assert_eq!(c["engine"], json!("gemini"));

        // Set BYO key: write-only — the key never appears in any response.
        let (st, r) = send(
            &app,
            "POST",
            "/api/dictation/config",
            Some(json!({ "key": "PLACEHOLDER_KEY_VALUE" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r, json!({ "ok": true, "configured": true, "source": "byo" }));
        let (_, c2) = send(&app, "GET", "/api/dictation/config", None).await;
        assert_eq!(c2["configured"], json!(true));
        assert_eq!(c2["source"], json!("byo"));
        assert!(!c2.to_string().contains("PLACEHOLDER_KEY_VALUE"));

        // Clear -> back to none (env pinned empty).
        let (_, r) = send(&app, "POST", "/api/dictation/config", Some(json!({ "key": "" }))).await;
        assert_eq!(r, json!({ "ok": true, "configured": false, "source": "none" }));

        // Server env key -> source "server".
        *ENV_KEY_OVERRIDE.lock().unwrap() = Some("PLACEHOLDER_ENV".into());
        let (_, c3) = send(&app, "GET", "/api/dictation/config", None).await;
        assert_eq!(c3["configured"], json!(true));
        assert_eq!(c3["source"], json!("server"));
        *ENV_KEY_OVERRIDE.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn dictate_is_honest_501_and_unknown_routes_404() {
        let (app, _dir) = app();
        let (st, e) = send(&app, "POST", "/api/dictate", Some(json!({ "audio": "abcd" }))).await;
        assert_eq!(st, StatusCode::NOT_IMPLEMENTED);
        assert!(e["error"].as_str().unwrap().contains("Whisper"), "{e}");
        assert!(e["error"].as_str().unwrap().contains("Gemini"), "{e}");

        let (st, e) = send(&app, "GET", "/api/dictation/bogus", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(e["error"], json!("dictation route not found"));

        // Non-numeric id: Python's regex miss -> module 404, not an axum 400.
        let (st, e) = send(&app, "DELETE", "/api/dictation/history/abc", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(e["error"], json!("dictation route not found"));
    }
}
