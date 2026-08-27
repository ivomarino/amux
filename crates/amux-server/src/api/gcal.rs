//! Google Calendar API endpoints (Phase 3)
//!
//! Multi-account Google Calendar sync via existing OAuth grants
//! in ~/.amux/connectors/google/<email>.json. Unified iCal feed
//! across every connected account.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use crate::db::calendar;
use super::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/accounts", get(list_accounts))
        .route("/events", get(list_events))
        .route("/events", post(create_event))
        .route("/events/{event_id}", axum::routing::put(update_event))
        .route("/events/{event_id}", axum::routing::delete(delete_event))
        .route("/status", get(sync_status))
        .route("/sync", post(trigger_sync))
        .route("/unified.ics", get(unified_ical))
}

#[derive(Serialize)]
pub struct AccountResponse {
    accounts: Vec<calendar::CalendarAccount>,
}

#[derive(Serialize)]
pub struct EventsResponse {
    events: Vec<calendar::CalendarEvent>,
    total: usize,
}

#[derive(Serialize)]
pub struct SyncStatusResponse {
    account_id: String,
    status: String,
    event_count: i32,
    last_sync: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateEventRequest {
    account_id: String,
    calendar_id: Option<String>,
    title: String,
    description: Option<String>,
    start_time: String,
    end_time: String,
    attendees: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct CreateEventResponse {
    event_id: String,
    message: String,
}

#[derive(Deserialize)]
pub struct UpdateEventRequest {
    title: Option<String>,
    description: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
}

#[derive(Deserialize)]
pub struct SyncRequest {
    account_id: Option<String>,
}

#[derive(Serialize)]
pub struct SyncResponse {
    synced: Vec<String>,
    failed: Vec<(String, String)>,
    total_events: i32,
}

/// GET /api/gcal/accounts
/// List all configured Google Calendar accounts with sync status
pub async fn list_accounts(
    State(state): State<AppState>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let accounts = calendar::get_accounts(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AccountResponse { accounts }))
}

/// GET /api/gcal/events
/// List synced Google Calendar events
#[derive(Deserialize)]
pub struct EventsQuery {
    account_id: Option<String>,
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let events = if let Some(account_id) = q.account_id {
        calendar::get_events_for_account(&conn, &account_id, None, None)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        calendar::get_all_events(&conn)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let total = events.len();
    Ok(Json(EventsResponse { events, total }))
}

/// POST /api/gcal/events
/// Create a new Google Calendar event with optional attendees/invites
pub async fn create_event(
    State(state): State<AppState>,
    Json(req): Json<CreateEventRequest>,
) -> Result<Json<CreateEventResponse>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let account = calendar::get_accounts(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|a| a.id == req.account_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Account not found".to_string()))?;

    // Load OAuth token for this account
    let oauth_token = crate::integrations::gcal_sync::load_refresh_token(&account.id, &account.email)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Failed to load OAuth token: {}", e)))?;

    let calendar_id = req.calendar_id.unwrap_or_else(|| "primary".to_string());
    let attendee_refs: Option<Vec<&str>> = req.attendees.as_ref().map(|a| a.iter().map(|s| s.as_str()).collect());

    let event_id = crate::integrations::gcal_sync::create_calendar_event(
        &oauth_token,
        &calendar_id,
        &req.title,
        req.description.as_deref(),
        &req.start_time,
        &req.end_time,
        attendee_refs,
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(CreateEventResponse {
        event_id: event_id.clone(),
        message: format!("Event created successfully. ID: {}", event_id),
    }))
}

/// PUT /api/gcal/events/:event_id
/// Update an existing Google Calendar event
pub async fn update_event(
    State(state): State<AppState>,
    axum::extract::Path(event_id): axum::extract::Path<String>,
    Json(req): Json<UpdateEventRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let events = calendar::get_all_events(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let event = events
        .iter()
        .find(|e| e.id.contains(&event_id))
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event not found".to_string()))?;

    let account = calendar::get_accounts(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|a| a.id == event.account_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Account not found".to_string()))?;

    // Load OAuth token for this account
    let oauth_token = crate::integrations::gcal_sync::load_refresh_token(&account.id, &account.email)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Failed to load OAuth token: {}", e)))?;

    crate::integrations::gcal_sync::update_calendar_event(
        &oauth_token,
        &event.calendar_id,
        &event.event_id,
        req.title.as_deref(),
        req.description.as_deref(),
        req.start_time.as_deref(),
        req.end_time.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "message": "Event updated successfully",
        "event_id": event_id
    })))
}

/// DELETE /api/gcal/events/:event_id
/// Delete a Google Calendar event
pub async fn delete_event(
    State(state): State<AppState>,
    axum::extract::Path(event_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let events = calendar::get_all_events(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let event = events
        .iter()
        .find(|e| e.id.contains(&event_id))
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event not found".to_string()))?;

    let account = calendar::get_accounts(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|a| a.id == event.account_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Account not found".to_string()))?;

    // Load OAuth token for this account
    let oauth_token = crate::integrations::gcal_sync::load_refresh_token(&account.id, &account.email)
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Failed to load OAuth token: {}", e)))?;

    crate::integrations::gcal_sync::delete_calendar_event(
        &oauth_token,
        &event.calendar_id,
        &event.event_id,
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "message": "Event deleted successfully",
        "event_id": event_id
    })))
}

/// GET /api/gcal/status
/// Get current sync status for all accounts
pub async fn sync_status(
    State(state): State<AppState>,
) -> Result<Json<Vec<SyncStatusResponse>>, (StatusCode, String)> {
    let conn = state
        .store
        .read()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let accounts = calendar::get_accounts(&conn)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let responses: Vec<SyncStatusResponse> = accounts
        .into_iter()
        .map(|acc| SyncStatusResponse {
            account_id: acc.id.clone(),
            status: acc.sync_status.clone(),
            event_count: acc.event_count,
            last_sync: acc.synced_at.clone(),
        })
        .collect();

    Ok(Json(responses))
}

/// POST /api/gcal/sync
/// Trigger manual sync of calendar events from Google Calendar API
/// Requires auth token
pub async fn trigger_sync(
    State(state): State<AppState>,
    body: Option<Json<SyncRequest>>,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    // Verify auth token (optional for now, will implement full auth later)
    if state.auth_token.is_none() {
        return Err((StatusCode::UNAUTHORIZED, "auth required".to_string()));
    }

    let accounts = {
        let conn = state
            .store
            .read()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        calendar::get_accounts(&conn)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let filter_id = body.as_ref().and_then(|b| b.account_id.clone());

    let mut synced = Vec::new();
    let mut failed = Vec::new();

    for account in &accounts {
        if let Some(ref id) = filter_id {
            if account.id != *id {
                continue;
            }
        }

        // Runs the same fetch-from-Google + store-in-SQLite logic as the
        // 15-minute background job (crate::runtime_jobs::gcal_sync_job) —
        // this used to be a stub that only echoed back event_count already
        // in the DB, so a manual sync never actually talked to Google.
        match crate::runtime_jobs::gcal_sync_job::sync_account(&state.store, &account.id, &account.email).await {
            Ok(()) => synced.push(account.id.clone()),
            Err(e) => failed.push((account.id.clone(), e.to_string())),
        }
    }

    let total_events = {
        let conn = state
            .store
            .read()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        calendar::get_accounts(&conn)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .iter()
            .map(|a| a.event_count)
            .sum()
    };

    Ok(Json(SyncResponse {
        synced,
        failed,
        total_events,
    }))
}

/// GET /api/gcal/unified.ics
/// Unified iCal feed from all Google Calendar accounts (Phase 4)
/// Bearer token gated for future access control
pub async fn unified_ical(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let conn = match state.store.read() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let events = match calendar::get_all_events(&conn) {
        Ok(e) => e,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut ical = String::new();
    ical.push_str("BEGIN:VCALENDAR\r\n");
    ical.push_str("VERSION:2.0\r\n");
    ical.push_str("PRODID:-//amux//Google Calendar sync//EN\r\n");
    ical.push_str("CALSCALE:GREGORIAN\r\n");
    ical.push_str("METHOD:PUBLISH\r\n");
    ical.push_str("X-WR-CALNAME:amux - Google Calendars (unified)\r\n");
    ical.push_str("X-WR-CALDESC:Unified feed from all connected Google accounts\r\n");
    ical.push_str("REFRESH-INTERVAL;VALUE=DURATION:PT15M\r\n");
    ical.push_str("X-PUBLISHED-TTL:PT15M\r\n");

    for event in events {
        ical.push_str("BEGIN:VEVENT\r\n");
        ical.push_str(&format!("UID:{}@amux.local\r\n", event.id));
        ical.push_str(&format!(
            "DTSTAMP:{}\r\n",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        ical.push_str(&format!(
            "SUMMARY:{}\r\n",
            event.title.replace(['\n', '\r'], " ")
        ));

        if let Some(desc) = event.description {
            ical.push_str(&format!(
                "DESCRIPTION:{}\r\n",
                desc.replace(['\n', '\r'], " ")
            ));
        }

        if let Some(start) = event.start_time {
            ical.push_str(&format!("DTSTART:{}\r\n", start));
        }
        if let Some(end) = event.end_time {
            ical.push_str(&format!("DTEND:{}\r\n", end));
        }

        if let Some(location) = event.location {
            ical.push_str(&format!(
                "LOCATION:{}\r\n",
                location.replace(['\n', '\r'], " ")
            ));
        }

        ical.push_str(&format!("STATUS:{}\r\n", event.status.to_uppercase()));
        ical.push_str("TRANSP:OPAQUE\r\n");
        ical.push_str("END:VEVENT\r\n");
    }

    ical.push_str("END:VCALENDAR\r\n");

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/calendar; charset=utf-8",
        )],
        ical,
    )
        .into_response()
}
