//! Measured throughput telemetry and bounded adaptive WIP state.

use amux_core::orchestrator::{adaptive_wip, AdaptiveWipInputs};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

const METRIC_KINDS: &[&str] = &[
    "queue_wait",
    "build_wait",
    "lock_wait",
    "conflict",
    "recovery",
];
const METRIC_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct WipState {
    pub mode: String,
    pub current_limit: usize,
    pub recommended: usize,
    pub min_limit: usize,
    pub max_limit: usize,
    pub sample_size: u64,
    pub evidence_version: u64,
    pub reason: String,
    pub updated_at: String,
}

fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_string(),
    )))
}

fn purge_expired_metrics(conn: &Connection, now: DateTime<Utc>) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM _amux_work_metrics WHERE created_at<?1",
        [(now - Duration::days(METRIC_RETENTION_DAYS)).to_rfc3339()],
    )
}

pub fn record_metric(
    conn: &Connection,
    kind: &str,
    source: &str,
    duration_ms: Option<u64>,
    task_id: Option<&str>,
    detail: Option<&str>,
    now: DateTime<Utc>,
) -> rusqlite::Result<String> {
    if !METRIC_KINDS.contains(&kind) {
        return Err(invalid("unknown work metric kind"));
    }
    if kind != "conflict" && duration_ms.is_none() {
        return Err(invalid("duration_ms is required for non-conflict metrics"));
    }
    if source.trim().is_empty() {
        return Err(invalid("work metric source is required"));
    }
    purge_expired_metrics(conn, now)?;
    let id = format!("metric_{}", ulid::Ulid::new().to_string().to_lowercase());
    conn.execute(
        "INSERT INTO _amux_work_metrics(id,kind,source,duration_ms,task_id,detail,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            id,
            kind,
            source,
            duration_ms,
            task_id,
            detail,
            now.to_rfc3339()
        ],
    )?;
    Ok(id)
}

pub fn get_wip_state(conn: &Connection) -> rusqlite::Result<WipState> {
    conn.query_row(
        "SELECT mode,current_limit,recommended,min_limit,max_limit,sample_size,evidence_version,
                reason,updated_at
         FROM _amux_adaptive_wip WHERE singleton=1",
        [],
        |r| {
            Ok(WipState {
                mode: r.get(0)?,
                current_limit: r.get::<_, i64>(1)? as usize,
                recommended: r.get::<_, i64>(2)? as usize,
                min_limit: r.get::<_, i64>(3)? as usize,
                max_limit: r.get::<_, i64>(4)? as usize,
                sample_size: r.get::<_, i64>(5)? as u64,
                evidence_version: r.get::<_, i64>(6)? as u64,
                reason: r.get(7)?,
                updated_at: r.get(8)?,
            })
        },
    )
}

pub fn configure_wip(
    conn: &Connection,
    mode: &str,
    current: usize,
    min: usize,
    max: usize,
    now: DateTime<Utc>,
) -> rusqlite::Result<WipState> {
    if !matches!(mode, "shadow" | "active") || min == 0 || min > current || current > max {
        return Err(invalid(
            "mode must be shadow|active and limits must satisfy 1 <= min <= current <= max",
        ));
    }
    let evidence_version: u64 = conn.query_row(
        "SELECT COALESCE(MAX(rowid),0) FROM _amux_work_metrics
         WHERE source IN ('runtime','reconciliation')",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "UPDATE _amux_adaptive_wip
         SET mode=?1,current_limit=?2,recommended=?2,min_limit=?3,max_limit=?4,
             evidence_version=?5,updated_at=?6
         WHERE singleton=1",
        params![mode, current, min, max, evidence_version, now.to_rfc3339()],
    )?;
    get_wip_state(conn)
}

