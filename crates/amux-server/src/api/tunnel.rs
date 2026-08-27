//! `/api/tunnel` — the client controls for the cloud tunnel relay (AMUX-2888).
//!
//! WHY THIS MODULE EXISTS BEFORE THE FEATURE DOES. The tunnel is real and in
//! production (it serves the public calendar.ics reverse proxy), but the client
//! that makes a tunnel LIVE — python's `_tunnel_start` / `_tunnel_loop` — was
//! never ported to the Rust server. The ROUTES were never mounted either, while
//! the callers stayed: the dashboard's tunnel panel (`app.js` `_tunnelStatus` /
//! `_tunnelSettingsStart` / `_tunnelSettingsStop`) and the `amux tunnel` CLI
//! (`cmd_tunnel`) both still call them.
//!
//! So the live behaviour was a 404 on `GET /api/tunnel/status` and a 405 on the
//! POSTs — the 405 because the SPA's GET-only catch-all answered a non-GET. A
//! caller cannot tell any of that apart from "amux is broken". That is the
//! `route.callers_have_routes` invariant failing, and it is the whole reason
//! this file lands before the port does.
//!
//! STATUS ANSWERS 200, THE ACTIONS ANSWER 501, and the split is deliberate.
//! `/api/proxies` already draws this line: its CRUD is native and only
//! `start`/`stop` return 501, because those are the verbs that need the relay.
//! A status READ is different in kind — amux genuinely knows the answer, and
//! the answer is "not running, and here is why". 501-ing it would replace a
//! true fact with an error and leave the panel unable to render anything.
//!
//! The port itself is still AMUX-2888. This is the honest interim, not the fix.

use super::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/start", post(start))
        .route("/stop", post(stop))
}

/// 200 with `ported: false`, not 501.
///
/// `running` is FALSE and that is a measurement, not a placeholder: nothing in
/// this build can start a tunnel, so nothing can be running. Publishing the
/// reason next to it is what lets a caller tell "off" from "broken" — the
/// distinction the 404 destroyed.
async fn status(State(_state): State<AppState>) -> Response {
    (StatusCode::OK, Json(status_body())).into_response()
}

/// The status FACTS, split out so the test can assert them (ethos rule 7). The
/// handler needs an `AppState` a unit test has no business constructing, and a
/// test that only exercised the 501 arm while being NAMED for this one is the
/// overclaiming test this repo keeps finding.
fn status_body() -> serde_json::Value {
    json!({
    "ok": true,
    "running": false,
    "ported": false,
    "url": serde_json::Value::Null,
    // The same two facts /api/proxies/config reports, so the two panels
    // cannot disagree about whether a tunnel is possible.
    "configured": std::env::var("AMUX_TUNNEL_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false),
    "card": "AMUX-2888",
    "note": "the tunnel client is not ported to the Rust server yet; CRUD and status are \
             native, the relay that makes a tunnel live is not",
    })
}

/// 501, not 404 and not a lie — the same reasoning as `proxies::tunnel_not_ported`.
/// The route exists, the capability is real, and THIS build is what lacks it.
fn not_ported(verb: &str) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "ok": false,
            "error": format!("cannot {verb} the tunnel: the tunnel client is not ported to the Rust server yet"),
            "card": "AMUX-2888",
            "why": "the relay loop (py:77931 _tunnel_start / py:77848 _tunnel_loop) has no Rust \
                    equivalent. Mounting this route is what turns a confusing 404/405 into a \
                    stated 'not implemented'; it does not add the capability.",
            "security_note": "when it IS ported it must keep python's refusal to tunnel amux's own \
                              port — the local control plane is unauthenticated, so exposing it \
                              publicly is unauthenticated RCE on YOLO sessions (py:77943, override \
                              only via AMUX_TUNNEL_ALLOW_SELF=1).",
        })),
    )
        .into_response()
}

async fn start() -> Response {
    not_ported("start")
}

async fn stop() -> Response {
    not_ported("stop")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of this module is that a caller can tell OFF from BROKEN.
    ///
    /// The second half is the control: if `status` also answered 501 there
    /// would be no way to learn "not running" from amux at all, and the panel
    /// would render an error where the truth is a state. A version that 501s
    /// everything passes "the routes are mounted" and fails the reason they
    /// were mounted.
    #[tokio::test]
    async fn status_states_a_fact_and_only_the_actions_refuse() {
        let s = not_ported_shape().await;
        assert_eq!(s.0, StatusCode::NOT_IMPLEMENTED, "actions must refuse honestly");
        assert!(s.1.contains("AMUX-2888"), "and name the card that carries the port: {}", s.1);
        assert!(
            s.1.contains("AMUX_TUNNEL_ALLOW_SELF"),
            "the refusal must carry the self-tunnel security note forward, or the port loses it: {}",
            s.1
        );

        // THE CONTROL, and the half this test is named for: status must STATE a
        // fact rather than refuse. If it 501s too, nothing can learn "not
        // running" from amux and the panel renders an error where the truth is a
        // state — which is the 404 this module was written to remove, wearing a
        // different code.
        let b = status_body();
        assert_eq!(b["running"], serde_json::json!(false), "status must answer, not refuse");
        assert_eq!(b["ported"], serde_json::json!(false), "and say WHY it is not running");
        assert_eq!(b["card"], serde_json::json!("AMUX-2888"), "naming the card that carries the port");
    }

    async fn not_ported_shape() -> (StatusCode, String) {
        let r = not_ported("start");
        let st = r.status();
        let b = axum::body::to_bytes(r.into_body(), 64 * 1024).await.unwrap_or_default();
        (st, String::from_utf8_lossy(&b).to_string())
    }
}
