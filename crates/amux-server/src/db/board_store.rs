//! Board store: the Python `issues` table <-> core [`Task`] interop shim
//! (Phase 2, RR-0049/RR-0053/RR-0055; strangler-fig rule from the plan).
//!
//! THE RUST API READS AND WRITES THE SAME ROWS THE PYTHON SERVER SERVES.
//! Phase 11's rollback requirement is that the Python server keeps working
//! against this DB at any moment, so every mapping here preserves the Python
//! vocabulary byte-for-byte:
//!
//! - `issues.id` is the wire id ("AMUX-123"). Core [`TaskId`]s are derived
//!   deterministically from it ([`internal_id`]) and never persisted.
//! - `status` is stored in the Python spelling (`needsyou`, not `needs_you`).
//!   Reads accept both; writes preserve whatever spelling the row already
//!   used ([`status_to_db`]) so a Rust write never rewrites Python's
//!   vocabulary in shared rows.
//! - `created`/`updated` are unix INTEGER seconds (0001_baseline: `created
//!   INTEGER NOT NULL`), never RFC3339 strings.
//! - `log` is the append-only history; lines are `` `HH:MM` <text> ``
//!   exactly as `_append_board_log` writes them ([`append_log`]).
//! - id minting replicates `_next_issue_id` / `_prefix_from_session` over
//!   the shared `issue_counters` table, so the two servers can never mint
//!   colliding ids.
//! - `deleted IS NULL` (soft delete) is filtered in EVERY query, and
//!   [`save_patched`] deliberately never touches Python-owned columns it
//!   does not model (`creator`, `created`, `notified`, `gcal_event_id`,
//!   `deleted`).

use amux_core::board::{self, Gate, GateCriterion, ItemType, Task, TaskStatus};
use amux_core::events::Actor;
use amux_core::ids::{GateId, TaskId};
use amux_core::verification::VerifierKind;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Ids: semantic ("AMUX-123") on the wire and in the DB, TaskId internally
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit. Stable, dependency-free; collisions across a board-sized
/// id space are negligible and would only affect in-memory graph checks.
fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// INTEROP SHIM (Phase 11 migrates ids): derive the internal core [`TaskId`]
/// deterministically from the semantic `issues.id`. The semantic id is the
/// only identity that exists in the shared DB and on the wire; the core state
/// machine and graph helpers want `TaskId`s, so we mint one per semantic id
/// via a fixed hash (timestamp part 0 so the mapping is pure). Never persist
/// these — the row keeps the semantic id, and API payloads always use it.
pub fn internal_id(semantic: &str) -> TaskId {
    TaskId::from_ulid(ulid::Ulid::from_parts(0, u128::from(fnv64(semantic))))
}

/// Deterministic [`GateId`] for the synthesized ack-gate on a target status —
/// same shim as [`internal_id`]: stable so `why_blocked` output is
/// reproducible (ethos rule 4), never persisted.
fn gate_id_for(target: TaskStatus) -> GateId {
    GateId::from_ulid(ulid::Ulid::from_parts(
        0,
        u128::from(fnv64(&format!("board-gate:{}", db_status_spelling(target)))),
    ))
}

// ---------------------------------------------------------------------------
// Status vocabulary: Python spellings <-> TaskStatus
// ---------------------------------------------------------------------------

/// Parse a stored/requested status into core vocabulary. Accepts BOTH
/// `needsyou` (the Python DB spelling — see amux-server.py's
/// `('needsyou','review','blocked','backlog')` queries) and core's
/// serde spelling `needs_you`, plus the Python `_STATUS_ALIASES` synonyms,
/// so a row written by either server parses on both sides.
pub fn parse_status(raw: &str) -> Option<TaskStatus> {
    match raw.trim().to_lowercase().as_str() {
        "backlog" => Some(TaskStatus::Backlog),
        "todo" => Some(TaskStatus::Todo),
        "doing" | "wip" | "in_progress" | "inprogress" => Some(TaskStatus::Doing),
        "review" | "in_review" | "inreview" | "in review" => Some(TaskStatus::Review),
        "needsyou" | "needs_you" => Some(TaskStatus::NeedsYou),
        "blocked" => Some(TaskStatus::Blocked),
        "done" | "resolved" | "complete" | "completed" | "closed" => Some(TaskStatus::Done),
        "verified" => Some(TaskStatus::Verified),
        "discarded" => Some(TaskStatus::Discarded),
        "armed" => Some(TaskStatus::Armed),
        "quarantined" => Some(TaskStatus::Quarantined),
        _ => None,
    }
}

/// The Python DB spelling for each status (what a FRESH write uses). Note
/// `needsyou` — the live board's own spelling, NOT core's `needs_you`.
pub fn db_status_spelling(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Backlog => "backlog",
        TaskStatus::Todo => "todo",
        TaskStatus::Doing => "doing",
        TaskStatus::Review => "review",
        TaskStatus::NeedsYou => "needsyou",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
        TaskStatus::Verified => "verified",
        TaskStatus::Discarded => "discarded",
        TaskStatus::Armed => "armed",
        TaskStatus::Quarantined => "quarantined",
    }
}

