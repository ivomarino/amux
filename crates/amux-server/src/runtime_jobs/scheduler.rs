//! Durable schedules (RR-0058 CRUD/history/audit/source, RR-0059 expression
//! parser, RR-0060 missed-run + retry) over the LIVE `schedules`,
//! `schedule_runs` and `schedule_audit` tables.
//!
//! # Strangler-fig contract
//!
//! These are the SAME rows the Python server on :8822 reads and writes, so
//! every column name, value shape and clock here mirrors `amux-server.py`:
//!
//! - `schedules.next_run` / `run_at`: LOCAL wall-clock strings
//!   `"%Y-%m-%dT%H:%M"` (never UTC — the Python fire loop compares them
//!   against `datetime.now()`; comparing against UTC would fire hours off).
//! - `schedules.last_run`, `schedule_runs.ran_at`, `schedule_audit.ts`,
//!   `schedules.created/updated`: UTC unix seconds (AMUX-1736 — one clock
//!   for run history).
//! - `schedule_runs.source` discriminates WHY a fire happened (ethos rule 4:
//!   the manual-vs-cron incident). Python writes `cron`, `manual:<who>`,
//!   `trigger:<event>`; the Rust loop writes `cron-rs` and run-now writes
//!   `manual:<who>` — the `-rs` suffix keeps "which scheduler fired this"
//!   answerable from the row during the dual-scheduler period.
//!
//! # Dual-scheduler safety (see runtime_jobs/mod.rs for the full statement)
//!
//! ONLY the Python server fires schedules until `AMUX_RS_SCHEDULER=1` is
//! set. With firing disabled, [`run_scheduler`] runs in SHADOW MODE: it
//! computes due schedules and journals `Other("schedule_shadow")` events
//! into `_amux_state_events` — never touching `schedules`/`schedule_runs` —
//! so Phase 11 can diff shadow events against Python's actual fires. Even
//! with firing enabled, each fire re-checks the row inside the write
//! transaction, so an occurrence Python already advanced is not re-fired.
//! NOTE: enabling `AMUX_RS_SCHEDULER=1` while the Python scheduler is also
//! running is the double-fire configuration; don't.
//!
//! # Timezone semantics
//!
//! Next-run computation uses `chrono::Local`, exactly like Python's
//! `datetime.now()` — a "daily at 9am" means 9am on this machine's wall
//! clock, through DST changes. Nonexistent local times (spring-forward gap)
//! resolve forward to the first representable instant; ambiguous times
//! (fall-back) take the earlier occurrence.

use crate::db::{PendingEvent, SharedStore, WriteOutcome};
use amux_core::revision::{EntityType, MutationKind};
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, LocalResult, NaiveDate, NaiveDateTime,
    TimeZone, Timelike, Weekday,
};
use regex::Regex;
use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

/// The firing/shadow loop's tick cadence.
pub const SCHEDULER_TICK_SECS: u64 = 30;

/// Catch-up cap: at most this many missed occurrences are replayed; the
/// remainder is REPORTED as overflow, never silently dropped (RR-0060).
pub const CATCH_UP_CAP: usize = 10;

/// Bound on how many occurrences [`runs_due`] will enumerate. A schedule
/// like `every 1m` that has been asleep for a year would otherwise scan
/// half a million occurrences to count them. When the scan hits this bound,
/// `DueRuns::truncated_scan` says so and `overflow` is a floor, not an
/// exact count — an announced omission, not a silent one.
pub const SCAN_LIMIT: usize = 1000;

/// Is real firing enabled? Anything but the literal `1` is shadow mode.
/// Read per-call (not cached) so a test or operator change is honored
/// without a restart; [`run_scheduler`] still takes `enabled` explicitly so
/// the loop's mode is decided once, visibly, by its caller.
pub fn firing_enabled() -> bool {
    std::env::var("AMUX_RS_SCHEDULER").map(|v| v == "1").unwrap_or(false)
}

// ---------------------------------------------------------------------------
// RR-0059: expression parser
// ---------------------------------------------------------------------------

/// A parse failure that names what was rejected. Python's parser returns
/// None and callers silently fall back (`"whenever i feel like it"` was
/// accepted and re-armed daily — the filter-that-matches-everything
/// incident, documented at `_skip_next_run`); here the API refuses with the
/// reason instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unrecognized schedule expression: {0}")]
pub struct ExprParseError(pub String);

/// A parsed schedule expression. Grammar (RR-0059, matching the Python
/// `_parse_next_run` forms so existing rows keep parsing):
///
/// - `every Nm` / `every Nh` / `every Nd` (also `in Nm` / `in Nh`, the
///   Python one-shot-relative spelling) -> [`ScheduleExpr::Interval`]
/// - `daily at HH:MM` / `daily at 6pm` / `every morning|evening|night`
/// - `every weekday at HH:MM` (Mon-Fri)
/// - `weekly on Monday at HH:MM` / `every monday at 9am`
/// - `monthly on N at HH:MM` (N in 1..=28, like Python — day 29-31 is
///   rejected rather than silently drifting on short months)
/// - 5-field cron `MIN HOUR DOM MON DOW` (standard DOW: 0 or 7 = Sunday)
///
/// `Cron` boxes the crate schedule: it is ~250 bytes against everyone
/// else's 16, and this enum is cloned into write closures.
#[derive(Debug, Clone)]
pub enum ScheduleExpr {
    /// Fixed interval from the last fire (not wall-clock aligned).
    Interval { every: ChronoDuration },
    Daily { hour: u32, minute: u32 },
    /// Mon-Fri at a time.
    Weekday { hour: u32, minute: u32 },
    Weekly { weekday: Weekday, hour: u32, minute: u32 },
    /// Day-of-month 1..=28 at a time.
    Monthly { day: u32, hour: u32, minute: u32 },
    /// 5-field cron, held as the `cron` crate's schedule (seconds pinned to
    /// 0, DOW translated — see [`translate_dow`]).
    Cron(Box<cron::Schedule>),
}

static RE_IN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^in\s+(\d+)\s*(m|min|minutes?|h|hr|hours?)$").unwrap());
static RE_EVERY_N: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^every\s+(\d+)\s*(m|min|minutes?|h|hr|hours?|d|days?)$").unwrap());
static RE_EVERY_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^every\s+(morning|evening|night)$").unwrap());
static RE_WEEKDAY_AT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^every\s+weekday\s+at\s+(.+)$").unwrap());
static RE_EVERY_DAY_AT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^every\s+(\w+)\s+at\s+(.+)$").unwrap());
static RE_DAILY_AT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^daily\s+at\s+(.+)$").unwrap());
static RE_WEEKLY_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^weekly\s+on\s+(\w+)\s+at\s+(.+)$").unwrap());
static RE_MONTHLY_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^monthly\s+on\s+(\d{1,2})\s+at\s+(.+)$").unwrap());
static RE_TIME_HM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2}):(\d{2})\s*(am|pm)?$").unwrap());
static RE_TIME_H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\s*(am|pm)$").unwrap());
/// Every cron field is digits, `*`, `/`, `-` or `,`. This SHAPE guard (not a
/// word-count guard) is the fix for the Python incident where any 5-word
/// string fell into the cron parser and exploded on `int('at')`.
static RE_CRON_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9*/,\-]+$").unwrap());

/// Flexible time-of-day: `18:00`, `6pm`, `6:30pm`, `9am`, `09:00`
/// (Python's `_parse_time_str`, plus range validation Python lacks —
/// `daily at 99:99` is rejected here, not defaulted).
fn parse_time(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (h, m, ampm) = if let Some(c) = RE_TIME_HM.captures(&s) {
        (
            c[1].parse::<u32>().ok()?,
            c[2].parse::<u32>().ok()?,
            c.get(3).map(|x| x.as_str().to_string()),
        )
    } else if let Some(c) = RE_TIME_H.captures(&s) {
        (c[1].parse::<u32>().ok()?, 0, Some(c[2].to_string()))
    } else {
        return None;
    };
    let h = match ampm.as_deref() {
        Some("pm") if h < 12 => h + 12,
        Some("am") if h == 12 => 0,
        Some(_) if h > 12 => return None, // "13pm" is nonsense, not 25:00
        _ => h,
    };
    if h < 24 && m < 60 {
        Some((h, m))
    } else {
        None
    }
}

