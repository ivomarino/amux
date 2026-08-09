//! Structured request log + native `/api/logs` (AMUX-2605).
//!
//! Three pieces, one file:
//!
//! 1. **The substrate**: [`middleware`] wraps the WHOLE router (outside the
//!    alias rewrite, so `path` is the RAW path the client sent) and records
//!    every API request into `_amux_request_log` (migration 0010). Rows ride
//!    a bounded channel to a background task that batch-inserts through the
//!    single-writer store — logging can never block or fail a request; on
//!    any error the row is dropped with a `tracing::warn`.
//! 2. **The Logs tab**: `GET /api/logs` + `GET /api/logs/raw`, ported from
//!    the Python server's LIVE handlers (amux-server.py:67673 — the second
//!    pair at :71933 is dead code: Python's dispatch is first-match and the
//!    :67673 block answers first). Same response shapes; where Python reads
//!    its own server.log, this serves the request log UNIONED with the
//!    tracing tail (`~/.amux/logs/server-rs.log`), each line labelled with
//!    its source.
//! 3. **Worker subset**: the `worker` column is derived from the path
//!    (`/api/sessions/{name}/*` -> name, `/api/workers/{id}/*` -> id), so a
//!    per-worker log is a FILTER over the global log (`?worker=`), never a
//!    second log to keep in step.
//!
//! Size discipline (the 25MB-dictation-upload rule): request bodies are
//! NEVER read by the logger — `req_bytes` comes from Content-Length only.
//! `error_body` (first [`ERROR_BODY_CHARS`] chars) is captured ONLY for
//! status >= 400, where bodies are small JSON by construction (and the
//! python proxy already buffers whole responses, so buffering a 4xx/5xx
//! here adds no new cost). `user_agent`/`req_meta` are char-capped.
//!
//! Retention: rows older than `AMUX_REQLOG_RETAIN_DAYS` (default 14; env >
//! server.env > default) are deleted opportunistically — every
//! [`SWEEP_EVERY`] inserted rows — and the delete COUNT is logged, so a
//! sweep that fires is visible and one that never fires is absent from the
//! log (ethos rule 4).

use super::AppState;
use crate::db::{SharedStore, WriteOutcome};
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::TimeZone;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

/// Caps, all applied BEFORE a row enters the channel. Everything stored is
/// bounded; nothing about a request can make its row large.
const USER_AGENT_CHARS: usize = 300;
const QUERY_CHARS: usize = 500;
const CONTENT_TYPE_CHARS: usize = 120;
const ERROR_BODY_CHARS: usize = 500;
/// Bounded channel: if the writer falls behind, rows are DROPPED (with a
/// warn), never queued unboundedly and never back-pressured onto requests.
const QUEUE_CAP: usize = 10_000;
/// Max rows folded into one write transaction.
const BATCH_MAX: usize = 512;
/// Retention sweep cadence: once per this many inserted rows.
const SWEEP_EVERY: u64 = 1000;
const DEFAULT_RETAIN_DAYS: f64 = 14.0;

// ---------------------------------------------------------------------------
// Row + logger (the substrate)
// ---------------------------------------------------------------------------

/// One request, fully capped, ready to insert.
#[derive(Debug)]
pub struct LogRow {
    pub ts: f64,
    pub method: String,
    pub path: String,
    pub family: String,
    pub status: u16,
    pub latency_ms: f64,
    pub client_ip: String,
    pub user_agent: String,
    pub amux_session: String,
    pub worker: Option<String>,
    pub req_bytes: Option<i64>,
    pub resp_bytes: Option<i64>,
    pub answered_by: String,
    pub error_body: Option<String>,
    pub req_meta: Option<String>,
}

/// Cheap-to-clone handle the middleware sends rows through.
#[derive(Clone)]
pub struct RequestLogger {
    tx: tokio::sync::mpsc::Sender<LogRow>,
}

impl RequestLogger {
    pub fn spawn(store: SharedStore) -> Self {
        Self::spawn_with(store, retain_days_config(), SWEEP_EVERY)
    }

