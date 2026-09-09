//! Redacted, bounded per-turn transcript artifacts.

use amux_core::ids::WorkerId;
use amux_core::protocol::{TurnTrace, TurnTraceKind};
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const MAX_TRACE_BYTES: usize = 64 * 1024;
const DEFAULT_RETAIN_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize)]
pub struct TraceRow {
    pub id: String,
    pub worker_id: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: String,
    pub kind: String,
    pub content: String,
    pub content_sha256: String,
    pub truncated: bool,
    pub redactions: u32,
    pub created_at: String,
}

fn secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"(?i)(authorization\s*:\s*bearer\s+)[^\s,;]+")
                .expect("valid bearer regex"),
            Regex::new(
                r#"(?i)((?:api[_-]?key|access[_-]?token|refresh[_-]?token|secret|password)\s*[:=]\s*)(\"[^\"]*\"|'[^']*'|[^\s,;}]+)"#,
            )
            .expect("valid named-secret regex"),
        ]
    })
}

fn json_secret_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)(\"(?:authorization|api[_-]?key|access[_-]?token|refresh[_-]?token|secret|password)\"\s*:\s*\")[^\"]*(\")"#,
        )
        .expect("valid JSON secret regex")
    })
}

pub(crate) fn redact_and_bound(raw: &str) -> (String, bool, u32) {
    let mut content = raw.replace('\0', "�");
    let mut redactions = 0u32;
    let json_pattern = json_secret_pattern();
    let json_count = json_pattern.find_iter(&content).count() as u32;
    if json_count > 0 {
        content = json_pattern
            .replace_all(&content, "${1}[REDACTED]${2}")
            .into_owned();
        redactions = redactions.saturating_add(json_count);
    }
    for pattern in secret_patterns() {
        let count = pattern.find_iter(&content).count() as u32;
        if count > 0 {
            content = pattern.replace_all(&content, "${1}[REDACTED]").into_owned();
            redactions = redactions.saturating_add(count);
        }
    }
    let truncated = content.len() > MAX_TRACE_BYTES;
    if truncated {
        let mut end = MAX_TRACE_BYTES;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
        content.push_str("\n[TRUNCATED]");
    }
    (content, truncated, redactions)
}

fn kind_token(kind: TurnTraceKind) -> &'static str {
    match kind {
        TurnTraceKind::Prompt => "prompt",
        TurnTraceKind::ProviderEvent => "provider_event",
        TurnTraceKind::CommandOutput => "command_output",
        TurnTraceKind::SystemAction => "system_action",
    }
}

fn retention_days() -> i64 {
    std::env::var("AMUX_TURN_TRACE_RETAIN_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|days| *days >= 1)
        .unwrap_or(DEFAULT_RETAIN_DAYS)
}

pub fn insert(
    conn: &Connection,
    worker: &WorkerId,
    trace: &TurnTrace,
    now: DateTime<Utc>,
) -> rusqlite::Result<TraceRow> {
    let correlation: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT task_id,session_id FROM _amux_turns WHERE id=?1",
            [trace.turn_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let (task_id, session_id) = correlation.unwrap_or_else(|| {
        let task = crate::db::commands::in_flight(conn, worker)
            .ok()
            .flatten()
            .and_then(|command| match command.command {
                amux_core::protocol::WorkerCommand::ExecuteTask(task) => Some(task.to_string()),
                _ => None,
            });
        let session = crate::db::queries::live_session_for(conn, worker.as_str())
            .ok()
            .flatten()
            .map(|session| session.id);
        (task, session)
    });
    let (content, truncated, redactions) = redact_and_bound(&trace.content);
    let sha = hex::encode(Sha256::digest(content.as_bytes()));
    let row = TraceRow {
        id: format!("trace_{}", ulid::Ulid::new().to_string().to_lowercase()),
        worker_id: worker.to_string(),
        task_id,
        session_id,
        turn_id: trace.turn_id.to_string(),
        kind: kind_token(trace.kind).to_string(),
        content,
        content_sha256: sha,
        truncated,
        redactions,
        created_at: now.to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO _amux_turn_traces
         (id,worker_id,task_id,session_id,turn_id,kind,content,content_sha256,truncated,
          redactions,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            row.id,
            row.worker_id,
            row.task_id,
            row.session_id,
            row.turn_id,
            row.kind,
            row.content,
            row.content_sha256,
            row.truncated,
            row.redactions,
            row.created_at,
        ],
    )?;
    let cutoff = (now - Duration::days(retention_days())).to_rfc3339();
    let removed = conn.execute(
        "DELETE FROM _amux_turn_traces WHERE created_at<?1",
        [cutoff],
    )?;
    if removed > 0 {
        tracing::info!(removed, "expired turn traces removed by retention sweep");
    }
    Ok(row)
}

pub fn list_for_turn(conn: &Connection, turn_id: &str) -> rusqlite::Result<Vec<TraceRow>> {
    let mut stmt = conn.prepare(
        "SELECT id,worker_id,task_id,session_id,turn_id,kind,content,content_sha256,truncated,
                redactions,created_at
         FROM _amux_turn_traces WHERE turn_id=?1 ORDER BY created_at,id",
    )?;
    let rows = stmt
        .query_map([turn_id], |r| {
            Ok(TraceRow {
                id: r.get(0)?,
                worker_id: r.get(1)?,
                task_id: r.get(2)?,
                session_id: r.get(3)?,
                turn_id: r.get(4)?,
                kind: r.get(5)?,
                content: r.get(6)?,
                content_sha256: r.get(7)?,
                truncated: r.get::<_, i64>(8)? != 0,
                redactions: r.get::<_, i64>(9)? as u32,
                created_at: r.get(10)?,
            })
        })?
        .collect();
    rows
}