fn day_from_name(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// Translate a standard-cron day-of-week field (0-7, 0 or 7 = Sunday — what
/// Python's `_cron_next_run` implements) into the `cron` crate's ordinals
/// (1-7, 1 = Sunday). Without this, `30 9 * * 1-5` — the documented
/// weekday form — would fire Sunday-Thursday. `*/step` passes through
/// unchanged: both conventions start their range at Sunday, so the stepped
/// set is identical. A range ending at 7 (`3-7`, Wed..Sun) wraps in crate
/// ordinals and is split into `4-7,1`.
fn translate_dow(field: &str) -> Option<String> {
    if field == "*" {
        return Some("*".into());
    }
    let mut out: Vec<String> = Vec::new();
    for tok in field.split(',') {
        let (body, step) = match tok.split_once('/') {
            Some((b, s)) => (b, Some(s)),
            None => (tok, None),
        };
        let translated = if body == "*" {
            "*".to_string()
        } else if let Some((lo, hi)) = body.split_once('-') {
            let lo: u32 = lo.parse().ok()?;
            let hi: u32 = hi.parse().ok()?;
            if lo > 7 || hi > 7 || lo > hi {
                return None;
            }
            let lo2 = lo % 7 + 1;
            let hi2 = hi % 7 + 1;
            if hi2 >= lo2 {
                format!("{lo2}-{hi2}")
            } else {
                // hi was 7 (Sunday): the translated range wraps. Emit the
                // pre-Sunday part and let Sunday ride separately. A stepped
                // wrap is ambiguous — refuse rather than guess.
                if step.is_some() {
                    return None;
                }
                out.push(format!("{lo2}-7"));
                "1".to_string()
            }
        } else {
            let n: u32 = body.parse().ok()?;
            if n > 7 {
                return None;
            }
            format!("{}", n % 7 + 1)
        };
        out.push(match step {
            Some(s) => format!("{translated}/{s}"),
            None => translated,
        });
    }
    Some(out.join(","))
}

impl ScheduleExpr {
    /// Parse a schedule expression. See the type docs for the grammar.
    pub fn parse(expr: &str) -> Result<ScheduleExpr, ExprParseError> {
        let orig = expr.trim();
        let s = orig.to_lowercase();
        if s.is_empty() {
            return Err(ExprParseError("(empty)".into()));
        }
        let fail = || ExprParseError(orig.to_string());

        // every Nm/Nh/Nd — and Python's `in Nm/Nh` one-shot-relative
        // spelling, which the Python parser re-arms as an interval anyway
        // once it lands in schedule_expr.
        if let Some(c) = RE_EVERY_N.captures(&s).or_else(|| RE_IN.captures(&s)) {
            let n: i64 = c[1].parse().map_err(|_| fail())?;
            if n == 0 {
                // Python accepts `every 0m` and would fire continuously; a
                // zero interval is a typo, not a cadence anyone chose.
                return Err(fail());
            }
            let every = match &c[2][..1] {
                "m" => ChronoDuration::minutes(n),
                "h" => ChronoDuration::hours(n),
                "d" => ChronoDuration::days(n),
                _ => return Err(fail()),
            };
            return Ok(ScheduleExpr::Interval { every });
        }

        // every morning / evening / night (Python aliases: 9am / 18:00)
        if let Some(c) = RE_EVERY_WORD.captures(&s) {
            let hour = if &c[1] == "morning" { 9 } else { 18 };
            return Ok(ScheduleExpr::Daily { hour, minute: 0 });
        }

        // every weekday at TIME (before the dayname form: "weekday" is not
        // a day name)
        if let Some(c) = RE_WEEKDAY_AT.captures(&s) {
            let (hour, minute) = parse_time(&c[1]).ok_or_else(fail)?;
            return Ok(ScheduleExpr::Weekday { hour, minute });
        }

        // every <dayname> at TIME
        if let Some(c) = RE_EVERY_DAY_AT.captures(&s) {
            if let Some(weekday) = day_from_name(&c[1]) {
                let (hour, minute) = parse_time(&c[2]).ok_or_else(fail)?;
                return Ok(ScheduleExpr::Weekly { weekday, hour, minute });
            }
            // fall through: "every foo at 5" might still be garbage-or-cron
        }

        // daily at TIME
        if let Some(c) = RE_DAILY_AT.captures(&s) {
            let (hour, minute) = parse_time(&c[1]).ok_or_else(fail)?;
            return Ok(ScheduleExpr::Daily { hour, minute });
        }

        // weekly on <dayname> at TIME
        if let Some(c) = RE_WEEKLY_ON.captures(&s) {
            let weekday = day_from_name(&c[1]).ok_or_else(fail)?;
            let (hour, minute) = parse_time(&c[2]).ok_or_else(fail)?;
            return Ok(ScheduleExpr::Weekly { weekday, hour, minute });
        }

        // monthly on N at TIME (1..=28, Python's bound: day 29-31 would
        // silently skip short months)
        if let Some(c) = RE_MONTHLY_ON.captures(&s) {
            let day: u32 = c[1].parse().map_err(|_| fail())?;
            let (hour, minute) = parse_time(&c[2]).ok_or_else(fail)?;
            if !(1..=28).contains(&day) {
                return Err(fail());
            }
            return Ok(ScheduleExpr::Monthly { day, hour, minute });
        }

        // 5-field cron — guarded on field SHAPE, not word count.
        let parts: Vec<&str> = orig.split_whitespace().collect();
        if parts.len() == 5 && parts.iter().all(|p| RE_CRON_FIELD.is_match(p)) {
            let dow = translate_dow(parts[4]).ok_or_else(fail)?;
            let six = format!("0 {} {} {} {} {}", parts[0], parts[1], parts[2], parts[3], dow);
            return cron::Schedule::from_str(&six)
                .map(|s| ScheduleExpr::Cron(Box::new(s)))
                .map_err(|_| fail());
        }

        Err(fail())
    }

    /// The first fire time STRICTLY after `after`, in local time. `None`
    /// when no future occurrence exists (a cron spec can be unsatisfiable).
    pub fn next_run_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            ScheduleExpr::Interval { every } => Some(after + *every),
            ScheduleExpr::Daily { hour, minute } => {
                next_date_matching(after, *hour, *minute, |_| true)
            }
            ScheduleExpr::Weekday { hour, minute } => {
                next_date_matching(after, *hour, *minute, |d| {
                    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
                })
            }
            ScheduleExpr::Weekly { weekday, hour, minute } => {
                let wd = *weekday;
                next_date_matching(after, *hour, *minute, move |d| d.weekday() == wd)
            }
            ScheduleExpr::Monthly { day, hour, minute } => {
                let (mut y, mut mo) = (after.year(), after.month());
                for _ in 0..=13 {
                    // day <= 28, so every month has it.
                    if let Some(date) = NaiveDate::from_ymd_opt(y, mo, *day) {
                        if let Some(t) = at_local_time(date, *hour, *minute) {
                            if t > after {
                                return Some(t);
                            }
                        }
                    }
                    if mo == 12 {
                        y += 1;
                        mo = 1;
                    } else {
                        mo += 1;
                    }
                }
                None
            }
            ScheduleExpr::Cron(sched) => sched.after(&after).next(),
        }
    }
}

/// First `date >= after.date()` accepted by `ok(date)` whose local hh:mm is
/// strictly after `after`. Scans at most ~53 weeks, like Python's 366-day
/// cron limit.
fn next_date_matching(
    after: DateTime<Local>,
    hour: u32,
    minute: u32,
    ok: impl Fn(NaiveDate) -> bool,
) -> Option<DateTime<Local>> {
    let mut d = after.date_naive();
    for _ in 0..=370 {
        if ok(d) {
            if let Some(t) = at_local_time(d, hour, minute) {
                if t > after {
                    return Some(t);
                }
            }
        }
        d = d.succ_opt()?;
    }
    None
}

