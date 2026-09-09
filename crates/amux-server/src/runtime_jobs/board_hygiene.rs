//! Board hygiene: age-based sweeps for needsyou and stale cards.
//!
//! Two sweeps in one job, at different cadences:
//!
//! 1. NEEDSYOU AGING (daily). Cards in `needsyou` for 14+ days get a
//!    desc_append noting the age. Cards there for 30+ days are auto-discarded.
//!    The 445-card needsyou pile (CLAUDE.md) is what happens when nothing
//!    enforces a time bound on "waiting for a human".
//!
//! 2. STALE CARD SWEEP (every 6 hours). Autofix-created todo cards older than
//!    72h are discarded (autofix files freely and most are addressed or
//!    irrelevant within a day). Backlog cards older than 30 days that were
//!    never promoted get a desc_append flagging them for review. A per-session
//!    status summary is logged each pass.

use crate::api::AppState;
use crate::db::board_store as bs;

const JOB: &str = "board-hygiene";
const TICK_SECS: u64 = 6 * 3600; // 6 hours; the needsyou sweep skips if < 24h since last run

const NEEDSYOU_WARN_DAYS: i64 = 14;
const NEEDSYOU_DISCARD_DAYS: i64 = 30;
const AUTOFIX_STALE_HOURS: i64 = 72;
const BACKLOG_STALE_DAYS: i64 = 30;

static LAST_NEEDSYOU_RUN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

struct NeedsyouCard {
    id: String,
    title: String,
    age_days: i64,
}

struct StaleAutofixCard {
    id: String,
    title: String,
    age_hours: i64,
}

struct StaleBacklogCard {
    id: String,
    title: String,
    age_days: i64,
}

struct StatusCount {
    session: String,
    status: String,
    count: i64,
}

fn find_needsyou_cards(conn: &rusqlite::Connection, now_secs: i64) -> Vec<NeedsyouCard> {
    let cutoff = now_secs - (NEEDSYOU_WARN_DAYS * 86_400);
    let mut out = Vec::new();
    let Ok(mut st) = conn.prepare(
        "SELECT id, title, created FROM issues \
         WHERE status = 'needsyou' AND deleted IS NULL \
         AND COALESCE(archived, 0) = 0 AND created < ?1 \
         ORDER BY created ASC",
    ) else {
        return out;
    };
    if let Ok(rows) = st.query_map(rusqlite::params![cutoff], |r| {
        let created: i64 = r.get(2)?;
        Ok(NeedsyouCard {
            id: r.get(0)?,
            title: r.get(1)?,
            age_days: (now_secs - created) / 86_400,
        })
    }) {
        out.extend(rows.flatten());
    }
    out
}

fn find_stale_autofix_cards(conn: &rusqlite::Connection, now_secs: i64) -> Vec<StaleAutofixCard> {
    let cutoff = now_secs - (AUTOFIX_STALE_HOURS * 3600);
    let mut out = Vec::new();
    let Ok(mut st) = conn.prepare(
        "SELECT id, title, created FROM issues \
         WHERE status = 'todo' AND creator = 'autofix' AND deleted IS NULL \
         AND COALESCE(archived, 0) = 0 AND created < ?1 \
         ORDER BY created ASC",
    ) else {
        return out;
    };
    if let Ok(rows) = st.query_map(rusqlite::params![cutoff], |r| {
        let created: i64 = r.get(2)?;
        Ok(StaleAutofixCard {
            id: r.get(0)?,
            title: r.get(1)?,
            age_hours: (now_secs - created) / 3600,
        })
    }) {
        out.extend(rows.flatten());
    }
    out
}

fn find_stale_backlog_cards(conn: &rusqlite::Connection, now_secs: i64) -> Vec<StaleBacklogCard> {
    let cutoff = now_secs - (BACKLOG_STALE_DAYS * 86_400);
    let mut out = Vec::new();
    // Cards that have been in backlog for 30+ days and were never moved to
    // doing or done. We check the log column for evidence of a status transition;
    // if the log is empty or NULL, the card was never worked.
    let Ok(mut st) = conn.prepare(
        "SELECT id, title, created FROM issues \
         WHERE status = 'backlog' AND deleted IS NULL \
         AND COALESCE(archived, 0) = 0 AND created < ?1 \
         AND (log IS NULL OR (log NOT LIKE '%doing%' AND log NOT LIKE '%done%')) \
         ORDER BY created ASC",
    ) else {
        return out;
    };
    if let Ok(rows) = st.query_map(rusqlite::params![cutoff], |r| {
        let created: i64 = r.get(2)?;
        Ok(StaleBacklogCard {
            id: r.get(0)?,
            title: r.get(1)?,
            age_days: (now_secs - created) / 86_400,
        })
    }) {
        out.extend(rows.flatten());
    }
    out
}

