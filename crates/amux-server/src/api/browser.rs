//! Browser API (RR-0092 + AMUX-2598 cutover): the FULL `/api/browser/*`
//! family, answered natively. Route names and request/response shapes follow
//! the Python server's handlers (amux-server.py `/api/browser` block,
//! py:74324-74828); the mechanics are `integrations::browser` — a native
//! Chrome launch plus a real CDP WebSocket client ([`chrome::CdpClient`]).
//!
//! What replaced the proxy (this module's rows used to be PROXIED_FAMILIES
//! row "browser driver verbs"):
//! - `/navigate`, `/screenshot`, `/state`, `/action`, `/inspect`(+`/clear`),
//!   `/search` — mechanical CDP against the Chrome the native `/start`
//!   launched. The pre-cutover seam (native /start's Chrome vs Python's own
//!   driver browser) is GONE: one browser, one owner.
//! - `/sessions`, `/pw-profiles`, `/save-profile`, `DELETE /profile/{name}`
//!   — process/filesystem state this server owns.
//! - `/agent` — an HONEST 501, not a port. The Python implementation runs a
//!   server-side model loop (Anthropic Computer Use driving browser-use);
//!   pinning a model + prompt loop inside the harness is the D1/D3 shape the
//!   ethos names — it cannot improve as models improve. The exit is the amux
//!   WORKER driving the browser through the native verbs with its own model:
//!   capability compounds with the model, and the harness stays mechanical.
//!
//! HARD INVARIANT (owner directive): **browser automation always executes on
//! the server machine; the dashboard is a remote viewer.** Every verb here
//! operates exclusively on the Chrome `integrations::browser` spawned — the
//! CDP client refuses non-loopback endpoints outright — and nothing in any
//! response asks the DASHBOARD-VIEWING browser to act. Artifacts a remote
//! viewer needs (screenshots) are served through the API
//! (`/screenshot/file`), never as a filesystem path the client is expected
//! to open itself.
//!
//! Session semantics (AC-293, ported): every verb resolves its session as
//! explicit `session` (body/query) → `X-Amux-Session` header → `"amux"`, and
//! each session binds to its own tab — two lanes never silently drive one
//! page (see `chrome::resolve_page`). With no amux-launched browser running,
//! driver verbs answer 409 + a pointer at `/start`; they NEVER attach to a
//! browser this process did not launch (a human's Chrome is not ours).
//!
//! Routes (nested at `/api/browser`):
//! - `POST /start {profile?, url?}`     — launch Chrome on a profile
//! - `GET  /status`                     — running child + CDP tab list
//! - `POST /stop`                       — SIGTERM, wait, clean stale locks
//! - `GET  /profiles?sizes=1`           — profile inventory
//! - `POST /profile/create {name,url?}` — create profile dir (+ sign-in window)
//! - `DELETE /profile/{name}`           — delete an amux-owned profile
//! - `POST /navigate {url, session?}`   — navigate the session's tab
//! - `GET  /screenshot?session=&url=`   — PNG to ~/.amux/browser-screenshots
//! - `GET  /screenshot/file?session=`   — those bytes, served via the API
//! - `GET  /state?session=`             — url/title/text + indexed elements
//! - `POST /action {action, session?, ...}` — click/type/input/key/scroll/
//!   eval/wait/extract/back/viewport (schema ported from Python)
//! - `GET  /inspect` + `POST /inspect/clear` — console/network/error capture
//! - `GET  /search?q=`                  — google scrape (mechanical)
//! - `GET  /sessions`                   — session→tab bindings
//! - `GET  /pw-profiles`                — playwright profile dirs
//! - `POST /save-profile`               — register profile↔domain
//! - `POST /agent`                      — 501 (see above)
//! - anything else                      — the route CATALOG as a 404 (ported:
//!   two sessions guessed /status for /state and read a bare "not found" as
//!   "the browser API is down")

use super::AppState;
use crate::integrations::browser as chrome;
use axum::extract::{OriginalUri, Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/start", post(start))
        .route("/status", get(status))
        .route("/stop", post(stop))
        .route("/profiles", get(profiles))
        .route("/profile/create", post(profile_create))
        .route("/profile/{name}", delete(profile_delete))
        .route("/navigate", post(navigate))
        .route("/screenshot", get(screenshot))
        .route("/screenshot/file", get(screenshot_file))
        .route("/state", get(state_verb))
        .route("/action", post(action))
        .route("/inspect", get(inspect))
        .route("/inspect/clear", post(inspect_clear))
        .route("/search", get(search))
        .route("/sessions", get(sessions))
        .route("/pw-profiles", get(pw_profiles_list))
        .route("/save-profile", post(save_profile))
        .route("/agent", post(agent))
        // Unknown /api/browser paths answer the route CATALOG (ported from
        // Python). EXPLICIT wildcard routes, not `.fallback()`: in the full
        // composition the static SPA catch-all (`/{*path}`) out-competes a
        // nested fallback and would serve index.html instead (AMUX-2594).
        .route("/", axum::routing::any(catalog_404))
        .route("/{*rest}", axum::routing::any(catalog_404))
}

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// The ATTRIBUTION resolution: explicit `session` (body/query) →
/// `X-Amux-Session` header → None. No default constant — amux-cloud's
/// validation of the takeover guard caught the harm: a header-less curl's
/// refusal said requested_by:"amux", framing that lane for every anonymous
/// call (AMUX-1768's class), and worse, the guard's same-session shortcut
/// let any TWO anonymous callers stomp each other's browsers because both
/// resolved to the same constant. Ownership records and guard comparisons
/// use THIS; an unattributed caller never matches any owner, its own
/// browsers included — anonymity forfeits the shortcut, and the refusal
/// says "(unattributed)".
fn explicit_session(explicit: Option<&str>, headers: &HeaderMap) -> Option<String> {
    if let Some(s) = explicit {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    headers
        .get("x-amux-session")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_string)
}