/// The string to WRITE for `target`, given the raw spelling currently in the
/// row: if the row already spells this status (e.g. a legacy `needs_you`
/// written by hand), keep that exact spelling — do not rewrite Python's
/// vocabulary in shared rows. Otherwise use the Python default spelling.
pub fn status_to_db(target: TaskStatus, prior_raw: &str) -> String {
    if parse_status(prior_raw) == Some(target) {
        prior_raw.to_string()
    } else {
        db_status_spelling(target).to_string()
    }
}

// ---------------------------------------------------------------------------
// Item types
// ---------------------------------------------------------------------------

/// The Python `_ITEM_TYPES` tuple, verbatim (order preserved for the
/// `valid_types` field the CLI prints).
pub const KNOWN_TYPES: [&str; 10] = [
    "code",
    "escalation",
    "blocker",
    "investigation",
    "ops",
    "research",
    "chore",
    "doc",
    "tripwire",
    "watch",
];

/// Core [`ItemType`] for GATE purposes. Unknown/legacy strings map to `Code`
/// — the strictest gate — matching Python's `_item_type_gate` fallthrough
/// ("never silently weaken a gate"). The raw string itself stays on the row
/// and in API payloads; this mapping is only ever used to derive gates and
/// drive the state machine.
pub fn core_item_type(raw: &str) -> ItemType {
    match raw.trim().to_lowercase().as_str() {
        "escalation" => ItemType::Escalation,
        "blocker" => ItemType::Blocker,
        "investigation" => ItemType::Investigation,
        "ops" => ItemType::Ops,
        "research" => ItemType::Research,
        "chore" => ItemType::Chore,
        "doc" => ItemType::Doc,
        "tripwire" => ItemType::Tripwire,
        "watch" => ItemType::Watch,
        _ => ItemType::Code,
    }
}

// ---------------------------------------------------------------------------
// Gates: the Python type-derived tables, ported verbatim
// ---------------------------------------------------------------------------

/// Default gate criteria for (item type, target status) — the Python tables
/// ported EXACTLY (amux-server.py `_TYPE_GATES` + the `statuses.gate`
/// bootstrap seeds that `code` falls through to). Strings must stay
/// byte-identical: `gate_checked` acks are matched by exact string against
/// these on BOTH servers, so a drifted criterion here would make an ack
/// minted against one server unusable against the other.
///
/// Scoped (worker/group) gate tiers land with RR-0051; until then this is
/// card-override -> type table -> global status defaults, the same first,
/// second, and last rungs of Python's `_GATE_PRECEDENCE`.
pub fn default_gates_for(item_type_raw: &str, target: TaskStatus) -> Vec<String> {
    let ty = core_item_type(item_type_raw);
    let list: &[&str] = match (ty, target) {
        // Dormant types (tripwire/watch): honest gates for what they ARE.
        (ItemType::Tripwire | ItemType::Watch, TaskStatus::Doing) => &[
            "Trigger condition documented on the card",
            "Armed and monitoring",
        ],
        (ItemType::Tripwire | ItemType::Watch, TaskStatus::Review) => {
            &["Fired: evidence of the triggering event recorded"]
        }
        (ItemType::Tripwire | ItemType::Watch, TaskStatus::Done) => {
            &["Fired and handled, or deliberately stood down (which, and why, on the card)"]
        }
        (ItemType::Tripwire | ItemType::Watch, TaskStatus::Verified) => {
            &["Outcome confirmed (handled recurrence, or stand-down still correct)"]
        }
        // Code (and unknown-typed legacy rows): the global status defaults.
        (ItemType::Code, TaskStatus::Doing) => &[
            "Scope & acceptance criteria are clear",
            "No blocking dependency",
            "Has an owner",
        ],
        (ItemType::Code, TaskStatus::Review) => &[
            "Implemented and self-tested",
            "Diff / PR is up",
            "Ready for another set of eyes",
        ],
        (ItemType::Code, TaskStatus::Done) => &["Implemented and merged", "Tests / lint pass"],
        (ItemType::Code, TaskStatus::Verified) => &[
            "CI/CD green (if e2e infra is unavailable, note why — that is not a failure)",
            "Deployed to prod",
            "Confirmed working in prod",
            "Zero regressions",
        ],
        // Every other (non-code, non-dormant) type: the honest non-code bar.
        (_, TaskStatus::Doing) => &["Scope is clear", "Has an owner"],
        (_, TaskStatus::Review) => &["Findings written up", "Ready for another set of eyes"],
        (_, TaskStatus::Done) => {
            &["Outcome recorded in the item (what happened, and why it is closed)"]
        }
        (_, TaskStatus::Verified) => &["Outcome confirmed to still hold"],
        // No status outside doing/review/done/verified is gated by default.
        _ => &[],
    };
    list.iter().map(|s| s.to_string()).collect()
}