fn count_by_session_status(conn: &rusqlite::Connection) -> Vec<StatusCount> {
    let mut out = Vec::new();
    let Ok(mut st) = conn.prepare(
        "SELECT COALESCE(session, '(unowned)'), status, COUNT(*) FROM issues \
         WHERE deleted IS NULL AND COALESCE(archived, 0) = 0 \
         GROUP BY session, status ORDER BY session, status",
    ) else {
        return out;
    };
    if let Ok(rows) = st.query_map([], |r| {
        Ok(StatusCount {
            session: r.get(0)?,
            status: r.get(1)?,
            count: r.get(2)?,
        })
    }) {
        out.extend(rows.flatten());
    }
    out
}

/// Run the needsyou aging sweep. Returns (warned, discarded).
async fn needsyou_sweep(state: &AppState, now_secs: i64) -> (usize, usize) {
    let last = LAST_NEEDSYOU_RUN.load(std::sync::atomic::Ordering::Relaxed);
    if last > 0 && (now_secs - last) < 23 * 3600 {
        return (0, 0);
    }
    LAST_NEEDSYOU_RUN.store(now_secs, std::sync::atomic::Ordering::Relaxed);

    let cards = {
        let Ok(conn) = state.store.read() else { return (0, 0) };
        find_needsyou_cards(&conn, now_secs)
    };

    let mut warned = 0usize;
    let mut discarded = 0usize;

    for card in &cards {
        if card.age_days >= NEEDSYOU_DISCARD_DAYS {
            let id = card.id.clone();
            let age = card.age_days;
            let note = format!(
                "Auto-discarded: {age} days in needsyou with no resolution"
            );
            let hhmm = chrono::Utc::now().format("%H:%M").to_string();
            let _ = state
                .store
                .write_async(move |conn| {
                    let old_desc: String = conn
                        .query_row(
                            "SELECT COALESCE(\"desc\", '') FROM issues WHERE id = ?1",
                            rusqlite::params![&id],
                            |r| r.get(0),
                        )
                        .unwrap_or_default();
                    let new_desc = if old_desc.trim().is_empty() {
                        note.clone()
                    } else {
                        format!("{}\n{note}", old_desc.trim_end())
                    };
                    let old_log: Option<String> = conn
                        .query_row(
                            "SELECT log FROM issues WHERE id = ?1",
                            rusqlite::params![&id],
                            |r| r.get(0),
                        )
                        .ok();
                    let new_log = bs::append_log(old_log.as_deref(), &hhmm, &note);
                    conn.execute(
                        "UPDATE issues SET status = 'discarded', \"desc\" = ?1, log = ?2, \
                         updated = ?3 WHERE id = ?4",
                        rusqlite::params![new_desc, new_log, now_secs, &id],
                    )?;
                    Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                })
                .await;
            tracing::info!(
                card_id = %card.id,
                age_days = card.age_days,
                title = %card.title,
                "board_hygiene: auto-discarded needsyou card"
            );
            discarded += 1;
        } else {
            let id = card.id.clone();
            let age = card.age_days;
            let note = format!(
                "Auto-aged: this card has been in needsyou for {age} days"
            );
            let hhmm = chrono::Utc::now().format("%H:%M").to_string();
            let _ = state
                .store
                .write_async(move |conn| {
                    let old_desc: String = conn
                        .query_row(
                            "SELECT COALESCE(\"desc\", '') FROM issues WHERE id = ?1",
                            rusqlite::params![&id],
                            |r| r.get(0),
                        )
                        .unwrap_or_default();
                    let new_desc = if old_desc.trim().is_empty() {
                        note.clone()
                    } else {
                        format!("{}\n{note}", old_desc.trim_end())
                    };
                    let old_log: Option<String> = conn
                        .query_row(
                            "SELECT log FROM issues WHERE id = ?1",
                            rusqlite::params![&id],
                            |r| r.get(0),
                        )
                        .ok();
                    let new_log = bs::append_log(old_log.as_deref(), &hhmm, &note);
                    conn.execute(
                        "UPDATE issues SET \"desc\" = ?1, log = ?2, updated = ?3 WHERE id = ?4",
                        rusqlite::params![new_desc, new_log, now_secs, &id],
                    )?;
                    Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                })
                .await;
            tracing::info!(
                card_id = %card.id,
                age_days = card.age_days,
                title = %card.title,
                "board_hygiene: warned needsyou card"
            );
            warned += 1;
        }
    }

    (warned, discarded)
}

