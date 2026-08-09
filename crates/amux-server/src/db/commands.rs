//! Durable command queue persistence (Invariant 34, Phase 4 seed).
//!
//! Rows mirror `amux_core::protocol::QueuedCommand`; the state machine is
//! the core's `next_state` — this module only persists what the pure logic
//! decides. FIFO per worker; idempotency-key dedup on enqueue (Invariant 9).

use amux_core::ids::{CommandId, WorkerId};
use amux_core::protocol::{
    CommandPrecondition, CommandState, CommandTransition, DeliveryTiming, QueuedCommand,
    WorkerCommand,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

fn corrupt(e: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

fn row_to_command(r: &rusqlite::Row) -> rusqlite::Result<QueuedCommand> {
    let id: String = r.get(0)?;
    let worker: String = r.get(1)?;
    let command: String = r.get(2)?;
    let state: String = r.get(3)?;
    let idempotency_key: String = r.get(4)?;
    let queued_at: String = r.get(5)?;
    let attempts: u32 = r.get(6)?;
    let timing: String = r.get(7)?;
    let precondition: Option<String> = r.get(8)?;
    Ok(QueuedCommand {
        id: CommandId::parse(&id).map_err(corrupt)?,
        worker: WorkerId::parse(&worker).map_err(corrupt)?,
        command: serde_json::from_str(&command).map_err(corrupt)?,
        state: serde_json::from_str(&state).map_err(corrupt)?,
        idempotency_key,
        queued_at: queued_at
            .parse::<DateTime<Utc>>()
            .map_err(corrupt)?,
        attempts,
        timing: serde_json::from_str(&timing).map_err(corrupt)?,
        precondition: precondition
            .map(|p| serde_json::from_str::<CommandPrecondition>(&p))
            .transpose()
            .map_err(corrupt)?,
    })
}

const COLS: &str = "id, worker_id, command, state, idempotency_key, queued_at, attempts, timing, precondition";

/// Enqueue, deduplicating on (worker, idempotency_key): a duplicate returns
/// the EXISTING command with `newly_created = false`, never a second row.
pub fn enqueue(
    conn: &Connection,
    id: CommandId,
    worker: &WorkerId,
    command: &WorkerCommand,
    idempotency_key: &str,
    timing: &DeliveryTiming,
    precondition: Option<&CommandPrecondition>,
    now: DateTime<Utc>,
) -> rusqlite::Result<(QueuedCommand, bool)> {
    if let Some(existing) = by_idempotency_key(conn, worker, idempotency_key)? {
        return Ok((existing, false));
    }
    conn.execute(
        &format!("INSERT INTO _amux_commands ({COLS}) VALUES (?1,?2,?3,?4,?5,?6,0,?7,?8)"),
        params![
            id.as_str(),
            worker.as_str(),
            serde_json::to_string(command).map_err(corrupt)?,
            serde_json::to_string(&CommandState::Queued).map_err(corrupt)?,
            idempotency_key,
            now.to_rfc3339(),
            serde_json::to_string(timing).map_err(corrupt)?,
            precondition
                .map(serde_json::to_string)
                .transpose()
                .map_err(corrupt)?,
        ],
    )?;
    let cmd = by_id(conn, &id)?.expect("row just inserted");
    Ok((cmd, true))
}

pub fn by_id(conn: &Connection, id: &CommandId) -> rusqlite::Result<Option<QueuedCommand>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM _amux_commands WHERE id = ?1"),
        params![id.as_str()],
        row_to_command,
    )
    .optional()
}

pub fn by_idempotency_key(
    conn: &Connection,
    worker: &WorkerId,
    key: &str,
) -> rusqlite::Result<Option<QueuedCommand>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM _amux_commands WHERE worker_id = ?1 AND idempotency_key = ?2"),
        params![worker.as_str(), key],
        row_to_command,
    )
    .optional()
}

/// Oldest Queued command for a worker (FIFO). In-flight (Dispatched or
/// Delivered) commands block the queue head: one command in flight per
/// worker, ordering preserved (Invariant 34).
pub fn next_deliverable(
    conn: &Connection,
    worker: &WorkerId,
) -> rusqlite::Result<Option<QueuedCommand>> {
    let queued_state = serde_json::to_string(&CommandState::Queued).map_err(corrupt)?;
    let in_flight: u32 = conn.query_row(
        "SELECT COUNT(*) FROM _amux_commands WHERE worker_id = ?1
         AND (state LIKE '%dispatched%' OR state LIKE '%delivered%')",
        params![worker.as_str()],
        |r| r.get(0),
    )?;
    if in_flight > 0 {
        return Ok(None);
    }
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM _amux_commands WHERE worker_id = ?1 AND state = ?2
             ORDER BY queued_at ASC, id ASC LIMIT 1"
        ),
        params![worker.as_str(), queued_state],
        row_to_command,
    )
    .optional()
}