/// Effective gate criteria for moving `row` into `target`: the card-level
/// `gate` column override (a JSON array) wins when non-empty, else the
/// type-derived defaults. Mirrors Python `_effective_gate` minus the
/// worker/group tiers (RR-0051).
pub fn effective_gate(row: &IssueRow, target: TaskStatus) -> Vec<String> {
    let over = row.gate_criteria();
    if !over.is_empty() {
        return over;
    }
    default_gates_for(&row.item_type, target)
}

/// Wrap criterion strings into a core [`Gate`] guarding `target`, so
/// [`board::apply_transition`] / [`board::why_blocked`] enforce and explain
/// them through the one shared code path. Criteria verify as `ModelJudgment`
/// (an acknowledgement IS a judgment call, recorded as `ModelTranscript`
/// evidence when the caller acks honestly) — the free verifier kinds land
/// when gates become first-class stored entities (RR-0051/Invariant 18).
pub fn core_gates(criteria: &[String], target: TaskStatus) -> Vec<Gate> {
    if criteria.is_empty() {
        return Vec::new();
    }
    vec![Gate {
        id: gate_id_for(target),
        scope: amux_core::scope::Scope::Global,
        guards: target,
        applies_to_types: None,
        criteria: criteria
            .iter()
            .map(|c| GateCriterion {
                description: c.clone(),
                verifier: VerifierKind::ModelJudgment { prompt: c.clone() },
                required: true,
            })
            .collect(),
    }]
}

// ---------------------------------------------------------------------------
// Log convention
// ---------------------------------------------------------------------------

/// Append one history line exactly the way Python's `_append_board_log`
/// does: `` (log.rstrip() + "\n`HH:MM` " + line).strip() `` — so logs written
/// by either server interleave without corrupting each other's lines.
pub fn append_log(existing: Option<&str>, hhmm: &str, line: &str) -> String {
    let base = existing.unwrap_or("").trim_end();
    format!("{base}\n`{hhmm}` {line}").trim().to_string()
}

// ---------------------------------------------------------------------------
// Id minting (shared issue_counters table)
// ---------------------------------------------------------------------------

/// Replicates Python `_prefix_from_session`: 'my-project' -> "MP",
/// single-word 'orch' -> "ORCH", empty -> "AMUX". Both servers must derive
/// the identical prefix or the shared counters stop preventing collisions.
pub fn prefix_from_session(session: &str) -> String {
    let words: Vec<&str> = session
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    let clean = |s: String| -> String {
        s.chars()
            .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            .take(5)
            .collect()
    };
    let raw = match words.len() {
        0 => return "AMUX".into(),
        1 => clean(words[0].to_uppercase()),
        _ => clean(
            words
                .iter()
                .filter_map(|w| w.chars().next())
                .collect::<String>()
                .to_uppercase(),
        ),
    };
    if raw.is_empty() {
        "AMUX".into()
    } else {
        raw
    }
}

/// Replicates Python `_next_issue_id` byte-for-byte against the SAME
/// `issue_counters` rows: seed the prefix at 1, post-increment, return
/// `<prefix>-<n>`. Because both servers use this one shared counter, ids
/// minted by either can never collide.
pub fn next_issue_id(conn: &Connection, prefix: &str) -> rusqlite::Result<String> {
    conn.execute(
        "INSERT OR IGNORE INTO issue_counters (prefix, next_n) VALUES (?1, 1)",
        params![prefix],
    )?;
    let n: i64 = conn.query_row(
        "UPDATE issue_counters SET next_n = next_n + 1 WHERE prefix = ?1 RETURNING next_n - 1",
        params![prefix],
        |r| r.get(0),
    )?;
    Ok(format!("{prefix}-{n}"))
}

// ---------------------------------------------------------------------------
// Row struct + queries
// ---------------------------------------------------------------------------

