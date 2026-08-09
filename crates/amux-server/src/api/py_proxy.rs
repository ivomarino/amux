//! Strangler-fig passthrough: session-management verbs on PYTHON-fleet
//! sessions forward to the Python server, which owns those sessions until
//! cutover.
//!
//! The lightning (YOLO) button, peek, send, start/stop, config — the SPA
//! calls them as /api/sessions/{name}/<verb>. Reimplementing Python's
//! env-rewrite + live-restart choreography per verb would duplicate the
//! exact machinery cutover retires; proxying keeps ONE implementation
//! authoritative and makes every verb correct from the Rust origin today
//! (Ethan: "the lightning button isn't correct").
//!
//! Rust-managed workers (wrk_ ids) do NOT proxy — their verbs live under
//! /api/workers; a legacy-path call against one gets a pointer, not a
//! silent Python 404.

use super::AppState;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

fn py_base() -> String {
    std::env::var("AMUX_PY_URL").unwrap_or_else(|_| "https://localhost:8822".into())
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

    // Forward verbatim: method, path+query, auth + amux headers, body.
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_default();
    let mut headers_fwd = vec![];
    for h in ["authorization", "content-type", "x-amux-session", "x-amux-ui-token"] {
        if let Some(v) = req.headers().get(h) {
            headers_fwd.push((h.to_string(), v.clone()));
        }
    }
    let body = match axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // the Python server's self-signed localhost cert
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("proxy client");
    let mut fwd = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        format!("{}{}", py_base(), path_and_query),
    );
    for (k, v) in headers_fwd {
        if let Ok(v) = v.to_str() {
            fwd = fwd.header(k, v);
        }
    }
    if !body.is_empty() {
        fwd = fwd.body(body.to_vec());
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
                "error": "python server unreachable for proxied session verb",
                "detail": e.to_string(),
                "python": py_base(),
            })),
        )
            .into_response(),
    }
}
