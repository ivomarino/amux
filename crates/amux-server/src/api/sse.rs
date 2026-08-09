//! `/api/events` — SSE (RR-0023/RR-0042, Invariants 35/26).
//!
//! Speaks BOTH dialects on one stream:
//! - Modern: revisioned StateEvents (`{"type":"state",...}`) + gap
//!   detection (`lagged`) + `hello` carrying the current rev.
//! - Legacy: the Python dashboard's own event vocabulary —
//!   `{"type":"board","payload":[...]}` and `{"type":"sessions",
//!   "payload":[...]}` full-list pushes, coalesced. Without these the SPA
//!   CONNECTS (so its polling fallback disarms) and then understands
//!   nothing — strictly worse than no SSE at all, which is exactly what
//!   the browser-golden run demonstrated. The 10s `{"type":"ping"}`
//!   keep-alive serves both dialects (the SPA's 18s staleness detector
//!   feeds on it).

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
    let store = state.store.clone();

    let stream = async_stream(current, move |yielder: tokio::sync::mpsc::Sender<Event>| async move {
        // Initial full pushes so a fresh client renders without a poll.
        push_legacy(&store, &yielder, true, true).await;
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
                    // Legacy dialect: coalesce this event plus everything
                    // already queued (a burst of N writes = one list push).
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
                    if (board_dirty || sessions_dirty)
                        && !push_legacy(&store, &yielder, board_dirty, sessions_dirty).await
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    // Backpressure (Invariant 26): a slow client that missed
                    // events is TOLD so, with the count — it must delta-sync,
                    // not assume continuity. Legacy clients get fresh full
                    // lists, which IS their recovery.
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
                    if !push_legacy(&store, &yielder, true, true).await {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(stream).keep_alive(
        // The keep-alive IS the ping contract: 10s cadence, real data event.
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("{\"type\":\"ping\"}"),
    )
}

/// Push the legacy full-list events. Returns false when the client is gone.
async fn push_legacy(
    store: &crate::db::SharedStore,
    yielder: &tokio::sync::mpsc::Sender<Event>,
    board: bool,
    sessions: bool,
) -> bool {
    if board {
        let payload = {
            let s = store.clone();
            tokio::task::spawn_blocking(move || legacy_board_json(&s))
                .await
                .ok()
                .flatten()
        };
        if let Some(json) = payload {
            let ev = format!("{{\"type\":\"board\",\"payload\":{json}}}");
            if yielder.send(Event::default().data(ev)).await.is_err() {
                return false;
            }
        }
    }
    if sessions {
        let payload = {
            let s = store.clone();
            tokio::task::spawn_blocking(move || legacy_sessions_json(&s))
                .await
                .ok()
                .flatten()
        };
        if let Some(json) = payload {
            let ev = format!("{{\"type\":\"sessions\",\"payload\":{json}}}");
            if yielder.send(Event::default().data(ev)).await.is_err() {
                return false;
            }
        }
    }
    true
}

fn legacy_board_json(store: &crate::db::SharedStore) -> Option<String> {
    let conn = store.read().ok()?;
    let rows = crate::db::board_store::list_issues(
        &conn,
        &[],
        &[],
        crate::db::board_store::ArchivedFilter::ActiveOnly,
    )
    .ok()?;
    // Python's SSE board channel (amux-server.py:65222-65249): the
    // _load_board(100) TERMINAL QUOTAS (verified gets its own 300-floor so
    // bulk-verifies stay visible), non-archived only, desc slimmed to its
    // first line — NOT the REST path's lumped cap_terminal(100).
    let kept = crate::db::board_store::sse_terminal_quota(rows, 100);
    // Same stale derivation as GET /api/board (a view must share its
    // mechanism's predicate) — skipped when no in-progress card needs it.
    let working = if kept
        .iter()
        .any(|r| matches!(r.status.as_str(), "doing" | "review"))
    {
        crate::api::sessions_legacy::active_python_sessions(&conn)
    } else {
        Default::default()
    };
    let now = chrono::Utc::now().timestamp();
    let items: Vec<serde_json::Value> = kept
        .iter()
        // The API's own list serializer — the SSE view and the GET view can
        // never disagree (a view must share its mechanism's predicate) —
        // plus Python's SSE-only desc slimming on top.
        .map(|r| {
            let mut v = crate::api::board::list_body(
                r,
                false,
                crate::api::board::is_stale(r, now, &working),
            );
            crate::api::board::apply_sse_desc_slim(&mut v, &r.desc);
            v
        })
        .collect();
    serde_json::to_string(&items).ok()
}

fn legacy_sessions_json(store: &crate::db::SharedStore) -> Option<String> {
    crate::api::sessions_legacy::legacy_sessions_array(store).ok()
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