/// One live `issues` row, Python column shapes preserved: raw status/type
/// strings, unix-second ints, JSON-array TEXT columns decoded to vecs.
#[derive(Debug, Clone)]
pub struct IssueRow {
    /// The semantic id ("AMUX-123") — the wire identity. See [`internal_id`].
    pub id: String,
    pub title: String,
    pub desc: String,
    /// RAW status spelling as stored (e.g. `needsyou`). Parse via
    /// [`parse_status`]; write via [`status_to_db`].
    pub status: String,
    /// Owner worker NAME (the Python board speaks names, not WorkerIds).
    pub session: Option<String>,
    pub creator: String,
    pub due: Option<String>,
    /// Unix seconds (INTEGER in the live schema).
    pub created: i64,
    /// Unix seconds.
    pub updated: i64,
    pub owner_type: String,
    pub due_time: Option<String>,
    pub pinned: i64,
    pub gcal_event_id: Option<String>,
    pub pos: f64,
    pub notified: i64,
    /// Card-level gate override: JSON array TEXT, or NULL.
    pub gate: Option<String>,
    pub shepherd: Option<String>,
    /// RAW type string — legacy values are exposed as-is; [`core_item_type`]
    /// maps them for gate/state-machine purposes only.
    pub item_type: String,
    pub archived: i64,
    /// Decoded from the JSON-array TEXT column (semantic ids).
    pub depends_on: Vec<String>,
    pub reviewer: Option<String>,
    /// Append-only history (see [`append_log`]); NULL until first line.
    pub log: Option<String>,
    /// The Python optimistic-concurrency counter (`expect_rev` checks this).
    pub rev: i64,
    pub source_ref: Option<String>,
    pub last_verified_at: Option<i64>,
    /// Rust per-row version (migration 0002). Bumped alongside `rev`.
    pub version: i64,
    pub tags: Vec<String>,
}

impl IssueRow {
    /// The card-level gate override as a criterion list ([] when unset).
    pub fn gate_criteria(&self) -> Vec<String> {
        match &self.gate {
            None => Vec::new(),
            Some(s) => serde_json::from_str::<Vec<serde_json::Value>>(s)
                .map(|v| {
                    v.into_iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// RR-0111a: the canonical replay snapshot of this row — also the API's
    /// detail body (`api::board::detail_body` delegates here). ONE function
    /// serializes the row at event-write time and at verify time, so
    /// `db::replay::verify_replay`'s comparison cannot drift from what the
    /// journal recorded.
    ///
    /// `tags` are SORTED: the live read assembles them via `GROUP_CONCAT`,
    /// whose order SQLite does not define, while an event snapshot carries
    /// the caller's staging order — without one canonical order, replay
    /// verification would report phantom tag divergences on identical sets.
    pub fn snapshot(&self) -> serde_json::Value {
        let mut tags = self.tags.clone();
        tags.sort();
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "desc": self.desc,
            "status": self.status,
            "session": self.session,
            "shepherd": self.shepherd,
            "type": self.item_type,
            "creator": self.creator,
            "due": self.due,
            "due_time": self.due_time,
            "created": self.created,
            "updated": self.updated,
            "owner_type": self.owner_type,
            "pinned": self.pinned,
            "pos": self.pos,
            "archived": self.archived,
            "depends_on": self.depends_on,
            "reviewer": self.reviewer,
            "log": self.log,
            "source_ref": self.source_ref,
            "last_verified_at": self.last_verified_at,
            "rev": self.rev,
            "gate": self.gate_criteria(),
            "tags": tags,
            "version": self.version,
        })
    }

    /// Bridge into the core [`Task`] so every status change runs through
    /// [`board::apply_transition`]. `None` when the stored status string is
    /// not in the shared vocabulary (a custom Python lane) — callers must
    /// refuse the transition honestly rather than guess.
    ///
    /// `worker` is always `None`: `issues.session` is an owner NAME, not a
    /// claim by `WorkerId` — atomic claims/leases land with RR-0052.
    pub fn to_task(&self) -> Option<Task> {
        let status = parse_status(&self.status)?;
        let creator = if self.creator.trim().is_empty() {
            Actor::System {
                component: "python-board".into(),
            }
        } else {
            Actor::Human {
                name: self.creator.clone(),
            }
        };
        Some(Task {
            id: internal_id(&self.id),
            title: self.title.clone(),
            desc: self.desc.clone(),
            status,
            worker: None,
            item_type: core_item_type(&self.item_type),
            creator,
            created_at: ts(self.created),
            updated_at: ts(self.updated),
            archived: self.archived != 0,
            pinned: self.pinned != 0,
            depends_on: self.depends_on.iter().map(|d| internal_id(d)).collect(),
            reviewer: self.reviewer.as_ref().map(|n| Actor::Human { name: n.clone() }),
            gate_override: None,
            tags: self.tags.clone(),
            version: u64::try_from(self.version).unwrap_or(0),
        })
    }
}

fn ts(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap_or_else(Utc::now)
}

/// Shared column list so row indices cannot drift from the query text.
/// `desc` is quoted — it is an SQL keyword. `deleted` is never selected;
/// it is filtered in every WHERE instead (soft delete, Python semantics).
const COLS: &str = "i.id, i.title, i.\"desc\", i.status, i.session, i.creator, i.due, \
     i.created, i.updated, i.owner_type, i.due_time, COALESCE(i.pinned,0), \
     i.gcal_event_id, COALESCE(i.pos,0), COALESCE(i.notified,0), i.gate, i.shepherd, \
     i.type, COALESCE(i.archived,0), i.depends_on, i.reviewer, i.log, \
     COALESCE(i.rev,0), i.source_ref, i.last_verified_at, COALESCE(i.version,0), \
     GROUP_CONCAT(t.tag)";

fn issue_from_row(r: &Row<'_>) -> rusqlite::Result<IssueRow> {
    let depends_raw: Option<String> = r.get(19)?;
    let depends_on = depends_raw
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
        .map(|v| {
            v.into_iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let tags_csv: Option<String> = r.get(26)?;
    let tags = tags_csv
        .unwrap_or_default()
        .split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    Ok(IssueRow {
        id: r.get(0)?,
        title: r.get(1)?,
        desc: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        status: r.get(3)?,
        session: r.get(4)?,
        creator: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        due: r.get(6)?,
        created: r.get(7)?,
        updated: r.get(8)?,
        owner_type: r.get::<_, Option<String>>(9)?.unwrap_or_else(|| "human".into()),
        due_time: r.get(10)?,
        pinned: r.get(11)?,
        gcal_event_id: r.get(12)?,
        pos: r.get(13)?,
        notified: r.get(14)?,
        gate: r.get(15)?,
        shepherd: r.get(16)?,
        item_type: r.get::<_, Option<String>>(17)?.unwrap_or_else(|| "code".into()),
        archived: r.get(18)?,
        depends_on,
        reviewer: r.get(20)?,
        log: r.get(21)?,
        rev: r.get(22)?,
        source_ref: r.get(23)?,
        last_verified_at: r.get(24)?,
        version: r.get(25)?,
        tags,
    })
}

/// One card by semantic id, tags joined, soft-delete filtered.
pub fn get_issue(conn: &Connection, id: &str) -> rusqlite::Result<Option<IssueRow>> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM issues i LEFT JOIN issue_tags t ON t.issue_id = i.id \
             WHERE i.id = ?1 AND i.deleted IS NULL GROUP BY i.id"
        ),
        params![id],
        issue_from_row,
    )
    .optional()
}

/// Archived filter for the list (`archived` query param), Python's grammar
/// (amux-server.py:14025): absent/"" = no filter, truthy = archived-only,
/// any other value = non-archived only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivedFilter {
    /// Only archived=0 rows (`archived=0` — and any other non-truthy value).
    ActiveOnly,
    /// Only archived=1 rows (`archived=1`/`true`/`yes`).
    ArchivedOnly,
    /// No filter (the `archived` param absent or empty).
    All,
}

