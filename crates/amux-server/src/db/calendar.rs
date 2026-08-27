//! Google Calendar database operations (Phase 2)
//!
//! Manages calendar accounts, events, and sync metadata for multi-account
//! calendar support (any number of connected Google accounts).

use rusqlite::{Connection, params, OptionalExtension};
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarAccount {
    pub id: String,
    pub email: String,
    pub service_name: String,
    pub display_name: Option<String>,
    pub is_primary: bool,
    pub sync_status: String, // pending, syncing, ok, error
    pub event_count: i32,
    pub synced_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub event_id: String,
    pub account_id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub status: String, // confirmed, tentative, cancelled
    pub synced_at: String,
    // attendees/organizer columns existed since migration 0034 but were
    // never populated; html_link/meeting_url added in 0035. All were
    // available from Google's response the whole time (see convert_event).
    pub attendees: Option<String>,  // JSON array: [{email,display_name,response_status,organizer,is_self}]
    pub organizer: Option<String>,  // organizer's email
    pub html_link: Option<String>,  // "open in Google Calendar" URL
    pub meeting_url: Option<String>, // Meet/conference join URL, if any
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSyncMetadata {
    pub account_id: String,
    pub total_events: i32,
    pub last_full_sync_at: Option<String>,
    pub last_incremental_sync_at: Option<String>,
    pub is_enabled: bool,
    pub owner: String,
    pub purpose: String,
}

/// Get all calendar accounts
pub fn get_accounts(conn: &Connection) -> anyhow::Result<Vec<CalendarAccount>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, service_name, display_name, is_primary, sync_status,
                event_count, synced_at, created_at
         FROM calendar_accounts
         ORDER BY is_primary DESC, email ASC"
    )?;

    let accounts = stmt.query_map([], |row| {
        Ok(CalendarAccount {
            id: row.get(0)?,
            email: row.get(1)?,
            service_name: row.get(2)?,
            display_name: row.get(3)?,
            is_primary: row.get::<_, i32>(4)? != 0,
            sync_status: row.get(5)?,
            event_count: row.get(6)?,
            synced_at: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;

    let mut result = Vec::new();
    for account in accounts {
        result.push(account?);
    }
    Ok(result)
}

/// Get account by ID
pub fn get_account(conn: &Connection, id: &str) -> anyhow::Result<Option<CalendarAccount>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, service_name, display_name, is_primary, sync_status,
                event_count, synced_at, created_at
         FROM calendar_accounts
         WHERE id = ?1"
    )?;

    stmt.query_row([id], |row| {
        Ok(CalendarAccount {
            id: row.get(0)?,
            email: row.get(1)?,
            service_name: row.get(2)?,
            display_name: row.get(3)?,
            is_primary: row.get::<_, i32>(4)? != 0,
            sync_status: row.get(5)?,
            event_count: row.get(6)?,
            synced_at: row.get(7)?,
            created_at: row.get(8)?,
        })
    })
    .optional()
    .map_err(|e| e.into())
}

/// Update account sync status
pub fn update_sync_status(
    conn: &Connection,
    account_id: &str,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE calendar_accounts
         SET sync_status = ?1, last_error = ?2, updated_at = ?3
         WHERE id = ?4",
        params![status, error, Utc::now().to_rfc3339(), account_id],
    )?;
    Ok(())
}

/// Mark sync as complete
pub fn mark_sync_complete(
    conn: &Connection,
    account_id: &str,
    event_count: i32,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE calendar_accounts
         SET sync_status = 'ok', synced_at = ?1, event_count = ?2, updated_at = ?3
         WHERE id = ?4",
        params![Utc::now().to_rfc3339(), event_count, Utc::now().to_rfc3339(), account_id],
    )?;
    Ok(())
}

/// Insert or update calendar event
pub fn upsert_event(
    conn: &Connection,
    event: &CalendarEvent,
) -> anyhow::Result<()> {
    // created_at/updated_at are NOT NULL with no DEFAULT (migration 0034).
    // The INSERT column list previously omitted created_at entirely, so
    // every insert failed the NOT NULL constraint and sync silently stored
    // zero rows while logging a nonzero "events stored" count (that count
    // came from events.len(), not from a successful-write tally).
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO calendar_events
         (id, event_id, account_id, calendar_id, title, description,
          start_time, end_time, all_day, location, status, synced_at,
          attendees, organizer, html_link, meeting_url,
          created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)
         ON CONFLICT(account_id, event_id) DO UPDATE SET
         title = ?5, description = ?6, start_time = ?7, end_time = ?8,
         all_day = ?9, location = ?10, status = ?11, synced_at = ?12,
         attendees = ?13, organizer = ?14, html_link = ?15, meeting_url = ?16,
         updated_at = ?17",
        params![
            &event.id, &event.event_id, &event.account_id, &event.calendar_id,
            &event.title, &event.description, &event.start_time, &event.end_time,
            if event.all_day { 1 } else { 0 },
            &event.location, &event.status, &event.synced_at,
            &event.attendees, &event.organizer, &event.html_link, &event.meeting_url,
            now
        ],
    )?;
    Ok(())
}

/// Delete one calendar_events row by its primary key. Used by sync to prune
/// local rows for events that no longer come back from Google (deleted,
/// including via our own DELETE /api/gcal/events/:id — that endpoint only
/// calls Google's API and relies on the next sync to reflect the removal
/// locally; before this, nothing ever pruned, so a deleted event stayed a
/// permanent ghost in the local cache).
pub fn delete_event(conn: &Connection, id: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM calendar_events WHERE id = ?1", params![id])?;
    Ok(())
}

