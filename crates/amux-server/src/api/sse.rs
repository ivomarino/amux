//! `/api/events` — SSE (RR-0023/RR-0042, Invariants 35/26).
//!
//! One stream, three event kinds:
//! - Revisioned StateEvents (`{"type":"state",...}`) + gap detection
//!   (`lagged`) + `hello` carrying the current rev.
//! - `{"type":"invalidate","keys":["board"|"sessions",...]}` — the signal
//!   that replaced the legacy full-list pushes (AMUX-3503). The old dialect
//!   shipped the ENTIRE board (883KB) + sessions (177KB) on every connect
//!   and every coalesced change, RAW — CompressionLayer exempts
//!   text/event-stream — while the client's fetch path was already gzipped
//!   AND ETag'd, so intermittent mobile paid ~1MB per reconnect for data a
//!   conditional fetch serves as a 304. Worse, half the push was DEAD: the
//!   server said `"type":"sessions"`, the client only handled `workers`
//!   (the AF-10 vocabulary-rename class), so 177KB per push was parsed and
//!   dropped for weeks. The client now fetches on invalidate through the
//!   cheap path; a burst of N writes still coalesces to one signal.
//! - The 10s `{"type":"ping"}` keep-alive (the SPA's 18s staleness detector
//!   feeds on it, and its `v` drives stale-shell self-reload — which is
//!   also what migrates an old full-push client off this contract within
//!   seconds of connecting to a new server).

use super::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use futures::StreamExt;
use std::convert::Infallible;
use std::time::Duration;

pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.store.subscribe();
    let current = state.store.current_rev().map(|r| r.0).unwrap_or(0);

    let stream = async_stream(current, move |yielder: tokio::sync::mpsc::Sender<Event>| async move {
        // No initial snapshot (AMUX-3503): `hello` above carries the rev, and
        // the client renders from its cache then conditional-fetches — a 304
        // when nothing changed, 176KB gzipped when something did, versus the
        // 1,060KB raw this used to push on every (re)connect.
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::json!({
                        "type": "state",
                        "payload": ev,
                    });
                    if yielder
                        .send(Event::default().data(payload.to_string()))
                        .await
                        .is_err()
                    {
                        break; // client went away
                    }
                    // Coalesce this event plus everything already queued:
                    // a burst of N writes = one invalidate signal.
                    let mut board_dirty = matches!(
                        ev.entity_type,
                        amux_core::revision::EntityType::Task
                    );
                    let mut sessions_dirty = matches!(
                        ev.entity_type,
                        amux_core::revision::EntityType::Worker
                            | amux_core::revision::EntityType::Session
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while let Ok(more) = rx.try_recv() {
                        board_dirty |= matches!(
                            more.entity_type,
                            amux_core::revision::EntityType::Task
                        );
                        sessions_dirty |= matches!(
                            more.entity_type,
                            amux_core::revision::EntityType::Worker
                                | amux_core::revision::EntityType::Session
                        );
                    }
                    if board_dirty || sessions_dirty {
                        let ev = invalidate_payload(board_dirty, sessions_dirty);
                        if yielder.send(Event::default().data(ev)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    // Backpressure (Invariant 26): a slow client that missed
                    // events is TOLD so, with the count — it must delta-sync,
                    // not assume continuity. The invalidate makes the SPA
                    // refetch both lists through the conditional path, which
                    // IS its recovery.
                    let payload = serde_json::json!({
                        "type": "lagged",
                        "missed": missed,
                    });
                    if yielder
                        .send(Event::default().data(payload.to_string()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    if yielder
                        .send(Event::default().data(invalidate_payload(true, true)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(stream).keep_alive(
        // The keep-alive IS the ping contract: 10s cadence, real data event.
        // `v` = the APP_VER of the app.js THIS binary embeds (Python parity,
        // amux-server.py:65292): the SPA's ping handler self-reloads on
        // mismatch (rate-limited, SW-nudged), which is how clients adopt a
        // new build without a human clicking anything — the server restarts
        // on deploy, the next ping carries the new version, every open
        // window follows. Ethan 2026-08-09: clients restart like the Python
        // server's, not behind a banner.
        // .event(data), NOT .text(): KeepAlive::text() is literally
        // Event::default().comment(t), and an SSE COMMENT never reaches
        // EventSource.onmessage — so the ping parsed right on the wire and
        // was invisible to the client: self-reload unreachable, _lastDataTime
        // starved, and the SPA's 18s zombie detector reconnect-looped on any
        // quiet fleet (browser re-verify, 2026-08-09: 2 comment-pings, 0
        // data-pings in 32s of raw wire). Axum keep-alives fire only after
        // `interval` of SILENCE, unlike Python's unconditional 10s ping —
        // acceptable, because flowing data events feed _lastDataTime
        // themselves; the ping covers exactly the quiet gaps.
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .event(Event::default().data(ping_payload())),
    )
}

/// `{"type":"ping","v":"<APP_VER>"}` with the version parsed ONCE from the
/// embedded app.js — the same file this server serves, so client and ping can
/// never disagree about what "current" means. Falls back to a bare ping if
/// the constant ever moves (a missing `v` disables self-reload, it does not
/// break liveness).
fn ping_payload() -> &'static str {
    static PAYLOAD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PAYLOAD.get_or_init(|| {
        let ver = amux_dashboard::DashboardAssets::get("app.js")
            .and_then(|f| {
                let s = String::from_utf8_lossy(&f.data).into_owned();
                s.split("const APP_VER = '")
                    .nth(1)
                    .and_then(|rest| rest.split('\'').next().map(String::from))
            })
            .unwrap_or_default();
        if ver.is_empty() {
            "{\"type\":\"ping\"}".to_string()
        } else {
            format!("{{\"type\":\"ping\",\"v\":\"{ver}\"}}")
        }
    })
}

/// The invalidate signal (AMUX-3503) — the entire replacement for the legacy
/// full-list pushes. `keys` uses the SPA's existing invalidate vocabulary
/// (`msg.keys`, an array), so the event shape predates this change on the
/// client side. Pure so the contract is pinned by test: this exact JSON is
/// what app.js parses, and a shape drift here silently stops every refetch.
fn invalidate_payload(board: bool, sessions: bool) -> String {
    let mut keys: Vec<&str> = Vec::new();
    if board {
        keys.push("board");
    }
    if sessions {
        keys.push("sessions");
    }
    format!(
        "{{\"type\":\"invalidate\",\"keys\":{}}}",
        serde_json::to_string(&keys).unwrap_or_else(|_| "[]".into())
    )
}

/// Bridge an async producer into an SSE stream, prefixed with a `hello`
/// event carrying the current revision so the client knows exactly where
/// the live stream begins (and what to pass to /api/sync).
fn async_stream<F, Fut>(
    current_rev: u64,
    producer: F,
) -> impl Stream<Item = Result<Event, Infallible>>
where
    F: FnOnce(tokio::sync::mpsc::Sender<Event>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(256);
    let hello = Event::default().data(
        serde_json::json!({"type": "hello", "rev": current_rev}).to_string(),
    );
    tokio::spawn(producer(tx));
    futures::stream::once(async move { Ok(hello) }).chain(futures::stream::unfold(
        rx,
        |mut rx| async move {
            rx.recv().await.map(|ev| (Ok(ev), rx))
        },
    ))
}

#[cfg(test)]
mod ping_tests {
    /// AMUX-3503 — this exact JSON is the ENTIRE replacement for the 1MB
    /// legacy pushes, and app.js parses it as `msg.keys` (an array). A shape
    /// drift here silently stops every SSE-driven refetch fleet-wide, so the
    /// bytes are pinned, not just the semantics. Both-false is the
    /// cannot-happen guard (callers gate on dirty) — pinned anyway so a
    /// future caller cannot ship a keyless invalidate the client ignores.
    #[test]
    fn invalidate_payload_is_the_exact_shape_the_client_parses() {
        assert_eq!(
            super::invalidate_payload(true, true),
            r#"{"type":"invalidate","keys":["board","sessions"]}"#
        );
        assert_eq!(
            super::invalidate_payload(true, false),
            r#"{"type":"invalidate","keys":["board"]}"#
        );
        assert_eq!(
            super::invalidate_payload(false, true),
            r#"{"type":"invalidate","keys":["sessions"]}"#
        );
        assert_eq!(
            super::invalidate_payload(false, false),
            r#"{"type":"invalidate","keys":[]}"#
        );
    }

    #[test]
    fn ping_carries_the_embedded_app_ver() {
        let p = super::ping_payload();
        // The version must be the one inside the EMBEDDED app.js — not a
        // hardcoded copy that drifts on the next client bump.
        let f = amux_dashboard::DashboardAssets::get("app.js").unwrap();
        let s = String::from_utf8_lossy(&f.data).into_owned();
        let ver = s
            .split("const APP_VER = '")
            .nth(1)
            .and_then(|r| r.split('\'').next())
            .expect("APP_VER present in embedded app.js");
        assert_eq!(p, &format!("{{\"type\":\"ping\",\"v\":\"{ver}\"}}"));
    }
}
