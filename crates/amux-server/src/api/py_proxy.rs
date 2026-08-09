//! Strangler-fig passthrough: endpoint families the PYTHON server still owns
//! forward to it until cutover.
//!
//! Three families ride this today:
//!
//! - **Session-management verbs** on Python-fleet sessions
//!   (`/api/sessions/{name}/<verb>`): the lightning (YOLO) button, peek, send,
//!   start/stop, config. Reimplementing Python's env-rewrite + live-restart
//!   choreography per verb would duplicate the exact machinery cutover
//!   retires (Ethan: "the lightning button isn't correct").
//! - **`/api/fs/*`** (the SPA's Files surface: multipart upload with a `dir`
//!   field answering `{saved:[...]}`, open, mkdir, rename, read, list,
//!   search, delete). This is a DIFFERENT contract from the Rust-native
//!   `/api/files` (raw-body upload, `?path=` relative to a root) — a route
//!   alias between the two would silently serve the wrong request AND
//!   response shapes, so the namespace proxies wholesale instead.
//! - **Browser driver verbs + transcription** (`/api/browser/*` fallback,
//!   `POST /api/dictate`, the Gemini half of dictation): the engines
//!   (playwright driver, whisper, Gemini) live in the Python process, on the
//!   same machine — which also satisfies "browser stuff must use the machine
//!   the server runs on".
//!
//! Rust-managed workers (wrk_ ids) do NOT proxy — their verbs live under
//! /api/workers; a legacy-path call against one gets a pointer, not a
//! silent Python 404.
//!
//! Transport notes, load-bearing:
//! - The Python server is `BaseHTTPRequestHandler`-based: it reads request
//!   bodies via `Content-Length` and does NOT speak chunked uploads. So the
//!   forwarder BUFFERS the body (giving reqwest a known length) rather than
//!   streaming — a streamed body would arrive at Python as zero bytes.
//!   Python buffers whole uploads in RAM too (`rfile.read(length)`), so this
//!   is parity, not a new cost.
//! - Headers forward denylist-style (hop-by-hop stripped) so multipart
//!   boundaries, `X-Amux-Session`, `X-Amux-UI-Token` and future headers
//!   survive without each needing to be remembered here.
//! - Every proxied response carries `x-amux-answered-by: python-proxy` so a
//!   debugging session can tell which server actually answered (ethos rule 4).

use super::AppState;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::json;

fn py_base() -> String {
    #[cfg(test)]
    if let Some(v) = tests::PY_BASE_OVERRIDE.lock().expect("py base override").clone() {
        return v;
    }
    std::env::var("AMUX_PY_URL").unwrap_or_else(|_| "https://localhost:8822".into())
}

/// Hop-by-hop / transport-computed headers that must not be copied onto the
/// forwarded request (reqwest derives its own).
const SKIP_REQUEST_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "accept-encoding",
    "expect",
    "upgrade",
    "keep-alive",
    "te",
    "trailer",
    "proxy-authorization",
    "proxy-authenticate",
];

/// Forward an already-decomposed request to the Python server. The shared
/// core under every proxied family; callers that still hold the whole
/// [`Request`] use [`forward_to_python`] instead.
pub async fn forward_built(
    method: axum::http::Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Vec<u8>,
) -> Response {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // the Python server's self-signed localhost cert
        // Long deliberately: /api/browser/agent runs a multi-minute model
        // loop and /api/fs/upload can carry a large file; a 30s cap here
        // turned real work into fake 502s.
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("proxy client");
    let mut fwd = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        format!("{}{}", py_base(), path_and_query),
    );
    for (k, v) in headers {
        if SKIP_REQUEST_HEADERS.contains(&k.as_str()) {
            continue;
        }
        if let Ok(v) = v.to_str() {
            fwd = fwd.header(k.as_str(), v);
        }
    }
    if !body.is_empty() {
        fwd = fwd.body(body);
    }

    match fwd.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();
            (
                status,
                [
                    ("content-type", ct),
                    // Name the origin so a debugging session can tell which
                    // server actually answered (ethos rule 4).
                    ("x-amux-answered-by", "python-proxy".into()),
                ],
                bytes.to_vec(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": "python server unreachable for proxied path",
                "path": path_and_query,
                "detail": e.to_string(),
                "python": py_base(),
            })),
        )
            .into_response(),
    }
}

/// Forward this request verbatim (method, ORIGINAL path+query, headers,
/// body) to the Python server.
pub async fn forward_to_python(req: Request) -> Response {
    // Nested routers strip their prefix from `req.uri()`; OriginalUri keeps
    // the path the CLIENT sent, which is the path Python routes on.
    let uri = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|u| u.0.clone())
        .unwrap_or_else(|| req.uri().clone());
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_default();
    let method = req.method().clone();
    let headers = req.headers().clone();
    // Buffered, unbounded: Python imposes no cap on /api/fs/upload and reads
    // the whole body into RAM the same way; inventing a cap here would make
    // the Rust origin reject uploads the Python origin accepts.
    let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b.to_vec(),
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    forward_built(method, &path_and_query, &headers, body).await
}

/// Axum handler shape of [`forward_to_python`] for direct route mounting.
pub async fn forward_handler(req: Request) -> Response {
    forward_to_python(req).await
}