/// AC-293's resolution order, ported: explicit `session` (body/query) →
/// `X-Amux-Session` header → `"amux"`. The header half is what keeps two
/// well-behaved lanes from silently driving one tab. The `"amux"` default is
/// the shared TAB-BINDING bucket and nothing more — it must never reach an
/// ownership record or an attribution field (see `explicit_session`).
fn resolve_session(explicit: Option<&str>, headers: &HeaderMap) -> String {
    if let Some(s) = explicit {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    if let Some(h) = headers.get("x-amux-session").and_then(|v| v.to_str().ok()) {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    "amux".into()
}

/// Map driver-layer failures: NotRunning is the caller's fixable state (409
/// + the fix), CDP trouble is the browser misbehaving (502).
fn driver_err(e: chrome::DriverError) -> Response {
    match e {
        chrome::DriverError::NotRunning => err(
            StatusCode::CONFLICT,
            json!({
                "error": "no amux-launched browser is running — POST /api/browser/start {\"profile\":\"...\",\"url\":\"...\"} first",
                "hint": "native driver verbs operate the Chrome started by /api/browser/start; \
                         GET /api/browser/status shows what is running. They never attach to a \
                         browser this server did not launch.",
            }),
        ),
        chrome::DriverError::Cdp(e) => err(cdp_status(&e), json!({ "error": e.to_string() })),
    }
}

/// 504 for a CDP call that never answered, 502 for one that failed (AMUX-3672).
///
/// Both were 502, and `/api/logs/analyze` groups by (status, method, target) —
/// so "the browser is WEDGED" and "the browser REJECTED our call" landed in the
/// same row and only the error body separated them. A wedged browser is the one
/// that means something else is wrong (a contended profile, a hung tab); a
/// protocol error usually means the caller asked for something impossible.
/// Making them different status codes makes them different groups in every log
/// view, for free.
///
/// Decided on the TYPE, never by matching the message: a status code that
/// depends on error wording breaks the first time someone rephrases it.
fn cdp_status(e: &anyhow::Error) -> StatusCode {
    if e.downcast_ref::<chrome::CdpTimeout>().is_some() {
        StatusCode::GATEWAY_TIMEOUT
    } else {
        StatusCode::BAD_GATEWAY
    }
}

/// Resolve session → page → connected CDP client, or the mapped error.
async fn connect_session(session: &str, create_url: Option<&str>) -> Result<(chrome::DriverPage, chrome::CdpClient), Response> {
    // A server restart leaves the browser running and the in-process handle
    // empty (AC-325). Re-adopt lazily HERE, on the path every verb takes, so a
    // sequence that spans a rebuild continues instead of reporting "no
    // amux-launched browser is running" about a browser that is right there.
    chrome::adopt_if_orphaned(&chrome::amux_home()).await;
    let page = chrome::resolve_page(session, create_url).await.map_err(driver_err)?;
    let cdp = chrome::CdpClient::connect(&page.ws_url)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })))?;
    Ok((page, cdp))
}

// ---------------------------------------------------------------------------
// Launch / lifecycle (native since RR-0092)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct StartBody {
    #[serde(default)]
    profile: String,
    #[serde(default)]
    url: String,
    /// Same resolution as every driver verb. `start` needs it for the reason
    /// AC-336 exists: the tab Chrome opens for `url` has to be OWNED by the
    /// caller, or a peer adopts it.
    #[serde(default)]
    session: Option<String>,
    /// Viewport at launch (AMUX-3403). Both start-time guesses a caller
    /// naturally makes now work: `device` (same preset table as the viewport
    /// action) or `width`+`height`. Applied right after the tab opens, so
    /// start-at-phone-width is one call instead of start-then-action.
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    /// AMUX-3508: launch with no window (`--headless=new`), same profile
    /// dirs — log in headfully once, reuse the cookies headlessly forever.
    #[serde(default)]
    headless: Option<bool>,
    /// Explicit consent to replace ANOTHER session's running browser
    /// (AMUX-3063). The browser is a machine singleton, so start stomps
    /// whoever staged it — on 2026-08-20 an unattributed default-profile
    /// start killed amux-gtm's staged NetSuite login mid-handoff. Without
    /// this flag, a cross-session start refuses and names the owner.
    #[serde(default)]
    takeover: bool,
    /// Every unknown field lands here and is echoed back as
    /// `ignored_fields` (the board API's pattern). Silently swallowing a
    /// misspelled or misplaced field manufactured "the feature does not
    /// work" evidence: two AF-18 validation probes measured a default
    /// viewport against ok:true responses whose viewport request had been
    /// dropped on the floor (AMUX-3403).
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

// `HeaderMap` before `Json` — axum requires the body extractor last.
async fn start(headers: HeaderMap, body: Option<Json<StartBody>>) -> Response {
    let Json(body) = body.unwrap_or_default();
    // Viewport request validated BEFORE launching — a 400 must not cost a
    // Chrome start, and the contract wording matches the viewport action's.
    let dev = body.device.as_deref().unwrap_or("").trim().to_lowercase();
    let named =
        VIEWPORT_DEVICES.iter().find(|(n, ..)| *n == dev).map(|(_, w, h)| (*w, *h));
    if !dev.is_empty() && named.is_none() {
        let mut names: Vec<&str> = VIEWPORT_DEVICES.iter().map(|(n, ..)| *n).collect();
        names.sort_unstable();
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("unknown device {dev:?} (one of {})", names.join(", ")) }),
        );
    }
    if body.width.is_some() != body.height.is_some() {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": "width and height go together" }),
        );
    }
    let viewport_wh = named.or(match (body.width, body.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    });
    let home = chrome::amux_home();
    let session = resolve_session(body.session.as_deref(), &headers);
    // ATTRIBUTION is explicit-only (amux-cloud's validation catch): the
    // tab-binding default must never become an ownership record or a
    // same-session match, or two anonymous callers stomp each other freely
    // and every anonymous refusal frames the default lane.
    let attrib = explicit_session(body.session.as_deref(), &headers);
    // TAKEOVER GUARD (AMUX-3063). One browser per machine means start REPLACES
    // whatever is running — including another session's staged, logged-in
    // page. Refuse a cross-session replace unless the caller says takeover
    // out loud, and name the owner in the refusal so the caller knows whose
    // work they are about to destroy. Adopt first so a browser surviving a
    // server restart is guarded too, not just one this process spawned.
    chrome::adopt_if_orphaned(&home).await;
    if let Some((r_profile, r_owner, r_started, r_pid)) = chrome::running_snapshot() {
        // Same-session requires BOTH sides attributed and equal. An
        // unattributed caller matches nothing — anonymity forfeits the
        // shortcut, including against an unattributed owner (two anonymous
        // callers are not one session).
        let same = attrib.as_deref().is_some_and(|a| !r_owner.is_empty() && a == r_owner);
        if !same && !body.takeover {
            return err(
                StatusCode::CONFLICT,
                json!({
                    "error": format!(
                        "a browser is already running under session '{}' — starting yours would DESTROY \
                         its state (staged logins included). Pass {{\"takeover\": true}} to replace it \
                         deliberately, or drive the running one via session-scoped verbs.",
                        if r_owner.is_empty() { "(unattributed)" } else { r_owner.as_str() }
                    ),
                    "running": {
                        "profile": r_profile, "started_by": r_owner,
                        "started_at": r_started, "pid": r_pid,
                    },
                    "requested_by": attrib.as_deref().unwrap_or("(unattributed)"),
                }),
            );
        }
        if !same {
            tracing::warn!(
                requested_by = %attrib.as_deref().unwrap_or("(unattributed)"),
                owner = %r_owner, profile = %r_profile, pid = r_pid,
                "browser TAKEOVER: replacing another session's running browser (explicit takeover flag)"
            );
        }
    }
    match chrome::start(&home, &body.profile, &body.url, &session, attrib.as_deref().unwrap_or(""), body.headless.unwrap_or(false))
        .await
    {
        Ok(info) => {
            let mut v = serde_json::to_value(&info).unwrap_or_else(|_| json!({}));
            v["ok"] = json!(true);
            // Apply the requested viewport to the tab start just opened —
            // same CDP call as the viewport action. A failure here degrades
            // the FIELD (`viewport_error`), never the start: the browser is
            // up either way, and the caller can retry via the action.
            if let Some((w, h)) = viewport_wh {
                match connect_session(&session, None).await {
                    Ok((_page, mut cdp)) => {
                        let r = cdp
                            .call(
                                "Emulation.setDeviceMetricsOverride",
                                json!({ "width": w, "height": h, "deviceScaleFactor": 0, "mobile": w <= 500 }),
                                Duration::from_secs(10),
                            )
                            .await;
                        match r {
                            Ok(_) => {
                                let seen = cdp
                                    .eval("({w:window.innerWidth,h:window.innerHeight})", 10)
                                    .await
                                    .unwrap_or(Value::Null);
                                v["viewport"] = json!({ "w": w, "h": h, "measured": seen });
                            }
                            Err(e) => v["viewport_error"] = json!(e.to_string()),
                        }
                    }
                    Err(_) => {
                        v["viewport_error"] =
                            json!("could not attach to the started tab to apply the viewport — retry with the viewport action")
                    }
                }
            }
            // Echo what was dropped (AMUX-3403): an accepted-and-ignored
            // field reads as "the feature does not work" to every caller who
            // guessed the contract. Same pattern as the board API's
            // ignored_fields.
            if !body.extra.is_empty() {
                v["ignored_fields"] = json!(body.extra.keys().collect::<Vec<_>>());
                v["ignored_note"] = json!(
                    "not part of POST /api/browser/start and did nothing; viewport at start is device or width+height"
                );
            }
            Json(v).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
    }
}

