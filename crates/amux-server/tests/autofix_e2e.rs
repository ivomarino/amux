//! End-to-end for AMUX-2681, both halves, through the REAL router.
//!
//! These go through `api::router(state)` with `oneshot`, so the status codes
//! and bodies asserted here are the ones a client actually receives — not a
//! paraphrase of the handler. That distinction is the whole reason job 1
//! existed: the classifier was correct in isolation and the STATUS CODE it was
//! attached to was not, and only an assertion at the response could tell.
//!
//! Deliberately NOT run against a spawned server. `ghost_rescue` and
//! `pane_size` have no off switch and act on the SHARED tmux fleet, so booting
//! a second amux-server to test something drives production panes as a side
//! effect (observed: a test instance resized three live lanes' windows within
//! 4 seconds of startup). In-process routing exercises the same handlers with
//! none of that blast radius.

use amux_server::api::AppState;
use amux_server::db::{Store, WriteOutcome};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn app() -> (axum::Router, AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("t.db")).unwrap());
    let state = AppState {
        store,
        started: std::time::Instant::now(),
        build_hash: "e2e".into(),
        auth_token: None,
    };
    (amux_server::api::router(state.clone()), state, dir)
}

async fn send(app: &axum::Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path);
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    let req = b
        .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let st = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (st, v)
}

fn log_5xx(state: &AppState, ts: f64, path: &str, body: &str) {
    let (p, b) = (path.to_string(), body.to_string());
    state
        .store
        .write(move |conn| {
            conn.execute(
                "INSERT INTO _amux_request_log (ts, method, path, family, status, latency_ms, \
                 client_ip, user_agent, amux_session, worker, answered_by, error_body) \
                 VALUES (?1,'POST',?2,'/api/sessions',500,14.5,'127.0.0.1','curl/8','', \
                 'probe','native',?3)",
                rusqlite::params![ts, p, b],
            )?;
            Ok(WriteOutcome { applied: true, events: vec![] })
        })
        .unwrap();
}

/// JOB 1, at the response. A send to a lane that is not running is a REFUSAL —
/// amux declining, correctly, with a way out — and it must not be an HTTP 500.
///
/// Pre-fix, `send_post` mapped everything except the literal "not running" to
/// 500, which is how one refusal became 14 of the night's 19 errors. This
/// asserts the code AND the actionable field the SPA renders.
#[tokio::test]
async fn a_send_refusal_answers_409_with_a_next_step_not_500() {
    let (app, _state, dir) = app();
    // AN ARCHIVED LANE, chosen deliberately. "not running" was ALREADY 409
    // before this fix, so a test built on it would pass against the bug — the
    // convenient case is convenient precisely because it lacks the property
    // that made the incident. An archived lane's send auto-wakes, the wake
    // declines, and the outcome comes back as
    //   "auto-wake failed: session is archived; wake it first"
    // which the pre-fix rule (`msg == "not running" ? 409 : 500`) mapped to
    // 500. That string is the discriminator, and it is asserted below.
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("probe.env"), "CC_DIR=/tmp\nCC_ARCHIVED=1\n").unwrap();
    let _home = TempHome::set(dir.path());

    let (st, v) = send(
        &app,
        "POST",
        "/api/sessions/probe/send",
        Some(serde_json::json!({"text": "hello"})),
    )
    .await;

    assert_ne!(st, StatusCode::INTERNAL_SERVER_ERROR, "a refusal is not a server error: {v}");
    assert_eq!(st, StatusCode::CONFLICT, "body was {v}");
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(
        v["fix"].as_str().is_some_and(|s| !s.is_empty()),
        "the SPA must be able to render a next step, got {v}"
    );
    // And the caller can still tell the message did not land.
    assert_eq!(v["submitted"], serde_json::json!(false));

    // THE DISCRIMINATOR, asserted rather than assumed: this exact outcome
    // string is one the PRE-FIX rule would have answered 500 for. If the
    // message ever changes to "not running", this test silently stops testing
    // the fix, so pin it.
    let msg = v["message"].as_str().unwrap_or_default();
    assert!(msg.contains("archived"), "expected the archived refusal, got {msg:?}");
    assert_ne!(msg, "not running", "this test must not drift onto the one case that was \
                                    already 409 before the fix");
}

