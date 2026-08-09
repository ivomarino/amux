//! Browser API (RR-0092): HTTP surface over `integrations::browser` — the
//! native Chrome profile manager (plan lesson L7). Route names follow the
//! Python server's `/api/browser/*` family; the NATIVE routes cover the
//! machine-local subset (profiles, start/stop/status, profile/create).
//!
//! Everything else — the Python server's browser-use driver verbs
//! (`/action`, `/state`, `/agent`, `/inspect`, `/save-profile`, `/navigate`,
//! `/search`, `/sessions`, `/pw-profiles`, ...) and `/screenshot` — proxies
//! to the Python server via the nest fallback (strangler-fig, same shape as
//! the session verbs). The playwright driver and its CDP websocket live in
//! the Python process ON THIS SAME MACHINE, which keeps the user directive
//! "browser stuff must use the machine the server runs on" true through the
//! proxy. Known pre-cutover seam, on purpose: a Chrome launched by the
//! native `/start` is NOT the browser the proxied driver verbs operate —
//! Python resolves its own per-session driver (auto-launching one on first
//! use). Cutover replaces the proxied half with a native driver; until then
//! one implementation stays authoritative.
//!
//! Routes (nested at `/api/browser`):
//! - `POST /start {profile?, url?}`     — launch Chrome on a profile; if one
//!   is already running it is stopped first (never two Chromes on a profile)
//! - `GET  /status`                     — running child + CDP tab list
//! - `POST /stop`                       — SIGTERM, wait, clean stale locks
//! - `GET  /profiles?sizes=1`           — inventory (sizes opt-in; see L7)
//! - `POST /profile/create {name, url?}`— create a profile dir, optionally
//!   open a headed window on it to sign in
//! - anything else                      — python-proxy fallback (incl.
//!   `/screenshot`, which needs the CDP WebSocket this build cannot speak;
//!   proxying replaced the former honest 501)

use super::AppState;
use crate::integrations::browser as chrome;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/start", post(start))
        .route("/status", get(status))
        .route("/stop", post(stop))
        .route("/profiles", get(profiles))
        .route("/profile/create", post(profile_create))
        // Unimplemented verbs (screenshot, action, state, agent, inspect,
        // save-profile, navigate, search, sessions, pw-profiles, DELETE
        // profile/{name}, ...) forward to the Python owner — including its
        // helpful 404 catalog for unknown routes, which stays identical on
        // both origins by construction. EXPLICIT wildcard routes, not
        // `.fallback()`: in the full composition the static SPA catch-all
        // (`/{*path}`) out-competes a nested fallback and served index.html
        // for every driver verb (caught live on 18940; unit tests missed it
        // because they had no competing static route).
        .route("/", axum::routing::any(super::py_proxy::forward_handler))
        .route("/{*rest}", axum::routing::any(super::py_proxy::forward_handler))
}

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

#[derive(Deserialize, Default)]
struct StartBody {
    #[serde(default)]
    profile: String,
    #[serde(default)]
    url: String,
}

