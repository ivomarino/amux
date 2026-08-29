//! Telegram connector: mapping CRUD + status + outbound send.
//!
//! Mounted at `/api/telegram`. The bot TOKEN itself is not managed here — it
//! rides the existing connector credential path (`api/connectors.rs`'s
//! `telegram` registry row, `POST /api/connectors/telegram/credentials`),
//! same as every other API-key connector. This module owns only what that
//! path does not: which Telegram chat maps to which amux session, and a
//! manual outbound send for a session/hook that wants to notify one.
//!
//! Inbound routing (Telegram -> session) lives in
//! `runtime_jobs::telegram_poll` — this module is the read/write surface
//! over its data, not the loop itself.

use super::AppState;
use crate::db::telegram as tg_db;
use crate::runtime_jobs::telegram_poll;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/mappings", get(list_mappings).post(create_mapping))
        .route("/mappings/{chat_id}", axum::routing::delete(delete_mapping))
        .route("/send", axum::routing::post(send))
}

fn err(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

async fn status(State(state): State<AppState>) -> Response {
    let mapping_count = state
        .store
        .read()
        .ok()
        .and_then(|conn| tg_db::list(&conn).ok())
        .map(|v| v.len())
        .unwrap_or(0);
    let report = telegram_poll::last_report();
    (
        StatusCode::OK,
        Json(json!({
            "bot_token_set": std::env::var("TELEGRAM_BOT_TOKEN").map(|v| !v.trim().is_empty()).unwrap_or(false),
            "mapping_count": mapping_count,
            "last_poll_at": report.last_poll_at,
            "last_error": report.last_error,
            "messages_routed": report.messages_routed,
            "messages_unlinked": report.messages_unlinked,
        })),
    )
        .into_response()
}

async fn list_mappings(State(state): State<AppState>) -> Response {
    match state.store.read().map_err(|e| e.to_string()).and_then(|conn| {
        tg_db::list(&conn).map_err(|e| e.to_string())
    }) {
        Ok(rows) => (StatusCode::OK, Json(json!({ "mappings": rows }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    }
}

#[derive(Deserialize)]
struct CreateMappingReq {
    chat_id: i64,
    session: String,
}

/// Manual mapping creation — mirrors what `/link <session>` does from the
/// Telegram side, for an operator who wants to pre-link a chat_id (e.g.
/// looked up via `getUpdates` before the bot has ever received `/link`) or
/// fix a mapping without going through the chat.
async fn create_mapping(State(state): State<AppState>, Json(body): Json<CreateMappingReq>) -> Response {
    if body.session.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "session is required" }));
    }
    let known = super::session_verbs::all_lane_names();
    if !known.iter().any(|n| n == &body.session) {
        return err(
            StatusCode::NOT_FOUND,
            json!({ "error": "no such session", "session": body.session, "known": known }),
        );
    }
    let result = state
        .store
        .read()
        .map_err(|e| e.to_string())
        .and_then(|conn| tg_db::upsert(&conn, body.chat_id, &body.session, None).map_err(|e| e.to_string()));
    match result {
        Ok(()) => (StatusCode::OK, Json(json!({ "chat_id": body.chat_id, "session": body.session }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    }
}

async fn delete_mapping(State(state): State<AppState>, Path(chat_id): Path<i64>) -> Response {
    match state.store.read().map_err(|e| e.to_string()).and_then(|conn| {
        tg_db::remove(&conn, chat_id).map_err(|e| e.to_string())
    }) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, json!({ "error": "no mapping for that chat_id" })),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    }
}

#[derive(Deserialize)]
struct SendReq {
    /// Either `session` (resolved to its most-recently-linked chat) or a raw
    /// `chat_id` — exactly one must be present. `session` is the common case
    /// (a hook knows the session it's running as, not the chat_id);
    /// `chat_id` exists for callers that already resolved it (or want to
    /// message a chat with no session mapping at all).
    session: Option<String>,
    chat_id: Option<i64>,
    text: String,
}

async fn send(State(state): State<AppState>, Json(body): Json<SendReq>) -> Response {
    if body.text.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "text is required" }));
    }
    let chat_id = match (body.chat_id, body.session.as_deref()) {
        (Some(id), _) => Some(id),
        (None, Some(session)) => {
            let looked_up = state
                .store
                .read()
                .ok()
                .and_then(|conn| tg_db::by_session(&conn, session).ok().flatten());
            match looked_up {
                Some(m) => Some(m.chat_id),
                None => {
                    return err(
                        StatusCode::NOT_FOUND,
                        json!({ "error": "no Telegram chat linked to that session", "session": session }),
                    )
                }
            }
        }
        (None, None) => {
            return err(StatusCode::BAD_REQUEST, json!({ "error": "one of chat_id or session is required" }))
        }
    };
    let Some(chat_id) = chat_id else { unreachable!() };
    let Some(token) = std::env::var("TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.trim().is_empty()) else {
        return err(StatusCode::PRECONDITION_FAILED, json!({ "error": "TELEGRAM_BOT_TOKEN not set" }));
    };
    match telegram_poll::send_message(&token, chat_id, &body.text).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "sent": true, "chat_id": chat_id }))).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, json!({ "error": e })),
    }
}