/// Resolve a naive local date+time to a real instant. Ambiguous (DST
/// fall-back) takes the earlier occurrence; nonexistent (spring-forward
/// gap) rolls forward to the first representable hour.
fn at_local_time(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Local>> {
    resolve_local(date.and_hms_opt(hour, minute, 0)?)
}

fn resolve_local(n: NaiveDateTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&n) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => {
            for h in 1..=3 {
                match Local.from_local_datetime(&(n + ChronoDuration::hours(h))) {
                    LocalResult::Single(t) => return Some(t),
                    LocalResult::Ambiguous(earliest, _) => return Some(earliest),
                    LocalResult::None => continue,
                }
            }
            None
        }
    }
}

/// Format an instant the way Python stores `next_run`/`run_at`: local
/// wall-clock, minute resolution.
pub fn fmt_minute(t: DateTime<Local>) -> String {
    t.format("%Y-%m-%dT%H:%M").to_string()
}

/// Parse a stored `next_run`/`run_at` string (local, minute resolution;
/// tolerant of trailing seconds).
pub fn parse_minute(s: &str) -> Option<DateTime<Local>> {
    let s = s.get(..16)?;
    resolve_local(NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").ok()?)
}

// ---------------------------------------------------------------------------
// RR-0060: missed runs + retry
// ---------------------------------------------------------------------------

/// What to do when occurrences were missed (server down, loop stalled).
/// `Skip` (the default, matching Python's advance-from-now behavior) fires
/// only the most recent occurrence; `CatchUp` replays each missed one, up
/// to [`CATCH_UP_CAP`]. Loop default comes from `AMUX_RS_MISSED_POLICY`
/// (`skip` | `catchup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissedRunPolicy {
    Skip,
    CatchUp,
}

pub fn missed_policy_from_env() -> MissedRunPolicy {
    match std::env::var("AMUX_RS_MISSED_POLICY").as_deref() {
        Ok("catchup") | Ok("catch-up") | Ok("catch_up") => MissedRunPolicy::CatchUp,
        _ => MissedRunPolicy::Skip,
    }
}

/// The result of [`runs_due`]. The bare Vec the plan sketched cannot say
/// "and N more were dropped", which is exactly the number RR-0060 requires
/// to be REPORTED — so the overflow rides along instead of vanishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRuns {
    /// Occurrences to fire, oldest first. Skip: at most the latest one.
    /// CatchUp: up to [`CATCH_UP_CAP`], oldest first (replay order).
    pub runs: Vec<DateTime<Local>>,
    /// Occurrences due in the window but NOT in `runs` (skipped by policy
    /// or beyond the cap).
    pub overflow: usize,
    /// True when enumeration stopped at [`SCAN_LIMIT`]; `overflow` is then
    /// a floor, not an exact count.
    pub truncated_scan: bool,
}

/// All occurrences strictly after `last_checked` and at or before `now`,
/// filtered through the missed-run policy.
pub fn runs_due(
    expr: &ScheduleExpr,
    last_checked: DateTime<Local>,
    now: DateTime<Local>,
    policy: MissedRunPolicy,
) -> DueRuns {
    let mut all: Vec<DateTime<Local>> = Vec::new();
    let mut t = last_checked;
    let mut truncated_scan = false;
    while all.len() < SCAN_LIMIT {
        match expr.next_run_after(t) {
            Some(n) if n <= now => {
                if n <= t {
                    break; // non-advancing expression: refuse to spin
                }
                all.push(n);
                t = n;
            }
            _ => break,
        }
    }
    if all.len() >= SCAN_LIMIT {
        truncated_scan = true;
    }
    let total = all.len();
    match policy {
        MissedRunPolicy::Skip => DueRuns {
            runs: all.last().cloned().into_iter().collect(),
            overflow: total.saturating_sub(1),
            truncated_scan,
        },
        MissedRunPolicy::CatchUp => {
            let overflow = total.saturating_sub(CATCH_UP_CAP);
            all.truncate(CATCH_UP_CAP);
            DueRuns { runs: all, overflow, truncated_scan }
        }
    }
}

/// Retry policy for a failed tick/fire write: exponential backoff, capped.
/// A schedule fire that still fails after retries records an `error` run
/// row and ADVANCES `next_run` (Python's poison-entry guard: a row that
/// stays due re-raises forever).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy { max_attempts: 3, base_delay_ms: 2_000, max_delay_ms: 30_000 }
    }
}

impl RetryPolicy {
    /// Delay before retry number `attempt` (1-based): base * 2^(attempt-1),
    /// capped.
    pub fn delay(&self, attempt: u32) -> std::time::Duration {
        let exp = attempt.saturating_sub(1).min(16);
        let ms = self.base_delay_ms.saturating_mul(1u64 << exp).min(self.max_delay_ms);
        std::time::Duration::from_millis(ms)
    }
}

// ---------------------------------------------------------------------------
// RR-0058: the durable row + CRUD against the live tables
// ---------------------------------------------------------------------------

/// The seven fields whose changes are audited into `schedule_audit`
/// (mirrors Python's `_AUDIT_FIELDS` exactly, so both servers' trails agree
/// on what counts as an auditable mutation).
pub const AUDIT_FIELDS: [&str; 7] =
    ["enabled", "session", "command", "schedule_expr", "done_action", "trigger_on", "kind"];

/// A row of the live `schedules` table. DB-backed with history and audit —
/// the durable half of the DurableSchedule/PeriodicTask split (see
/// runtime_jobs/mod.rs). Holds every column as tolerant JSON (`SELECT *`,
/// like Python's dict-of-row) because the live table's columns carry mixed
/// affinities — e.g. `last_run` is declared TEXT and holds unix ints today
/// but held ISO strings before AMUX-1736 — and a strictly typed struct
/// would refuse rows Python happily serves.
#[derive(Debug, Clone)]
pub struct DurableSchedule {
    raw: Map<String, Value>,
}

impl DurableSchedule {
    pub fn from_map(raw: Map<String, Value>) -> Self {
        DurableSchedule { raw }
    }

    fn from_row(row: &rusqlite::Row<'_>, cols: &[String]) -> rusqlite::Result<Self> {
        let mut raw = Map::new();
        for (i, name) in cols.iter().enumerate() {
            let v = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => Value::from(n),
                rusqlite::types::ValueRef::Real(f) => {
                    serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)
                }
                rusqlite::types::ValueRef::Text(t) => {
                    Value::String(String::from_utf8_lossy(t).into_owned())
                }
                rusqlite::types::ValueRef::Blob(b) => {
                    Value::String(String::from_utf8_lossy(b).into_owned())
                }
            };
            raw.insert(name.clone(), v);
        }
        Ok(DurableSchedule { raw })
    }

    pub fn str_field(&self, key: &str) -> &str {
        self.raw.get(key).and_then(Value::as_str).unwrap_or("")
    }

    pub fn i64_field(&self, key: &str, default: i64) -> i64 {
        match self.raw.get(key) {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(default),
            Some(Value::Bool(b)) => *b as i64,
            Some(Value::String(s)) => s.parse().unwrap_or(default),
            _ => default,
        }
    }

    pub fn id(&self) -> &str {
        self.str_field("id")
    }

    pub fn enabled(&self) -> bool {
        self.i64_field("enabled", 0) != 0
    }

    pub fn is_deleted(&self) -> bool {
        !matches!(self.raw.get("deleted"), None | Some(Value::Null))
    }

    pub fn schedule_expr(&self) -> Option<&str> {
        let e = self.str_field("schedule_expr");
        if e.trim().is_empty() {
            None
        } else {
            Some(e)
        }
    }

    pub fn set(&mut self, key: &str, v: Value) {
        self.raw.insert(key.to_string(), v);
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.raw.clone())
    }

    pub fn raw(&self) -> &Map<String, Value> {
        &self.raw
    }
}

