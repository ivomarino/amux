//! `/api/events` — SSE with revisioned StateEvents (RR-0023, Invariant 35).
//!
//! Every event carries the global revision, so a client can detect a gap
//! (missed events while backgrounded) and recover via `/api/sync?since_rev=`
//! instead of trusting an incomplete picture. A real `{"type":"ping"}` every
//! 10s lets clients detect zombie connections (carried over from the Python
//! SSE contract — clients declare staleness at 18s of silence).

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
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    // Backpressure (Invariant 26): a slow client that missed
                    // events is TOLD so, with the count — it must delta-sync,
                    // not assume continuity.
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