async fn start(body: Option<Json<StartBody>>) -> Response {
    let Json(body) = body.unwrap_or_default();
    let home = chrome::amux_home();
    match chrome::start(&home, &body.profile, &body.url).await {
        Ok(info) => {
            let mut v = serde_json::to_value(&info).unwrap_or_else(|_| json!({}));
            v["ok"] = json!(true);
            Json(v).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
    }
}

async fn status() -> Response {
    // Read the registry under the lock, then drop it before any await —
    // holding a std::sync::Mutex across an await point deadlocks the runtime.
    let snapshot = {
        let guard = chrome::RUNNING.lock().expect("browser registry poisoned");
        guard.as_ref().map(|r| (r.profile.clone(), r.cdp_port, r.started_at))
    };
    let Some((profile, cdp_port, started_at)) = snapshot else {
        return Json(json!({ "running": false })).into_response();
    };
    // Tabs are best-effort: a hung Chrome should degrade the field, not the
    // endpoint. `tabs: null` + `tabs_error` says "could not ask", which is a
    // different fact from "no tabs" (ethos rule 4).
    let (tabs, tabs_error) = match chrome::cdp_list(cdp_port).await {
        Ok(t) => (t, Value::Null),
        Err(e) => (Value::Null, json!(e.to_string())),
    };
    Json(json!({
        "running": true,
        "profile": profile,
        "cdp_port": cdp_port,
        "started_at": started_at,
        "tabs": tabs,
        "tabs_error": tabs_error,
    }))
    .into_response()
}

async fn stop() -> Response {
    let home = chrome::amux_home();
    let report = chrome::stop(&home).await;
    let mut v = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
    v["ok"] = json!(true);
    Json(v).into_response()
}

#[derive(Deserialize, Default)]
struct ProfilesQuery {
    #[serde(default)]
    sizes: Option<String>,
}

async fn profiles(Query(q): Query<ProfilesQuery>) -> Response {
    let with_sizes = q.sizes.as_deref().is_some_and(|s| !s.is_empty() && s != "0");
    let home = chrome::amux_home();
    // Size walks touch a few hundred files per profile — off the runtime.
    let list =
        match tokio::task::spawn_blocking(move || chrome::list_profiles(&home, with_sizes)).await {
            Ok(l) => l,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
        };
    Json(json!({ "profiles": list, "backends": ["native"] })).into_response()
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    url: String,
}

async fn profile_create(Json(body): Json<CreateBody>) -> Response {
    let name = body.name.trim().to_string();
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": "profile name must be [A-Za-z0-9._-]+" }),
        );
    }
    if name == "default" {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "'default' already exists" }));
    }
    let home = chrome::amux_home();
    // Create in the amux-owned location so create-path == use-path (the L7
    // mismatch this subsystem exists to end). resolve_profile_dir will find
    // it here from now on because the dir exists.
    let dir = home.join("playwright-auth").join("profiles").join(&name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() }));
    }
    // A sign-in URL means "open a headed window on the new profile so a
    // human can log in" — same intent as the Python create flow.
    let mut launched = false;
    let mut launch_error = Value::Null;
    if !body.url.trim().is_empty() {
        match chrome::start(&home, &name, body.url.trim()).await {
            Ok(_) => launched = true,
            Err(e) => launch_error = json!(e.to_string()),
        }
    }
    Json(json!({
        "ok": true,
        "profile": name,
        "path": dir.display().to_string(),
        "launched": launched,
        "launch_error": launch_error,
        "note": "sign in through the opened window, then POST /api/browser/stop to flush the profile",
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Tests — proxy routing only (native launch paths need a real Chrome).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::py_proxy::tests::{fake_python, PY_BASE_OVERRIDE, PY_LOCK};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn app() -> Router {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::db::Store::open(&dir.path().join("t.db")).unwrap());
        std::mem::forget(dir);
        let state = AppState {
            store,
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        Router::new().nest("/api/browser", routes()).with_state(state)
    }

    #[tokio::test]
    async fn driver_verbs_and_screenshot_proxy_but_status_stays_native() {
        let _guard = PY_LOCK.lock().await;
        let (base, log) =
            fake_python(StatusCode::OK, r#"{"ok": true, "path": "/tmp/shot.png"}"#).await;
        *PY_BASE_OVERRIDE.lock().unwrap() = Some(base);
        let app = app();

        // /screenshot: forwarded with full original path+query.
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/browser/screenshot?session=t1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("x-amux-answered-by").unwrap(), "python-proxy");

        // /action (a driver verb with no native route): forwarded too.
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/browser/action")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"action":"eval","session":"t1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("x-amux-answered-by").unwrap(), "python-proxy");

        let seen = log.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "both requests reached the fake python");
        assert_eq!(seen[0].path_and_query, "/api/browser/screenshot?session=t1");
        assert_eq!(seen[1].path_and_query, "/api/browser/action");
        assert_eq!(seen[1].body, br#"{"action":"eval","session":"t1"}"#.to_vec());

        // /status is NATIVE: no third request may appear at the fake python,
        // and with no launched Chrome it answers running:false locally.
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/browser/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().get("x-amux-answered-by").is_none(), "status answered locally");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["running"], json!(false));
        assert_eq!(log.lock().unwrap().len(), 2, "status did not proxy");
        *PY_BASE_OVERRIDE.lock().unwrap() = None;
    }
}