/// Filtered, Python-sorted board list. Filters run BEFORE any terminal cap
/// (the AC-291/AC-301 lesson: cap the filtered set, not the population it is
/// drawn from) — [`cap_terminal`] is a separate step the API applies after.
/// Status filter values are canonicalized on both sides so `needs_you`
/// matches a `needsyou` row.
pub fn list_issues(
    conn: &Connection,
    status_filter: &[String],
    session_filter: &[String],
    archived: ArchivedFilter,
) -> rusqlite::Result<Vec<IssueRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM issues i LEFT JOIN issue_tags t ON t.issue_id = i.id \
         WHERE i.deleted IS NULL GROUP BY i.id"
    ))?;
    let canon = |s: &str| -> String {
        parse_status(s)
            .map(|st| db_status_spelling(st).to_string())
            .unwrap_or_else(|| s.trim().to_lowercase())
    };
    let want_status: Vec<String> = status_filter.iter().map(|s| canon(s)).collect();
    let mut rows = Vec::new();
    for row in stmt.query_map([], issue_from_row)? {
        let row = row?;
        if !want_status.is_empty() && !want_status.contains(&canon(&row.status)) {
            continue;
        }
        if !session_filter.is_empty()
            && !session_filter.contains(&row.session.clone().unwrap_or_default())
        {
            continue;
        }
        match archived {
            ArchivedFilter::ActiveOnly if row.archived != 0 => continue,
            ArchivedFilter::ArchivedOnly if row.archived == 0 => continue,
            _ => {}
        }
        rows.push(row);
    }
    // Python sort: pinned first, then explicitly-positioned (pos != 0) by pos
    // ascending, then the rest by updated descending.
    rows.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| {
                let za = i32::from(a.pos == 0.0);
                let zb = i32::from(b.pos == 0.0);
                za.cmp(&zb)
            })
            .then_with(|| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.updated.cmp(&a.updated))
    });
    Ok(rows)
}

/// The Python `_BOARD_TERMINAL` set for the done_limit cap. NOTE: this is
/// the PAYLOAD-size cap's notion of terminal (done/verified/discarded, per
/// `_cap_terminal`), which is narrower than core's `is_terminal` — `done` is
/// capped although semantically non-terminal, and `quarantined` is included
/// here as the Rust addition core introduced (Python has no such status, so
/// no Python row can ever carry it).
fn cap_terminal_status(raw: &str) -> bool {
    matches!(
        raw.trim().to_lowercase().as_str(),
        "done" | "verified" | "discarded" | "quarantined"
    )
}