fn to_sql(v: Option<&Value>) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sv;
    match v {
        None | Some(Value::Null) => Sv::Null,
        Some(Value::Bool(b)) => Sv::Integer(*b as i64),
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                Sv::Integer(i)
            } else {
                Sv::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Some(Value::String(s)) => Sv::Text(s.clone()),
        Some(other) => Sv::Text(other.to_string()), // objects/arrays as JSON text (exit_actions)
    }
}

fn select_schedules(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<DurableSchedule>> {
    let mut stmt = conn.prepare(sql)?;
    let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt.query_map(params, |r| DurableSchedule::from_row(r, &cols))?;
    rows.collect()
}

/// Mint the next `SCHED-N` id from the shared `issue_counters` table — the
/// SAME counter Python's `_next_issue_id` uses, so ids never collide across
/// the two servers.
pub fn mint_schedule_id(conn: &Connection) -> rusqlite::Result<String> {
    conn.execute("INSERT OR IGNORE INTO issue_counters (prefix, next_n) VALUES ('SCHED', 1)", [])?;
    let n: i64 = conn.query_row(
        "UPDATE issue_counters SET next_n = next_n + 1 WHERE prefix = 'SCHED' RETURNING next_n - 1",
        [],
        |r| r.get(0),
    )?;
    Ok(format!("SCHED-{n}"))
}

/// All non-deleted schedules; with `session`, only that session's (the
/// scope test: a schedule bound to a session surfaces only for it).
/// Ordering matches Python: `next_run ASC, created ASC`.
pub fn list_schedules(
    conn: &Connection,
    session: Option<&str>,
) -> rusqlite::Result<Vec<DurableSchedule>> {
    match session {
        Some(s) => select_schedules(
            conn,
            "SELECT * FROM schedules WHERE deleted IS NULL AND session = ?1
             ORDER BY next_run ASC, created ASC",
            &[&s],
        ),
        None => select_schedules(
            conn,
            "SELECT * FROM schedules WHERE deleted IS NULL ORDER BY next_run ASC, created ASC",
            &[],
        ),
    }
}

pub fn get_schedule(conn: &Connection, id: &str) -> rusqlite::Result<Option<DurableSchedule>> {
    let mut v = select_schedules(conn, "SELECT * FROM schedules WHERE id = ?1", &[&id])?;
    Ok(v.pop())
}

fn due_schedules(conn: &Connection, now_str: &str) -> rusqlite::Result<Vec<DurableSchedule>> {
    // String compare of "%Y-%m-%dT%H:%M" local strings — the same predicate
    // as the Python loop, so both schedulers agree on "due".
    select_schedules(
        conn,
        "SELECT * FROM schedules WHERE deleted IS NULL AND enabled = 1
         AND next_run IS NOT NULL AND next_run != '' AND next_run <= ?1",
        &[&now_str],
    )
}

/// INSERT with Python's exact column list (create parity).
pub fn insert_schedule(conn: &Connection, s: &DurableSchedule) -> rusqlite::Result<()> {
    const COLS: [&str; 24] = [
        "id", "title", "session", "command", "kind", "sched_type", "recurrence", "run_at",
        "next_run", "last_run", "enabled", "run_count", "schedule_expr", "watch", "watch_timeout",
        "done_pattern", "done_action", "trigger_on", "trigger_cooldown", "trigger_sessions",
        "exit_actions", "created", "updated", "deleted",
    ];
    let placeholders: Vec<String> = (1..=COLS.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO schedules ({}) VALUES ({})",
        COLS.join(","),
        placeholders.join(",")
    );
    let vals: Vec<rusqlite::types::Value> = COLS.iter().map(|c| to_sql(s.raw.get(*c))).collect();
    conn.execute(&sql, rusqlite::params_from_iter(vals))?;
    Ok(())
}

/// Full-row UPDATE with Python's PATCH column list.
pub fn update_schedule(conn: &Connection, s: &DurableSchedule) -> rusqlite::Result<usize> {
    const COLS: [&str; 19] = [
        "title", "session", "command", "kind", "sched_type", "recurrence", "run_at", "next_run",
        "enabled", "schedule_expr", "watch", "watch_timeout", "done_pattern", "done_action",
        "trigger_on", "trigger_cooldown", "trigger_sessions", "exit_actions", "updated",
    ];
    let sets: Vec<String> = COLS.iter().enumerate().map(|(i, c)| format!("{c}=?{}", i + 1)).collect();
    let sql = format!("UPDATE schedules SET {} WHERE id=?{}", sets.join(","), COLS.len() + 1);
    let mut vals: Vec<rusqlite::types::Value> =
        COLS.iter().map(|c| to_sql(s.raw.get(*c))).collect();
    vals.push(rusqlite::types::Value::Text(s.id().to_string()));
    conn.execute(&sql, rusqlite::params_from_iter(vals))
}

/// Soft delete (Python parity: rows survive for the audit/history hanging
/// off them). Returns rows affected; 0 = already deleted or missing.
pub fn soft_delete_schedule(conn: &Connection, id: &str, now_ts: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE schedules SET deleted=?1, updated=?1 WHERE id=?2 AND deleted IS NULL",
        rusqlite::params![now_ts, id],
    )
}

/// Audit a schedule mutation into `schedule_audit` (AMUX-1735/1812: every
/// create/update/delete leaves a `by_who` trail; an unattributed write on
/// this table is a forensics gap by definition).
pub fn insert_audit(
    conn: &Connection,
    schedule_id: &str,
    field: &str,
    old: &str,
    new: &str,
    source: &str,
    by_who: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO schedule_audit (schedule_id, ts, field, old_value, new_value, source, by_who)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            schedule_id,
            chrono::Utc::now().timestamp(),
            field,
            old,
            new,
            source,
            by_who
        ],
    )?;
    Ok(())
}

/// Record a run into `schedule_runs` WITH its source (ethos rule 4: a
/// manual tap and a cron fire must never be byte-identical rows).
pub fn insert_run(
    conn: &Connection,
    schedule_id: &str,
    ran_at: i64,
    status: &str,
    note: Option<&str>,
    source: &str,
) -> rusqlite::Result<()> {
    // char-safe 64-cap, matching Python's `source[:64]`.
    let source: String = source.chars().take(64).collect();
    conn.execute(
        "INSERT INTO schedule_runs (schedule_id, ran_at, status, note, source)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![schedule_id, ran_at, status, note, source],
    )?;
    Ok(())
}

/// Record a manual (run-now) fire: run row with `manual:<who>` source,
/// run_count bump, last_run/updated — Python's run-now writes, minus the
/// tmux delivery (see api/schedules.rs run_now for why that is honest).
pub fn record_manual_run(
    conn: &Connection,
    schedule_id: &str,
    by_who: &str,
    note: Option<&str>,
) -> rusqlite::Result<()> {
    let now_ts = chrono::Utc::now().timestamp();
    insert_run(conn, schedule_id, now_ts, "ok", note, &format!("manual:{by_who}"))?;
    conn.execute(
        "UPDATE schedules SET run_count = COALESCE(run_count,0) + 1, last_run=?1, updated=?1
         WHERE id=?2",
        rusqlite::params![now_ts, schedule_id],
    )?;
    Ok(())
}