async fn status() -> Response {
    // Adopt BEFORE reading (AMUX-3414): status is the first place anyone looks
    // after a death, and it was the one verb that never ran the adopt — so a
    // browser surviving a restart read as "running": false here while every
    // driver verb would have found it, and a browser that DIED with the old
    // server left its corpse-record untriggered because nothing on the
    // observability path opened the running-file.
    chrome::adopt_if_orphaned(&chrome::amux_home()).await;
    // Read the registry under the lock, then drop it before any await —
    // holding a std::sync::Mutex across an await point deadlocks the runtime.
    let snapshot = {
        let guard = chrome::RUNNING.lock().expect("browser registry poisoned");
        guard
            .as_ref()
            .map(|r| (r.profile.clone(), r.cdp_port, r.started_at, r.started_by.clone()))
    };
    // AMUX-3414: why the LAST browser is gone. In-memory, so a server restart
    // clears it — absent means "no exit recorded by this process", not "no
    // exit happened"; the field's note says so rather than letting the two
    // read identically.
    let last_exit = chrome::LAST_EXIT.lock().expect("last-exit poisoned").clone();
    let Some((profile, cdp_port, started_at, started_by)) = snapshot else {
        return Json(json!({
            "running": false,
            "last_exit": last_exit,
            "last_exit_note": "in-memory: a server restart clears it; null means no exit recorded by THIS server process",
        }))
        .into_response();
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
        "started_by": started_by,
        "tabs": tabs,
        "tabs_error": tabs_error,
        "last_exit": last_exit,
    }))
    .into_response()
}

async fn stop(headers: HeaderMap, body: Option<Json<Value>>) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    // Explicit attribution only — the tab-binding default must not sign the
    // stop record as "amux" for an anonymous caller (amux-cloud's catch).
    let attrib = explicit_session(body.get("session").and_then(Value::as_str), &headers);
    let actor = attrib.as_deref().unwrap_or("(unattributed)");
    let home = chrome::amux_home();
    // Cross-session stop stays PERMITTED (a wedged browser must be cleanable
    // by whoever notices) but LOUD: the log and the response both name owner
    // and actor, so an anonymous stop can no longer read as a mystery death
    // (AMUX-3063's other half — the 09:05 stop had no actor on record).
    let owner = chrome::running_snapshot().map(|(_, o, _, _)| o);
    if let Some(o) = owner.as_deref() {
        if attrib.as_deref() != Some(o) {
            tracing::warn!(
                stopped_by = %actor, owner = %o,
                "browser: cross-session STOP of another session's browser"
            );
        }
    }
    let report = chrome::stop_as(&home, attrib.as_deref().unwrap_or("")).await;
    let mut v = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
    v["ok"] = json!(true);
    v["stopped_by"] = json!(actor);
    if let Some(o) = owner {
        v["owner"] = json!(o);
    }
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
    let chrome_profiles = chrome::list_chrome_profiles();
    Json(json!({ "profiles": list, "backends": ["native"], "chrome_profiles": chrome_profiles })).into_response()
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    session: Option<String>,
}

async fn profile_create(headers: HeaderMap, Json(body): Json<CreateBody>) -> Response {
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
        let session = resolve_session(body.session.as_deref(), &headers);
        let attrib = explicit_session(body.session.as_deref(), &headers);
        match chrome::start(&home, &name, body.url.trim(), &session, attrib.as_deref().unwrap_or(""),
            // create-profile launches headfully by definition: its purpose is a
            // human logging in (AMUX-3508's headless is for REUSING the result).
            false)
            .await
        {
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

async fn profile_delete(Path(name): Path<String>) -> Response {
    // Python's route regex admits [A-Za-z0-9._-]+ only; anything else falls
    // through to its catalog 404. Same here.
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return catalog_body(&format!("/api/browser/profile/{name}"));
    }
    match chrome::delete_profile(&chrome::amux_home(), &name) {
        Ok(v) => Json(v).into_response(),
        Err((code, v)) => {
            err(StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), v)
        }
    }
}

// ---------------------------------------------------------------------------
// Driver verbs (native since AMUX-2598)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct NavigateBody {
    #[serde(default)]
    url: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    profile: Option<String>,
}

async fn navigate(headers: HeaderMap, body: Option<Json<NavigateBody>>) -> Response {
    let Json(b) = body.unwrap_or_default();
    let url = b.url.trim().to_string();
    if url.is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "url required" }));
    }
    let session = resolve_session(b.session.as_deref(), &headers);
    let (page, mut cdp) = match connect_session(&session, Some(&url)).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match chrome::navigate_and_settle(&mut cdp, &url).await {
        Ok(mut v) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("native"));
                o.insert("session".into(), json!(session));
                o.insert("target".into(), json!(page.target_id));
                // Name the ignored field rather than dropping it silently
                // (the cold-outbound `ignored_fields` lesson, ethos rule 7).
                if b.profile.as_deref().is_some_and(|p| !p.trim().is_empty()) {
                    o.insert(
                        "profile_note".into(),
                        json!("profile is chosen at POST /api/browser/start; /navigate drives the already-started browser"),
                    );
                }
            }
            Json(v).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize, Default)]