/// Cap terminal-status items to the `limit` most recently updated, AFTER
/// filtering — Python `_cap_terminal`, ported with its exact return contract:
/// `(kept, terminal_total, terminal_kept)`, `limit <= 0` -> uncapped with
/// `(_, 0, 0)`. Active items are never capped; order is preserved.
pub fn cap_terminal(items: Vec<IssueRow>, limit: i64) -> (Vec<IssueRow>, usize, usize) {
    if limit <= 0 {
        return (items, 0, 0);
    }
    let term_idx: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, r)| cap_terminal_status(&r.status))
        .map(|(i, _)| i)
        .collect();
    let total = term_idx.len();
    if total as i64 <= limit {
        return (items, total, total);
    }
    let mut by_updated = term_idx.clone();
    by_updated.sort_by(|a, b| items[*b].updated.cmp(&items[*a].updated));
    let keep: std::collections::HashSet<usize> =
        by_updated.into_iter().take(limit as usize).collect();
    let kept = items
        .into_iter()
        .enumerate()
        .filter(|(i, r)| !cap_terminal_status(&r.status) || keep.contains(i))
        .map(|(_, r)| r)
        .collect();
    (kept, total, limit as usize)
}

/// Python `_load_board(done_limit=100)`'s terminal quotas — the SSE board
/// channel's shape (amux-server.py:15825-15860): active items unlimited,
/// `verified` gets its OWN quota of max(done_limit, 300) so the flood of
/// `done` cannot crowd prod-confirmed work out of the UI, and done/discarded
/// share done_limit. Both quotas keep the most recently UPDATED. Discovered
/// live 2026-08-09: ~130 cards were verified in bulk and the Rust SSE push
/// (single lumped 100-cap) showed 9 of them while Python showed 141.
pub fn sse_terminal_quota(items: Vec<IssueRow>, done_limit: usize) -> Vec<IssueRow> {
    let verified_limit = done_limit.max(300);
    let keep_top = |status_match: &dyn Fn(&str) -> bool, limit: usize| -> std::collections::HashSet<usize> {
        let mut idx: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, r)| status_match(&r.status.trim().to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        idx.sort_by(|a, b| items[*b].updated.cmp(&items[*a].updated));
        idx.into_iter().take(limit).collect()
    };
    let keep_verified = keep_top(&|s: &str| s == "verified", verified_limit);
    let keep_done = keep_top(&|s: &str| matches!(s, "done" | "discarded"), done_limit);
    items
        .into_iter()
        .enumerate()
        .filter(|(i, r)| match r.status.trim().to_lowercase().as_str() {
            "verified" => keep_verified.contains(i),
            "done" | "discarded" => keep_done.contains(i),
            _ => true,
        })
        .map(|(_, r)| r)
        .collect()
}

/// Fields for a new card. Everything the Python POST persists (reviewer and
/// depends_on included — accepting a field and dropping it is worse than
/// rejecting it, per the POST handler's own comment).
pub struct NewIssue {
    pub title: String,
    pub desc: String,
    /// RAW status spelling to store (already canonicalized by the API).
    pub status: String,
    pub session: Option<String>,
    pub item_type: String,
    pub creator: String,
    pub owner_type: String,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub reviewer: Option<String>,
    pub shepherd: Option<String>,
    /// Card-level gate override criteria ([] = none).
    pub gate: Vec<String>,
    pub depends_on: Vec<String>,
    pub tags: Vec<String>,
}

/// Insert a new card, replicating the Python POST exactly: id minted from
/// the shared counter, `pos` = (min non-zero pos in the column) - 1024 (new
/// card at the top of its lane), int timestamps, `notified` 0. Returns the
/// row as stored.
pub fn create_issue(conn: &Connection, new: &NewIssue, now: i64) -> rusqlite::Result<IssueRow> {
    let prefix = prefix_from_session(new.session.as_deref().unwrap_or(""));
    let id = next_issue_id(conn, &prefix)?;
    let min_pos: f64 = conn.query_row(
        "SELECT COALESCE(MIN(NULLIF(pos, 0)), 0) FROM issues WHERE status = ?1 AND deleted IS NULL",
        params![new.status],
        |r| r.get(0),
    )?;
    let pos = min_pos - 1024.0;
    let gate_json = if new.gate.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&new.gate).unwrap_or_default())
    };
    let dep_json = if new.depends_on.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&new.depends_on).unwrap_or_default())
    };
    conn.execute(
        "INSERT INTO issues (id, title, \"desc\", status, session, shepherd, type, creator, \
             due, due_time, created, updated, owner_type, pos, gate, reviewer, depends_on, \
             notified, pinned, archived, rev, version) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
             0, 0, 0, 0, 0)",
        params![
            id,
            new.title,
            new.desc,
            new.status,
            new.session.as_deref().filter(|s| !s.is_empty()),
            new.shepherd,
            new.item_type,
            new.creator,
            new.due,
            new.due_time,
            now,
            now,
            new.owner_type,
            pos,
            gate_json,
            new.reviewer,
            dep_json,
        ],
    )?;
    for tag in &new.tags {
        conn.execute(
            "INSERT OR IGNORE INTO issue_tags (issue_id, tag, added_at) VALUES (?1, ?2, ?3)",
            params![id, tag, now],
        )?;
    }
    Ok(get_issue(conn, &id)?.expect("row just inserted"))
}