/// Python's legacy `_next_run_dt`: recurrence-based schedules (no
/// schedule_expr) — once / hourly / daily / weekly / monthly with the
/// weekday-or-mday prefix encoding in `run_at`.
pub fn legacy_next_run(
    sched_type: &str,
    recurrence: Option<&str>,
    run_at: Option<&str>,
    now: DateTime<Local>,
) -> Option<String> {
    if sched_type == "once" {
        return run_at.map(str::to_string);
    }
    let rec = recurrence.unwrap_or("daily");
    let run_at = run_at.unwrap_or("");
    let tail_time = |s: &str| -> (u32, u32) {
        s.len()
            .checked_sub(5)
            .and_then(|i| s.get(i..))
            .and_then(parse_time)
            .unwrap_or((9, 0))
    };
    let (h, m) = tail_time(run_at);
    let next = match rec {
        "hourly" => {
            let mut t = resolve_local(now.date_naive().and_hms_opt(now.hour(), m, 0)?)?;
            if t <= now {
                t += ChronoDuration::hours(1);
            }
            t
        }
        "weekly" => {
            let wd = run_at
                .split(':')
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .and_then(weekday_from_mon0);
            let time = run_at.split_once(':').map(|(_, rest)| tail_time(rest)).unwrap_or((h, m));
            match wd {
                Some(wd) => {
                    ScheduleExpr::Weekly { weekday: wd, hour: time.0, minute: time.1 }
                        .next_run_after(now)?
                }
                None => ScheduleExpr::Daily { hour: h, minute: m }.next_run_after(now)?,
            }
        }
        "monthly" => {
            let day = run_at
                .split(':')
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|d| (1..=28).contains(d));
            let time = run_at.split_once(':').map(|(_, rest)| tail_time(rest)).unwrap_or((h, m));
            match day {
                Some(day) => {
                    ScheduleExpr::Monthly { day, hour: time.0, minute: time.1 }
                        .next_run_after(now)?
                }
                None => ScheduleExpr::Daily { hour: h, minute: m }.next_run_after(now)?,
            }
        }
        // "daily" and anything unrecognized: Python defaults recurrence to
        // daily-at-time (this is create-path defaulting, NOT the re-arm
        // fallback that caused the accepts-garbage incident — schedule_expr
        // garbage is rejected at parse, upstream of here).
        _ => ScheduleExpr::Daily { hour: h, minute: m }.next_run_after(now)?,
    };
    Some(fmt_minute(next))
}

fn weekday_from_mon0(n: u32) -> Option<Weekday> {
    Some(match n {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        6 => Weekday::Sun,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// The shadow/firing loop
// ---------------------------------------------------------------------------

/// What one tick did — returned so tests (and logs) verify behavior instead
/// of inferring it from silence.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    pub due: usize,
    pub fired: usize,
    pub shadowed: usize,
    /// Due schedules whose shadow event was already journaled for this
    /// occurrence (in-memory dedupe hit).
    pub deduped: usize,
    pub errors: usize,
}

/// One scheduler pass. `firing=false` is SHADOW MODE (see module docs):
/// journal-only, no `schedules`/`schedule_runs` writes. `shadow_seen` is
/// the (schedule id -> next_run string) dedupe map; in-memory by design —
/// after a restart at most one duplicate shadow event per due schedule is
/// journaled, which the Phase 11 diff tolerates.
pub async fn scheduler_tick(
    store: &SharedStore,
    firing: bool,
    policy: MissedRunPolicy,
    shadow_seen: &mut HashMap<String, String>,
) -> anyhow::Result<TickReport> {
    let now = Local::now();
    let now_str = fmt_minute(now);
    let store_read = store.clone();
    let ns = now_str.clone();
    let due: Vec<DurableSchedule> = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let conn = store_read.read()?;
        Ok(due_schedules(&conn, &ns)?)
    })
    .await??;

    let mut report = TickReport { due: due.len(), ..Default::default() };

    for sched in due {
        let id = sched.id().to_string();
        let due_at = sched.str_field("next_run").to_string();
        if !firing {
            if shadow_seen.get(&id) == Some(&due_at) {
                report.deduped += 1;
                continue;
            }
            // Journal-only write: no table rows change, but the event is a
            // real state change (the journal IS the artifact shadow mode
            // exists to produce), so applied=true is honest.
            let (eid, edue) = (id.clone(), due_at.clone());
            let res = store
                .write_async(move |_conn| {
                    Ok(WriteOutcome {
                        applied: true,
                        events: vec![PendingEvent {
                            entity_type: EntityType::Other("schedule_shadow".into()),
                            entity_id: eid,
                            // The occurrence rides IN the event so the Phase
                            // 11 diff never has to guess which fire this
                            // shadow corresponds to (ethos rule 4).
                            mutation: MutationKind::StatusChanged {
                                from: edue,
                                to: "would-fire".into(),
                            },
                        }],
                    })
                })
                .await;
            match res {
                Ok(_) => {
                    shadow_seen.insert(id, due_at);
                    report.shadowed += 1;
                }
                Err(e) => {
                    report.errors += 1;
                    tracing::warn!(schedule = %id, error = %e, "shadow event write failed");
                }
            }
            continue;
        }

        // FIRING mode: one write transaction PER schedule (the Python loop's
        // MO-3058 lesson: a shared batch turns one poison entry into missed
        // and double fires for everyone else).
        match fire_one(store, &id, now, policy).await {
            Ok(true) => report.fired += 1,
            Ok(false) => {} // raced: no longer due inside the txn
            Err(e) => {
                report.errors += 1;
                tracing::warn!(schedule = %id, error = %e, "fire failed");
                // Poison-entry guard (Python parity): record the error run
                // and push next_run out so the row cannot wedge every tick.
                let eid = id.clone();
                let bump = fmt_minute(now + ChronoDuration::minutes(15));
                let emsg: String = format!("fire aborted: {e}").chars().take(500).collect();
                let _ = store
                    .write_async(move |conn| {
                        insert_run(
                            conn,
                            &eid,
                            chrono::Utc::now().timestamp(),
                            "error",
                            Some(&emsg),
                            "cron-rs",
                        )?;
                        conn.execute(
                            "UPDATE schedules SET next_run=?1, updated=?2 WHERE id=?3",
                            rusqlite::params![bump, chrono::Utc::now().timestamp(), eid],
                        )?;
                        Ok(WriteOutcome { applied: true, events: vec![] })
                    })
                    .await;
            }
        }
    }
    Ok(report)
}

