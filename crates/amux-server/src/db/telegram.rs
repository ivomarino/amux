//! Telegram chat <-> session mapping store, over `telegram_mappings` +
//! `telegram_poll_state` (migration 0035).
//!
//! Deliberately NOT routed through the state-event journal (`PendingEvent`)
//! that `board`/`memories` use: a mapping is operator config, not a fact the
//! rest of the fleet needs to react to via SSE, and the audit weight those
//! modules carry (revisions, dedupe keys, delta sync) buys nothing here. If
//! that changes — e.g. the dashboard wants live mapping updates — promote it
//! then, don't pre-build it now (ethos rule 2).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TelegramMapping {
    pub chat_id: i64,
    pub session: String,
    pub telegram_username: Option<String>,
    pub linked_at: String,
    pub last_message_at: Option<String>,
}

fn row_to_mapping(r: &rusqlite::Row<'_>) -> rusqlite::Result<TelegramMapping> {
    Ok(TelegramMapping {
        chat_id: r.get(0)?,
        session: r.get(1)?,
        telegram_username: r.get(2)?,
        linked_at: r.get(3)?,
        last_message_at: r.get(4)?,
    })
}

const COLS: &str = "chat_id, session, telegram_username, linked_at, last_message_at";

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<TelegramMapping>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {COLS} FROM telegram_mappings ORDER BY linked_at DESC"))?;
    let rows = stmt.query_map([], row_to_mapping)?;
    rows.collect()
}

pub fn by_chat(conn: &Connection, chat_id: i64) -> rusqlite::Result<Option<TelegramMapping>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM telegram_mappings WHERE chat_id = ?1"),
        params![chat_id],
        row_to_mapping,
    )
    .optional()
}

/// The most-recently-linked chat for a session. A session can only be the
/// TARGET of one active outbound mapping at a time in this MVP — if two
/// chats link the same session, the newer link wins for outbound sends
/// (inbound routing is unaffected: both chats still deliver in).
pub fn by_session(conn: &Connection, session: &str) -> rusqlite::Result<Option<TelegramMapping>> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM telegram_mappings WHERE session = ?1 ORDER BY linked_at DESC LIMIT 1"
        ),
        params![session],
        row_to_mapping,
    )
    .optional()
}

pub fn upsert(
    conn: &Connection,
    chat_id: i64,
    session: &str,
    telegram_username: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO telegram_mappings (chat_id, session, telegram_username, linked_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(chat_id) DO UPDATE SET
           session = excluded.session,
           telegram_username = COALESCE(excluded.telegram_username, telegram_mappings.telegram_username),
           linked_at = datetime('now')",
        params![chat_id, session, telegram_username],
    )?;
    Ok(())
}

pub fn touch_last_message(conn: &Connection, chat_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE telegram_mappings SET last_message_at = datetime('now') WHERE chat_id = ?1",
        params![chat_id],
    )?;
    Ok(())
}

pub fn remove(conn: &Connection, chat_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM telegram_mappings WHERE chat_id = ?1", params![chat_id])?;
    Ok(n > 0)
}

pub fn last_update_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT last_update_id FROM telegram_poll_state WHERE id = 1", [], |r| r.get(0))
}

pub fn set_last_update_id(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE telegram_poll_state SET last_update_id = ?1, updated_at = datetime('now') WHERE id = 1",
        params![id],
    )?;
    Ok(())
}