pub fn evaluate_wip(conn: &Connection, now: DateTime<Utc>) -> rusqlite::Result<WipState> {
    let state = get_wip_state(conn)?;
    let cutoff = (now - Duration::hours(1)).to_rfc3339();
    let sample_size: u64 = conn.query_row(
        "SELECT COUNT(*) FROM _amux_work_metrics
         WHERE kind='queue_wait' AND source='runtime' AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let evidence_version: u64 = conn.query_row(
        "SELECT COALESCE(MAX(rowid),0) FROM _amux_work_metrics
         WHERE source IN ('runtime','reconciliation') AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    if evidence_version <= state.evidence_version {
        return Ok(state);
    }
    let average_queue_wait_ms: Option<f64> = conn.query_row(
        "SELECT AVG(duration_ms) FROM _amux_work_metrics
         WHERE kind='queue_wait' AND source='runtime' AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let average_lock_wait_ms: Option<f64> = conn.query_row(
        "SELECT AVG(duration_ms) FROM _amux_work_metrics
         WHERE kind='lock_wait' AND source='runtime' AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let conflicts: u64 = conn.query_row(
        "SELECT COUNT(*) FROM _amux_work_metrics
         WHERE kind='conflict' AND source='runtime' AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let queue_depth: u64 = conn.query_row(
        "SELECT COUNT(*) FROM issues
         WHERE deleted IS NULL AND archived=0 AND LOWER(status) IN ('todo','backlog')",
        [],
        |r| r.get(0),
    )?;
    let active_workers: u64 =
        conn.query_row("SELECT COUNT(*) FROM _amux_workers", [], |r| r.get(0))?;
    let (passed, reworked): (u64, u64) = conn.query_row(
        "SELECT
           COUNT(DISTINCT CASE WHEN verdict='passed' THEN task_id END),
           COUNT(DISTINCT CASE WHEN verdict='passed' AND EXISTS(
             SELECT 1 FROM _amux_verifications f
             WHERE f.task_id=_amux_verifications.task_id AND f.verdict='failed'
               AND f.created_at<=_amux_verifications.created_at) THEN task_id END)
         FROM _amux_verifications WHERE created_at>=?1",
        [now.timestamp() - 3600],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let input = AdaptiveWipInputs {
        current: state.current_limit,
        min: state.min_limit,
        max: state.max_limit,
        sample_size,
        queue_depth,
        active_workers,
        average_queue_wait_ms,
        average_lock_wait_ms,
        conflict_rate: (sample_size + conflicts > 0)
            .then(|| conflicts as f64 / (sample_size + conflicts) as f64),
        rework_rate: (passed > 0).then(|| reworked as f64 / passed as f64),
    };
    let decision = adaptive_wip(&input);
    let current = if state.mode == "active" {
        decision.recommended
    } else {
        state.current_limit
    };
    conn.execute(
        "UPDATE _amux_adaptive_wip
         SET current_limit=?1,recommended=?2,sample_size=?3,evidence_version=?4,
             reason=?5,updated_at=?6
         WHERE singleton=1",
        params![
            current,
            decision.recommended,
            sample_size,
            evidence_version,
            decision.reason,
            now.to_rfc3339()
        ],
    )?;
    get_wip_state(conn)
}

/// Record each genuinely queued runtime task once. Re-evaluating the same
/// queue on every tick would turn elapsed time into fabricated sample volume;
/// the partial unique index makes the observation durable and idempotent.
pub fn observe_runtime_queue(
    conn: &Connection,
    tasks: &[amux_core::board::Task],
    now: DateTime<Utc>,
) -> rusqlite::Result<usize> {
    purge_expired_metrics(conn, now)?;
    let mut inserted = 0;
    for task in tasks
        .iter()
        .filter(|task| task.status == amux_core::board::TaskStatus::Todo)
    {
        let duration_ms = now
            .signed_duration_since(task.created_at)
            .num_milliseconds()
            .max(0) as u64;
        let id = format!("queue:{}", task.id);
        inserted += conn.execute(
            "INSERT OR IGNORE INTO _amux_work_metrics
             (id,kind,source,duration_ms,task_id,detail,created_at)
             VALUES(?1,'queue_wait','runtime',?2,?3,'orchestrator queue observation',?4)",
            params![id, duration_ms, task.id.as_str(), now.to_rfc3339()],
        )?;
    }
    Ok(inserted)
}

pub fn effective_wip_limit(conn: &Connection) -> rusqlite::Result<usize> {
    Ok(get_wip_state(conn)?.current_limit.max(1))
}

fn percentile(mut values: Vec<u64>, pct: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * pct).ceil() as usize;
    values.get(index).copied()
}

fn durations(conn: &Connection, kind: &str, cutoff: &str) -> rusqlite::Result<Vec<u64>> {
    let mut stmt = conn.prepare(
        "SELECT duration_ms FROM _amux_work_metrics
         WHERE kind=?1 AND created_at>=?2 AND duration_ms IS NOT NULL",
    )?;
    let values = stmt
        .query_map(params![kind, cutoff], |r| r.get(0))?
        .collect();
    values
}

fn distribution(values: Vec<u64>) -> Value {
    json!({
        "measured": true,
        "n_considered": values.len(),
        "p50_ms": percentile(values.clone(), 0.50),
        "p95_ms": percentile(values, 0.95)
    })
}

pub fn health_snapshot(
    conn: &Connection,
    since_unix: i64,
    window_hours: f64,
) -> rusqlite::Result<Value> {
    let verified: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT task_id) FROM _amux_verifications
         WHERE verdict='passed' AND created_at>=?1",
        [since_unix],
        |r| r.get(0),
    )?;
    let mut lead_stmt = conn.prepare(
        "SELECT MAX(0,MIN(v.created_at)-i.created)
         FROM _amux_verifications v JOIN issues i ON i.id=v.task_id
         WHERE v.verdict='passed' AND v.created_at>=?1 GROUP BY v.task_id",
    )?;
    let lead_seconds: Vec<u64> = lead_stmt
        .query_map([since_unix], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let cutoff = DateTime::<Utc>::from_timestamp(since_unix, 0)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339();
    let conflicts: u64 = conn.query_row(
        "SELECT COUNT(*) FROM _amux_work_metrics
         WHERE kind='conflict' AND source='runtime' AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let observations: u64 = conn.query_row(
        "SELECT COUNT(*) FROM _amux_work_metrics
         WHERE source='runtime' AND kind IN ('queue_wait','conflict') AND created_at>=?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let wip = get_wip_state(conn)?;
    Ok(json!({
        "measured": true,
        "n_considered": verified + observations,
        "verified_tasks_per_hour": if window_hours > 0.0 {
            Some(verified as f64 / window_hours)
        } else { None },
        "lead_time": {
            "measured": true,
            "n_considered": lead_seconds.len(),
            "p50_ms": percentile(lead_seconds.clone(), 0.50).map(|s| s.saturating_mul(1000)),
            "p95_ms": percentile(lead_seconds, 0.95).map(|s| s.saturating_mul(1000))
        },
        "waits": {
            "queue": distribution(durations(conn, "queue_wait", &cutoff)?),
            "build": distribution(durations(conn, "build_wait", &cutoff)?),
            "lock": distribution(durations(conn, "lock_wait", &cutoff)?),
            "recovery": distribution(durations(conn, "recovery", &cutoff)?)
        },
        "conflict_rate": {
            "numerator": conflicts,
            "denominator": observations,
            "value": (observations > 0).then(|| conflicts as f64 / observations as f64)
        },
        "adaptive_wip": wip
    }))
}