/// Whole-namespace passthrough router (mounted in mod.rs for `/api/fs`,
/// `/api/groups`, `/api/tags`). EXPLICIT `/` + `/{*rest}` routes, not a
/// `.fallback()`: in the full app composition the static SPA catch-all
/// (`/{*path}`) out-competes a nested router's fallback, so a fallback-based
/// passthrough silently served index.html instead of proxying — the exact
/// swallow that broke the SPA's group picker (AMUX-2594) and misled an auth
/// probe into reporting an unauthenticated 200 on /api/fs.
///
/// Body limit disabled: Python's upload handler has NO size cap ("No size
/// limit on file uploads", amux-server.py:857), and axum's 2MB default would
/// 413 a drag-and-dropped file the Python origin accepts.
pub fn passthrough_routes() -> Router<AppState> {
    Router::new()
        .route("/", any(forward_handler))
        .route("/{*rest}", any(forward_handler))
        .layer(axum::extract::DefaultBodyLimit::disable())
}

pub async fn proxy_session_verb(
    State(state): State<AppState>,
    Path(params): Path<Vec<(String, String)>>,
    req: Request,
) -> Response {
    let name = params
        .iter()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    // Rust-managed worker? Its verbs are the modern API's.
    let is_rust_worker = {
        match state.store.read() {
            Ok(conn) => crate::db::queries::get_worker(&conn, &name)
                .ok()
                .flatten()
                .is_some(),
            Err(_) => false,
        }
    };
    if is_rust_worker {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": "rust-managed worker — use /api/workers",
                "worker": name,
                "hint": format!("/api/workers/{name}"),
            })),
        )
            .into_response();
    }

    forward_to_python(req).await
}

// ---------------------------------------------------------------------------
// Tests — a local fake "Python" listener; no live servers touched.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Test override for the Python base URL (same pattern as dictation's
    /// ENV_KEY_OVERRIDE: process-env reads are not hermetic).
    pub(crate) static PY_BASE_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
    /// Serializes tests that set the override (async-aware).
    pub(crate) static PY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// One captured request, as the fake Python saw it.
    #[derive(Clone, Debug)]
    pub(crate) struct Seen {
        pub method: String,
        pub path_and_query: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    /// Spin a plain-HTTP fake Python server answering `status`/`body` to
    /// every request and recording what arrived. Returns (base_url, log).
    pub(crate) async fn fake_python(
        status: StatusCode,
        body: &'static str,
    ) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let log: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let log_c = log.clone();
        let app = Router::new().fallback(move |req: Request| {
            let log = log_c.clone();
            async move {
                let method = req.method().to_string();
                let path_and_query = req
                    .uri()
                    .path_and_query()
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_default();
                let headers = req
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap_or_default();
                log.lock().expect("seen log").push(Seen {
                    method,
                    path_and_query,
                    headers,
                    body: bytes.to_vec(),
                });
                (status, [("content-type", "application/json")], body)
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (format!("http://{addr}"), log)
    }

    fn state() -> AppState {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::db::Store::open(&dir.path().join("t.db")).unwrap());
        std::mem::forget(dir);
        AppState {
            store,
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        }
    }

    #[tokio::test]
    async fn fs_passthrough_forwards_verbatim_and_stamps_answered_by() {
        let _guard = PY_LOCK.lock().await;
        let (base, log) = fake_python(StatusCode::OK, r#"{"saved": [{"name": "a.txt", "size": 2}]}"#).await;
        *PY_BASE_OVERRIDE.lock().unwrap() = Some(base);

        let app: Router = Router::new()
            .nest("/api/fs", passthrough_routes())
            .with_state(state());
        // Multipart-shaped body with a boundary in the content-type: the
        // header must survive the hop byte-for-byte or Python cannot parse
        // the form at all.
        let body = b"--XX\r\nContent-Disposition: form-data; name=\"dir\"\r\n\r\n/tmp\r\n--XX--\r\n".to_vec();
        use tower::ServiceExt;
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/fs/upload?x=1")
                    .header("content-type", "multipart/form-data; boundary=XX")
                    .header("x-amux-session", "updict-test")
                    .header("authorization", "Bearer tok-abc")
                    .body(axum::body::Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("x-amux-answered-by").unwrap(),
            "python-proxy"
        );
        let out = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&out[..], br#"{"saved": [{"name": "a.txt", "size": 2}]}"#);

        let seen = log.lock().unwrap().first().cloned().expect("request reached fake python");
        assert_eq!(seen.method, "POST");
        // OriginalUri: the nested router must NOT strip /api/fs off the
        // forwarded path — Python routes on the full path.
        assert_eq!(seen.path_and_query, "/api/fs/upload?x=1");
        assert_eq!(seen.body, body, "body forwarded byte-for-byte");
        let h = |k: &str| {
            seen.headers
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(h("content-type").unwrap(), "multipart/form-data; boundary=XX");
        assert_eq!(h("x-amux-session").unwrap(), "updict-test");
        assert_eq!(h("authorization").unwrap(), "Bearer tok-abc");
        *PY_BASE_OVERRIDE.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn unreachable_python_is_an_honest_502() {
        let _guard = PY_LOCK.lock().await;
        // A port nothing listens on: reserve via bind-then-drop.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        *PY_BASE_OVERRIDE.lock().unwrap() = Some(dead);

        let app: Router = Router::new()
            .nest("/api/fs", passthrough_routes())
            .with_state(state());
        use tower::ServiceExt;
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/fs/list?path=/tmp")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        let out = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["error"], json!("python server unreachable for proxied path"));
        assert_eq!(v["path"], json!("/api/fs/list?path=/tmp"));
        *PY_BASE_OVERRIDE.lock().unwrap() = None;
    }
}