/// Apply a transition via the core state machine and persist the result.
pub fn transition(
    conn: &Connection,
    id: &CommandId,
    t: CommandTransition,
    max_attempts: u32,
) -> rusqlite::Result<QueuedCommand> {
    let mut cmd = by_id(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    cmd.apply(t, max_attempts).map_err(corrupt)?;
    conn.execute(
        "UPDATE _amux_commands SET state = ?1, attempts = ?2 WHERE id = ?3",
        params![
            serde_json::to_string(&cmd.state).map_err(corrupt)?,
            cmd.attempts,
            id.as_str()
        ],
    )?;
    Ok(cmd)
}

/// Dead-lettered commands, for the observability surface (Invariant 34's
/// dead-letter requirement: a command that gave up must be FINDABLE).
pub fn dead_letters(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<QueuedCommand>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM _amux_commands WHERE state LIKE '%dead_lettered%'
         ORDER BY queued_at DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map(params![limit], row_to_command)?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::migrate::apply_all(&mut c).unwrap();
        c
    }

    fn wid(n: u128) -> WorkerId {
        WorkerId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, n))
    }

    fn cid(n: u128) -> CommandId {
        CommandId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, 500 + n))
    }

    fn t(secs: i64) -> DateTime<Utc> {
        chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::seconds(secs)
    }

    #[test]
    fn enqueue_dedups_on_idempotency_key() {
        let c = conn();
        let (first, created1) = enqueue(
            &c, cid(1), &wid(1), &WorkerCommand::Continue, "k1",
            &DeliveryTiming::AtTurnBoundary, None, t(0),
        )
        .unwrap();
        assert!(created1);
        let (dup, created2) = enqueue(
            &c, cid(2), &wid(1), &WorkerCommand::Continue, "k1",
            &DeliveryTiming::AtTurnBoundary, None, t(1),
        )
        .unwrap();
        assert!(!created2);
        assert_eq!(dup.id, first.id, "duplicate returns the ORIGINAL");
        // Same key on a DIFFERENT worker is a different command.
        let (_, created3) = enqueue(
            &c, cid(3), &wid(2), &WorkerCommand::Continue, "k1",
            &DeliveryTiming::AtTurnBoundary, None, t(2),
        )
        .unwrap();
        assert!(created3);
    }

    #[test]
    fn fifo_and_single_in_flight() {
        let c = conn();
        enqueue(&c, cid(1), &wid(1), &WorkerCommand::Continue, "a",
                &DeliveryTiming::AtTurnBoundary, None, t(0)).unwrap();
        enqueue(&c, cid(2), &wid(1), &WorkerCommand::Cancel, "b",
                &DeliveryTiming::AtTurnBoundary, None, t(1)).unwrap();

        let head = next_deliverable(&c, &wid(1)).unwrap().unwrap();
        assert_eq!(head.id, cid(1), "oldest first");

        // Dispatch the head: queue blocks until it resolves.
        transition(&c, &cid(1), CommandTransition::Dispatch, 3).unwrap();
        assert!(next_deliverable(&c, &wid(1)).unwrap().is_none(), "in-flight blocks");

        transition(&c, &cid(1), CommandTransition::Deliver, 3).unwrap();
        transition(&c, &cid(1), CommandTransition::Confirm, 3).unwrap();
        let next = next_deliverable(&c, &wid(1)).unwrap().unwrap();
        assert_eq!(next.id, cid(2), "confirmed head unblocks the next");
    }

    #[test]
    fn failure_retry_and_dead_letter_persist() {
        let c = conn();
        enqueue(&c, cid(1), &wid(1), &WorkerCommand::Continue, "a",
                &DeliveryTiming::Immediate, None, t(0)).unwrap();
        for i in 0..3 {
            transition(&c, &cid(1), CommandTransition::Dispatch, 3).unwrap();
            let failed = transition(
                &c, &cid(1),
                CommandTransition::Fail { reason: format!("boom {i}") }, 3,
            ).unwrap();
            assert_eq!(failed.attempts, i + 1);
            transition(&c, &cid(1), CommandTransition::Retry, 3).unwrap();
        }
        // Third retry dead-lettered it (attempts == max).
        let dead = dead_letters(&c, 10).unwrap();
        assert_eq!(dead.len(), 1);
        assert!(matches!(&dead[0].state, CommandState::DeadLettered { reason } if reason.contains("boom 2")));
        assert!(next_deliverable(&c, &wid(1)).unwrap().is_none());
    }
}