    /// Test seam: retention window + sweep cadence injectable so the sweep
    /// can be exercised without inserting 1000 rows.
    pub fn spawn_with(store: SharedStore, retain_days: f64, sweep_every: u64) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<LogRow>(QUEUE_CAP);
        // No runtime (a sync-context caller building a router for
        // inspection): rows will be dropped at send. Every production and
        // test caller runs inside tokio, so this is a guard, not a path.
        if tokio::runtime::Handle::try_current().is_err() {
            tracing::warn!("request log: no tokio runtime at spawn — rows will be dropped");
            return Self { tx };
        }
        tokio::spawn(async move {
            let mut since_sweep: u64 = 0;
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < BATCH_MAX {
                    match rx.try_recv() {
                        Ok(r) => batch.push(r),
                        Err(_) => break,
                    }
                }
                since_sweep += batch.len() as u64;
                let sweep = since_sweep >= sweep_every;
                if sweep {
                    since_sweep = 0;
                }
                let res = store
                    .write_async(move |conn| {
                        {
                            let mut stmt = conn.prepare_cached(
                                "INSERT INTO _amux_request_log \
                                 (ts, method, path, family, status, latency_ms, client_ip, \
                                  user_agent, amux_session, worker, req_bytes, resp_bytes, \
                                  answered_by, error_body, req_meta) \
                                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                            )?;
                            for r in &batch {
                                stmt.execute(rusqlite::params![
                                    r.ts,
                                    r.method,
                                    r.path,
                                    r.family,
                                    r.status,
                                    r.latency_ms,
                                    r.client_ip,
                                    r.user_agent,
                                    r.amux_session,
                                    r.worker,
                                    r.req_bytes,
                                    r.resp_bytes,
                                    r.answered_by,
                                    r.error_body,
                                    r.req_meta,
                                ])?;
                            }
                        }
                        if sweep {
                            let cutoff = unix_now() - retain_days * 86400.0;
                            let deleted = conn.execute(
                                "DELETE FROM _amux_request_log WHERE ts < ?1",
                                rusqlite::params![cutoff],
                            )?;
                            // The COUNT is the point (mandate: "count logged"):
                            // a sweep whose effect is invisible is a sweep
                            // nobody can verify fired.
                            tracing::info!(
                                deleted,
                                retain_days,
                                "request-log retention sweep"
                            );
                        }
                        // applied:false — deliberately NO revision bump. The
                        // request log is observability substrate, not fleet
                        // state: bumping `_amux_rev` here would publish a
                        // phantom state change to every delta-sync client on
                        // EVERY request, and the SPA's own polling would then
                        // generate revisions that trigger more polling. The
                        // transaction still commits (apply_write commits
                        // regardless of `applied`).
                        Ok(WriteOutcome { applied: false, events: vec![] })
                    })
                    .await;
                if let Err(e) = res {
                    // Never propagate: logging must not fail anything.
                    tracing::warn!(error = %e, "request-log insert failed; rows dropped");
                }
            }
        });
        Self { tx }
    }

    fn send(&self, row: LogRow) {
        if let Err(e) = self.tx.try_send(row) {
            // Full or closed: drop the row, say so. A blocked logger must
            // never become back-pressure on the request path.
            tracing::warn!(error = %e, "request-log queue rejected a row (dropped)");
        }
    }
}

