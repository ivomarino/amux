//! Browser API (RR-0092): HTTP surface over `integrations::browser` — the
//! native Chrome profile manager (plan lesson L7). Route names follow the
//! Python server's `/api/browser/*` family; the shapes cover the native
//! subset (profiles, start/stop/status). The Python server's browser-use
//! driver verbs (`/action`, `/state`, `/agent`, ...) are model-driven and out
//! of scope for the native manager.
//!
//! Routes (nested at `/api/browser`):
//! - `POST /start {profile?, url?}`     — launch Chrome on a profile; if one
//!   is already running it is stopped first (never two Chromes on a profile)
//! - `GET  /status`                     — running child + CDP tab list
//! - `POST /stop`                       — SIGTERM, wait, clean stale locks
//! - `GET  /profiles?sizes=1`           — inventory (sizes opt-in; see L7)
//! - `POST /profile/create {name, url?}`— create a profile dir, optionally
//!   open a headed window on it to sign in
//! - `GET  /screenshot`                 — honest 501: needs the CDP
//!   WebSocket, which this build does not speak (see below)

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
        .route("/screenshot", get(screenshot))
        .route("/profiles", get(profiles))
        .route("/profile/create", post(profile_create))
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

// TODO(RR-0092): CDP WS screenshot needs a websocket dep decision.
// `Page.captureScreenshot` is only reachable over the DevTools WebSocket;
// the HTTP endpoints (/json/list, /json/new) cannot express it, and this
// workspace has no websocket client crate. 501 with the missing capability
// named beats a fake success or a shelled-out workaround (ethos rule 7).
async fn screenshot() -> Response {
    err(
        StatusCode::NOT_IMPLEMENTED,
        json!({
            "error": "screenshot requires the Chrome DevTools WebSocket (Page.captureScreenshot); \
                      this build speaks CDP over HTTP only (/json/list, /json/new)",
            "missing_capability": "websocket client dependency (undecided — see TODO(RR-0092))",
        }),
    )
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