/// SOFT-delete a card: stamp `deleted` so every query in this module (all of
/// which filter `deleted IS NULL`) stops returning it. Python's DELETE
/// /api/board/{id} does exactly this, and the row stays for forensics.
/// Returns false when the id does not resolve to a live row.
///
/// This is the one write that legitimately touches `deleted` — [`save_patched`]
/// deliberately excludes it (see its note), which is why the delete path needs
/// its own statement rather than a patched row.
pub fn soft_delete(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let now = Utc::now().timestamp();
    let n = conn.execute(
        "UPDATE issues SET deleted = ?2, updated = ?2 WHERE id = ?1 AND deleted IS NULL",
        params![id, now],
    )?;
    Ok(n > 0)
}

/// Write back a patched row. Only columns this API models are touched —
/// `creator`, `created`, `notified`, `gcal_event_id` and `deleted` are
/// deliberately NOT in the SET list so a Rust write can never corrupt a
/// Python-owned column it does not understand (Phase 11 rollback safety).
/// The caller is responsible for having bumped `rev`, `version` and
/// `updated` on the struct (writes bump rev AND version).
pub fn save_patched(conn: &Connection, row: &IssueRow) -> rusqlite::Result<usize> {
    let dep_json = if row.depends_on.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&row.depends_on).unwrap_or_default())
    };
    conn.execute(
        "UPDATE issues SET title = ?1, \"desc\" = ?2, status = ?3, session = ?4, due = ?5, \
             due_time = ?6, owner_type = ?7, pinned = ?8, pos = ?9, gate = ?10, shepherd = ?11, \
             type = ?12, archived = ?13, depends_on = ?14, reviewer = ?15, log = ?16, \
             rev = ?17, version = ?18, updated = ?19, source_ref = ?20, last_verified_at = ?21 \
         WHERE id = ?22 AND deleted IS NULL",
        params![
            row.title,
            row.desc,
            row.status,
            row.session.as_deref().filter(|s| !s.is_empty()),
            row.due,
            row.due_time,
            row.owner_type,
            row.pinned,
            row.pos,
            row.gate,
            row.shepherd,
            row.item_type,
            row.archived,
            dep_json,
            row.reviewer,
            row.log,
            row.rev,
            row.version,
            row.updated,
            row.source_ref,
            row.last_verified_at,
            row.id,
        ],
    )
}

/// Replace the tag set (Python PATCH semantics: `tags` is the full new set).
pub fn set_tags(conn: &Connection, id: &str, tags: &[String], now: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM issue_tags WHERE issue_id = ?1", params![id])?;
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO issue_tags (issue_id, tag, added_at) VALUES (?1, ?2, ?3)",
            params![id, tag, now],
        )?;
    }
    Ok(())
}