/// `AMUX_REQLOG_RETAIN_DAYS`: process env wins, then server.env, then 14 —
/// the same precedence config.rs gives every other knob.
fn retain_days_config() -> f64 {
    if let Some(d) = std::env::var("AMUX_REQLOG_RETAIN_DAYS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        return d;
    }
    crate::config::parse_env_file(&super::settings::amux_home().join("server.env"))
        .get("AMUX_REQLOG_RETAIN_DAYS")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(DEFAULT_RETAIN_DAYS)
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Wrap the finished app (AFTER `aliases::alias_layer`, so this middleware
/// runs BEFORE the alias rewrite and sees the RAW client path). Same
/// wrapping shape as `alias_layer` — outer router whose fallback is the real
/// app — so it provably applies to every route including fallbacks.
pub fn layer(app: Router, store: SharedStore) -> Router {
    layer_with(app, RequestLogger::spawn(store))
}

/// Seam for tests: exact production wiring, injectable logger.
pub fn layer_with(app: Router, logger: RequestLogger) -> Router {
    Router::new()
        .fallback_service(app)
        .layer(axum::middleware::from_fn_with_state(logger, middleware))
}

/// What gets logged: every /api request EXCEPT `/api/events` (a long-lived
/// SSE stream — Python skips it too) and `/api/debug/*` (a debugging session
/// polling the instruments must not flood the instrument). Everything
/// outside /api — the SPA shell, /health, sw.js, icons — is a static-asset
/// or liveness path and is skipped by the prefix rule. Nothing else is
/// excluded.
fn should_log(path: &str) -> bool {
    if path != "/api" && !path.starts_with("/api/") {
        return false;
    }
    if path == "/api/events" || path.starts_with("/api/events/") {
        return false;
    }
    if path == "/api/debug" || path.starts_with("/api/debug/") {
        return false;
    }
    true
}

pub async fn middleware(State(logger): State<RequestLogger>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !should_log(&path) {
        return next.run(req).await;
    }
    let ts = unix_now();
    let started = std::time::Instant::now();
    let method = req.method().as_str().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    // Scoped block: a closure borrowing `req` may not outlive the borrow
    // into `next.run(req)` — and `&Request` is !Send (Body is !Sync), so the
    // closure must also drop before the await or the whole future loses Send.
    let (user_agent, amux_session, content_type) = {
        let hdr = |name: &str| {
            req.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        (
            truncate_chars(&hdr("user-agent"), USER_AGENT_CHARS),
            hdr("x-amux-session"),
            truncate_chars(&hdr("content-type"), CONTENT_TYPE_CHARS),
        )
    };
    // Content-Length ONLY — the logger never reads a request body (a
    // dictation upload is 25MB of audio; the SIZE is the telemetry).
    let req_bytes = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_default();
    let worker = worker_of(&path, &query);
    let family = family_of(&path);

    let res = next.run(req).await;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;

    let status = res.status().as_u16();
    let answered_by = res
        .headers()
        .get("x-amux-answered-by")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("native")
        .to_string();
    let mut resp_bytes = res
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    // error_body only for failures. Error responses in this server (and the
    // python proxy, which buffers whole bodies anyway) are small JSON, so
    // buffering one is bounded in practice; success responses — including
    // multi-GB /api/file/raw streams — are never touched.
    let (res, error_body) = if status >= 400 {
        let (parts, body) = res.into_parts();
        match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => {
                resp_bytes = Some(bytes.len() as i64);
                let text = truncate_chars(&String::from_utf8_lossy(&bytes), ERROR_BODY_CHARS);
                (
                    Response::from_parts(parts, axum::body::Body::from(bytes)),
                    if text.is_empty() { None } else { Some(text) },
                )
            }
            Err(e) => {
                // The underlying body stream errored — the response was
                // already undeliverable; hand back what remains.
                tracing::warn!(error = %e, %path, "error-body capture failed");
                (Response::from_parts(parts, axum::body::Body::empty()), None)
            }
        }
    } else {
        (res, None)
    };

    let mut meta = serde_json::Map::new();
    if !query.is_empty() {
        meta.insert("query".into(), json!(truncate_chars(&query, QUERY_CHARS)));
    }
    if !content_type.is_empty() {
        meta.insert("content_type".into(), json!(content_type));
    }
    let req_meta = if meta.is_empty() {
        None
    } else {
        Some(Value::Object(meta).to_string())
    };

    logger.send(LogRow {
        ts,
        method,
        path,
        family,
        status,
        latency_ms,
        client_ip,
        user_agent,
        amux_session,
        worker,
        req_bytes,
        resp_bytes,
        answered_by,
        error_body,
        req_meta,
    });
    res
}

// ---------------------------------------------------------------------------
// Derivations
// ---------------------------------------------------------------------------

/// Family = the boundary-registry family that owns this path (longest
/// match), else the first two segments (`/api/<seg>`). Registry-derived so
/// sweep groupings share the predicate of the ownership table they report
/// on; the fallback covers python-only paths (e.g. `/api/git/...`) that a
/// client sent to this origin.
pub fn family_of(path: &str) -> String {
    let mut best: Option<&str> = None;
    let owns = |fam: &str| path == fam || (path.starts_with(fam) && path[fam.len()..].starts_with('/'));
    for (fam, _) in super::py_proxy::NATIVE_FAMILIES {
        if owns(fam) && best.is_none_or(|b| fam.len() > b.len()) {
            best = Some(fam);
        }
    }
    for f in super::py_proxy::PROXIED_FAMILIES {
        if owns(f.family) && best.is_none_or(|b| f.family.len() > b.len()) {
            best = Some(f.family);
        }
    }
    best.map(str::to_string)
        .unwrap_or_else(|| path.split('/').take(3).collect::<Vec<_>>().join("/"))
}

/// Path-derived TARGET worker: `/api/sessions/{name}/*` -> name,
/// `/api/workers/{id}/*` -> id, else None. This single derivation is what
/// makes the worker log a subset of the global log. `/api/sessions/self`
/// resolves through its `?session=` query param (that route's contract)
/// rather than recording the literal "self".
pub fn worker_of(path: &str, query: &str) -> Option<String> {
    for prefix in ["/api/sessions/", "/api/workers/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let seg = rest.split('/').next().unwrap_or("");
            if seg.is_empty() {
                return None;
            }
            let name = percent_decode(seg);
            if name == "self" {
                return query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("session=").or_else(|| kv.strip_prefix("worker=")))
                    .map(percent_decode)
                    .filter(|s| !s.is_empty());
            }
            return Some(name);
        }
    }
    None
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Char-boundary-safe truncation (a byte slice through a UTF-8 char panics).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Minimal %XX decoder for path segments (session names arrive encoded from
/// the SPA). Invalid escapes pass through untouched.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// GET /api/logs + /api/logs/raw (the SPA Logs tab)
// ---------------------------------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_logs))
        .route("/raw", get(get_logs_raw))
}

/// GET /api/logs — Python's LIVE handler shape (amux-server.py:67673):
/// `{"events": [...], "count": N}`, params `category` / `session` / `limit`
/// (SPA sends `limit=500` + optional `category`, app.js:16520). Events carry
/// the exact key set of Python's ring events (ts/type/action/target/session/
/// detail/status/ip/actor/req/resp/method/ms) plus additive fields the sweep
/// needs (family/worker/latency_ms/answered_by/level/source/...).
///
/// Additive params (not sent by the SPA today, needed by the daily sweep —
/// docs/rust-migration/log-sweep.md): `worker` (the per-worker subset),
/// `since` (unix ts), `family`, `min_status`, `answered_by`. Additive
/// response field: `total_matched` — the pre-LIMIT count, so volume
/// questions are answerable without paging (the page-vs-corpus trap).
async fn get_logs(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let limit: i64 = q
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    let category = q.get("category").map(String::as_str).unwrap_or("");
    // Every row here is an HTTP event. A category filter for anything else
    // honestly matches nothing (Python's semantic categories — board,
    // session, memory — live in its own process; this origin does not
    // fabricate them).
    if !category.is_empty() && category != "http" {
        return Json(json!({"events": [], "count": 0, "total_matched": 0})).into_response();
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(s) = q.get("session").filter(|s| !s.is_empty()) {
        // Python's `session` concept on http events mixes the TARGET session
        // (classified from the path) with the caller header; match either so
        // neither reading silently filters to zero.
        clauses.push("(worker = ? OR amux_session = ?)".into());
        params.push(s.clone().into());
        params.push(s.clone().into());
    }
    if let Some(w) = q.get("worker").filter(|s| !s.is_empty()) {
        clauses.push("worker = ?".into());
        params.push(w.clone().into());
    }
    if let Some(f) = q.get("family").filter(|s| !s.is_empty()) {
        clauses.push("family = ?".into());
        params.push(f.clone().into());
    }
    if let Some(ts) = q.get("since").and_then(|v| v.parse::<f64>().ok()) {
        clauses.push("ts > ?".into());
        params.push(ts.into());
    }
    if let Some(ms) = q.get("min_status").and_then(|v| v.parse::<i64>().ok()) {
        clauses.push("status >= ?".into());
        params.push(ms.into());
    }
    if let Some(a) = q.get("answered_by").filter(|s| !s.is_empty()) {
        clauses.push("answered_by = ?".into());
        params.push(a.clone().into());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };

    let conn = match state.store.read() {
        Ok(c) => c,
        Err(e) => return internal(e),
    };
    let total: i64 = match conn.query_row(
        &format!("SELECT COUNT(*) FROM _amux_request_log{where_sql}"),
        rusqlite::params_from_iter(params.iter()),
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(e) => return internal(e),
    };
    let sql = format!(
        "SELECT ts, method, path, family, status, latency_ms, client_ip, user_agent, \
                amux_session, worker, req_bytes, resp_bytes, answered_by, error_body, req_meta \
         FROM _amux_request_log{where_sql} ORDER BY ts DESC LIMIT {limit}"
    );
    let events: Vec<Value> = match (|| -> rusqlite::Result<Vec<Value>> {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_event)?;
        rows.collect()
    })() {
        Ok(v) => v,
        Err(e) => return internal(e),
    };
    Json(json!({
        "events": events,
        "count": events.len(),
        "total_matched": total,
    }))
    .into_response()
}

/// One DB row -> one Python-ring-shaped event (+ additive fields).
fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let ts: f64 = r.get(0)?;
    let method: String = r.get(1)?;
    let path: String = r.get(2)?;
    let family: String = r.get(3)?;
    let status: i64 = r.get(4)?;
    let latency_ms: f64 = r.get(5)?;
    let client_ip: Option<String> = r.get(6)?;
    let user_agent: Option<String> = r.get(7)?;
    let amux_session: Option<String> = r.get(8)?;
    let worker: Option<String> = r.get(9)?;
    let req_bytes: Option<i64> = r.get(10)?;
    let resp_bytes: Option<i64> = r.get(11)?;
    let answered_by: String = r.get(12)?;
    let error_body: Option<String> = r.get(13)?;
    let req_meta: Option<String> = r.get(14)?;
    let session = worker.clone().or_else(|| amux_session.clone()).unwrap_or_default();
    let level = if status >= 500 {
        "error"
    } else if status >= 400 {
        "warn"
    } else {
        "info"
    };
    Ok(json!({
        // Python ring-event keys, verbatim (amux-server.py:3809):
        "ts": ts,
        "type": "http",
        "action": method.to_lowercase(),
        "target": path,
        "session": session,
        "detail": "",
        "status": status,
        "ip": client_ip.clone().unwrap_or_default(),
        "actor": amux_session.clone().unwrap_or_default(),
        "req": req_meta.clone().unwrap_or_default(),
        "resp": error_body.clone().unwrap_or_default(),
        "method": method,
        "ms": latency_ms.round() as i64,
        // Additive (rust request log; the sweep's discriminators):
        "category": "http",
        "level": level,
        "latency_ms": latency_ms,
        "family": family,
        "worker": worker,
        "amux_session": amux_session,
        "answered_by": answered_by,
        "req_bytes": req_bytes,
        "resp_bytes": resp_bytes,
        "user_agent": user_agent,
        "source": "request_log",
    }))
}