/// Run the stale card sweep. Returns (autofix_discarded, backlog_flagged).
async fn stale_sweep(state: &AppState, now_secs: i64) -> (usize, usize) {
    let (autofix_cards, backlog_cards, status_counts) = {
        let Ok(conn) = state.store.read() else { return (0, 0) };
        (
            find_stale_autofix_cards(&conn, now_secs),
            find_stale_backlog_cards(&conn, now_secs),
            count_by_session_status(&conn),
        )
    };

    let mut autofix_discarded = 0usize;
    for card in &autofix_cards {
        let id = card.id.clone();
        let age = card.age_hours;
        let note = format!("Auto-discarded: autofix todo card stale for {age}h");
        let hhmm = chrono::Utc::now().format("%H:%M").to_string();
        let _ = state
            .store
            .write_async(move |conn| {
                let old_log: Option<String> = conn
                    .query_row(
                        "SELECT log FROM issues WHERE id = ?1",
                        rusqlite::params![&id],
                        |r| r.get(0),
                    )
                    .ok();
                let new_log = bs::append_log(old_log.as_deref(), &hhmm, &note);
                conn.execute(
                    "UPDATE issues SET status = 'discarded', log = ?1, updated = ?2 WHERE id = ?3",
                    rusqlite::params![new_log, now_secs, &id],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await;
        tracing::info!(
            card_id = %card.id,
            age_hours = card.age_hours,
            title = %card.title,
            "board_hygiene: discarded stale autofix card"
        );
        autofix_discarded += 1;
    }

    let mut backlog_flagged = 0usize;
    for card in &backlog_cards {
        let id = card.id.clone();
        let age = card.age_days;
        let note = format!(
            "Stale review: this card has been in backlog for {age} days and was never promoted"
        );
        let hhmm = chrono::Utc::now().format("%H:%M").to_string();
        let _ = state
            .store
            .write_async(move |conn| {
                let old_desc: String = conn
                    .query_row(
                        "SELECT COALESCE(\"desc\", '') FROM issues WHERE id = ?1",
                        rusqlite::params![&id],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                // Only append if we haven't already flagged this card
                if old_desc.contains("Stale review:") {
                    return Ok(crate::db::WriteOutcome { applied: false, events: vec![] });
                }
                let new_desc = if old_desc.trim().is_empty() {
                    note.clone()
                } else {
                    format!("{}\n{note}", old_desc.trim_end())
                };
                let old_log: Option<String> = conn
                    .query_row(
                        "SELECT log FROM issues WHERE id = ?1",
                        rusqlite::params![&id],
                        |r| r.get(0),
                    )
                    .ok();
                let new_log = bs::append_log(old_log.as_deref(), &hhmm, &note);
                conn.execute(
                    "UPDATE issues SET \"desc\" = ?1, log = ?2, updated = ?3 WHERE id = ?4",
                    rusqlite::params![new_desc, new_log, now_secs, &id],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await;
        tracing::info!(
            card_id = %card.id,
            age_days = card.age_days,
            title = %card.title,
            "board_hygiene: flagged stale backlog card"
        );
        backlog_flagged += 1;
    }

    // Log the per-session status summary
    if !status_counts.is_empty() {
        use std::collections::BTreeMap;
        let mut by_session: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
        for sc in &status_counts {
            by_session
                .entry(sc.session.clone())
                .or_default()
                .push((sc.status.clone(), sc.count));
        }
        let total_sessions = by_session.len();
        let total_cards: i64 = status_counts.iter().map(|s| s.count).sum();
        let summary: Vec<String> = by_session
            .iter()
            .map(|(s, counts)| {
                let parts: Vec<String> = counts.iter().map(|(st, c)| format!("{st}={c}")).collect();
                format!("{s}: {}", parts.join(" "))
            })
            .collect();
        tracing::info!(
            sessions = total_sessions,
            total_cards = total_cards,
            "board_hygiene: status summary: {}",
            summary.join("; ")
        );
    }

    (autofix_discarded, backlog_flagged)
}

pub async fn tick(state: AppState) {
    let now_secs = crate::config::now_f64() as i64;

    let (ny_warned, ny_discarded) = needsyou_sweep(&state, now_secs).await;
    let (af_discarded, bl_flagged) = stale_sweep(&state, now_secs).await;

    if ny_warned > 0 || ny_discarded > 0 || af_discarded > 0 || bl_flagged > 0 {
        tracing::info!(
            needsyou_warned = ny_warned,
            needsyou_discarded = ny_discarded,
            autofix_discarded = af_discarded,
            backlog_flagged = bl_flagged,
            "board_hygiene: sweep complete"
        );
    }
}

pub fn spawn(state: AppState) -> super::PeriodicTask {
    super::spawn_periodic(JOB, TICK_SECS, move || {
        let state = state.clone();
        async move {
            tick(state).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needsyou_constants_are_sane() {
        const { assert!(NEEDSYOU_WARN_DAYS < NEEDSYOU_DISCARD_DAYS) };
        const { assert!(NEEDSYOU_DISCARD_DAYS > 0) };
        const { assert!(AUTOFIX_STALE_HOURS > 0) };
        const { assert!(BACKLOG_STALE_DAYS > 0) };
    }

    #[test]
    fn find_needsyou_uses_correct_cutoff() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE issues (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, \"desc\" TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL, session TEXT, creator TEXT NOT NULL DEFAULT '',
                created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER,
                archived INTEGER NOT NULL DEFAULT 0, log TEXT,
                owner_type TEXT NOT NULL DEFAULT 'human', type TEXT NOT NULL DEFAULT 'code'
            )",
        )
        .unwrap();

        let now = 1_000_000i64;
        // Card at 15 days: should be found (>= 14 day cutoff)
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["NY-1", "old needsyou", "needsyou", now - 15 * 86_400, now],
        )
        .unwrap();
        // Card at 5 days: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["NY-2", "fresh needsyou", "needsyou", now - 5 * 86_400, now],
        )
        .unwrap();
        // Card at 35 days: should be found (and will be discarded)
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["NY-3", "ancient needsyou", "needsyou", now - 35 * 86_400, now],
        )
        .unwrap();
        // Deleted card at 20 days: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["NY-4", "deleted needsyou", "needsyou", now - 20 * 86_400, now, 1],
        )
        .unwrap();

        let cards = find_needsyou_cards(&conn, now);
        assert_eq!(cards.len(), 2, "expected 2 cards, got {}", cards.len());
        assert_eq!(cards[0].id, "NY-3"); // oldest first
        assert_eq!(cards[0].age_days, 35);
        assert_eq!(cards[1].id, "NY-1");
        assert_eq!(cards[1].age_days, 15);
    }

    #[test]
    fn find_stale_autofix_uses_correct_cutoff() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE issues (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, \"desc\" TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL, session TEXT, creator TEXT NOT NULL DEFAULT '',
                created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER,
                archived INTEGER NOT NULL DEFAULT 0, log TEXT,
                owner_type TEXT NOT NULL DEFAULT 'human', type TEXT NOT NULL DEFAULT 'code'
            )",
        )
        .unwrap();

        let now = 1_000_000i64;
        // Autofix card at 80h: should be found
        conn.execute(
            "INSERT INTO issues (id, title, status, creator, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["AF-1", "old autofix", "todo", "autofix", now - 80 * 3600, now],
        )
        .unwrap();
        // Autofix card at 24h: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, creator, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["AF-2", "fresh autofix", "todo", "autofix", now - 24 * 3600, now],
        )
        .unwrap();
        // Non-autofix card at 80h: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, creator, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["AF-3", "old human card", "todo", "human", now - 80 * 3600, now],
        )
        .unwrap();

        let cards = find_stale_autofix_cards(&conn, now);
        assert_eq!(cards.len(), 1, "expected 1 card, got {}", cards.len());
        assert_eq!(cards[0].id, "AF-1");
        assert_eq!(cards[0].age_hours, 80);
    }

    #[test]
    fn find_stale_backlog_excludes_promoted_cards() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE issues (
                id TEXT PRIMARY KEY, title TEXT NOT NULL, \"desc\" TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL, session TEXT, creator TEXT NOT NULL DEFAULT '',
                created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER,
                archived INTEGER NOT NULL DEFAULT 0, log TEXT,
                owner_type TEXT NOT NULL DEFAULT 'human', type TEXT NOT NULL DEFAULT 'code'
            )",
        )
        .unwrap();

        let now = 1_000_000i64;
        // Old backlog card, never promoted: should be found
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["BL-1", "stale backlog", "backlog", now - 35 * 86_400, now],
        )
        .unwrap();
        // Old backlog card with "doing" in log: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated, log) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["BL-2", "promoted backlog", "backlog", now - 35 * 86_400, now, "`12:00` moved to doing"],
        )
        .unwrap();
        // Young backlog card: should NOT be found
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["BL-3", "fresh backlog", "backlog", now - 5 * 86_400, now],
        )
        .unwrap();

        let cards = find_stale_backlog_cards(&conn, now);
        assert_eq!(cards.len(), 1, "expected 1 card, got {}", cards.len());
        assert_eq!(cards[0].id, "BL-1");
        assert_eq!(cards[0].age_days, 35);
    }
}