/// Fire one due schedule inside a single write transaction. Returns
/// Ok(false) when the row was no longer due at write time (deleted,
/// disabled, or its next_run moved — i.e. the OTHER scheduler got there
/// first; skipping is the dual-scheduler-safe answer).
async fn fire_one(
    store: &SharedStore,
    id: &str,
    now: DateTime<Local>,
    policy: MissedRunPolicy,
) -> anyhow::Result<bool> {
    let id = id.to_string();
    let now_str = fmt_minute(now);
    let reply = store
        .write_async(move |conn| {
            let Some(sched) = get_schedule(conn, &id)? else {
                return Ok(WriteOutcome { applied: false, events: vec![] });
            };
            let stored_next = sched.str_field("next_run").to_string();
            if sched.is_deleted()
                || !sched.enabled()
                || stored_next.is_empty()
                || stored_next.as_str() > now_str.as_str()
            {
                return Ok(WriteOutcome { applied: false, events: vec![] });
            }

            let now_ts = chrono::Utc::now().timestamp();
            let expr = sched.schedule_expr().and_then(|e| ScheduleExpr::parse(e).ok());
            let prev = parse_minute(&stored_next).unwrap_or(now);

            // Occurrences due in (anchor, now]. The anchor sits one step
            // before the stored due time so the stored occurrence itself is
            // included: intervals step back one interval, calendar forms one
            // minute (their resolution).
            let due = match &expr {
                Some(e) => {
                    let anchor = match e {
                        ScheduleExpr::Interval { every } => prev - *every,
                        _ => prev - ChronoDuration::minutes(1),
                    };
                    runs_due(e, anchor, now, policy)
                }
                None => DueRuns { runs: vec![now], overflow: 0, truncated_scan: false },
            };
            let fallback = [now];
            let occurrences: &[DateTime<Local>] =
                if due.runs.is_empty() { &fallback } else { &due.runs };

            let mut events = Vec::new();
            for (i, occ) in occurrences.iter().enumerate() {
                let is_catch_up = occurrences.len() > 1 && i + 1 < occurrences.len();
                let note = if is_catch_up {
                    Some(format!("catch-up for missed occurrence {}", fmt_minute(*occ)))
                } else if due.overflow > 0 {
                    Some(format!(
                        "{}{} missed occurrence(s) not replayed (policy={:?})",
                        if due.truncated_scan { ">=" } else { "" },
                        due.overflow,
                        policy
                    ))
                } else {
                    None
                };
                // Delivery honesty: the Rust runtime cannot send to a tmux
                // session yet (RR-0041); a fire here is the durable record.
                insert_run(conn, sched.id(), now_ts, "ok", note.as_deref(), "cron-rs")?;
            }
            conn.execute(
                "UPDATE schedules SET run_count = COALESCE(run_count,0) + ?1 WHERE id=?2",
                rusqlite::params![occurrences.len() as i64, sched.id()],
            )?;

            if sched.str_field("sched_type") == "once" {
                conn.execute(
                    "UPDATE schedules SET enabled=0, last_run=?1, updated=?1 WHERE id=?2",
                    rusqlite::params![now_ts, sched.id()],
                )?;
                // run-once disable is audited exactly like Python's
                // ("run-once" source; ours is suffixed for the dual period).
                insert_audit(conn, sched.id(), "enabled", "1", "0", "run-once-rs", "")?;
            } else {
                let next = expr
                    .as_ref()
                    .and_then(|e| e.next_run_after(now))
                    .map(fmt_minute)
                    .or_else(|| {
                        legacy_next_run(
                            sched.str_field("sched_type"),
                            Some(sched.str_field("recurrence")).filter(|s| !s.is_empty()),
                            Some(sched.str_field("run_at")).filter(|s| !s.is_empty()),
                            now,
                        )
                    })
                    // Could not compute one: push out a fixed step rather
                    // than leave it due (Python parity — never wedge).
                    .unwrap_or_else(|| fmt_minute(now + ChronoDuration::minutes(15)));
                conn.execute(
                    "UPDATE schedules SET last_run=?1, next_run=?2, updated=?1 WHERE id=?3",
                    rusqlite::params![now_ts, next, sched.id()],
                )?;
            }

            events.push(PendingEvent {
                entity_type: EntityType::Schedule,
                entity_id: sched.id().to_string(),
                mutation: MutationKind::Updated,
            });
            events.push(PendingEvent {
                entity_type: EntityType::Other("schedule_fire".into()),
                entity_id: sched.id().to_string(),
                mutation: MutationKind::StatusChanged { from: stored_next, to: "fired".into() },
            });
            if due.overflow > 0 {
                events.push(PendingEvent {
                    entity_type: EntityType::Other("schedule_missed".into()),
                    entity_id: sched.id().to_string(),
                    mutation: MutationKind::StatusChanged {
                        from: due.overflow.to_string(),
                        to: format!("{policy:?}").to_lowercase(),
                    },
                });
            }
            Ok(WriteOutcome { applied: true, events })
        })
        .await?;
    Ok(reply.applied)
}