/// GET /api/logs/raw — Python's shape (`{"lines": [...], "total": N}`,
/// param `lines`; SPA sends `lines=300`, app.js:16549). Python tails its
/// server.log; this origin's equivalents are BOTH the tracing tail
/// (`~/.amux/logs/server-rs.log`) and the request log formatted in Python's
/// own slog line format, merged by timestamp. The additive parallel
/// `sources` array names where each line came from (`server_log` /
/// `request_log`) without perturbing the lines the SPA renders.
async fn get_logs_raw(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let lines_n = q
        .get("lines")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 5000);
    let log_path = super::settings::amux_home().join("logs").join("server-rs.log");
    match raw_payload(&log_path, lines_n, &state) {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

fn raw_payload(log_path: &Path, lines_n: usize, state: &AppState) -> anyhow::Result<Value> {
    // Tracing tail. Missing file = empty, not an error (Python parity:
    // FileNotFoundError answers {"lines": [], "total": 0}).
    let text = std::fs::read_to_string(log_path).unwrap_or_default();
    let file_total = text.lines().count();
    let mut merged: Vec<(f64, String, &'static str)> = Vec::new();
    let mut last_ts = 0.0f64;
    for line in text.lines().rev().take(lines_n).collect::<Vec<_>>().into_iter().rev() {
        // tracing fmt lines start with an RFC3339 timestamp; continuation
        // lines (panics, multi-line fields) inherit the previous line's ts.
        if let Some(ts) = line
            .split_whitespace()
            .next()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        {
            last_ts = ts.timestamp() as f64 + f64::from(ts.timestamp_subsec_millis()) / 1000.0;
        }
        merged.push((last_ts, line.to_string(), "server_log"));
    }

    let conn = state.store.read()?;
    let reqlog_total: i64 =
        conn.query_row("SELECT COUNT(*) FROM _amux_request_log", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT ts, method, path, status, latency_ms, client_ip, amux_session, worker, answered_by \
         FROM _amux_request_log ORDER BY ts DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([lines_n as i64], |r| {
        let ts: f64 = r.get(0)?;
        let method: String = r.get(1)?;
        let path: String = r.get(2)?;
        let status: i64 = r.get(3)?;
        let latency_ms: f64 = r.get(4)?;
        let ip: Option<String> = r.get(5)?;
        let session: Option<String> = r.get(6)?;
        let worker: Option<String> = r.get(7)?;
        let answered_by: String = r.get(8)?;
        Ok((ts, method, path, status, latency_ms, ip, session, worker, answered_by))
    })?;
    for row in rows {
        let (ts, method, path, status, latency_ms, ip, session, worker, answered_by) = row?;
        // Python's slog line format ("%Y-%m-%d %H:%M:%S [ip] METHOD path
        // status Nms"), so the SPA's raw-log styling treats both sources the
        // same; attribution fields append only when present.
        let when = chrono::Local
            .timestamp_opt(ts as i64, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("{ts:.0}"));
        let mut line = format!(
            "{when} [{}] {method} {path} {status} {:.0}ms",
            ip.unwrap_or_default(),
            latency_ms
        );
        if let Some(s) = session.filter(|s| !s.is_empty()) {
            line.push_str(&format!(" session={s}"));
        }
        if let Some(w) = worker.filter(|w| !w.is_empty()) {
            line.push_str(&format!(" worker={w}"));
        }
        if answered_by != "native" {
            line.push_str(&format!(" via={answered_by}"));
        }
        merged.push((ts, line, "request_log"));
    }

    merged.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let tail: Vec<_> = merged
        .into_iter()
        .rev()
        .take(lines_n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(json!({
        "lines": tail.iter().map(|(_, l, _)| l.clone()).collect::<Vec<_>>(),
        "total": file_total as i64 + reqlog_total,
        "sources": tail.iter().map(|(_, _, s)| *s).collect::<Vec<_>>(),
    }))
}

fn internal(e: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.to_string()})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests — temp DBs only; the middleware is exercised through the SHIPPED
// wiring (layer_with), not a paraphrase of it (ethos rule 7).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Live-oracle fixture: `GET https://localhost:8822/api/logs?limit=3`
    /// captured 2026-08-09 against the running Python server (build of that
    /// day). One representative ring event verbatim — the KEY SET is the
    /// contract the SPA maps over (app.js:16524-16529).
    const PYTHON_LOGS_FIXTURE: &str = r#"{
        "events": [{
            "ts": 1786315119.485713,
            "type": "http",
            "action": "get",
            "target": "/api/sessions/mixpeek-autopilot/peek",
            "session": "",
            "detail": "",
            "status": 304,
            "ip": "127.0.0.1",
            "actor": "",
            "req": "",
            "resp": "",
            "method": "GET",
            "ms": 376
        }],
        "count": 1
    }"#;

    /// `GET https://localhost:8822/api/logs/raw?lines=3`, same capture:
    /// {"lines": ["2026-08-09 18:38:39 [127.0.0.1] GET /api/... 304 377ms",
    /// ...], "total": 334708}.
    const PYTHON_RAW_KEYS: &[&str] = &["lines", "total"];

    fn store() -> (Arc<crate::db::Store>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::db::Store::open(&dir.path().join("t.db")).unwrap();
        (Arc::new(s), dir)
    }

    fn state(store: Arc<crate::db::Store>) -> AppState {
        AppState {
            store,
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        }
    }

    /// A tiny app behind the SHIPPED layer wiring: routes that let each test
    /// provoke exactly one property (proxy stamp, slow handler, big body,
    /// error body).
    fn test_app(logger: RequestLogger) -> Router {
        let inner: Router = Router::new()
            .route(
                "/api/sessions/{name}/peek",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    ([("x-amux-answered-by", "python-proxy")], "ok")
                }),
            )
            .route(
                "/api/echo",
                axum::routing::post(|b: axum::body::Bytes| async move { format!("{}", b.len()) })
                    .layer(axum::extract::DefaultBodyLimit::disable()),
            )
            .route(
                "/api/fail",
                get(|| async {
                    (StatusCode::INTERNAL_SERVER_ERROR, "E".repeat(10_000))
                }),
            )
            .route("/api/board", get(|| async { "[]" }));
        layer_with(inner, logger)
    }

    async fn hit(app: &Router, req: HttpRequest<Body>) -> (StatusCode, Vec<u8>) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, body.to_vec())
    }

    /// Poll until the async writer has landed `n` rows (the channel is
    /// deliberately out-of-band; tests wait on the DB, the source of truth).
    async fn wait_rows(store: &crate::db::Store, n: i64) -> i64 {
        for _ in 0..200 {
            let c = store.read().unwrap();
            let got: i64 = c
                .query_row("SELECT COUNT(*) FROM _amux_request_log", [], |r| r.get(0))
                .unwrap();
            if got >= n {
                return got;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let c = store.read().unwrap();
        c.query_row("SELECT COUNT(*) FROM _amux_request_log", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn derivations_worker_and_family() {
        assert_eq!(worker_of("/api/sessions/amux/peek", ""), Some("amux".into()));
        assert_eq!(worker_of("/api/sessions/my%20w/send", ""), Some("my w".into()));
        assert_eq!(worker_of("/api/workers/wrk_01ABC/status", ""), Some("wrk_01ABC".into()));
        assert_eq!(worker_of("/api/workers/wrk_01ABC", ""), Some("wrk_01ABC".into()));
        assert_eq!(worker_of("/api/sessions", ""), None);
        assert_eq!(worker_of("/api/board/AMUX-1", ""), None);
        // /api/sessions/self resolves through its query param, never "self".
        assert_eq!(worker_of("/api/sessions/self", "session=amux"), Some("amux".into()));
        assert_eq!(worker_of("/api/sessions/self", ""), None);

        assert_eq!(family_of("/api/board/AMUX-1"), "/api/board");
        assert_eq!(family_of("/api/sessions/amux/peek"), "/api/sessions");
        assert_eq!(family_of("/api/calendar.ics"), "/api/calendar.ics");
        // Registry miss (python-only path): first two segments.
        assert_eq!(family_of("/api/git/staged-guard"), "/api/git");
    }

    #[tokio::test]
    async fn request_becomes_row_with_attribution_latency_and_answered_by() {
        let (store, _dir) = store();
        let app = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 1_000_000));
        let (st, _) = hit(
            &app,
            HttpRequest::builder()
                .uri("/api/sessions/w1/peek?lines=600")
                .header("x-amux-session", "caller-lane")
                .header("user-agent", "test-agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(wait_rows(&store, 1).await, 1);
        let c = store.read().unwrap();
        let (path, family, worker, sess, status, latency, answered, meta): (
            String, String, Option<String>, String, i64, f64, String, Option<String>,
        ) = c
            .query_row(
                "SELECT path, family, worker, amux_session, status, latency_ms, answered_by, req_meta \
                 FROM _amux_request_log",
                [],
                |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
                },
            )
            .unwrap();
        assert_eq!(path, "/api/sessions/w1/peek");
        assert_eq!(family, "/api/sessions");
        assert_eq!(worker.as_deref(), Some("w1"), "worker attribution from path");
        assert_eq!(sess, "caller-lane");
        assert_eq!(status, 200);
        assert!(latency >= 10.0, "handler sleeps 15ms; measured {latency}ms");
        assert_eq!(answered, "python-proxy", "x-amux-answered-by response header");
        let meta: Value = serde_json::from_str(&meta.unwrap()).unwrap();
        assert_eq!(meta["query"], "lines=600");
    }

    #[tokio::test]
    async fn a_25mb_body_never_lands_in_the_log() {
        let (store, _dir) = store();
        let app = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 1_000_000));
        let big = vec![b'a'; 25 * 1024 * 1024];
        let (st, body) = hit(
            &app,
            HttpRequest::builder()
                .method("POST")
                .uri("/api/echo")
                .header("content-type", "application/octet-stream")
                .header("content-length", big.len().to_string())
                .body(Body::from(big.clone()))
                .unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body, format!("{}", big.len()).into_bytes(), "handler saw the full body");
        assert_eq!(wait_rows(&store, 1).await, 1);
        let c = store.read().unwrap();
        let (req_bytes, err_body, meta_len, row_bytes): (i64, Option<String>, i64, i64) = c
            .query_row(
                "SELECT req_bytes, error_body, LENGTH(COALESCE(req_meta,'')), \
                        LENGTH(COALESCE(path,''))+LENGTH(COALESCE(user_agent,''))+\
                        LENGTH(COALESCE(req_meta,''))+LENGTH(COALESCE(error_body,'')) \
                 FROM _amux_request_log",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(req_bytes, 25 * 1024 * 1024, "SIZE is recorded");
        assert_eq!(err_body, None, "success bodies are never captured");
        assert!(meta_len < 700, "req_meta stays capped: {meta_len}");
        assert!(row_bytes < 2000, "whole row stays small: {row_bytes} bytes");
    }

    #[tokio::test]
    async fn error_body_captured_capped_and_response_undamaged() {
        let (store, _dir) = store();
        let app = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 1_000_000));
        let (st, body) = hit(
            &app,
            HttpRequest::builder().uri("/api/fail").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.len(), 10_000, "client still receives the FULL error body");
        assert_eq!(wait_rows(&store, 1).await, 1);
        let c = store.read().unwrap();
        let (err_body, resp_bytes): (String, i64) = c
            .query_row("SELECT error_body, resp_bytes FROM _amux_request_log", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(err_body.chars().count(), ERROR_BODY_CHARS, "first 500 chars only");
        assert_eq!(resp_bytes, 10_000, "exact buffered size recorded");
    }

    #[tokio::test]
    async fn excluded_paths_never_log_and_everything_else_does() {
        let (store, _dir) = store();
        let app = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 1_000_000));
        for path in ["/health", "/api/events", "/api/debug/boundary", "/app.js", "/"] {
            let _ = hit(&app, HttpRequest::builder().uri(path).body(Body::empty()).unwrap()).await;
        }
        // A logged request AFTER the excluded ones: the channel is FIFO, so
        // when this row is visible, any (wrongly) sent earlier row would be
        // too — the absence check cannot pass by racing.
        let _ = hit(&app, HttpRequest::builder().uri("/api/board").body(Body::empty()).unwrap()).await;
        assert_eq!(wait_rows(&store, 1).await, 1, "exactly the /api/board row");
        let c = store.read().unwrap();
        let path: String =
            c.query_row("SELECT path FROM _amux_request_log", [], |r| r.get(0)).unwrap();
        assert_eq!(path, "/api/board");
    }

    #[tokio::test]
    async fn retention_sweep_fires_and_deletes_old_rows() {
        let (store, _dir) = store();
        // Pre-plant two ancient rows (90 days old) straight through the
        // writer — the specimen the sweep exists to delete.
        let old_ts = unix_now() - 90.0 * 86400.0;
        store
            .write_async(move |conn| {
                for i in 0..2 {
                    conn.execute(
                        "INSERT INTO _amux_request_log \
                         (ts, method, path, family, status, latency_ms, answered_by) \
                         VALUES (?1, 'GET', ?2, '/api/board', 200, 1.0, 'native')",
                        rusqlite::params![old_ts + f64::from(i), format!("/api/board/old{i}")],
                    )?;
                }
                Ok(WriteOutcome { applied: false, events: vec![] })
            })
            .await
            .unwrap();
        // sweep_every=3: the third inserted row triggers the delete.
        let app = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 3));
        for _ in 0..3 {
            let _ = hit(&app, HttpRequest::builder().uri("/api/board").body(Body::empty()).unwrap()).await;
        }
        for _ in 0..200 {
            let c = store.read().unwrap();
            let old_left: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM _amux_request_log WHERE ts < ?1",
                    [unix_now() - 14.0 * 86400.0],
                    |r| r.get(0),
                )
                .unwrap();
            let fresh: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM _amux_request_log WHERE ts > ?1",
                    [unix_now() - 3600.0],
                    |r| r.get(0),
                )
                .unwrap();
            if old_left == 0 && fresh == 3 {
                return; // swept the ancient rows, kept the fresh ones
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("retention sweep did not fire (old rows still present after 5s)");
    }

    #[tokio::test]
    async fn api_logs_matches_python_fixture_shape_and_worker_param_subsets() {
        let (store, _dir) = store();
        let app_state = state(store.clone());
        let logged = test_app(RequestLogger::spawn_with(store.clone(), 14.0, 1_000_000));
        // Two rows: one worker-scoped, one not.
        let _ = hit(
            &logged,
            HttpRequest::builder()
                .uri("/api/sessions/w1/peek")
                .header("x-amux-session", "caller")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let _ = hit(&logged, HttpRequest::builder().uri("/api/board").body(Body::empty()).unwrap()).await;
        wait_rows(&store, 2).await;

        let api: Router = Router::new()
            .nest("/api/logs", routes())
            .with_state(app_state);
        let (st, body) = hit(
            &api,
            HttpRequest::builder().uri("/api/logs?limit=500").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let ours: Value = serde_json::from_slice(&body).unwrap();
        let fixture: Value = serde_json::from_str(PYTHON_LOGS_FIXTURE).unwrap();
        // Top-level: python's exact keys, present with python's types.
        assert!(ours["events"].is_array());
        assert!(ours["count"].is_number());
        // Event keys: every key python's live ring event carries must be
        // present on our events (the SPA maps over exactly these).
        let py_event = fixture["events"][0].as_object().unwrap();
        let our_event = ours["events"][0].as_object().unwrap();
        for key in py_event.keys() {
            assert!(our_event.contains_key(key), "python event key {key:?} missing from ours");
        }
        assert_eq!(our_event["type"], "http");
        assert_eq!(our_event["action"], "get");

        // Worker subset: same endpoint, ?worker= filter.
        let (st, body) = hit(
            &api,
            HttpRequest::builder().uri("/api/logs?worker=w1").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["count"], 1, "{v}");
        assert_eq!(v["total_matched"], 1);
        assert_eq!(v["events"][0]["worker"], "w1");
        assert_eq!(v["events"][0]["target"], "/api/sessions/w1/peek");

        // A category python COULD send that this origin has no events for:
        // honestly empty, python's shape.
        let (_, body) = hit(
            &api,
            HttpRequest::builder().uri("/api/logs?category=board").body(Body::empty()).unwrap(),
        )
        .await;
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["events"], json!([]));
        assert_eq!(v["count"], 0);
    }

    #[tokio::test]
    async fn api_logs_raw_merges_both_sources_and_labels_them() {
        let (store, _dir) = store();
        // A fake tracing log with an RFC3339-stamped line + a continuation.
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("logs")).unwrap();
        std::fs::write(
            home.path().join("logs/server-rs.log"),
            "2026-08-09T18:00:00.000000Z  INFO amux_server: listening\n  continuation line\n",
        )
        .unwrap();
        // One request-log row, newer than the file lines.
        store
            .write_async(|conn| {
                conn.execute(
                    "INSERT INTO _amux_request_log \
                     (ts, method, path, family, status, latency_ms, client_ip, amux_session, worker, answered_by) \
                     VALUES (?1, 'GET', '/api/board', '/api/board', 200, 12.3, '127.0.0.1', 'caller', NULL, 'python-proxy')",
                    [unix_now()],
                )?;
                Ok(WriteOutcome { applied: false, events: vec![] })
            })
            .await
            .unwrap();

        let payload =
            raw_payload(&home.path().join("logs/server-rs.log"), 300, &state(store.clone())).unwrap();
        for key in PYTHON_RAW_KEYS {
            assert!(payload.get(*key).is_some(), "python raw key {key:?} missing");
        }
        let lines: Vec<String> = payload["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let sources: Vec<String> = payload["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(lines.len(), sources.len(), "sources labels every line");
        assert_eq!(payload["total"], 3, "2 file lines + 1 request-log row");
        assert!(sources.contains(&"server_log".to_string()));
        assert!(sources.contains(&"request_log".to_string()));
        // The request-log line uses python's slog format and is the NEWEST,
        // so it merges last; proxy attribution rides the line.
        let last = lines.last().unwrap();
        assert!(last.contains("[127.0.0.1] GET /api/board 200 12ms"), "{last}");
        assert!(last.contains("session=caller"), "{last}");
        assert!(last.contains("via=python-proxy"), "{last}");
        assert!(
            regex::Regex::new(r"^\d{4}-\d{2}-\d{2} ").unwrap().is_match(last),
            "python slog date shape so SPA styling applies: {last}"
        );
        // Missing file: python's empty-shape parity, request log still served.
        let empty = raw_payload(Path::new("/nonexistent/nope.log"), 10, &state(store)).unwrap();
        assert_eq!(empty["lines"].as_array().unwrap().len(), 1);
        assert_eq!(empty["total"], 1);
    }
}