/// JOB 2, end to end: a real 5xx in the request log becomes exactly one board
/// card carrying its evidence, the debug surface reports what happened, and
/// the EXISTING board drive selects that card for the lane — i.e. an autofix
/// card reaches a worker through the delivery path that already exists, with
/// no new delivery path anywhere.
#[tokio::test]
async fn a_5xx_becomes_one_card_that_the_board_drive_hands_to_a_lane() {
    let (app, state, dir) = app();
    // NO TempHome here on purpose. `autofix_tick` takes its home as an
    // argument, so this test needs no process-global — and when it DID set
    // one, it raced the sibling test above (cargo runs tests as threads in one
    // process) and made that test's lane look like it did not exist. The pin
    // on the refusal message is what caught it; without that pin the sibling
    // would have gone on passing while testing a different code path.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    // Seven identical failures — the shape of the real incident.
    for i in 0..7 {
        log_5xx(
            &state,
            now - 120.0 - i as f64,
            "/api/sessions/probe/send",
            "{\"message\":\"send-keys failed\"}",
        );
    }

    let rep = amux_server::runtime_jobs::autofix::autofix_tick(&state, dir.path()).await;
    assert_eq!(rep.filed.len(), 1, "seven identical failures are ONE card: {rep:?}");
    let card_id = rep.filed[0].0.clone();

    // The card, as a lane would receive it.
    let (st, card) = send(&app, "GET", &format!("/api/board/{card_id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{card}");
    let desc = card["desc"].as_str().unwrap_or_default();
    assert!(desc.contains("send-keys failed"), "the real error body rides on the card:\n{desc}");
    assert!(desc.contains("count: 7"), "the count is evidence:\n{desc}");
    assert!(desc.contains("re-check"), "the card must carry the query to re-check it:\n{desc}");
    assert_eq!(card["type"], serde_json::json!("investigation"));
    assert_eq!(card["status"], serde_json::json!("todo"));

    // The debug surface answers "what did it do" in one request.
    let (st, dbg) = send(&app, "GET", "/api/debug/autofix", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(dbg["enabled"], serde_json::json!(true));
    assert!(dbg["thresholds"]["window_h"].is_number(), "thresholds must be readable: {dbg}");
    assert_eq!(
        dbg["last"]["filed"][0]["card"].as_str(),
        Some(card_id.as_str()),
        "the debug surface must name the card it filed: {dbg}"
    );

    // THE HANDOFF. Not a new delivery path — board_drive's own pickup
    // predicate, unmodified, run against the database the card was filed into.
    // If this selects the card, the existing loop steers it to the lane at its
    // next turn boundary, which is the entire integration.
    let conn = state.store.read().unwrap();
    use amux_server::runtime_jobs::board_drive::Pickup;
    let picked = amux_server::runtime_jobs::board_drive::select_pickup(&conn, "probe", now);
    match picked {
        Pickup::Claim { card, prompt } => {
            assert_eq!(card, card_id, "the drive claimed a different card");
            assert!(!prompt.is_empty(), "the lane must receive a prompt with the card");
        }
        Pickup::Decompose { ids, text } => panic!(
            "the drive asked for a decomposition instead of claiming: ids={ids:?} text={text}"
        ),
        Pickup::None { reason, detail } => panic!(
            "board_drive did not select the autofix card for its lane — the card would sit \
             forever. reason={reason} detail={detail}"
        ),
    }
}

/// A no-op guard so the tempdir HOME override is restored even on panic, and
/// so the two tests above cannot leak `AMUX_HOME` into each other.
struct TempHome(Option<std::ffi::OsString>);
impl TempHome {
    fn set(p: &std::path::Path) -> Self {
        let prev = std::env::var_os("AMUX_HOME");
        std::env::set_var("AMUX_HOME", p);
        TempHome(prev)
    }
}
impl Drop for TempHome {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("AMUX_HOME", v),
            None => std::env::remove_var("AMUX_HOME"),
        }
    }
}