struct ShotQuery {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

async fn screenshot(headers: HeaderMap, Query(q): Query<ShotQuery>) -> Response {
    let session = resolve_session(q.session.as_deref(), &headers);
    let (page, mut cdp) = match connect_session(&session, None).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Some(u) = q.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        if let Err(e) = chrome::navigate_and_settle(&mut cdp, u).await {
            // Log signal (two-fixes rule): a wedged navigation / capture shows as
            // a blank live view in the dashboard with no other trace. Surface it
            // so a log sweep / /api/logs catches "browser session X can't render"
            // without a human noticing the empty viewport first.
            tracing::warn!("[browser] navigate failed for session {session:?} → {u}: {e}");
            return err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() }));
        }
    }
    match chrome::screenshot_to_file(&mut cdp, &chrome::amux_home(), &session).await {
        Ok((path, size)) => {
            let url = cdp
                .eval("location.href", 10)
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or(page.url);
            Json(json!({
                "ok": true,
                "backend": "native",
                "path": path.display().to_string(),
                "size": size,
                "url": url,
                // `path` is a SERVER-machine filesystem path (Python returns
                // only that; its dashboard bridges it via /api/file/raw). A
                // remote viewer cannot read it directly, so the bytes are
                // also served through this family:
                "serve": format!("/api/browser/screenshot/file?session={}", urlenc(&session)),
            }))
            .into_response()
        }
        Err(e) => {
            // e.g. "CDP Page.captureScreenshot timed out after 30s" (amux-gtm,
            // 2026-08-13) — the tab wedged and the viewport went blank. WARN so
            // the failure is visible in the logs, not only as an empty view.
            tracing::warn!("[browser] screenshot capture failed for session {session:?}: {e}");
            err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() }))
        }
    }
}

/// GET /api/browser/screenshot/file?session= — the screenshot BYTES, served
/// through the API. The dashboard may be a phone across the network; a bare
/// filesystem path only resolves on the server machine (where the browser
/// and this file both live, per the server-machine invariant), so the API
/// itself serves the image for remote viewers.
async fn screenshot_file(headers: HeaderMap, Query(q): Query<SessionQuery>) -> Response {
    let session = resolve_session(q.session.as_deref(), &headers);
    let file = chrome::amux_home()
        .join("browser-screenshots")
        .join(format!("native-{}.png", chrome::safe_file_component(&session)));
    match tokio::fs::read(&file).await {
        Ok(bytes) => (
            [
                (axum::http::header::CONTENT_TYPE, "image/png"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => err(
            StatusCode::NOT_FOUND,
            json!({
                "error": format!("no native screenshot for session {session:?} — GET /api/browser/screenshot first"),
                "path": file.display().to_string(),
            }),
        ),
    }
}

#[derive(Deserialize, Default)]
struct SessionQuery {
    #[serde(default)]
    session: Option<String>,
}

/// The state payload both `/state` and `action:extract` serve: url/title/
/// viewport/indexed elements plus capped page text.
async fn state_payload(cdp: &mut chrome::CdpClient, session: &str) -> Result<Value, Response> {
    let mut v = cdp
        .eval(&chrome::state_js(), 20)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })))?;
    let text = v.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(o) = v.as_object_mut() {
        o.insert("text".into(), json!(chrome::obs_cap(&text, chrome::obs_state_cap())));
        o.insert("ok".into(), json!(true));
        o.insert("backend".into(), json!("native"));
        o.insert("session".into(), json!(session));
    }
    Ok(v)
}