/// Would giving `self_id` the dependency set `new_deps` create a cycle?
/// Returns the cycle as SEMANTIC ids for the error message, or `None` when
/// acyclic. Uses core's [`board::detect_cycle`] over the whole board's
/// `DependsOn` edges (self's existing edges are replaced by `new_deps`,
/// matching PATCH replace semantics).
pub fn depends_on_cycle(
    conn: &Connection,
    self_id: &str,
    new_deps: &[String],
) -> rusqlite::Result<Option<Vec<String>>> {
    let mut names: HashMap<TaskId, String> = HashMap::new();
    let intern = |sem: &str, names: &mut HashMap<TaskId, String>| -> TaskId {
        let t = internal_id(sem);
        names.entry(t.clone()).or_insert_with(|| sem.to_string());
        t
    };
    let mut edges: Vec<(TaskId, TaskId)> = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, depends_on FROM issues \
         WHERE deleted IS NULL AND depends_on IS NOT NULL AND depends_on != ''",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, dep_json) = row?;
        if id == self_id {
            continue; // replaced by new_deps below
        }
        if let Ok(deps) = serde_json::from_str::<Vec<serde_json::Value>>(&dep_json) {
            for d in deps.iter().filter_map(|v| v.as_str()) {
                let from = intern(&id, &mut names);
                let to = intern(d, &mut names);
                edges.push((from, to));
            }
        }
    }
    for d in new_deps {
        let from = intern(self_id, &mut names);
        let to = intern(d, &mut names);
        edges.push((from, to));
    }
    Ok(board::detect_cycle(&edges).map(|cycle| {
        cycle
            .iter()
            .map(|t| {
                names
                    .get(t)
                    .cloned()
                    .unwrap_or_else(|| t.as_str().to_string())
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_terminal_quota_gives_verified_its_own_floor() {
        // 400 verified + 150 done + 5 doing: Python keeps ALL doing, the 300
        // newest verified, and the 100 newest done — the lumped 100-cap
        // showed 9 of a 141-card bulk-verify while Python showed all of it.
        let mk = |i: i64, status: &str| IssueRow {
            id: format!("T-{i}"),
            title: String::new(),
            desc: String::new(),
            status: status.into(),
            session: None,
            creator: String::new(),
            due: None,
            created: i,
            updated: i,
            owner_type: "human".into(),
            due_time: None,
            pinned: 0,
            gcal_event_id: None,
            pos: 0.0,
            notified: 0,
            gate: None,
            shepherd: None,
            item_type: "code".into(),
            archived: 0,
            depends_on: vec![],
            reviewer: None,
            log: None,
            rev: 0,
            source_ref: None,
            last_verified_at: None,
            version: 0,
            tags: vec![],
        };
        let mut items: Vec<IssueRow> = Vec::new();
        for i in 0..400 {
            items.push(mk(i, "verified"));
        }
        for i in 400..550 {
            items.push(mk(i, "done"));
        }
        for i in 550..555 {
            items.push(mk(i, "doing"));
        }
        let kept = sse_terminal_quota(items, 100);
        let count = |s: &str| kept.iter().filter(|r| r.status == s).count();
        assert_eq!(count("verified"), 300);
        assert_eq!(count("done"), 100);
        assert_eq!(count("doing"), 5);
        // The newest survive: verified 399 kept, verified 0 evicted.
        assert!(kept.iter().any(|r| r.id == "T-399"));
        assert!(!kept.iter().any(|r| r.id == "T-0"));
    }

    #[test]
    fn internal_id_is_deterministic_and_distinct() {
        assert_eq!(internal_id("AMUX-1"), internal_id("AMUX-1"));
        assert_ne!(internal_id("AMUX-1"), internal_id("AMUX-2"));
        assert!(internal_id("AMUX-1").as_str().starts_with("tsk_"));
    }

    #[test]
    fn status_spellings_round_trip_the_python_vocabulary() {
        // Both spellings parse; the DB default spelling is the Python one.
        assert_eq!(parse_status("needsyou"), Some(TaskStatus::NeedsYou));
        assert_eq!(parse_status("needs_you"), Some(TaskStatus::NeedsYou));
        assert_eq!(db_status_spelling(TaskStatus::NeedsYou), "needsyou");
        // Writing back the status a row already has preserves ITS spelling.
        assert_eq!(status_to_db(TaskStatus::NeedsYou, "needs_you"), "needs_you");
        assert_eq!(status_to_db(TaskStatus::NeedsYou, "todo"), "needsyou");
        // Python _STATUS_ALIASES.
        assert_eq!(parse_status("in_review"), Some(TaskStatus::Review));
        assert_eq!(parse_status("resolved"), Some(TaskStatus::Done));
        assert_eq!(parse_status("wip"), Some(TaskStatus::Doing));
        assert_eq!(parse_status("someday"), None);
    }

    #[test]
    fn prefix_derivation_matches_python() {
        assert_eq!(prefix_from_session("my-project"), "MP");
        assert_eq!(prefix_from_session("orch"), "ORCH");
        assert_eq!(prefix_from_session("amux-cloud"), "AC");
        assert_eq!(prefix_from_session(""), "AMUX");
        assert_eq!(prefix_from_session("---"), "AMUX");
        assert_eq!(prefix_from_session("general-canvas-apps"), "GCA");
    }

    #[test]
    fn append_log_matches_python_format() {
        assert_eq!(append_log(None, "12:01", "x -> y"), "`12:01` x -> y");
        assert_eq!(
            append_log(Some("`09:00` created\n"), "12:01", "a: todo -> doing"),
            "`09:00` created\n`12:01` a: todo -> doing"
        );
    }

    #[test]
    fn gate_table_matches_python() {
        assert_eq!(
            default_gates_for("code", TaskStatus::Done),
            vec!["Implemented and merged", "Tests / lint pass"]
        );
        assert_eq!(
            default_gates_for("escalation", TaskStatus::Done),
            vec!["Outcome recorded in the item (what happened, and why it is closed)"]
        );
        // Unknown/legacy types inherit the strictest (code) gate.
        assert_eq!(
            default_gates_for("decision", TaskStatus::Done),
            default_gates_for("code", TaskStatus::Done)
        );
        assert_eq!(
            default_gates_for("watch", TaskStatus::Review),
            vec!["Fired: evidence of the triggering event recorded"]
        );
        // Ungated statuses stay ungated.
        assert!(default_gates_for("code", TaskStatus::Todo).is_empty());
    }
}