/// Get events for an account within a time range
pub fn get_events_for_account(
    conn: &Connection,
    account_id: &str,
    start: Option<&str>,
    end: Option<&str>,
) -> anyhow::Result<Vec<CalendarEvent>> {
    // Placeholders for start/end are always present and always bound — the
    // `(?N IS NULL OR ...)` form makes a None filter a no-op in SQL rather
    // than conditionally omitting the clause from the query text while still
    // always binding 3 params via params![account_id, start, end] below.
    // That mismatch (1 placeholder in the SQL when both were None, 3 values
    // bound regardless) made this function error on every call with start
    // and/or end omitted — including every account-filtered
    // GET /api/gcal/events?account_id=... request, and the sync prune step
    // added later, which swallowed the error via `if let Ok(...)` and so
    // silently pruned nothing, ever.
    let query = "SELECT id, event_id, account_id, calendar_id, title, description,
                      start_time, end_time, all_day, location, status, synced_at,
                      attendees, organizer, html_link, meeting_url
               FROM calendar_events
               WHERE account_id = ?1
                 AND (?2 IS NULL OR start_time >= ?2)
                 AND (?3 IS NULL OR end_time <= ?3)
               ORDER BY start_time ASC";

    let mut stmt = conn.prepare(query)?;
    let events = stmt.query_map(params![account_id, start, end], |row| {
        Ok(CalendarEvent {
            id: row.get(0)?,
            event_id: row.get(1)?,
            account_id: row.get(2)?,
            calendar_id: row.get(3)?,
            title: row.get(4)?,
            description: row.get(5)?,
            start_time: row.get(6)?,
            end_time: row.get(7)?,
            all_day: row.get::<_, i32>(8)? != 0,
            location: row.get(9)?,
            status: row.get(10)?,
            synced_at: row.get(11)?,
            attendees: row.get(12)?,
            organizer: row.get(13)?,
            html_link: row.get(14)?,
            meeting_url: row.get(15)?,
        })
    })?;

    let mut result = Vec::new();
    for event in events {
        result.push(event?);
    }
    Ok(result)
}

/// Get all events across all accounts
pub fn get_all_events(conn: &Connection) -> anyhow::Result<Vec<CalendarEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_id, account_id, calendar_id, title, description,
                start_time, end_time, all_day, location, status, synced_at,
                attendees, organizer, html_link, meeting_url
         FROM calendar_events
         ORDER BY start_time ASC"
    )?;

    let events = stmt.query_map([], |row| {
        Ok(CalendarEvent {
            id: row.get(0)?,
            event_id: row.get(1)?,
            account_id: row.get(2)?,
            calendar_id: row.get(3)?,
            title: row.get(4)?,
            description: row.get(5)?,
            start_time: row.get(6)?,
            end_time: row.get(7)?,
            all_day: row.get::<_, i32>(8)? != 0,
            location: row.get(9)?,
            status: row.get(10)?,
            synced_at: row.get(11)?,
            attendees: row.get(12)?,
            organizer: row.get(13)?,
            html_link: row.get(14)?,
            meeting_url: row.get(15)?,
        })
    })?;

    let mut result = Vec::new();
    for event in events {
        result.push(event?);
    }
    Ok(result)
}

/// Get sync metadata for an account
pub fn get_sync_metadata(conn: &Connection, account_id: &str) -> anyhow::Result<Option<CalendarSyncMetadata>> {
    let mut stmt = conn.prepare(
        "SELECT account_id, total_events, last_full_sync_at, last_incremental_sync_at,
                is_enabled, owner, purpose
         FROM calendar_sync_metadata
         WHERE account_id = ?1"
    )?;

    stmt.query_row([account_id], |row| {
        Ok(CalendarSyncMetadata {
            account_id: row.get(0)?,
            total_events: row.get(1)?,
            last_full_sync_at: row.get(2)?,
            last_incremental_sync_at: row.get(3)?,
            is_enabled: row.get::<_, i32>(4)? != 0,
            owner: row.get(5)?,
            purpose: row.get(6)?,
        })
    })
    .optional()
    .map_err(|e| e.into())
}

/// Update sync metadata
pub fn update_sync_metadata(
    conn: &Connection,
    account_id: &str,
    total_events: i32,
    is_full_sync: bool,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    if is_full_sync {
        conn.execute(
            "UPDATE calendar_sync_metadata
             SET total_events = ?1, last_full_sync_at = ?2, updated_at = ?3
             WHERE account_id = ?4",
            params![total_events, now, now, account_id],
        )?;
    } else {
        conn.execute(
            "UPDATE calendar_sync_metadata
             SET total_events = ?1, last_incremental_sync_at = ?2, updated_at = ?3
             WHERE account_id = ?4",
            params![total_events, now, now, account_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calendar_schema_creation() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate::apply_all(&mut conn).unwrap();

        // Verify tables exist
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'calendar_%'"
        ).unwrap();

        let tables: Vec<String> = stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"calendar_accounts".to_string()));
        assert!(tables.contains(&"calendar_events".to_string()));
        assert!(tables.contains(&"calendar_sync_metadata".to_string()));
    }
}