async fn state_verb(headers: HeaderMap, Query(q): Query<SessionQuery>) -> Response {
    let session = resolve_session(q.session.as_deref(), &headers);
    let (_page, mut cdp) = match connect_session(&session, None).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match state_payload(&mut cdp, &session).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// Python's viewport device table, same names and dimensions (AF-18).
const VIEWPORT_DEVICES: &[(&str, u32, u32)] = &[
    ("iphone", 390, 844),
    ("iphone-se", 375, 667),
    ("ipad", 820, 1180),
    ("desktop", 1280, 900),
];

async fn action(headers: HeaderMap, body: Option<Json<Value>>) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let action = body.get("action").and_then(Value::as_str).unwrap_or("").to_string();
    let session = resolve_session(body.get("session").and_then(Value::as_str), &headers);

    let get_str = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let get_f64 = |k: &str| body.get(k).and_then(Value::as_f64);
    let get_usize = |k: &str| body.get(k).and_then(Value::as_u64).map(|v| v as usize);

    // -- Request-shape validation FIRST: a 400 must not depend on whether a
    //    browser happens to be running (and the schema stays testable
    //    without Chrome). Messages match the Python handler's.
    let mut viewport_wh: Option<(u32, u32)> = None;
    match action.as_str() {
        "click" => {
            if get_str("selector").is_none()
                && get_usize("index").is_none()
                && !(get_f64("x").is_some() && get_f64("y").is_some())
            {
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": "click needs selector, index, or x,y" }),
                );
            }
        }
        "eval" => {
            if get_str("script").map(|s| s.trim().is_empty()).unwrap_or(true) {
                return err(StatusCode::BAD_REQUEST, json!({ "error": "script required" }));
            }
        }
        "input" => {
            if get_usize("index").is_none() || get_str("text").is_none() {
                return err(StatusCode::BAD_REQUEST, json!({ "error": "input needs index and text" }));
            }
        }
        "key" => {
            let k = get_str("key").unwrap_or_default();
            if !chrome::CDP_KEYS.iter().any(|(n, ..)| *n == k) {
                let supported =
                    chrome::CDP_KEYS.iter().map(|(n, ..)| *n).collect::<Vec<_>>().join(", ");
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": format!("unsupported key {k:?} (supported: {supported})") }),
                );
            }
        }
        "viewport" => {
            let dev = get_str("device").unwrap_or_default().trim().to_lowercase();
            let named = VIEWPORT_DEVICES.iter().find(|(n, ..)| *n == dev).map(|(_, w, h)| (*w, *h));
            let explicit = match (body.get("width").and_then(Value::as_u64), body.get("height").and_then(Value::as_u64)) {
                (Some(w), Some(h)) => Some((w as u32, h as u32)),
                _ => None,
            };
            viewport_wh = named.or(explicit);
            if viewport_wh.is_none() {
                let mut names: Vec<&str> = VIEWPORT_DEVICES.iter().map(|(n, ..)| *n).collect();
                names.sort_unstable();
                return err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": format!("viewport needs width+height, or device (one of {})", names.join(", ")) }),
                );
            }
        }
        "wait" => {
            let has = |k: &str| get_str(k).map(|s| !s.trim().is_empty()).unwrap_or(false);
            if !has("selector") && !has("text") {
                return err(StatusCode::BAD_REQUEST, json!({ "error": "wait needs selector or text" }));
            }
        }
        "type" | "scroll" | "back" | "extract" => {}
        other => {
            return err(StatusCode::BAD_REQUEST, json!({ "error": format!("unknown action: {other}") }))
        }
    }

    let (_page, mut cdp) = match connect_session(&session, None).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let ten = Duration::from_secs(10);

    match action.as_str() {
        "click" => {
            let out = if let Some(sel) = get_str("selector") {
                // Selector first, matching Python's precedence (AMUX-2272).
                chrome::click_selector(&mut cdp, &sel).await
            } else if let Some(i) = get_usize("index") {
                chrome::click_index(&mut cdp, i).await
            } else {
                let (x, y) = (get_f64("x").unwrap_or(0.0), get_f64("y").unwrap_or(0.0));
                chrome::click_xy(&mut cdp, x, y)
                    .await
                    .map(|()| json!({ "ok": true, "clicked": { "x": x, "y": y } }))
            };
            match out {
                Ok(v) if v.get("error").is_some() => err(StatusCode::BAD_REQUEST, v),
                Ok(v) => Json(v).into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        "type" => {
            let text = get_str("text").unwrap_or_default();
            match cdp.call("Input.insertText", json!({ "text": text }), ten).await {
                Ok(_) => Json(json!({ "ok": true, "typed": text.chars().count(), "backend": "native" }))
                    .into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        "input" => {
            let idx = get_usize("index").unwrap_or(0);
            let text = get_str("text").unwrap_or_default();
            // Focus + clear element[idx] of the /state list, then type.
            let js = format!(
                "(function(){{var els=window.__amux_els||[];var e=els[{idx}];\
                 if(!e)return 'NOELEMENT';if(!e.isConnected)return 'STALE';\
                 e.scrollIntoView({{block:'center'}});e.focus();\
                 if('value' in e)e.value='';return 'FOCUSED';}})()"
            );
            let raw = match cdp.eval(&js, 20).await {
                Ok(r) => r,
                Err(e) => return err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            };
            if raw.as_str() != Some("FOCUSED") {
                let v = chrome::click_outcome(
                    &raw,
                    &format!("element index {idx}"),
                    "indexes come from GET /api/browser/state — re-fetch it",
                );
                return err(StatusCode::BAD_REQUEST, v);
            }
            match cdp.call("Input.insertText", json!({ "text": text }), ten).await {
                Ok(_) => Json(json!({ "ok": true, "index": idx, "typed": text.chars().count() }))
                    .into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        "key" => {
            let k = get_str("key").unwrap_or_default();
            match chrome::dispatch_key(&mut cdp, &k).await {
                Ok(()) => Json(json!({ "ok": true, "key": k })).into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        "scroll" => {
            let dy = body.get("dy").and_then(Value::as_i64).unwrap_or(500);
            match cdp.eval(&format!("window.scrollBy(0,{dy})"), 10).await {
                Ok(_) => Json(json!({ "ok": true, "dy": dy })).into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        "eval" => {
            let script = get_str("script").unwrap_or_default();
            match cdp.eval(&script, 30).await {
                Ok(mut result) => {
                    // AMUX-3062: an eval result over the cap was truncated with the
                    // notice appended INTO the string while the envelope still read
                    // {ok:true} — silent data loss that reads as success (amux-gtm
                    // scraped a 10589-char page, got 23 of 34 records, and the run
                    // would have passed review). Surface the cut in the ENVELOPE so
                    // a run/reviewer detects it structurally, not by substring match.
                    let mut cut: Option<(usize, usize)> = None; // (full_len, cap)
                    if let Some(s) = result.as_str() {
                        let cap = chrome::obs_eval_cap();
                        cut = chrome::obs_truncation(s, cap).map(|full| (full, cap));
                        result = json!(chrome::obs_cap(s, cap));
                    }
                    // `data.result` mirrors the browser-use CLI shape some
                    // dashboard readers still consume (viewport probe).
                    let mut env = json!({
                        "ok": true,
                        "result": result,
                        "data": { "result": result },
                        "backend": "native",
                    });
                    if let Some((full, cap)) = cut {
                        // Two-fixes log signal: the next silent truncation is now
                        // visible in the request log without comparing to a direct
                        // fetch — grep "browser eval truncated".
                        tracing::warn!(
                            session = %session, full_length = full, cap,
                            "browser eval truncated at the cap — {} chars past the cap are NOT in the result (AMUX-3062); envelope carries truncated=true",
                            full - cap
                        );
                        env["truncated"] = json!(true);
                        env["full_length"] = json!(full);
                        env["eval_cap"] = json!(cap);
                    }
                    Json(env).into_response()
                }
                // Page exceptions AND transport failures both answer 400
                // with the description — Python's eval contract.
                Err(e) => err(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": e.to_string(), "backend": "native" }),
                ),
            }
        }
        "back" => match cdp.eval("history.back()", 10).await {
            Ok(_) => Json(json!({ "ok": true })).into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
        },
        "extract" => match state_payload(&mut cdp, &session).await {
            Ok(v) => Json(v).into_response(),
            Err(r) => r,
        },
        "wait" => {
            let timeout_ms =
                body.get("timeout").and_then(Value::as_u64).unwrap_or(5000).min(60_000);
            let (what, probe) = if let Some(sel) = get_str("selector").filter(|s| !s.trim().is_empty()) {
                (format!("selector {sel:?}"), format!("!!document.querySelector({})", json!(sel)))
            } else {
                let text = get_str("text").unwrap_or_default();
                (
                    format!("text {text:?}"),
                    format!(
                        "(((document.body&&document.body.innerText)||'').indexOf({}) !== -1)",
                        json!(text)
                    ),
                )
            };
            let started = std::time::Instant::now();
            let deadline = started + Duration::from_millis(timeout_ms);
            // Declares requested-wait semantics to the request log
            // (AMUX-3513): a wait's latency is the CALLER's budget, and a
            // timed-out wait is a 200 at exactly that budget — twelve of
            // those read as a 7.1x /api/browser p95 regression to the
            // latency detector, which now skips rows carrying this marker.
            let slow_ok = [("x-amux-slow-ok", "wait-budget")];
            loop {
                match cdp.eval(&probe, 10).await {
                    Ok(v) if v.as_bool() == Some(true) => {
                        return (
                            slow_ok,
                            Json(json!({
                                "ok": true,
                                "found": what,
                                "waited_ms": started.elapsed().as_millis() as u64,
                            })),
                        )
                            .into_response();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() }))
                    }
                }
                if std::time::Instant::now() >= deadline {
                    // A timeout is an OUTCOME, not a malformed request: 200
                    // with ok:false, like the CLI shape Python relays.
                    return (
                        slow_ok,
                        Json(json!({
                            "ok": false,
                            "error": format!("timed out after {timeout_ms}ms waiting for {what}"),
                        })),
                    )
                        .into_response();
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        "viewport" => {
            let (w, h) = viewport_wh.unwrap_or((1280, 900));
            let r = cdp
                .call(
                    "Emulation.setDeviceMetricsOverride",
                    // mobile:true under 500px wide — phone-width testing is
                    // what AF-18 exists for.
                    json!({ "width": w, "height": h, "deviceScaleFactor": 0, "mobile": w <= 500 }),
                    ten,
                )
                .await;
            match r {
                Ok(_) => {
                    let seen = cdp
                        .eval("({w:window.innerWidth,h:window.innerHeight})", 10)
                        .await
                        .unwrap_or(Value::Null);
                    Json(json!({ "ok": true, "viewport": { "w": w, "h": h }, "measured": seen }))
                        .into_response()
                }
                Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
            }
        }
        _ => unreachable!("validated above"),
    }
}

#[derive(Deserialize, Default)]
struct InspectQuery {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    clear: Option<String>,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default, rename = "type")]
    type_: Option<String>,
}

async fn inspect(headers: HeaderMap, Query(q): Query<InspectQuery>) -> Response {
    let session = resolve_session(q.session.as_deref(), &headers);
    let clear = matches!(q.clear.as_deref(), Some("1") | Some("true") | Some("yes"));
    let limit = q
        .limit
        .as_deref()
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 500);
    let (_page, mut cdp) = match connect_session(&session, None).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match inspect_payload(&mut cdp, clear, limit).await {
        Ok(data) => {
            // `type` filter, ported: keep url/installed/counts + one bucket.
            let tf = q.type_.as_deref().unwrap_or("all");
            if matches!(tf, "console" | "network" | "errors" | "resources") {
                return Json(json!({
                    "url": data.get("url"),
                    "installed": data.get("installed"),
                    tf: data.get(tf),
                    "counts": data.get("counts"),
                }))
                .into_response();
            }
            Json(data).into_response()
        }
        Err(r) => r,
    }
}

async fn inspect_payload(cdp: &mut chrome::CdpClient, clear: bool, limit: usize) -> Result<Value, Response> {
    // Install the capture shim first (idempotent per page) so counts start
    // accruing from the first inspect even when nobody navigated natively;
    // Resource Timing back-fills earlier requests either way.
    let _ = cdp.eval(chrome::CAPTURE_JS, 15).await;
    cdp.eval(&chrome::inspect_js(limit, clear), 15)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })))
}

async fn inspect_clear(headers: HeaderMap, body: Option<Json<Value>>) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let session = resolve_session(body.get("session").and_then(Value::as_str), &headers);
    let (_page, mut cdp) = match connect_session(&session, None).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    match inspect_payload(&mut cdp, true, 1).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
}

/// Python's google-scrape search, mechanically: navigate the dedicated
/// `search` session's tab, settle, scrape result cards.
async fn search(Query(sq): Query<SearchQuery>) -> Response {
    let Some(q) = sq.q.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "q required" }));
    };
    let url = format!("https://www.google.com/search?q={}", urlenc(&q));
    let (_page, mut cdp) = match connect_session("search", Some(&url)).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Err(e) = chrome::navigate_and_settle(&mut cdp, &url).await {
        return err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() }));
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let scrape = r#"
        Array.from(document.querySelectorAll('div.g')).slice(0,8).map(el => ({
            title: (el.querySelector('h3') || {}).textContent || '',
            url: (el.querySelector('a') || {}).href || '',
            snippet: (el.querySelector('.VwiC3b, [data-sncf]') || {}).textContent || ''
        })).filter(r => r.title)
    "#;
    match cdp.eval(scrape, 30).await {
        Ok(v) if v.is_array() => Json(json!({ "results": v })).into_response(),
        Ok(v) => Json(json!({ "results": [], "raw": v, "note": "scrape matched nothing — google may be showing a consent/challenge page" })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })),
    }
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn sessions() -> Response {
    let bindings = chrome::session_bindings().await;
    Json(json!({
        "sessions": bindings
            .into_iter()
            .map(|(name, target, alive)| json!({
                "name": name,
                "status": if alive { "active" } else { "closed" },
                "target": target,
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

async fn pw_profiles_list() -> Response {
    Json(json!({ "profiles": chrome::pw_profiles(&chrome::amux_home()) })).into_response()
}

#[derive(Deserialize, Default)]
struct SaveProfileBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

async fn save_profile(headers: HeaderMap, body: Option<Json<SaveProfileBody>>) -> Response {
    let Json(b) = body.unwrap_or_default();
    let session = resolve_session(b.session.as_deref(), &headers);
    let mut name = b.name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        // Python defaults to the session's active profile; natively the one
        // running browser's profile IS that (or 'default' with none running).
        name = {
            let guard = chrome::RUNNING.lock().expect("browser registry poisoned");
            guard.as_ref().map(|r| r.profile.clone()).unwrap_or_default()
        };
        if name.trim().is_empty() {
            name = "default".into();
        }
    }
    let label = b.label.unwrap_or_default().trim().to_string();
    let mut host = b.host.unwrap_or_default().trim().to_lowercase();
    if host.is_empty() {
        // Best-effort: derive from the session's current page. No browser →
        // the registration still succeeds with no domain, like Python when
        // its eval fails.
        if let Ok(page) = chrome::resolve_page(&session, None).await {
            if let Ok(mut cdp) = chrome::CdpClient::connect(&page.ws_url).await {
                if let Ok(v) = cdp.eval("location.hostname", 10).await {
                    host = v.as_str().unwrap_or("").trim().to_lowercase();
                }
            }
        }
    }
    match chrome::registry_register(&chrome::amux_home(), &name, &host, &label) {
        Ok(entry) => Json(json!({
            "success": true,
            "profile": name,
            "host": host,
            "domains": entry.get("domains").cloned().unwrap_or_else(|| json!([])),
            "label": entry.get("label").cloned().unwrap_or_else(|| json!("")),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
    }
}

/// POST /api/browser/agent — an honest 501, deliberately NOT a port. The
/// Python endpoint runs a server-side model loop (Computer Use driving
/// browser-use). A model + prompt loop pinned inside the harness is the
/// D1/D3 shape: it cannot improve as models improve, and it spends the
/// harness's judgment where the WORKER's model should spend its own. The
/// exit is the amux worker driving the browser itself through the native
/// verbs below — capability that compounds with the model.
async fn agent() -> Response {
    err(
        StatusCode::NOT_IMPLEMENTED,
        json!({
            "error": "the model-driven browser agent loop is not implemented on this server",
            "capability": "POST /api/browser/agent — server-side vision/act loop over the page",
            "use_instead": [
                "POST /api/browser/start", "POST /api/browser/navigate",
                "GET /api/browser/state (indexed elements)", "GET /api/browser/screenshot",
                "POST /api/browser/action (click/type/input/key/scroll/eval/wait/viewport)",
                "GET /api/browser/inspect",
            ],
            "why": "a server-side agent loop pins a model + prompt inside the harness; the \
                    session's own model drives the browser through the native verbs instead, \
                    so browsing capability compounds as models improve (ethos D1/D3)",
        }),
    )
}

async fn catalog_404(uri: OriginalUri) -> Response {
    catalog_body(uri.0.path())
}

/// Name the routes (ported from Python): two sessions independently guessed
/// /api/browser/status for /state and read the bare "not found" as "the
/// browser API is down" — an error that sends you chasing the wrong cause is
/// the bug, not the typo.
fn catalog_body(path: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": format!("browser route not found: {path}"),
            "routes": [
                "GET /api/browser/status", "GET /api/browser/state", "GET /api/browser/screenshot",
                "GET /api/browser/profiles", "GET /api/browser/pw-profiles", "GET /api/browser/sessions",
                "GET /api/browser/inspect", "GET /api/browser/search",
                "POST /api/browser/start (profile, url, session; viewport at launch via device or width+height)",
                "POST /api/browser/navigate", "POST /api/browser/action",
                "POST /api/browser/stop", "POST /api/browser/inspect/clear",
                "POST /api/browser/save-profile", "POST /api/browser/profile/create",
                "DELETE /api/browser/profile/{name}",
                "POST /api/browser/agent (answers 501 — the session's model drives the native verbs)",
            ],
            "actions": ["click (selector|index|x,y)", "type", "input", "key",
                        "scroll", "eval", "wait", "extract", "back",
                        "viewport (width+height, or device=iphone|iphone-se|ipad|desktop)"],
            "eval_contract": "script must be a bare EXPRESSION; a `return` statement yields null",
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests — schema + routing only (live CDP paths need a real Chrome; the
// gated e2e lives in integrations::browser's #[ignore]d test + AMUX-2598's
// scripted run).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value, bool) {
        let mut req = axum::http::Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                axum::body::Body::from(b.to_string())
            }
            None => axum::body::Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let proxied = res.headers().get("x-amux-answered-by").is_some();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, v, proxied)
    }

    /// AMUX-3672. A wedged browser and a rejected call must not share a status,
    /// because `/api/logs/analyze` groups by (status, method, target) and would
    /// otherwise fold two different faults into one row.
    ///
    /// The specimen: `Emulation.setDeviceMetricsOverride timed out after 10s`
    /// on 2026-08-24 15:57, which was five Chromes contending one profile
    /// (AMUX-3674). Diagnosing it needed the error BODY off the card, because
    /// the grouping could not say "this one is a hang".
    ///
    /// This citation read AMUX-3669 until AMUX-3688. That card is an autofix
    /// LATENCY report about `/api/sessions-git`; the stray-Chrome work is
    /// AMUX-3674. Six comments across three files carried the wrong id, all from
    /// writing it into prose before the card existed. AMUX-3669's own body
    /// records the slip. Commits 02197674 and a19bedbf still quote the wrong id
    /// in their subjects and cannot be rewritten.
    #[test]
    fn a_cdp_timeout_is_504_and_a_protocol_error_is_502() {
        let timeout = anyhow::Error::new(chrome::CdpTimeout {
            method: "Emulation.setDeviceMetricsOverride".into(),
            secs: 10,
        });
        assert_eq!(cdp_status(&timeout), StatusCode::GATEWAY_TIMEOUT);
        // The wording is quoted in existing card bodies and the AMUX-3207
        // notes, so it must not drift — but nothing about the STATUS depends on
        // it any more, which is the point of deciding on the type.
        assert_eq!(
            timeout.to_string(),
            "CDP Emulation.setDeviceMetricsOverride timed out after 10s"
        );

        // CONTROL: an ordinary CDP failure stays 502. A change that answered
        // 504 for everything would pass the assertion above and relabel every
        // browser fault as a hang — the same row-folding defect, new status.
        let protocol = anyhow::anyhow!("CDP websocket closed during Page.navigate");
        assert_eq!(cdp_status(&protocol), StatusCode::BAD_GATEWAY);

        // CONTROL: a timeout wrapped in CONTEXT is still a timeout. `?` adds
        // context freely on this path, and a downcast inspecting only the
        // outermost error would silently regress to 502 the first time someone
        // wrote `.context(...)`.
        let wrapped = anyhow::Error::new(chrome::CdpTimeout { method: "X".into(), secs: 1 })
            .context("while resizing the viewport");
        assert_eq!(cdp_status(&wrapped), StatusCode::GATEWAY_TIMEOUT);
    }

    /// AMUX-3403: an unknown field posted to start is CAPTURED, not silently
    /// dropped — the serde seam that feeds `ignored_fields`. Both of the wrong
    /// guesses that motivated the card land in `extra`; the fields that
    /// became real ones do not.
    #[test]
    fn start_body_captures_unknown_fields_and_owns_viewport_params() {
        let b: StartBody = serde_json::from_str(
            r#"{"profile":"default","url":"x","viewport":"iphone","emulate":"ipad","device":"iphone"}"#,
        )
        .unwrap();
        let mut extras: Vec<&String> = b.extra.keys().collect();
        extras.sort();
        assert_eq!(extras, ["emulate", "viewport"], "unknown fields must be captured for echo");
        assert_eq!(b.device.as_deref(), Some("iphone"), "device is a real start field now");
    }

    /// AMUX-3063 incident replay: the EXACT anonymous curl that destroyed
    /// amux-gtm's staged NetSuite login on 2026-08-20 09:24:57 — POST /start
    /// {"profile":"default"}, no session header, while another session's
    /// browser was running — must now refuse, naming the owner, and must
    /// refuse BEFORE any Chrome is touched (hermetic: seeded registry, temp
    /// home, no launch). Takeover by the OWNER's own session passes the guard
    /// (their browser, their restart).
    #[tokio::test]
    async fn cross_session_start_refuses_without_takeover_naming_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let _home = crate::api::settings::test_env::set_home(dir.path());
        chrome::test_seed_running("netsuite", "amux-gtm", 424242);
        let app = app();
        // The incident's own request shape: anonymous, default profile.
        let (status, v, _) =
            send(&app, "POST", "/api/browser/start", Some(r#"{"profile":"default"}"#)).await;
        chrome::test_clear_running();
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["running"]["started_by"], json!("amux-gtm"), "{v}");
        assert_eq!(v["running"]["profile"], json!("netsuite"), "{v}");
        assert!(
            v["error"].as_str().unwrap_or("").contains("takeover"),
            "the refusal must name the escape: {v}"
        );
        assert_eq!(
            v["requested_by"],
            json!("(unattributed)"),
            "a header-less caller must not be framed as any lane (amux-cloud's catch): {v}"
        );
        // amux-cloud's validation catch: the tab-binding default ("amux") must
        // never become a same-session match. An anonymous caller resolves to
        // that default for TAB purposes only — against a browser OWNED by the
        // real amux session it must still refuse, or every anonymous caller
        // could stomp amux's browsers (and each other's, via the shared
        // constant).
        chrome::test_seed_running("default", "amux", 424242);
        let (st_amux, v_amux, _) =
            send(&app, "POST", "/api/browser/start", Some(r#"{"profile":"default"}"#)).await;
        chrome::test_clear_running();
        assert_eq!(st_amux, StatusCode::CONFLICT, "anonymous must not match the default bucket: {v_amux}");
        // Two anonymous callers are not one session: an unattributed OWNER is
        // matched by nobody, its (anonymous) starter included.
        chrome::test_seed_running("default", "", 424242);
        let (st_anon, v_anon, _) =
            send(&app, "POST", "/api/browser/start", Some(r#"{"profile":"default"}"#)).await;
        chrome::test_clear_running();
        assert_eq!(st_anon, StatusCode::CONFLICT, "anonymous-vs-anonymous is not same-session: {v_anon}");
        assert!(
            v_anon["error"].as_str().unwrap_or("").contains("(unattributed)"),
            "an unattributed owner is named as such: {v_anon}"
        );
        // The pass-through cases (same session; takeover:true) proceed to a
        // REAL Chrome launch and so cannot run hermetically — they are
        // exercised by the live post-deploy verification on the incident's
        // own machine state, recorded on AMUX-3063's card.
    }

    /// The action schema answers 400 for malformed requests BEFORE any
    /// browser state is consulted — hermetic, message parity with Python.
    #[tokio::test]
    async fn action_schema_validates_before_browser_state() {
        let app = app();
        for (body, needle) in [
            (r#"{"action":"click"}"#, "selector, index, or x,y"),
            (r#"{"action":"eval"}"#, "script required"),
            (r#"{"action":"input","text":"x"}"#, "input needs index and text"),
            (r#"{"action":"key","key":"F13"}"#, "unsupported key"),
            (r#"{"action":"viewport"}"#, "viewport needs width+height, or device"),
            (r#"{"action":"wait"}"#, "wait needs selector or text"),
            (r#"{"action":"dance"}"#, "unknown action: dance"),
        ] {
            let (status, v, proxied) = send(&app, "POST", "/api/browser/action", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {v}");
            assert!(!proxied, "{body} must answer natively");
            assert!(
                v["error"].as_str().unwrap_or("").contains(needle),
                "{body}: {v}"
            );
        }
        // start's viewport validation runs BEFORE any Chrome launch (AMUX-3403),
        // so a bad request is hermetic too.
        for (body, needle) in [
            (r#"{"device":"nokia"}"#, "unknown device"),
            (r#"{"width":390}"#, "width and height go together"),
        ] {
            let (status, v, _) = send(&app, "POST", "/api/browser/start", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {v}");
            assert!(v["error"].as_str().unwrap_or("").contains(needle), "{body}: {v}");
        }
        // navigate's own required field.
        let (status, v, _) = send(&app, "POST", "/api/browser/navigate", Some("{}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "url required");
        // search's.
        let (status, v, _) = send(&app, "GET", "/api/browser/search", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "q required", "{v}");
    }

    /// Post-cutover invariant: NO browser verb reaches a proxy. With no
    /// amux-launched Chrome the driver verbs answer an honest 409 naming
    /// /start, the catalog 404 answers unknown paths, /agent answers 501 — all
    /// natively, with no `x-amux-answered-by` header anywhere.
    ///
    /// This used to arm a fake python listener and assert it received nothing.
    /// AMUX-2906 deleted the forwarder, so that half could no longer fail and
    /// was removed (ethos rule 7). The `!proxied` assertions below are KEPT:
    /// unlike the fake listener, they would still fail if a proxy hop were
    /// ever reintroduced here, which makes them a live regression guard rather
    /// than theatre.
    #[tokio::test]
    async fn driver_verbs_answer_natively_never_proxy() {
        // AF-109: this test asserts 409-when-not-running, but connect_session
        // deliberately runs adopt_if_orphaned(&amux_home()) first (AC-325),
        // and with no home guard that probe reaches the DEVELOPER'S REAL
        // ~/.amux — on a machine with an amux-launched Chrome there is
        // genuinely something to adopt, so the test raced the adopt probe and
        // failed ~6.5% of runs (13/200 measured), while staying green in CI
        // where no Chrome exists. The sanctioned fix is the existing
        // HomeGuard: a temp home has no browser-running.json, so the adopt is
        // a deterministic no-op and the 409 is about the fixture, not about
        // whatever the host happens to be running.
        let dir = tempfile::tempdir().unwrap();
        let _home = crate::api::settings::test_env::set_home(dir.path());
        let app = app();

        for (method, uri, body) in [
            ("GET", "/api/browser/state?session=t1", None),
            ("GET", "/api/browser/screenshot?session=t1", None),
            ("GET", "/api/browser/inspect?session=t1", None),
            ("GET", "/api/browser/search?q=rust", None),
            ("POST", "/api/browser/action", Some(r#"{"action":"click","selector":"a","session":"t1"}"#)),
            ("POST", "/api/browser/navigate", Some(r#"{"url":"https://example.com","session":"t1"}"#)),
            ("POST", "/api/browser/inspect/clear", Some(r#"{"session":"t1"}"#)),
        ] {
            let (status, v, proxied) = send(&app, method, uri, body).await;
            assert_eq!(status, StatusCode::CONFLICT, "{method} {uri}: {v}");
            assert!(!proxied, "{method} {uri} must not proxy");
            assert!(
                v["error"].as_str().unwrap_or("").contains("/api/browser/start"),
                "{method} {uri}: 409 must point at /start: {v}"
            );
        }

        // /agent: honest 501 naming the native verbs, never a proxy hop.
        let (status, v, proxied) =
            send(&app, "POST", "/api/browser/agent", Some(r#"{"task":"buy milk"}"#)).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{v}");
        assert!(!proxied);
        assert!(v["use_instead"].is_array(), "{v}");
        assert!(v["error"].as_str().unwrap().contains("agent loop"), "{v}");

        // Unknown routes: the ported catalog, natively.
        let (status, v, proxied) = send(&app, "GET", "/api/browser/definitely-not", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!proxied);
        assert!(
            v["error"].as_str().unwrap().starts_with("browser route not found"),
            "{v}"
        );
        assert!(v["routes"].is_array() && v["actions"].is_array(), "{v}");

        // Local inventory verbs answer without a browser or a proxy.
        let (status, v, proxied) = send(&app, "GET", "/api/browser/sessions", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!proxied);
        assert!(v["sessions"].is_array(), "{v}");
        let (status, v, proxied) = send(&app, "GET", "/api/browser/pw-profiles", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!proxied);
        assert!(v["profiles"].is_array(), "{v}");

        // /status stays native and honest.
        let (status, v, proxied) = send(&app, "GET", "/api/browser/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!proxied);
        assert_eq!(v["running"], json!(false));
    }
}