/// The scheduler loop: ticks every [`SCHEDULER_TICK_SECS`]. `enabled=false`
/// (the default deployment — see module docs) is shadow mode. Failed ticks
/// retry per [`RetryPolicy`] before waiting for the next tick.
pub async fn run_scheduler(store: SharedStore, enabled: bool) {
    let policy = missed_policy_from_env();
    let retry = RetryPolicy::default();
    let mut shadow_seen: HashMap<String, String> = HashMap::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(SCHEDULER_TICK_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(
        firing = enabled,
        policy = ?policy,
        "schedule loop starting ({})",
        if enabled { "FIRING — ensure the Python scheduler is stopped" } else { "shadow mode" }
    );
    loop {
        tick.tick().await;
        let mut attempt = 0u32;
        loop {
            match scheduler_tick(&store, enabled, policy, &mut shadow_seen).await {
                Ok(r) => {
                    if r.due > 0 {
                        tracing::info!(
                            due = r.due, fired = r.fired, shadowed = r.shadowed,
                            deduped = r.deduped, errors = r.errors, "scheduler tick"
                        );
                    }
                    break;
                }
                Err(e) => {
                    attempt += 1;
                    if attempt >= retry.max_attempts {
                        tracing::warn!(error = %e, attempts = attempt, "scheduler tick failed; giving up until next tick");
                        break;
                    }
                    tracing::warn!(error = %e, attempt, "scheduler tick failed; retrying");
                    tokio::time::sleep(retry.delay(attempt)).await;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — all DB tests run against a temp-file Store, never ~/.amux.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Store;
    use std::sync::Arc;

    fn store() -> (SharedStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("sched-test.db")).unwrap();
        (Arc::new(s), dir)
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        resolve_local(
            NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap(),
        )
        .unwrap()
    }

    // ---- RR-0059: parser table over every grammar form -------------------

    #[test]
    fn parses_every_documented_grammar_form() {
        // (expr, matcher) — one row per form the docs promise, plus the
        // Python-compat extras existing live rows use.
        type Check = Box<dyn Fn(&ScheduleExpr) -> bool>;
        let ok: Vec<(&str, Check)> = vec![
            ("every 15m", Box::new(|e| matches!(e, ScheduleExpr::Interval { every } if *every == ChronoDuration::minutes(15)))),
            ("every 2h", Box::new(|e| matches!(e, ScheduleExpr::Interval { every } if *every == ChronoDuration::hours(2)))),
            ("every 3d", Box::new(|e| matches!(e, ScheduleExpr::Interval { every } if *every == ChronoDuration::days(3)))),
            ("in 30m", Box::new(|e| matches!(e, ScheduleExpr::Interval { every } if *every == ChronoDuration::minutes(30)))),
            ("daily at 09:00", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 9, minute: 0 }))),
            ("daily at 6pm", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 18, minute: 0 }))),
            ("daily at 6:30pm", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 18, minute: 30 }))),
            ("daily at 12am", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 0, minute: 0 }))),
            ("every morning", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 9, minute: 0 }))),
            ("every evening", Box::new(|e| matches!(e, ScheduleExpr::Daily { hour: 18, minute: 0 }))),
            ("every weekday at 09:30", Box::new(|e| matches!(e, ScheduleExpr::Weekday { hour: 9, minute: 30 }))),
            ("weekly on Monday at 10:00", Box::new(|e| matches!(e, ScheduleExpr::Weekly { weekday: Weekday::Mon, hour: 10, minute: 0 }))),
            ("weekly on fri at 5pm", Box::new(|e| matches!(e, ScheduleExpr::Weekly { weekday: Weekday::Fri, hour: 17, minute: 0 }))),
            ("every sunday at 8am", Box::new(|e| matches!(e, ScheduleExpr::Weekly { weekday: Weekday::Sun, hour: 8, minute: 0 }))),
            ("monthly on 1 at 9am", Box::new(|e| matches!(e, ScheduleExpr::Monthly { day: 1, hour: 9, minute: 0 }))),
            ("monthly on 15 at 18:45", Box::new(|e| matches!(e, ScheduleExpr::Monthly { day: 15, hour: 18, minute: 45 }))),
            ("*/5 * * * *", Box::new(|e| matches!(e, ScheduleExpr::Cron(_)))),
            ("30 9 * * 1-5", Box::new(|e| matches!(e, ScheduleExpr::Cron(_)))),
            ("0 0 1 1 *", Box::new(|e| matches!(e, ScheduleExpr::Cron(_)))),
            ("0 12 * * 0,6", Box::new(|e| matches!(e, ScheduleExpr::Cron(_)))),
        ];
        for (expr, check) in ok {
            let parsed = ScheduleExpr::parse(expr)
                .unwrap_or_else(|e| panic!("'{expr}' should parse: {e}"));
            assert!(check(&parsed), "'{expr}' parsed to the wrong variant: {parsed:?}");
        }

        // Rejections — including the incident shapes: 5-word garbage that
        // Python's word-count guard once fed to int('at'), and garbage that
        // the +1-day fallback once silently re-armed daily.
        for bad in [
            "",
            "whenever i feel like it",
            "every 3 potatoes",
            "sometime after lunch maybe ok",
            "monthly on 31 at 9am", // short-month drift — rejected like Python
            "daily at 99:99",
            "daily at 13pm",
            "every 0m",
            "weekly on funday at 9am",
        ] {
            assert!(
                ScheduleExpr::parse(bad).is_err(),
                "'{bad}' should be rejected"
            );
        }
    }

    #[test]
    fn cron_dow_is_standard_not_crate_ordinals() {
        // Standard cron: 1 = Monday. The `cron` crate: 1 = Sunday. The
        // translator must make "0 9 * * 1" fire on a MONDAY — if this test
        // fails, every documented weekday cron fires a day off.
        let e = ScheduleExpr::parse("0 9 * * 1").unwrap();
        let next = e.next_run_after(local(2026, 8, 12, 12, 0)).unwrap(); // a Wednesday
        assert_eq!(next.weekday(), Weekday::Mon);

        // 1-5 = Mon-Fri: from Friday 10:00, next 09:30 fire is MONDAY.
        let e = ScheduleExpr::parse("30 9 * * 1-5").unwrap();
        let next = e.next_run_after(local(2026, 8, 14, 10, 0)).unwrap(); // Friday
        assert_eq!(next.weekday(), Weekday::Mon);
        assert_eq!(fmt_minute(next), "2026-08-17T09:30");

        // 0,6 = weekend; 7 also = Sunday; a range ending at 7 wraps.
        let e = ScheduleExpr::parse("0 12 * * 0,6").unwrap();
        let next = e.next_run_after(local(2026, 8, 12, 12, 0)).unwrap();
        assert!(matches!(next.weekday(), Weekday::Sat | Weekday::Sun));
        let e = ScheduleExpr::parse("0 12 * * 7").unwrap();
        assert_eq!(e.next_run_after(local(2026, 8, 12, 12, 0)).unwrap().weekday(), Weekday::Sun);
        let e = ScheduleExpr::parse("0 12 * * 5-7").unwrap(); // Fri..Sun
        let next = e.next_run_after(local(2026, 8, 12, 12, 0)).unwrap(); // Wed
        assert_eq!(next.weekday(), Weekday::Fri);
    }

    // ---- next-run correctness around boundaries --------------------------

    #[test]
    fn next_run_month_and_week_boundaries() {
        // Monthly across a month boundary: Jan 31 -> Feb 1.
        let e = ScheduleExpr::parse("monthly on 1 at 9am").unwrap();
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 1, 31, 12, 0)).unwrap()), "2026-02-01T09:00");
        // Same day before the time: today.
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 2, 1, 8, 0)).unwrap()), "2026-02-01T09:00");
        // STRICTLY after: at the exact minute, next month.
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 2, 1, 9, 0)).unwrap()), "2026-03-01T09:00");
        // December -> January (year boundary).
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 12, 15, 0, 0)).unwrap()), "2027-01-01T09:00");

        // Daily across a month boundary.
        let e = ScheduleExpr::parse("daily at 09:00").unwrap();
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 1, 31, 10, 0)).unwrap()), "2026-02-01T09:00");

        // Weekly wrap: 2026-08-10 is a Monday; after Monday 11:00, the next
        // Monday-10:00 is seven days out.
        let e = ScheduleExpr::parse("weekly on monday at 10:00").unwrap();
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 8, 10, 11, 0)).unwrap()), "2026-08-17T10:00");
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 8, 10, 9, 0)).unwrap()), "2026-08-10T10:00");

        // Weekday skips the weekend: Friday 18:00 -> Monday.
        let e = ScheduleExpr::parse("every weekday at 09:30").unwrap();
        let next = e.next_run_after(local(2026, 8, 14, 18, 0)).unwrap(); // Friday evening
        assert_eq!(fmt_minute(next), "2026-08-17T09:30");
        assert_eq!(next.weekday(), Weekday::Mon);

        // Interval: anchored on `after`, not wall-aligned.
        let e = ScheduleExpr::parse("every 90m").unwrap();
        assert_eq!(fmt_minute(e.next_run_after(local(2026, 8, 10, 23, 0)).unwrap()), "2026-08-11T00:30");
    }

    // ---- RR-0060: missed runs, skip vs catch-up, cap ---------------------

    #[test]
    fn missed_runs_skip_vs_catchup_and_cap() {
        let e = ScheduleExpr::parse("every 10m").unwrap();
        let t0 = local(2026, 8, 10, 10, 0);
        let now = t0 + ChronoDuration::hours(2); // 12 occurrences due

        let skip = runs_due(&e, t0, now, MissedRunPolicy::Skip);
        assert_eq!(skip.runs.len(), 1);
        assert_eq!(fmt_minute(skip.runs[0]), "2026-08-10T12:00"); // the LATEST
        assert_eq!(skip.overflow, 11); // the skipped ones are REPORTED
        assert!(!skip.truncated_scan);

        let catch = runs_due(&e, t0, now, MissedRunPolicy::CatchUp);
        assert_eq!(catch.runs.len(), CATCH_UP_CAP); // capped at 10
        assert_eq!(fmt_minute(catch.runs[0]), "2026-08-10T10:10"); // oldest first
        assert_eq!(catch.overflow, 2); // 12 due - 10 kept, reported

        // Nothing due -> empty either way.
        let none = runs_due(&e, t0, t0 + ChronoDuration::minutes(5), MissedRunPolicy::CatchUp);
        assert!(none.runs.is_empty());
        assert_eq!(none.overflow, 0);

        // A year of every-1m: the scan bound announces itself.
        let e1 = ScheduleExpr::parse("every 1m").unwrap();
        let huge = runs_due(&e1, t0, t0 + ChronoDuration::days(365), MissedRunPolicy::Skip);
        assert!(huge.truncated_scan);
        assert_eq!(huge.runs.len(), 1);
        assert!(huge.overflow >= SCAN_LIMIT - 1);
    }

    #[test]
    fn retry_policy_backoff_doubles_and_caps() {
        let r = RetryPolicy { max_attempts: 5, base_delay_ms: 100, max_delay_ms: 500 };
        assert_eq!(r.delay(1).as_millis(), 100);
        assert_eq!(r.delay(2).as_millis(), 200);
        assert_eq!(r.delay(3).as_millis(), 400);
        assert_eq!(r.delay(4).as_millis(), 500); // capped
        assert_eq!(r.delay(40).as_millis(), 500); // no overflow panic
    }

    // ---- RR-0058: CRUD + audit against the live schema -------------------

    fn make_row(id: &str, session: &str, expr: Option<&str>, next_run: &str) -> DurableSchedule {
        let now_ts = chrono::Utc::now().timestamp();
        let mut m = Map::new();
        m.insert("id".into(), Value::from(id));
        m.insert("title".into(), Value::from(format!("t-{id}")));
        m.insert("session".into(), Value::from(session));
        m.insert("command".into(), Value::from("do the thing"));
        m.insert("kind".into(), Value::from("tmux"));
        m.insert(
            "sched_type".into(),
            Value::from(if expr.is_some() { "recurring" } else { "once" }),
        );
        m.insert("recurrence".into(), Value::Null);
        m.insert("run_at".into(), Value::from(next_run));
        m.insert("next_run".into(), Value::from(next_run));
        m.insert("last_run".into(), Value::Null);
        m.insert("enabled".into(), Value::from(1));
        m.insert("run_count".into(), Value::from(0));
        m.insert("schedule_expr".into(), expr.map(Value::from).unwrap_or(Value::Null));
        m.insert("watch".into(), Value::from(0));
        m.insert("watch_timeout".into(), Value::from(120));
        m.insert("done_pattern".into(), Value::Null);
        m.insert("done_action".into(), Value::from("disable"));
        m.insert("trigger_on".into(), Value::Null);
        m.insert("trigger_cooldown".into(), Value::from(120));
        m.insert("trigger_sessions".into(), Value::Null);
        m.insert("exit_actions".into(), Value::Null);
        m.insert("created".into(), Value::from(now_ts));
        m.insert("updated".into(), Value::from(now_ts));
        m.insert("deleted".into(), Value::Null);
        DurableSchedule::from_map(m)
    }

    #[tokio::test]
    async fn crud_and_audit_rows_with_attribution() {
        let (store, _dir) = store();
        // Create (minted id + audit row, like the API will do).
        let created_id = store
            .write_async(|conn| {
                let id = mint_schedule_id(conn)?;
                let row = make_row(&id, "alpha", Some("daily at 09:00"), "2026-08-10T09:00");
                insert_schedule(conn, &row)?;
                insert_audit(conn, &id, "created", "", "{\"title\":\"t\"}", "api-create", "tester")?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();
        assert!(created_id.applied);

        let conn = store.read().unwrap();
        let all = list_schedules(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        let id = all[0].id().to_string();
        assert_eq!(id, "SCHED-1"); // Python's counter format, shared table
        // Scope: bound to alpha, invisible to beta.
        assert_eq!(list_schedules(&conn, Some("alpha")).unwrap().len(), 1);
        assert_eq!(list_schedules(&conn, Some("beta")).unwrap().len(), 0);
        let (by, src): (String, String) = conn
            .query_row(
                "SELECT by_who, source FROM schedule_audit WHERE schedule_id=?1 AND field='created'",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(by, "tester");
        assert_eq!(src, "api-create");
        drop(conn);

        // Update: disable, audited.
        let uid = id.clone();
        store
            .write_async(move |conn| {
                let mut s = get_schedule(conn, &uid)?.unwrap();
                s.set("enabled", Value::from(0));
                s.set("updated", Value::from(chrono::Utc::now().timestamp()));
                update_schedule(conn, &s)?;
                insert_audit(conn, &uid, "enabled", "1", "0", "api-patch", "tester")?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();

        // Delete: soft, idempotent, audited.
        let did = id.clone();
        store
            .write_async(move |conn| {
                let n = soft_delete_schedule(conn, &did, chrono::Utc::now().timestamp())?;
                assert_eq!(n, 1);
                let again = soft_delete_schedule(conn, &did, chrono::Utc::now().timestamp())?;
                assert_eq!(again, 0); // idempotent — no second audit row
                insert_audit(conn, &did, "deleted", "{}", "now", "api-delete", "tester")?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();

        let conn = store.read().unwrap();
        assert_eq!(list_schedules(&conn, None).unwrap().len(), 0); // gone from list
        assert!(get_schedule(&conn, &id).unwrap().unwrap().is_deleted()); // row survives
        let audit_n: i64 = conn
            .query_row("SELECT COUNT(*) FROM schedule_audit WHERE schedule_id=?1", [&id], |r| r.get(0))
            .unwrap();
        assert_eq!(audit_n, 3); // created + enabled + deleted
    }

    #[tokio::test]
    async fn run_now_records_manual_source() {
        let (store, _dir) = store();
        store
            .write_async(|conn| {
                let row = make_row("SCHED-77", "alpha", Some("every 1h"), "2026-08-10T09:00");
                insert_schedule(conn, &row)?;
                record_manual_run(conn, "SCHED-77", "tester", Some("run-now"))?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();
        let conn = store.read().unwrap();
        let src: String = conn
            .query_row("SELECT source FROM schedule_runs WHERE schedule_id='SCHED-77'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(src, "manual:tester"); // discriminable from 'cron'/'cron-rs'
        let rc: i64 = conn
            .query_row("SELECT run_count FROM schedules WHERE id='SCHED-77'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rc, 1);
    }

    // ---- dual-scheduler: shadow mode fires NOTHING -----------------------

    #[tokio::test]
    async fn shadow_mode_journals_and_fires_nothing() {
        let (store, _dir) = store();
        store
            .write_async(|conn| {
                let row = make_row("SCHED-1", "alpha", Some("every 10m"), "2020-01-01T00:00");
                insert_schedule(conn, &row)?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();

        let mut seen = HashMap::new();
        let r1 = scheduler_tick(&store, false, MissedRunPolicy::Skip, &mut seen).await.unwrap();
        assert_eq!(r1, TickReport { due: 1, shadowed: 1, ..Default::default() });

        // Second tick: same occurrence, deduped — one shadow event total.
        let r2 = scheduler_tick(&store, false, MissedRunPolicy::Skip, &mut seen).await.unwrap();
        assert_eq!(r2.deduped, 1);
        assert_eq!(r2.shadowed, 0);

        let conn = store.read().unwrap();
        let shadow_n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _amux_state_events
                 WHERE entity_type LIKE '%schedule_shadow%' AND entity_id='SCHED-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(shadow_n, 1);
        // The occurrence it would have fired is IN the journaled mutation.
        let mutation: String = conn
            .query_row(
                "SELECT mutation FROM _amux_state_events WHERE entity_type LIKE '%schedule_shadow%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(mutation.contains("2020-01-01T00:00"), "{mutation}");
        // NOTHING fired: no runs, next_run untouched (Python's to advance).
        let runs: i64 =
            conn.query_row("SELECT COUNT(*) FROM schedule_runs", [], |r| r.get(0)).unwrap();
        assert_eq!(runs, 0);
        let next: String = conn
            .query_row("SELECT next_run FROM schedules WHERE id='SCHED-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(next, "2020-01-01T00:00");
    }

    #[tokio::test]
    async fn firing_mode_records_run_with_source_and_advances() {
        let (store, _dir) = store();
        store
            .write_async(|conn| {
                insert_schedule(conn, &make_row("SCHED-1", "alpha", Some("every 10m"), "2020-01-01T00:00"))?;
                // A once-type row, also due.
                let mut once = make_row("SCHED-2", "beta", None, "2020-01-01T00:00");
                once.set("sched_type", Value::from("once"));
                insert_schedule(conn, &once)?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();

        let mut seen = HashMap::new();
        let r = scheduler_tick(&store, true, MissedRunPolicy::Skip, &mut seen).await.unwrap();
        assert_eq!(r.due, 2);
        assert_eq!(r.fired, 2);
        assert_eq!(r.errors, 0);

        let conn = store.read().unwrap();
        // Recurring: run recorded with the rust-cron source, next_run in the
        // future, run_count bumped, skip-overflow reported in the note.
        let (src, note): (String, Option<String>) = conn
            .query_row(
                "SELECT source, note FROM schedule_runs WHERE schedule_id='SCHED-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(src, "cron-rs");
        assert!(note.unwrap().contains("missed occurrence"), "skip overflow must be reported");
        let next: String = conn
            .query_row("SELECT next_run FROM schedules WHERE id='SCHED-1'", [], |r| r.get(0))
            .unwrap();
        assert!(next.as_str() > "2026-01", "next_run should advance to the future: {next}");
        let rc: i64 = conn
            .query_row("SELECT run_count FROM schedules WHERE id='SCHED-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rc, 1);

        // Once: fired then disabled, with the audited run-once flip.
        let enabled: i64 = conn
            .query_row("SELECT enabled FROM schedules WHERE id='SCHED-2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(enabled, 0);
        let audit_src: String = conn
            .query_row(
                "SELECT source FROM schedule_audit WHERE schedule_id='SCHED-2' AND field='enabled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit_src, "run-once-rs");
        // A fire event was journaled for each.
        let fire_n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _amux_state_events WHERE entity_type LIKE '%schedule_fire%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fire_n, 2);
    }

    #[tokio::test]
    async fn catch_up_replays_missed_occurrences_capped() {
        let (store, _dir) = store();
        let prev = fmt_minute(Local::now() - ChronoDuration::minutes(45));
        store
            .write_async(move |conn| {
                insert_schedule(conn, &make_row("SCHED-9", "alpha", Some("every 10m"), &prev))?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();
        let mut seen = HashMap::new();
        let r = scheduler_tick(&store, true, MissedRunPolicy::CatchUp, &mut seen).await.unwrap();
        assert_eq!(r.fired, 1);
        let conn = store.read().unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM schedule_runs WHERE schedule_id='SCHED-9'", [], |r| r.get(0))
            .unwrap();
        // 45 minutes of every-10m backlog: occurrences at +0,+10,+20,+30,+40.
        assert_eq!(runs, 5, "catch-up should replay each missed occurrence");
        let catch_notes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schedule_runs WHERE schedule_id='SCHED-9' AND note LIKE 'catch-up%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(catch_notes, 4); // all but the final occurrence are marked
    }
}
