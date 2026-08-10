//! Autofix — notice, file, hand off. **Not a repair engine.**
//!
//! The name is the owner's, the shape is deliberately smaller than the name:
//! amux does not fix anything here. It NOTICES that something broke, files one
//! board card per distinct fault with the evidence already computed, and lets
//! the existing board drive hand that card to a lane at its next turn
//! boundary. Every step is an existing primitive:
//!
//! | step        | primitive        | who does it            |
//! |-------------|------------------|------------------------|
//! | notice      | this runtime job | the server             |
//! | record      | board card       | the server             |
//! | route       | `board_drive`    | the server             |
//! | **fix**     | the worker       | a model, with a turn   |
//!
//! There is no ninth thing. No queue of its own (the board is the queue), no
//! scheduler of its own (`PeriodicTask`), no delivery path of its own
//! (`board_drive` already claims a `todo` card into `doing` under WIP-1 and
//! steers the lane). The single new artifact is a *detector*, and a detector
//! is not a primitive — it is a read over instruments amux already keeps.
//!
//! # It runs OUTSIDE every worker, on purpose
//!
//! Detection and filing happen in the SERVER process. Nothing in this file
//! touches a pane, a send, a tmux capture or a turn boundary; it reads SQLite
//! and the filesystem and it writes a row. That is the entire point: **the
//! thing that watches for breakage must not share fate with the thing that
//! breaks.** If every lane in the fleet is wedged, rate-limited, crashed or
//! simply idle, the cards still get filed and queue up for whenever a worker
//! comes back. Noticing is infrastructure; fixing is work.
//!
//! The corollary is that this job must never be "smart". It has no model call
//! anywhere — titles are COMPUTED from the signature (ethos rule 2: the
//! 12-15k-token label call is the most wasteful touchpoint this repo has
//! measured, and the throttle it needed is why most commands never reached the
//! board). Everything a model should decide — is this real, is it worth
//! fixing, what is the fix — is left for the lane that picks the card up, with
//! the evidence in hand. That is what compounds when the model gets better.
//!
//! # One filing path, many detectors
//!
//! [`DetectorKind`] enumerates what we watch. Each detector is a pure function
//! over a `&Connection` (plus, for the two that need it, the filesystem)
//! returning `Vec<Finding>` and `Vec<Suppressed>` — so dedupe, card shape,
//! ownership, quiet-detection and the debug surface are written ONCE and
//! cannot drift between detectors. Adding a seventh detector is a new match
//! arm, never a new job.
//!
//! # Dedupe is durable, because a restart must not refile
//!
//! The key is `session_events.idem = "autofix:<signature>"`, protected by the
//! existing `idx_sev_idem` unique partial index — the same mechanism
//! `board_drive` uses for its decompose ask. A signature filed before the
//! process died is still filed after it comes back. The card also carries
//! `issues.source_ref = "autofix:<signature>"`, which is how the quiet sweep
//! finds it again later without a second bookkeeping table.
//!
//! # "Stopped happening" is not "fixed"
//!
//! When a filed signature goes quiet for `AMUX_AUTOFIX_QUIET_H`, the card gets
//! a log line saying so — and nothing else. It is not closed, not archived,
//! not moved. Whether silence means fixed, or means the caller gave up, or
//! means the feature is now unreachable, is a judgment with evidence on both
//! sides, and it belongs to the worker holding the card (ethos rule 8).
//!
//! # Suppression is visible or it did not happen
//!
//! Every decision NOT to file is recorded with its reason and surfaced on
//! `GET /api/debug/autofix`. A detector that silently declines is exactly the
//! failure this whole subsystem exists to end: an absence nobody can see.

use crate::api::request_log as rl;
use crate::api::AppState;
use crate::db::board_store as bs;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Knobs. Every threshold in this file is named, defaulted and printed on the
// debug surface — a threshold you cannot read back is a threshold nobody can
// argue with (and the spin-catcher's `cpu >= 70`, which sat below the idle
// baseline, is what that costs).
// ---------------------------------------------------------------------------

/// Default tick. Slow on purpose: nothing here is time-critical, and a filer
/// that runs hot competes with the traffic it is measuring.
pub const AUTOFIX_TICK_SECS: u64 = 120;

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}
fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}
fn env_str(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

/// Window each detector looks back over.
fn window_h() -> f64 {
    env_f64("AMUX_AUTOFIX_WINDOW_H", 6.0).clamp(0.1, 24.0 * 30.0)
}
/// Trailing baseline for the latency comparison (excludes the window).
fn baseline_h() -> f64 {
    env_f64("AMUX_AUTOFIX_BASELINE_H", 72.0).clamp(1.0, 24.0 * 90.0)
}
/// p95 must exceed the trailing p95 by this multiple to be a regression.
fn latency_mult() -> f64 {
    env_f64("AMUX_AUTOFIX_LATENCY_MULT", 3.0).max(1.1)
}
/// Sample floor, both sides. Below this a p95 is one request wearing a
/// percentile — the "filter that matched everything" failure in slow motion.
fn latency_min_samples() -> i64 {
    env_i64("AMUX_AUTOFIX_LATENCY_MIN_SAMPLES", 30).max(5)
}
/// A single request slower than this is absurd on its face, whatever the
/// family's norm is.
fn outlier_ms() -> f64 {
    env_f64("AMUX_AUTOFIX_OUTLIER_MS", 10_000.0).max(250.0)
}
/// How many times a dead route must be hit by a browser before it is a card.
fn dead_route_min_hits() -> i64 {
    env_i64("AMUX_AUTOFIX_DEAD_ROUTE_MIN_HITS", 3).max(1)
}
/// A schedule this far past `next_run` with no run recorded is not late, it is
/// not firing.
fn schedule_overdue_min() -> f64 {
    env_f64("AMUX_AUTOFIX_SCHEDULE_OVERDUE_MIN", 45.0).max(5.0)
}
/// Oldest steering row older than this means the queue is not draining.
fn steering_stale_min() -> f64 {
    env_f64("AMUX_AUTOFIX_STEERING_STALE_MIN", 90.0).max(5.0)
}
/// An invariant must stay breached across at least this many evaluations. A
/// flapping invariant is noise, and noise is how a board stops being read.
fn invariant_min_occurrences() -> i64 {
    env_i64("AMUX_AUTOFIX_INVARIANT_MIN_OCCURRENCES", 3).max(2)
}
/// HEAD moved this long ago and the running build still has not — the deploy
/// path is broken, and every session believes its work shipped.
fn build_stale_h() -> f64 {
    env_f64("AMUX_AUTOFIX_BUILD_STALE_H", 1.0).max(0.1)
}
/// Silence for this long marks a filed signature quiet (noted, never closed).
fn quiet_h() -> f64 {
    env_f64("AMUX_AUTOFIX_QUIET_H", 24.0).max(1.0)
}

/// Signature substrings that never file. Environmental causes with no code
/// fix: put them here rather than teaching a detector to lie about them.
// --- CI detector knobs (AMUX-2780) ------------------------------------------

/// Repos to watch, `owner/name`, comma-separated. Defaults to amux's own repo:
/// the detector that exists because THIS repo's deploy was red for three days
/// must watch this repo without anyone opting in (ethos rule 1 — an extension
/// point nobody is enrolled in is decoration).
fn ci_repos() -> Vec<String> {
    let raw = env_str("AMUX_CI_REPOS");
    let raw = if raw.is_empty() { "mixpeek/amux".to_string() } else { raw };
    raw.split(',').map(|s| s.trim().to_string()).filter(|s| s.contains('/')).collect()
}
/// How often to actually ask GitHub. The tick is 120s; asking every tick would
/// be 720 API calls/day per repo for a signal that changes on push.
fn ci_poll_min() -> f64 {
    env_f64("AMUX_CI_POLL_MIN", 15.0).clamp(1.0, 24.0 * 60.0)
}
/// How many consecutive failures before filing.
///
/// The honest note: BOTH 1 and 2 are guesses, and the ethos is explicit that
/// picking a threshold at all is the tell that you are guessing. There is no
/// structurally-absent signal available here — "red on main" is a level, not an
/// absence. 2 is chosen because it is the smallest number that distinguishes a
/// fault from a single flaky run, and the cost of the extra wait is one push;
/// the incident this detector exists for ran to 31. Set to 1 to file on the
/// first red run.
fn ci_min_failures() -> i64 {
    env_i64("AMUX_CI_MIN_FAILURES", 2).max(1)
}
/// How many recent runs to fetch per repo.
///
/// NOT merely a page size. The window is shared across ALL workflows, and every
/// push to this repo starts three of them — so 100 runs is only ~33 per
/// workflow, and the streak anchor is stable only while the last SUCCESS is
/// still inside it. Measured on the real 2026-08-10 outage: at 100 the deploy
/// streak reported "AT LEAST 21, last success unknown" when the truth was 31
/// with a known green run. 200 recovers both. When the success still falls out,
/// the finding says so rather than reporting a floor as a count.
fn ci_run_limit() -> i64 {
    env_i64("AMUX_CI_RUN_LIMIT", 200).clamp(10, 600)
}
/// Hard timeout on each `gh` invocation. A hung network call must not stall the
/// tick that every other detector shares.
fn ci_cmd_timeout_s() -> u64 {
    env_i64("AMUX_CI_TIMEOUT_S", 20).clamp(2, 120) as u64
}

fn ignore_list() -> Vec<String> {
    env_str("AMUX_AUTOFIX_IGNORE")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The lane that owns a card when the fault names no worker of its own.
/// Empty (the default) means UNOWNED: the board's own pickup decides, and no
/// lane is volunteered for work nobody asked it to do.
fn fixer_session() -> String {
    env_str("AMUX_AUTOFIX_SESSION")
}

/// The dashboard toggle. Persisted in `prefs` like every other setting, read
/// on EVERY tick so the switch takes effect live — a pref that needs a restart
/// is a pref that silently disagrees with the UI showing it.
///
/// Default ON. When OFF the detectors still run and their findings are still
/// published on the debug surface as suppressed; only the WRITE is skipped. A
/// disabled watcher that also goes blind loses the evidence for exactly the
/// window somebody turned it off to look at.
pub fn enabled(conn: &Connection) -> bool {
    let v: Option<String> = conn
        .query_row("SELECT value FROM prefs WHERE key='autofix_enabled'", [], |r| r.get(0))
        .ok();
    !matches!(v.as_deref().map(str::trim), Some("0") | Some("false") | Some("off"))
}

// ---------------------------------------------------------------------------
// The shared vocabulary: every detector speaks in these.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetectorKind {
    /// An unhandled server failure. 501/503 are excluded by construction —
    /// they are honest degradations that NAME what is missing.
    Http5xx,
    /// A family's p95 far above its own trailing norm, or a single request
    /// that is absurd on its face.
    Latency,
    /// A 404/405 on a path a BROWSER asked for — i.e. the SPA calls it and it
    /// is not there. `/api/logs/analyze` already computes the verdict.
    DeadRoute,
    /// An instrument that should be moving and is not. These cost the most,
    /// precisely because absence is not an event.
    SilentSubsystem,
    /// An invariant that stays breached across evaluations.
    InvariantBreach,
    /// The build/deploy path itself is stuck.
    BuildDeploy,
    /// Free disk is below the floor, or falling fast enough to reach it soon.
    DiskPressure,
    /// A GitHub Actions workflow is failing on the default branch and has
    /// stayed failing. The one detector whose instrument lives OUTSIDE this
    /// machine — see [`detect_ci`] for why that changes the failure modes.
    CiFailure,
}

impl DetectorKind {
    pub fn slug(self) -> &'static str {
        match self {
            DetectorKind::Http5xx => "5xx",
            DetectorKind::Latency => "latency",
            DetectorKind::DeadRoute => "dead-route",
            DetectorKind::SilentSubsystem => "silent",
            DetectorKind::InvariantBreach => "invariant",
            DetectorKind::BuildDeploy => "build",
            DetectorKind::DiskPressure => "disk",
            DetectorKind::CiFailure => "ci",
        }
    }

    /// The board type. Never `code`: an auto-filed fault has no merged commit
    /// to claim, so a `code` card could only ever be closed by acknowledging a
    /// gate that is not true (ethos rule 3 — 1,143 of 1,215 cards were `code`
    /// and most of them could not exit honestly). `investigation` and
    /// `blocker` both close on "outcome recorded on the item", which is a
    /// sentence the fixing lane can write truthfully either way.
    pub fn item_type(self) -> &'static str {
        match self {
            DetectorKind::BuildDeploy
            | DetectorKind::SilentSubsystem
            | DetectorKind::DiskPressure
            | DetectorKind::CiFailure => "blocker",
            _ => "investigation",
        }
    }

    pub fn all() -> [DetectorKind; 8] {
        [
            DetectorKind::Http5xx,
            DetectorKind::Latency,
            DetectorKind::DeadRoute,
            DetectorKind::SilentSubsystem,
            DetectorKind::InvariantBreach,
            DetectorKind::BuildDeploy,
            DetectorKind::DiskPressure,
            DetectorKind::CiFailure,
        ]
    }
}

/// One fault worth a card.
#[derive(Debug, Clone)]
pub struct Finding {
    pub kind: DetectorKind,
    /// Stable across restarts and across reworded messages. This is the dedupe
    /// key and the `source_ref`; if it moves, the same fault files twice.
    pub signature: String,
    /// COMPUTED from the signature. Never a model call.
    pub title: String,
    /// Ordered evidence, rendered into the card body as `key: value` lines.
    pub evidence: Vec<(String, String)>,
    /// The exact query/command that re-checks this. The card must let a lane
    /// start from evidence rather than from a description of evidence.
    pub recheck: String,
    /// The lane best placed to fix it, or None for unowned.
    pub owner: Option<String>,
    pub count: u64,
    pub last_ts: f64,
}

/// A fault we deliberately did not file, and why. Published, always.
#[derive(Debug, Clone)]
pub struct Suppressed {
    pub kind: DetectorKind,
    pub signature: String,
    pub reason: String,
}

/// What one tick did. Held in memory for `/api/debug/autofix`; the durable
/// record is the `session_events` rows and the cards themselves.
#[derive(Debug, Clone, Default)]
pub struct AutofixReport {
    pub at: f64,
    pub enabled: bool,
    pub signatures_seen: Vec<(String, String)>,
    pub filed: Vec<(String, String)>,
    pub already_filed: Vec<String>,
    pub suppressed: Vec<(String, String, String)>,
    pub quiet_noted: Vec<(String, String)>,
    pub errors: Vec<String>,
    pub took_ms: f64,
}

static LAST_REPORT: std::sync::OnceLock<std::sync::RwLock<Option<AutofixReport>>> =
    std::sync::OnceLock::new();

fn last_report_cell() -> &'static std::sync::RwLock<Option<AutofixReport>> {
    LAST_REPORT.get_or_init(|| std::sync::RwLock::new(None))
}

pub fn last_report() -> Option<AutofixReport> {
    last_report_cell().read().ok().and_then(|g| g.clone())
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Detector 1 — 5xx.
// ---------------------------------------------------------------------------

/// Group every 5xx in the window by the SAME key `/api/logs/analyze` groups
/// by: `(status, method, family, normalize_target(path))`. Reused rather than
/// re-derived — `normalize_target` is the route-table-aware collapse, and a
/// second copy of it would drift the first time somebody adds a route.
///
/// Excluded by construction:
/// - **501 and 503.** These are the honest-degradation codes: they name the
///   absent thing and how to supply it (`/api/email/search` on an unconnected
///   account, `/api/torrents` with aria2c down, and now the iTerm2 send path).
///   Filing them would put a card on the board that no lane can act on.
/// - anything matching `AMUX_AUTOFIX_IGNORE`.
pub fn detect_5xx(conn: &Connection, now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let cutoff = now - window_h() * 3600.0;
    let mut groups: BTreeMap<(i64, String, String, String), Group> = BTreeMap::new();
    let mut suppressed = Vec::new();
    let q = conn.prepare(
        "SELECT ts, method, path, family, status, latency_ms, client_ip, amux_session, \
                worker, error_body \
         FROM _amux_request_log WHERE status >= 500 AND ts >= ?1 ORDER BY ts ASC LIMIT 200000",
    );
    let mut stmt = match q {
        Ok(s) => s,
        Err(e) => return (vec![], vec![sup(DetectorKind::Http5xx, "query", &e.to_string())]),
    };
    let rows = stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, f64>(5)?,
            r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            r.get::<_, Option<String>>(7)?.unwrap_or_default(),
            r.get::<_, Option<String>>(8)?.unwrap_or_default(),
            r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        ))
    });
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return (vec![], vec![sup(DetectorKind::Http5xx, "query", &e.to_string())]),
    };
    for row in rows.flatten() {
        let (ts, method, path, family, status, latency, ip, session, worker, body) = row;
        // The honest-degradation gate, applied BEFORE grouping so a suppressed
        // code cannot dilute a real group's sample.
        if status == 501 || status == 503 {
            suppressed.push(sup(
                DetectorKind::Http5xx,
                &format!("5xx|{status}|{method}|{}", rl::normalize_target(&path)),
                &format!(
                    "{status} is an honest degradation — the response names what is missing; \
                     not a defect to file"
                ),
            ));
            continue;
        }
        let target = rl::normalize_target(&path);
        let key = (status, method.clone(), family.clone(), target.clone());
        let g = groups.entry(key).or_insert_with(|| Group {
            count: 0,
            first_ts: ts,
            last_ts: ts,
            clients: Default::default(),
            workers: Default::default(),
            sample_path: path.clone(),
            sample_body: String::new(),
            max_latency: 0.0,
        });
        g.count += 1;
        g.last_ts = ts;
        g.max_latency = g.max_latency.max(latency);
        g.clients.insert(rl::client_identity(&session, &ip));
        if !worker.is_empty() {
            g.workers.insert(worker);
        }
        // Prefer the newest row that actually carries a body — same rule
        // /analyze uses to pick its sample.
        if !body.is_empty() {
            g.sample_body = body;
            g.sample_path = path;
        }
    }

    let mut out = Vec::new();
    for ((status, method, family, target), g) in groups {
        let signature = format!("5xx|{status}|{method}|{family}|{target}");
        // COMPUTED title: the signature in English, plus the count. No model.
        let title = format!("{method} {target} → HTTP {status} ({}x)", g.count);
        let recheck = format!(
            "curl -sk \"$AMUX_URL/api/logs/analyze?since_h={}\" | python3 -c \"import json,sys; \
             [print(json.dumps(x,indent=2)) for x in json.load(sys.stdin)['groups'] \
             if x['status']=={status} and x['method']=='{method}' and x['target']=='{target}']\"",
            window_h() as i64
        );
        let evidence = vec![
            ("verdict".into(), format!(
                "{method} {target} answered HTTP {status} {} time(s) from {} distinct client(s). \
                 A 5xx is amux failing, not amux declining — if this is really a refusal, the fix \
                 is the STATUS CODE, not the caller.",
                g.count,
                g.clients.len()
            )),
            ("sample_request".into(), format!("{method} {}", g.sample_path)),
            ("error_body".into(), truncate(&g.sample_body, 800)),
            ("first_seen".into(), rl::local_when(g.first_ts)),
            ("last_seen".into(), rl::local_when(g.last_ts)),
            ("count".into(), g.count.to_string()),
            ("distinct_clients".into(), format!(
                "{} ({})",
                g.clients.len(),
                g.clients.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            )),
            ("slowest_ms".into(), format!("{:.1}", g.max_latency)),
        ];
        out.push(Finding {
            kind: DetectorKind::Http5xx,
            signature,
            title,
            evidence,
            recheck,
            owner: g.workers.iter().next().cloned(),
            count: g.count,
            last_ts: g.last_ts,
        });
    }
    (out, suppressed)
}

#[derive(Default)]
struct Group {
    count: u64,
    first_ts: f64,
    last_ts: f64,
    clients: std::collections::BTreeSet<String>,
    workers: std::collections::BTreeSet<String>,
    sample_path: String,
    sample_body: String,
    max_latency: f64,
}

// ---------------------------------------------------------------------------
// Detector 2 — latency.
// ---------------------------------------------------------------------------

/// Two shapes, because they fail differently.
///
/// **Regression**: a family's window p95 above `mult ×` its own trailing p95.
/// Compared against the family's OWN history, never a fleet-wide number — an
/// upload family is legitimately slow and a `/health` family is not, and one
/// absolute threshold would either scream at the first or never fire for the
/// second. Both sides need `min_samples`, because a p95 over four requests is
/// one request wearing a percentile.
///
/// **Outlier**: a single request past `outlier_ms`, which is absurd whatever
/// the norm is. Filed separately because the mechanism is different — a 27s
/// upload chunk is not "the upload family got slower", it is one request that
/// went wrong, and folding it into a percentile hides it.
///
/// The percentile is `rl::percentile_sorted` — the same nearest-rank function
/// `/api/logs/stats` reports, so a card and the endpoint can never quote
/// different p95s for the same window.
pub fn detect_latency(conn: &Connection, now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let w_start = now - window_h() * 3600.0;
    let b_start = now - baseline_h() * 3600.0;
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    let min_n = latency_min_samples();

    let mut per_family: BTreeMap<String, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT family, latency_ms, ts FROM _amux_request_log WHERE ts >= ?1 LIMIT 400000",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![b_start], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?))
        }) {
            for (fam, ms, ts) in rows.flatten() {
                let e = per_family.entry(fam).or_default();
                if ts >= w_start {
                    e.0.push(ms);
                } else {
                    e.1.push(ms);
                }
            }
        }
    }
    for (fam, (mut win, mut base)) in per_family {
        if (win.len() as i64) < min_n {
            continue; // Not enough traffic to say anything. Not a suppression:
                      // there is no candidate here, only silence.
        }
        if (base.len() as i64) < min_n {
            suppressed.push(sup(
                DetectorKind::Latency,
                &format!("latency|p95|{fam}"),
                &format!(
                    "baseline has {} samples (<{min_n}) — no trailing norm to compare against yet",
                    base.len()
                ),
            ));
            continue;
        }
        win.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        base.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_w = rl::percentile_sorted(&win, 0.95);
        let p95_b = rl::percentile_sorted(&base, 0.95);
        let mult = latency_mult();
        if p95_b <= 0.0 || p95_w < p95_b * mult {
            continue;
        }
        let signature = format!("latency|p95|{fam}");
        let title = format!(
            "{fam} p95 {:.0}ms — {:.1}x its trailing norm",
            p95_w,
            p95_w / p95_b
        );
        out.push(Finding {
            kind: DetectorKind::Latency,
            signature,
            title,
            evidence: vec![
                ("verdict".into(), format!(
                    "{fam} p95 over the last {:.1}h is {p95_w:.0}ms against a {:.0}h trailing p95 \
                     of {p95_b:.0}ms ({:.1}x). Threshold: {mult}x with at least {min_n} samples \
                     on both sides.",
                    window_h(), baseline_h(), p95_w / p95_b
                )),
                ("window_samples".into(), win.len().to_string()),
                ("baseline_samples".into(), base.len().to_string()),
                ("window_p50_p95_max".into(), format!(
                    "{:.0} / {:.0} / {:.0} ms",
                    rl::percentile_sorted(&win, 0.5), p95_w,
                    win.last().copied().unwrap_or(0.0)
                )),
                ("baseline_p50_p95".into(), format!(
                    "{:.0} / {p95_b:.0} ms", rl::percentile_sorted(&base, 0.5)
                )),
                ("percentile_method".into(),
                 "nearest-rank over the sorted window (same function /api/logs/stats reports)".into()),
            ],
            recheck: format!(
                "curl -sk \"$AMUX_URL/api/logs/stats?since_h={}\" | python3 -c \"import json,sys; \
                 d=json.load(sys.stdin); print([f for f in d['families'] if f.get('family')=='{fam}'])\"",
                window_h() as i64
            ),
            owner: None,
            count: win.len() as u64,
            last_ts: now,
        });
    }

    // Single-request outliers: one card per (method, target), not per request.
    let mut seen: BTreeMap<(String, String), (u64, f64, f64, String)> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT method, path, latency_ms, ts, status FROM _amux_request_log \
         WHERE ts >= ?1 AND latency_ms >= ?2 ORDER BY latency_ms DESC LIMIT 2000",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![w_start, outlier_ms()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        }) {
            for (method, path, ms, ts, status) in rows.flatten() {
                let target = rl::normalize_target(&path);
                let e = seen
                    .entry((method.clone(), target))
                    .or_insert((0, 0.0, ts, format!("{method} {path} → {status}")));
                e.0 += 1;
                e.1 = e.1.max(ms);
                e.2 = e.2.max(ts);
            }
        }
    }
    for ((method, target), (n, worst, last_ts, sample)) in seen {
        out.push(Finding {
            kind: DetectorKind::Latency,
            signature: format!("latency|outlier|{method}|{target}"),
            title: format!("{method} {target} took {:.1}s ({n}x over {:.0}s)", worst / 1000.0, outlier_ms() / 1000.0),
            evidence: vec![
                ("verdict".into(), format!(
                    "{n} request(s) to {method} {target} exceeded {:.0}s in the last {:.1}h; \
                     worst {:.1}s. This is not a percentile shift — it is individual requests \
                     going wrong, so look at the request, not the family.",
                    outlier_ms() / 1000.0, window_h(), worst / 1000.0
                )),
                ("worst_ms".into(), format!("{worst:.0}")),
                ("sample_request".into(), sample),
                ("threshold_ms".into(), format!("{:.0}", outlier_ms())),
                ("last_seen".into(), rl::local_when(last_ts)),
            ],
            recheck: format!(
                "sqlite3 ~/.amux/amux.db \"SELECT datetime(ts,'unixepoch','localtime'), \
                 latency_ms, status, path FROM _amux_request_log WHERE method='{method}' \
                 AND latency_ms >= {:.0} ORDER BY ts DESC LIMIT 20;\"",
                outlier_ms()
            ),
            owner: None,
            count: n,
            last_ts,
        });
    }
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// Detector 3 — dead routes.
// ---------------------------------------------------------------------------

/// A 404/405 that a BROWSER asked for. The user-agent is the discriminator and
/// it is the right one: a curl 404 is usually a human or an agent probing, and
/// filing those would bury the board under every mistyped path anyone ever
/// tried (`/api/credits`, `/api/burn`, `/api/canary/vendor-credit` were all in
/// tonight's window, all from curl, none of them defects). A path the SPA
/// FETCHED is a feature that is broken for every user of the dashboard, and
/// every one of tonight's was invisible until a person happened to notice.
///
/// The verdict is `/api/logs/analyze`'s own annotation, verbatim: which methods
/// ARE mounted (`routed_methods_at`), the nearest siblings (`nearest_routes`),
/// and for a 405 the computed sentence (`verdict_405`) that distinguishes
/// "wrong method on a real path" from "no such path wearing the GET-only SPA
/// catch-all's 405". That last cell is the whole reason the annotation exists,
/// and re-deriving it here would be the second grouping this file refuses to
/// write.
pub fn detect_dead_routes(conn: &Connection, now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let cutoff = now - window_h() * 3600.0;
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    /// (count, first_ts, last_ts, sample_path, ever_called_by_a_browser)
    type DeadHits = (u64, f64, f64, String, bool);
    let mut groups: BTreeMap<(i64, String, String), DeadHits> = BTreeMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT ts, method, path, status, user_agent FROM _amux_request_log \
         WHERE status IN (404, 405) AND ts >= ?1 ORDER BY ts ASC LIMIT 200000",
    ) else {
        return (out, suppressed);
    };
    let Ok(rows) = stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        ))
    }) else {
        return (out, suppressed);
    };
    for (ts, method, path, status, ua) in rows.flatten() {
        let target = rl::normalize_target(&path);
        let browser = ua.contains("Mozilla/");
        let e = groups
            .entry((status, method.clone(), target))
            .or_insert((0, ts, ts, path.clone(), false));
        e.0 += 1;
        e.2 = ts;
        e.4 |= browser;
        e.3 = path;
    }
    for ((status, method, target), (count, first, last, sample_path, browser)) in groups {
        let signature = format!("dead-route|{status}|{method}|{target}");
        if !browser {
            suppressed.push(sup(
                DetectorKind::DeadRoute,
                &signature,
                "no browser ever requested this path — a curl/agent 404 is a probe, not a \
                 broken feature",
            ));
            continue;
        }
        if (count as i64) < dead_route_min_hits() {
            suppressed.push(sup(
                DetectorKind::DeadRoute,
                &signature,
                &format!("{count} hit(s) < {} — one fetch is not yet a pattern", dead_route_min_hits()),
            ));
            continue;
        }
        let routed = rl::routed_methods_at(&sample_path);
        let near = rl::nearest_routes(&sample_path, 3);
        let verdict = if status == 405 {
            rl::verdict_405(&method, &target, &routed, &sample_path)
        } else if routed.is_empty() {
            format!(
                "{method} {target}: no route is mounted at this path in the current build, and \
                 the SPA fetches it. Nearest routes: {}",
                if near.is_empty() { "none".into() } else { near.join(", ") }
            )
        } else {
            format!(
                "{method} {target}: the path IS routed (methods: {}) but answered 404 — the \
                 handler ran and found nothing, so this is a data/id problem, not a mount \
                 problem",
                routed.join(", ")
            )
        };
        out.push(Finding {
            kind: DetectorKind::DeadRoute,
            signature,
            title: format!("SPA calls {method} {target} → {status} ({count}x)"),
            evidence: vec![
                ("verdict".into(), verdict),
                ("routed_methods".into(), if routed.is_empty() { "none".into() } else { routed.join(", ") }),
                ("nearest_routes".into(), if near.is_empty() { "none".into() } else { near.join(", ") }),
                ("sample_request".into(), format!("{method} {sample_path}")),
                ("count".into(), count.to_string()),
                ("first_seen".into(), rl::local_when(first)),
                ("last_seen".into(), rl::local_when(last)),
                ("called_by".into(), "a browser (Mozilla/* user-agent) — i.e. the dashboard".into()),
            ],
            recheck: format!(
                "curl -sk \"$AMUX_URL/api/debug/routes\" | grep -F '{target}' ; \
                 curl -sk \"$AMUX_URL/api/logs/analyze?since_h={}\"",
                window_h() as i64
            ),
            owner: None,
            count,
            last_ts: last,
        });
    }
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// Detector 4 — silent subsystems.
// ---------------------------------------------------------------------------

/// The expensive class: an instrument that should be moving and is not.
/// Nothing here fires on an event, because there is no event — that is the
/// point. Each check states the deadline it compared against, so the card can
/// be argued with instead of believed.
///
/// Both checks read tables the SERVER owns, deliberately: a silent-subsystem
/// detector that needed a worker to answer would go silent exactly when the
/// fleet did.
pub fn detect_silent(conn: &Connection, now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let mut out = Vec::new();
    let suppressed = Vec::new();

    // (a) Schedules whose next_run has passed with no run recorded since.
    let overdue_s = schedule_overdue_min() * 60.0;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, title, session, next_run, \
                (SELECT MAX(ran_at) FROM schedule_runs r WHERE r.schedule_id = s.id) \
         FROM schedules s \
         WHERE s.deleted IS NULL AND s.enabled = 1 AND s.next_run IS NOT NULL AND s.next_run != ''",
    ) {
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        });
        let mut late: Vec<(String, String, String, f64)> = Vec::new();
        if let Ok(rows) = rows {
            for (id, title, session, next_run, last_ran) in rows.flatten() {
                let Some(due) = parse_ts(&next_run) else { continue };
                let overdue_by = now - due;
                if overdue_by < overdue_s {
                    continue;
                }
                // A run recorded AFTER the due time means it fired and
                // next_run simply has not been advanced yet — not a stall.
                if last_ran.is_some_and(|t| t as f64 >= due) {
                    continue;
                }
                late.push((id, title, session, overdue_by));
            }
        }
        if !late.is_empty() {
            late.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
            let worst = late[0].3 / 60.0;
            let listing = late
                .iter()
                .take(10)
                .map(|(id, t, s, o)| format!("  {id} [{s}] {t} — {:.0} min late", o / 60.0))
                .collect::<Vec<_>>()
                .join("\n");
            out.push(Finding {
                kind: DetectorKind::SilentSubsystem,
                // Signature does NOT include the count: the subsystem is the
                // fault, not each schedule. One card, not N.
                signature: "silent|schedules|overdue".into(),
                title: format!("{} schedule(s) overdue, worst by {:.0} min", late.len(), worst),
                evidence: vec![
                    ("verdict".into(), format!(
                        "{} enabled schedule(s) are past next_run by more than {:.0} min with no \
                         schedule_runs row at or after their due time. The firing loop is not \
                         firing them — this is absence, so nothing else will report it.",
                        late.len(), schedule_overdue_min()
                    )),
                    ("overdue".into(), listing),
                    ("threshold_min".into(), format!("{:.0}", schedule_overdue_min())),
                ],
                recheck: "sqlite3 ~/.amux/amux.db \"SELECT id, next_run, (SELECT MAX(ran_at) FROM \
                          schedule_runs r WHERE r.schedule_id=s.id) FROM schedules s WHERE \
                          deleted IS NULL AND enabled=1 ORDER BY next_run;\"".into(),
                owner: None,
                count: late.len() as u64,
                last_ts: now,
            });
        }
    }

    // (b) A steering queue that is not draining.
    let stale_s = steering_stale_min() * 60.0;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT session, COUNT(*), MIN(queued_at) FROM steering_queue GROUP BY session",
    ) {
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
        });
        if let Ok(rows) = rows {
            for (session, n, oldest) in rows.flatten() {
                let age = now - oldest;
                if age < stale_s {
                    continue;
                }
                out.push(Finding {
                    kind: DetectorKind::SilentSubsystem,
                    signature: format!("silent|steering|{session}"),
                    title: format!("{session}: steering queue stuck, oldest {:.0} min", age / 60.0),
                    evidence: vec![
                        ("verdict".into(), format!(
                            "{n} message(s) queued for {session}; the oldest has waited {:.0} min \
                             (deadline {:.0} min). Queued is not delivered — every one of these \
                             was reported to its sender as accepted.",
                            age / 60.0, steering_stale_min()
                        )),
                        ("queued".into(), n.to_string()),
                        ("oldest_queued_at".into(), rl::local_when(oldest)),
                        ("threshold_min".into(), format!("{:.0}", steering_stale_min())),
                    ],
                    recheck: format!(
                        "sqlite3 ~/.amux/amux.db \"SELECT id, datetime(queued_at,'unixepoch',\
                         'localtime'), substr(text,1,80) FROM steering_queue WHERE \
                         session='{session}' ORDER BY queued_at;\""
                    ),
                    owner: Some(session),
                    count: n as u64,
                    last_ts: now,
                });
            }
        }
    }
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// Detector 5 — invariant breaches.
// ---------------------------------------------------------------------------

/// The invariants monitor already writes typed incidents with `occurrences`
/// and `resolved_at`. We file only the ones still open AND seen at least
/// `min_occurrences` times: a flapping invariant is noise, and one card per
/// flap is how a board stops being read.
pub fn detect_invariants(conn: &Connection, now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    let min_occ = invariant_min_occurrences();
    let Ok(mut stmt) = conn.prepare(
        "SELECT invariant_id, entity_key, status, first_seen, last_seen, occurrences, \
                expected, observed \
         FROM _amux_invariant_incident WHERE resolved_at IS NULL AND status = 'fail' \
         ORDER BY occurrences DESC LIMIT 200",
    ) else {
        return (out, suppressed);
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, f64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
        ))
    }) else {
        return (out, suppressed);
    };
    for (id, entity, _status, first, last, occ, expected, observed) in rows.flatten() {
        let sig_entity = if entity.is_empty() { "fleet".to_string() } else { entity.clone() };
        let signature = format!("invariant|{id}|{sig_entity}");
        if occ < min_occ {
            suppressed.push(sup(
                DetectorKind::InvariantBreach,
                &signature,
                &format!("{occ} occurrence(s) < {min_occ} — a flapping invariant is noise, not a card"),
            ));
            continue;
        }
        out.push(Finding {
            kind: DetectorKind::InvariantBreach,
            signature,
            title: format!("invariant {id} failing ({occ}x) — {sig_entity}"),
            evidence: vec![
                ("verdict".into(), format!(
                    "Invariant `{id}` has been failing for {sig_entity} across {occ} evaluations \
                     and has not self-healed. Threshold to file: {min_occ}.",
                )),
                ("expected".into(), truncate(&expected, 500)),
                ("observed".into(), truncate(&observed, 500)),
                ("first_seen".into(), rl::local_when(first)),
                ("last_seen".into(), rl::local_when(last)),
                ("occurrences".into(), occ.to_string()),
            ],
            recheck: format!(
                "curl -sk \"$AMUX_URL/api/health/invariants\" | python3 -c \"import json,sys; \
                 print([r for r in json.load(sys.stdin).get('results',[]) if r.get('id')=='{id}'])\""
            ),
            owner: None,
            count: occ as u64,
            last_ts: last.max(now - 1.0),
        });
    }
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// Detector 6 — build / deploy.
// ---------------------------------------------------------------------------

/// The deploy path itself. Two failure shapes, both of which let a whole fleet
/// believe its work shipped when it did not:
///
/// - the auto-builder is failing (its log records `BUILD FAILED` and does not
///   advance the stamp, by design — so a broken main is invisible unless
///   somebody reads that file);
/// - the stamp has not moved for hours while committed Rust source HAS.
///
/// Reads the builder's own log and stamp — filesystem, not a worker. If the
/// builder is dead the log simply stops, which the stamp-age check catches.
pub fn detect_build(_conn: &Connection, now: f64, home: &std::path::Path) -> (Vec<Finding>, Vec<Suppressed>) {
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    let stamp = home.join("rust-build-stamp");
    let log = home.join("logs").join("rust-auto-build.log");

    let tail = std::fs::read_to_string(&log).unwrap_or_default();
    let tail: String = {
        let lines: Vec<&str> = tail.lines().collect();
        lines[lines.len().saturating_sub(80)..].join("\n")
    };
    let failures = tail.matches("BUILD FAILED").count();
    // Only the CURRENT streak counts: a failure followed by an install is a
    // build that was fixed, and filing it would be filing history.
    let fixed_since = tail
        .rfind("== installed")
        .zip(tail.rfind("BUILD FAILED"))
        .map(|(i, f)| i > f)
        .unwrap_or(false);
    if failures > 0 && !fixed_since {
        let last = tail
            .lines()
            .rev()
            .find(|l| l.contains("BUILD FAILED"))
            .unwrap_or_default()
            .to_string();
        out.push(Finding {
            kind: DetectorKind::BuildDeploy,
            signature: "build|failing".into(),
            title: format!("auto-build failing — {failures} failure(s), none fixed since"),
            evidence: vec![
                ("verdict".into(),
                 "The Rust auto-builder is failing and has not installed a build since. The \
                  running server keeps the last good binary BY DESIGN, so nothing looks broken: \
                  every session's committed work is simply not deploying, silently.".into()),
                ("last_failure_line".into(), last),
                ("failures_in_tail".into(), failures.to_string()),
                ("log".into(), log.display().to_string()),
            ],
            recheck: "tail -40 ~/.amux/logs/rust-auto-build.log; \
                      CARGO_TARGET_DIR=/tmp/amux-check cargo build --release -p amux-server".into(),
            owner: None,
            count: failures as u64,
            last_ts: now,
        });
    }

    if let Ok(meta) = std::fs::metadata(&stamp) {
        if let Ok(modified) = meta.modified() {
            let age_h = modified
                .elapsed()
                .map(|d| d.as_secs_f64() / 3600.0)
                .unwrap_or(0.0);
            if age_h > build_stale_h() && out.is_empty() {
                // Only meaningful when source has actually moved; otherwise a
                // quiet night looks like a broken deploy. Absent a repo to
                // ask, we suppress with the reason rather than guessing.
                suppressed.push(sup(
                    DetectorKind::BuildDeploy,
                    "build|stamp-stale",
                    &format!(
                        "stamp is {age_h:.1}h old (>{:.1}h) but the builder reports no failure — \
                         cannot distinguish a quiet night from a stalled builder without asking \
                         the repo; not filing on a guess",
                        build_stale_h()
                    ),
                ));
            }
        }
    }
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// Disk pressure (AMUX-2700, after the 2026-08-10 ENOSPC incident).
// ---------------------------------------------------------------------------

/// Free space below this files a card. Default 50GB: high enough that a fleet
/// of 50 sessions plus a Rust build (a debug target tree is 10-15GB on its own)
/// has room to finish what it started, rather than a floor that is only crossed
/// once writes are already failing.
fn disk_min_free_gb() -> f64 {
    env_f64("AMUX_DISK_MIN_FREE_GB", 50.0).max(1.0)
}
/// The RATE trigger. An absolute floor is a lagging indicator: on 2026-08-10
/// the volume went from comfortable to 741MB with nothing reported, because
/// every sample before the last one was fine. Falling faster than this — over a
/// window long enough to not be a single build — is the signal that would have
/// fired hours earlier.
fn disk_fall_gb_per_h() -> f64 {
    env_f64("AMUX_DISK_FALL_GB_PER_H", 20.0).max(1.0)
}
/// Minimum span before a rate is a rate and not two adjacent samples.
fn disk_rate_window_min() -> f64 {
    env_f64("AMUX_DISK_RATE_WINDOW_MIN", 20.0).clamp(2.0, 24.0 * 60.0)
}

/// Free-space samples, newest last. **In memory, and therefore fiction across a
/// restart** — this process re-execs whenever anyone saves the binary, and the
/// ethos file is explicit that in-memory state which survives nothing is the
/// same as no state. That is tolerable HERE and only here: the absolute floor
/// is a pure function of one sample and keeps working through a restart, so a
/// restart costs the rate trigger its window (~20 min) and costs the floor
/// nothing. Persisting a sample every tick would itself be an append-only table
/// with no retention, which is the bug this detector exists to report.
fn disk_samples() -> &'static std::sync::RwLock<Vec<(f64, u64)>> {
    static CELL: std::sync::OnceLock<std::sync::RwLock<Vec<(f64, u64)>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::RwLock::new(Vec::new()))
}

fn du_top(paths: &[std::path::PathBuf], limit: usize) -> Vec<(String, u64)> {
    let mut sized: Vec<(String, u64)> = paths
        .iter()
        .filter(|p| p.exists())
        .filter_map(|p| {
            let out = std::process::Command::new("/usr/bin/du")
                .args(["-skx"])
                .arg(p)
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let kb: u64 = text.split_whitespace().next()?.parse().ok()?;
            Some((p.display().to_string(), kb * 1024))
        })
        .collect();
    sized.sort_by(|a, b| b.1.cmp(&a.1));
    sized.truncate(limit);
    sized
}

/// Candidate consumers. Deliberately NOT just `~/.amux`: on the night this was
/// written the top three consumers were a Trash full of screen recordings, a
/// pile of per-agent cargo target dirs under /private/tmp, and 24 hourly Time
/// Machine snapshots — none of which live under amux's home. A detector that
/// could only see its own directory would have named the innocent party.
fn disk_candidates(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(home) {
        for e in rd.flatten() {
            if e.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                v.push(e.path());
            }
        }
    }
    // Build trees, wherever sessions put them.
    if let Ok(rd) = std::fs::read_dir("/private/tmp") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.contains("target") && e.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                v.push(e.path());
            }
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        for p in ["Library/Caches", ".cache", ".claude", ".Trash", ".npm"] {
            v.push(std::path::Path::new(&h).join(p));
        }
    }
    v
}

/// Count APFS local snapshots. **This is the instrument that was missing.**
/// On 2026-08-10 roughly 450GB was deleted and free space moved by 8GB, because
/// 24 hourly Time Machine snapshots were pinning every deleted block. Without
/// this number in the evidence, the only available conclusion is "we deleted
/// the wrong things" and the next action is deleting more of the right ones,
/// which also does nothing. Snapshots are purgeable by macOS under pressure and
/// thinnable with `tmutil`, so this line turns an unexplainable non-recovery
/// into a one-command fix.
fn local_snapshot_count() -> Option<usize> {
    let out = std::process::Command::new("/usr/bin/tmutil")
        .args(["listlocalsnapshots", "/"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).matches("com.apple.TimeMachine").count())
}

pub fn detect_disk(now: f64, home: &std::path::Path) -> (Vec<Finding>, Vec<Suppressed>) {
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    const GB: f64 = 1_073_741_824.0;

    let Some(free) = crate::runtime_jobs::storage::disk_free_bytes(home) else {
        suppressed.push(sup(
            DetectorKind::DiskPressure,
            "disk|unreadable",
            "df returned nothing — cannot measure free space, so not filing either way",
        ));
        return (out, suppressed);
    };
    let free_gb = free as f64 / GB;

    // Record the sample, keep a bounded window.
    let (mut rate_gb_h, mut span_min) = (0.0f64, 0.0f64);
    if let Ok(mut s) = disk_samples().write() {
        s.push((now, free));
        let keep_from = now - 6.0 * 3600.0;
        s.retain(|(t, _)| *t >= keep_from);
        if let (Some(first), Some(last)) = (s.first().copied(), s.last().copied()) {
            span_min = (last.0 - first.0) / 60.0;
            if span_min >= disk_rate_window_min() {
                let dropped_gb = (first.1 as f64 - last.1 as f64) / GB;
                rate_gb_h = dropped_gb / (span_min / 60.0);
            }
        }
    }

    let floor = disk_min_free_gb();
    let falling = rate_gb_h >= disk_fall_gb_per_h();
    if free_gb >= floor && !falling {
        return (out, suppressed);
    }

    // Only now pay for `du` — a directory walk on every tick would be a
    // detector whose cost lands on the resource it is watching.
    let top = du_top(&disk_candidates(home), 8);
    let listing = top
        .iter()
        .map(|(p, b)| format!("  {:>8.1} GB  {p}", *b as f64 / GB))
        .collect::<Vec<_>>()
        .join("\n");
    let snaps = local_snapshot_count();

    let (signature, title, verdict) = if free_gb < floor {
        (
            "disk|below-floor".to_string(),
            format!("disk: {free_gb:.1} GB free, below the {floor:.0} GB floor"),
            "Free space is under the floor with a fleet running. Writes fail with ENOSPC \
             before anything else reports a fault: on 2026-08-10 this surfaced as transcript \
             writes failing, not as a disk alert."
                .to_string(),
        )
    } else {
        (
            "disk|falling".to_string(),
            format!("disk: falling {rate_gb_h:.0} GB/h — {free_gb:.1} GB left"),
            format!(
                "Free space is dropping at {rate_gb_h:.1} GB/h over the last {span_min:.0} min. \
                 At this rate the {floor:.0} GB floor is {:.1} h away. Nothing is broken yet — \
                 this is the lead time an absolute floor cannot give you.",
                ((free_gb - floor) / rate_gb_h).max(0.0)
            ),
        )
    };

    let mut evidence = vec![
        ("verdict".into(), verdict),
        ("free_gb".into(), format!("{free_gb:.1}")),
        ("floor_gb".into(), format!("{floor:.0} (AMUX_DISK_MIN_FREE_GB)")),
        (
            "fall_gb_per_h".into(),
            format!("{rate_gb_h:.1} over {span_min:.0} min (trigger: {:.0}, AMUX_DISK_FALL_GB_PER_H)", disk_fall_gb_per_h()),
        ),
        ("top_consumers".into(), format!("\n{listing}")),
    ];
    if let Some(n) = snaps {
        evidence.push((
            "apfs_local_snapshots".into(),
            format!(
                "{n} — READ THIS BEFORE DELETING ANYTHING. Each hourly Time Machine snapshot \
                 pins the blocks of every file deleted since it was taken, so with snapshots \
                 present, deleting files frees NOTHING until they age out (24h) or are thinned. \
                 On 2026-08-10, 450GB was deleted and free space moved 8GB for exactly this \
                 reason. Thin them first: sudo tmutil thinlocalsnapshots / 500000000000 4"
            ),
        ));
    }

    out.push(Finding {
        kind: DetectorKind::DiskPressure,
        signature,
        title,
        evidence,
        recheck: "df -h /System/Volumes/Data; tmutil listlocalsnapshots / | wc -l; \
                  curl -sk $AMUX_URL/api/debug/storage"
            .into(),
        owner: None,
        count: 1,
        last_ts: now,
    });
    (out, suppressed)
}

// ---------------------------------------------------------------------------
// The one filing path.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Detector 8 — CI / deploy (AMUX-2780).
//
// The motivating incident: `deploy-cloud.yml` failed 31 consecutive times over
// three days and NOBODY SAW IT. cloud.amux.io received no deploy at all in that
// window. Every push printed a red X that no instrument in amux read, because
// amux had no instrument that read GitHub at all. The defect is not the SSH key
// that broke — that is one incident. The defect is that CI can fail every night
// and nothing tells anyone.
//
// Two things make this detector different from the other seven, and both are
// worth stating because they change what can go wrong:
//
//  1. **Its instrument is off-machine.** Every other detector reads SQLite or
//     the local filesystem, so "no data" means "nothing happened". Here "no
//     data" can also mean gh is missing, unauthenticated, rate-limited, or the
//     network is down — i.e. the detector is blind, which looks EXACTLY like
//     the all-clear. So every one of those cases emits a `Suppressed` naming
//     the reason. A blind CI detector that reports silence is the precise
//     failure this file exists to end.
//  2. **Failure here is a LEVEL, not an absence.** The ethos warns that
//     picking a threshold is the tell that you are guessing, and it is right:
//     `ci_min_failures()` is a guess. What is NOT a guess is the dedupe key —
//     see the streak anchor below, which is what turns "red every night" into
//     one card instead of thirty.
// ---------------------------------------------------------------------------

/// One completed workflow run, as GitHub reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct CiRun {
    pub repo: String,
    /// `workflowName` — the human name, which is what a card should say.
    pub workflow: String,
    pub run_id: i64,
    /// `completed` / `in_progress` / `queued`.
    pub status: String,
    /// `success` / `failure` / `cancelled` / `timed_out` / `startup_failure` / …
    pub conclusion: String,
    /// `push` / `pull_request` / `schedule` / `workflow_dispatch` / …
    pub event: String,
    pub branch: String,
    /// Computed at fetch time against the repo's real default branch, so the
    /// decision function stays pure and testable without a network call.
    pub on_default_branch: bool,
    pub created_at: f64,
    pub url: String,
    /// Populated only for the newest failing run of a failing workflow — it
    /// costs one extra API call each, so it is not fetched for history.
    pub failing_step: Option<String>,
}

/// Conclusions that mean "this run failed".
fn ci_is_failure(c: &str) -> bool {
    matches!(c, "failure" | "timed_out" | "startup_failure")
}

/// Conclusions that carry NO evidence either way and must not break a streak.
///
/// `cancelled` is here because the owner cancelling a run is not a fault (the
/// task's own suppression requirement), and because letting it break the streak
/// would reset the anchor and refile a card mid-outage — the 08-10 16:42 run in
/// the motivating incident was exactly this.
fn ci_is_inconclusive(c: &str) -> bool {
    matches!(c, "cancelled" | "skipped" | "neutral" | "stale" | "action_required" | "")
}

fn ci_sanitize(s: &str) -> String {
    s.replace('|', "/").trim().to_string()
}

fn ci_when(ts: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_else(|| format!("{ts:.0}"))
}

/// The decision. Pure: same runs in, same findings out, no clock beyond `now`
/// and no network — which is what lets the tests build a 31-failure streak and
/// a recovery without touching GitHub.
///
/// # The dedupe key is a STREAK ANCHOR, not the workflow
///
/// `ci|<repo>|<workflow>|<run id of the OLDEST failure in the current streak>`.
///
/// Keying on the workflow alone files once and then never again, so a workflow
/// that is fixed and breaks again months later stays silent forever. Keying on
/// the latest run files nightly, which is the thing the card asks us not to do.
/// The anchor is stable for as long as the outage lasts and changes exactly
/// when a green run splits one outage from the next — so a three-day outage is
/// one card, and a genuinely new outage after a fix is a genuinely new card.
///
/// The anchor's one dependency is that the last SUCCESS is still inside the
/// fetched window. When it is not, the anchor would silently drift older every
/// time a run ages out and refile on every drift; so that case gets the fixed
/// anchor `since-beyond-window` and says so in the evidence, rather than
/// pretending to a precision it does not have.
pub fn ci_findings(runs: &[CiRun], now: f64) -> (Vec<Finding>, Vec<Suppressed>) {
    let mut out = Vec::new();
    let mut suppressed = Vec::new();
    let min_failures = ci_min_failures();

    // Group by (repo, workflow). BTreeMap so the output order is deterministic
    // — a detector whose findings reorder between ticks is one nobody can diff.
    let mut groups: BTreeMap<(String, String), Vec<&CiRun>> = BTreeMap::new();
    for r in runs {
        groups.entry((r.repo.clone(), ci_sanitize(&r.workflow))).or_default().push(r);
    }

    for ((repo, workflow), all) in groups {
        // --- eligibility, with the exclusions PUBLISHED -----------------
        let mut n_pr = 0usize;
        let mut n_branch = 0usize;
        let mut n_running = 0usize;
        let mut eligible: Vec<&CiRun> = Vec::new();
        for r in &all {
            if r.event == "pull_request" || r.event == "pull_request_target" {
                n_pr += 1;
                continue;
            }
            if !r.on_default_branch {
                n_branch += 1;
                continue;
            }
            if r.status != "completed" {
                n_running += 1;
                continue;
            }
            eligible.push(r);
        }
        if n_pr + n_branch + n_running > 0 {
            suppressed.push(sup(
                DetectorKind::CiFailure,
                &format!("ci|{repo}|{workflow}"),
                &format!(
                    "excluded {n_pr} pull-request/fork run(s), {n_branch} non-default-branch \
                     run(s), {n_running} still-running run(s) — only completed default-branch \
                     runs are evidence about main"
                ),
            ));
        }

        // Newest first. Sorted here rather than trusting the caller: `gh`
        // happens to return newest-first today and a pure function that
        // silently depends on its input order is a bug waiting for a caller.
        eligible.sort_by(|a, b| {
            b.created_at
                .partial_cmp(&a.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.run_id.cmp(&a.run_id))
        });

        // Inconclusive runs are dropped ENTIRELY (not counted, not
        // streak-breaking) — see `ci_is_inconclusive`.
        let decisive: Vec<&&CiRun> = eligible.iter().filter(|r| !ci_is_inconclusive(&r.conclusion)).collect();
        let n_cancelled = eligible.len() - decisive.len();
        if n_cancelled > 0 {
            suppressed.push(sup(
                DetectorKind::CiFailure,
                &format!("ci|{repo}|{workflow}|cancelled"),
                &format!(
                    "{n_cancelled} cancelled/skipped run(s) ignored — a cancelled run is not a \
                     fault and must not break or reset the failure streak"
                ),
            ));
        }

        let Some(newest) = decisive.first() else { continue };
        if !ci_is_failure(&newest.conclusion) {
            // Green right now. Nothing to file. Whether an EXISTING card should
            // hear about it is `ci_recovered`'s job, and the answer is a note,
            // never a close.
            continue;
        }

        // --- the streak -------------------------------------------------
        let mut streak: Vec<&&CiRun> = Vec::new();
        let mut last_success: Option<&&CiRun> = None;
        for r in &decisive {
            if ci_is_failure(&r.conclusion) {
                streak.push(r);
            } else {
                last_success = Some(r);
                break;
            }
        }
        let count = streak.len() as i64;
        if count < min_failures {
            suppressed.push(sup(
                DetectorKind::CiFailure,
                &format!("ci|{repo}|{workflow}"),
                &format!(
                    "{count} consecutive failure(s), below AMUX_CI_MIN_FAILURES={min_failures} — \
                     not filed yet; a single red run is not distinguishable from a flake"
                ),
            ));
            continue;
        }

        let oldest = streak.last().copied().expect("streak is non-empty: count >= min_failures >= 1");
        let anchor = match last_success {
            Some(_) => oldest.run_id.to_string(),
            // The window ran out before a green run did. Fixed anchor so the
            // key cannot drift as runs age out; the evidence says so out loud.
            None => "since-beyond-window".to_string(),
        };
        let signature = format!("ci|{repo}|{workflow}|{anchor}");
        let first_seen = oldest.created_at;
        let step = newest.failing_step.clone().unwrap_or_else(|| "<step not resolved>".into());

        let mut evidence: Vec<(String, String)> = vec![
            (
                "verdict".into(),
                format!(
                    "`{workflow}` has failed {count} consecutive time(s) on {repo}'s default \
                     branch and is failing NOW. Nothing else in amux reads GitHub, so a red CI \
                     is invisible unless a human happens to look — this card is the only place \
                     it shows up."
                ),
            ),
            ("repo".into(), repo.clone()),
            ("workflow".into(), workflow.clone()),
            ("consecutive_failures".into(), count.to_string()),
            ("first_failure_at".into(), format!("{} (run {})", ci_when(first_seen), oldest.run_id)),
            ("first_failure_url".into(), oldest.url.clone()),
            ("failing_for".into(), format!("{:.1}h", ((now - first_seen) / 3600.0).max(0.0))),
            ("latest_run".into(), format!("{} (run {})", ci_when(newest.created_at), newest.run_id)),
            ("latest_run_url".into(), newest.url.clone()),
            ("failing_step".into(), step),
            ("trigger".into(), format!("{} on {}", newest.event, newest.branch)),
        ];
        match last_success {
            Some(s) => evidence.push((
                "last_success".into(),
                format!("{} (run {}) — {}", ci_when(s.created_at), s.run_id, s.url),
            )),
            None => evidence.push((
                "last_success".into(),
                format!(
                    "NONE in the last {} runs fetched — the streak is AT LEAST {count} and \
                     started before the window, so `consecutive_failures` is a floor, not a \
                     count. Raise AMUX_CI_RUN_LIMIT to see further back.",
                    ci_run_limit()
                ),
            )),
        }

        out.push(Finding {
            kind: DetectorKind::CiFailure,
            signature,
            title: format!(
                "CI red: {workflow} — {count}x consecutive since {}",
                ci_when(first_seen)
            ),
            evidence,
            recheck: format!(
                "gh run list --repo {repo} --workflow '{workflow}' --limit 10; \
                 gh run view {} --repo {repo} --log-failed",
                newest.run_id
            ),
            owner: None,
            count: count as u64,
            last_ts: newest.created_at,
        });
    }

    (out, suppressed)
}

/// Has the workflow behind an already-filed `ci|…` signature gone green?
///
/// Returns the sentence to LOG on the card. It never returns a status, because
/// there is no status this may set: "stopped failing is not fixed" applies with
/// full force here — a green run can mean the bug is fixed, or that someone
/// disabled the failing step, or that the workflow no longer runs the assertion
/// at all. The lane holding the card decides (ethos rule 8).
pub fn ci_recovered(runs: &[CiRun], signature: &str) -> Option<String> {
    let rest = signature.strip_prefix("ci|")?;
    let parts: Vec<&str> = rest.split('|').collect();
    if parts.len() < 3 {
        return None;
    }
    let (repo, workflow) = (parts[0], parts[1]);
    let mut mine: Vec<&CiRun> = runs
        .iter()
        .filter(|r| {
            r.repo == repo
                && ci_sanitize(&r.workflow) == workflow
                && r.on_default_branch
                && r.status == "completed"
                && r.event != "pull_request"
                && r.event != "pull_request_target"
                && !ci_is_inconclusive(&r.conclusion)
        })
        .collect();
    mine.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap_or(std::cmp::Ordering::Equal));
    let newest = mine.first()?;
    if ci_is_failure(&newest.conclusion) {
        return None;
    }
    Some(format!(
        "autofix: `{workflow}` is GREEN again as of {} (run {}, {}). STOPPED FAILING IS NOT \
         FIXED — this may be the fix, or the failing step may have been removed, disabled or \
         skipped. This card stays open and stays yours; say which it was and close it yourself.",
        ci_when(newest.created_at),
        newest.run_id,
        newest.url
    ))
}

/// Cached run list, so the 120s tick does not become 720 GitHub calls a day.
#[allow(clippy::type_complexity)]
fn ci_cache() -> &'static std::sync::RwLock<Option<(f64, Vec<CiRun>, Vec<Suppressed>)>> {
    static CELL: std::sync::OnceLock<std::sync::RwLock<Option<(f64, Vec<CiRun>, Vec<Suppressed>)>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::RwLock::new(None))
}

async fn gh(args: &[String], timeout_s: u64) -> Result<String, String> {
    let fut = tokio::process::Command::new("gh").args(args).output();
    let out = match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), fut).await {
        Err(_) => return Err(format!("`gh {}` timed out after {timeout_s}s", args.join(" "))),
        Ok(Err(e)) => return Err(format!("cannot run `gh`: {e}")),
        Ok(Ok(o)) => o,
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("`gh {}` failed: {}", args.join(" "), truncate(err.trim(), 300)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn ci_parse_ts(s: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp() as f64).unwrap_or(0.0)
}

/// Ask GitHub. Impure by definition — kept in its own function so
/// [`ci_findings`] can stay a pure decision over a `Vec<CiRun>`.
///
/// Runs OUTSIDE the store lock (see [`autofix_tick`]): a network call while
/// holding the SQLite read lock would make every other request on the server
/// wait on GitHub.
async fn fetch_ci_runs(now: f64) -> (Vec<CiRun>, Vec<Suppressed>) {
    if let Ok(g) = ci_cache().read() {
        if let Some((at, runs, sup)) = g.as_ref() {
            if now - at < ci_poll_min() * 60.0 {
                return (runs.clone(), sup.clone());
            }
        }
    }

    let mut runs: Vec<CiRun> = Vec::new();
    let mut suppressed: Vec<Suppressed> = Vec::new();
    let timeout_s = ci_cmd_timeout_s();

    for repo in ci_repos() {
        let default_branch = match gh(
            &["api".into(), format!("repos/{repo}"), "--jq".into(), ".default_branch".into()],
            timeout_s,
        )
        .await
        {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                // BLINDNESS IS PUBLISHED. gh missing, unauthenticated, rate
                // limited or offline all land here, and all of them look
                // identical to "CI is fine" if we return quietly.
                suppressed.push(sup(
                    DetectorKind::CiFailure,
                    &format!("ci|{repo}"),
                    &format!("cannot read {repo} — detector is BLIND, not clear: {e}"),
                ));
                continue;
            }
        };

        let list = match gh(
            &[
                "run".into(), "list".into(),
                "--repo".into(), repo.clone(),
                "--limit".into(), ci_run_limit().to_string(),
                "--json".into(),
                "databaseId,conclusion,createdAt,event,headBranch,url,workflowName,status".into(),
            ],
            timeout_s,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                suppressed.push(sup(
                    DetectorKind::CiFailure,
                    &format!("ci|{repo}"),
                    &format!("cannot list runs for {repo} — detector is BLIND, not clear: {e}"),
                ));
                continue;
            }
        };
        let parsed: Vec<Value> = serde_json::from_str(&list).unwrap_or_default();
        if parsed.is_empty() {
            suppressed.push(sup(
                DetectorKind::CiFailure,
                &format!("ci|{repo}"),
                "gh returned no runs — either the repo has none or the response did not parse",
            ));
        }
        for v in parsed {
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let branch = s("headBranch");
            runs.push(CiRun {
                repo: repo.clone(),
                workflow: s("workflowName"),
                run_id: v.get("databaseId").and_then(Value::as_i64).unwrap_or(0),
                status: s("status"),
                conclusion: s("conclusion"),
                event: s("event"),
                on_default_branch: branch == default_branch,
                branch,
                created_at: ci_parse_ts(&s("createdAt")),
                url: s("url"),
                failing_step: None,
            });
        }
    }

    // Resolve the failing STEP, but only for the newest failing run of each
    // workflow — one extra API call each, versus one per run in the window.
    let mut want: BTreeMap<(String, String), (i64, f64)> = BTreeMap::new();
    for r in &runs {
        if !ci_is_failure(&r.conclusion) || !r.on_default_branch || r.status != "completed" {
            continue;
        }
        let key = (r.repo.clone(), ci_sanitize(&r.workflow));
        let e = want.entry(key).or_insert((r.run_id, r.created_at));
        if r.created_at > e.1 {
            *e = (r.run_id, r.created_at);
        }
    }
    for ((repo, _wf), (run_id, _)) in want {
        let jq = r#".jobs[] | select(.conclusion=="failure") | .name as $j | .steps[] | select(.conclusion=="failure") | "\($j) / \(.name)""#;
        let step = gh(
            &[
                "api".into(),
                format!("repos/{repo}/actions/runs/{run_id}/jobs"),
                "--jq".into(),
                jq.into(),
            ],
            timeout_s,
        )
        .await
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty());
        if let Some(step) = step {
            for r in runs.iter_mut().filter(|r| r.run_id == run_id) {
                r.failing_step = Some(step.clone());
            }
        }
    }

    if let Ok(mut g) = ci_cache().write() {
        *g = Some((now, runs.clone(), suppressed.clone()));
    }
    (runs, suppressed)
}

fn sup(kind: DetectorKind, signature: &str, reason: &str) -> Suppressed {
    Suppressed { kind, signature: signature.to_string(), reason: reason.to_string() }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let head: String = s.chars().take(n).collect();
    format!("{head}… [{} chars truncated]", s.chars().count() - n)
}

/// `autofix:<signature>` — the durable dedupe key AND the card's `source_ref`.
/// One string, two jobs, so "have we filed this?" and "which card is it?"
/// cannot answer differently.
pub fn idem_of(signature: &str) -> String {
    format!("autofix:{signature}")
}

/// The card body. Evidence first, in a fixed order, with the recheck command
/// last — a lane picking this up should be able to reproduce the finding
/// before reading a word of prose.
pub fn render_desc(f: &Finding) -> String {
    let mut s = String::new();
    s.push_str("Filed automatically by amux (runtime_jobs/autofix) — nobody has looked at this yet.\n");
    s.push_str("It is a REPORT, not a diagnosis: the evidence below is computed, the cause is not.\n\n");
    for (k, v) in &f.evidence {
        s.push_str(&format!("{k}: {v}\n"));
    }
    s.push_str(&format!("\ndetector: {}\nsignature: {}\n", f.kind.slug(), f.signature));
    s.push_str(&format!("\nre-check (run this first — if it is now clean, say so on the card):\n  {}\n", f.recheck));
    s.push_str(
        "\nIf this turns out not to be a defect, the fix is usually the INSTRUMENT: a refusal \
         wearing a 5xx, a threshold below its own baseline, or a probe that cannot express the \
         answer. Say which, and the detector stops filing it.\n",
    );
    s
}

/// True when this signature has already been filed — durably, across restarts.
fn already_filed(conn: &Connection, signature: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM session_events WHERE idem = ?1 LIMIT 1",
        rusqlite::params![idem_of(signature)],
        |_| Ok(()),
    )
    .is_ok()
}

/// One tick: run every detector, file what is new, note what went quiet.
///
/// Returns the report rather than logging it, so the debug surface shows
/// exactly what the tick decided — including the suppressions, which are the
/// half that is normally invisible.
pub async fn autofix_tick(state: &AppState, home: &std::path::Path) -> AutofixReport {
    let t0 = std::time::Instant::now();
    let now = unix_now();
    let mut rep = AutofixReport { at: now, ..Default::default() };

    // BEFORE the store lock, on purpose: this is a network call, and holding
    // the SQLite read lock across GitHub's latency would stall every request on
    // the server behind a detector.
    let (ci_runs, ci_sup) = fetch_ci_runs(now).await;

    let (findings, suppressed, on) = {
        let conn = match state.store.read() {
            Ok(c) => c,
            Err(e) => {
                rep.errors.push(format!("store read: {e}"));
                rep.took_ms = t0.elapsed().as_secs_f64() * 1000.0;
                *last_report_cell().write().unwrap() = Some(rep.clone());
                return rep;
            }
        };
        let on = enabled(&conn);
        let mut findings = Vec::new();
        let mut suppressed = ci_sup;
        for kind in DetectorKind::all() {
            let (f, s) = match kind {
                DetectorKind::Http5xx => detect_5xx(&conn, now),
                DetectorKind::Latency => detect_latency(&conn, now),
                DetectorKind::DeadRoute => detect_dead_routes(&conn, now),
                DetectorKind::SilentSubsystem => detect_silent(&conn, now),
                DetectorKind::InvariantBreach => detect_invariants(&conn, now),
                DetectorKind::BuildDeploy => detect_build(&conn, now, home),
                DetectorKind::DiskPressure => detect_disk(now, home),
                DetectorKind::CiFailure => ci_findings(&ci_runs, now),
            };
            findings.extend(f);
            suppressed.extend(s);
        }
        (findings, suppressed, on)
    };
    rep.enabled = on;

    let ignore = ignore_list();
    let mut to_file: Vec<Finding> = Vec::new();
    for f in findings {
        rep.signatures_seen.push((f.kind.slug().to_string(), f.signature.clone()));
        if let Some(pat) = ignore.iter().find(|p| f.signature.contains(p.as_str())) {
            rep.suppressed.push((
                f.kind.slug().into(),
                f.signature.clone(),
                format!("matches AMUX_AUTOFIX_IGNORE entry {pat:?}"),
            ));
            continue;
        }
        if !on {
            // THE TOGGLE DOES NOT BLIND THE WATCHER. Detection ran, the
            // finding is published; only the board write is skipped. Otherwise
            // the window somebody switched it off is the one window with no
            // evidence in it.
            rep.suppressed.push((
                f.kind.slug().into(),
                f.signature.clone(),
                "autofix_enabled=0 (Settings toggle) — detected, not filed".into(),
            ));
            continue;
        }
        to_file.push(f);
    }
    for s in suppressed {
        rep.suppressed.push((s.kind.slug().into(), s.signature, s.reason));
    }

    for f in to_file {
        match file_finding(state, &f).await {
            Ok(Some(card)) => rep.filed.push((card, f.signature.clone())),
            Ok(None) => rep.already_filed.push(f.signature.clone()),
            Err(e) => rep.errors.push(format!("{}: {e}", f.signature)),
        }
    }

    match note_quiet_signatures(state, now, &ci_runs).await {
        Ok(v) => rep.quiet_noted = v,
        Err(e) => rep.errors.push(format!("quiet sweep: {e}")),
    }

    rep.took_ms = t0.elapsed().as_secs_f64() * 1000.0;
    if let Ok(mut g) = last_report_cell().write() {
        *g = Some(rep.clone());
    }
    rep
}

/// File one finding. Returns the new card id, or None when the signature was
/// already filed (the durable-dedupe path, which is the common case).
///
/// The dedupe row and the card are written in the SAME transaction as the
/// card: if the insert fails, no idem row is left claiming a card that does
/// not exist, and if the idem row exists the card does too.
async fn file_finding(state: &AppState, f: &Finding) -> anyhow::Result<Option<String>> {
    {
        let conn = state.store.read()?;
        if already_filed(&conn, &f.signature) {
            return Ok(None);
        }
    }
    let owner = f.owner.clone().unwrap_or_else(fixer_session);
    let owner = if owner.trim().is_empty() { None } else { Some(owner) };
    let title = truncate(&f.title, 160);
    let desc = render_desc(f);
    let item_type = f.kind.item_type();
    let signature = f.signature.clone();
    let idem = idem_of(&signature);
    let kind_slug = f.kind.slug().to_string();
    let now_s = unix_now() as i64;

    // The new id has to come back OUT of the writer closure, which runs on the
    // single writer thread — so it cannot ride a thread_local, and widening
    // `write_async`'s return type would touch every writer in the crate for
    // one string.
    let created_id: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let sink = created_id.clone();
    state
        .store
        .write_async(move |conn| {
            // Re-check inside the writer: two ticks cannot race a double file.
            if conn
                .query_row("SELECT 1 FROM session_events WHERE idem=?1 LIMIT 1", rusqlite::params![idem], |_| Ok(()))
                .is_ok()
            {
                return Ok(crate::db::WriteOutcome { applied: false, events: vec![] });
            }
            let new = bs::NewIssue {
                title: title.clone(),
                desc: desc.clone(),
                status: "todo".into(),
                session: owner.clone(),
                item_type: item_type.into(),
                creator: "autofix".into(),
                // owner_type=agent is what makes board_drive eligible to pick
                // it up. A human-owned card would sit forever: the drive
                // deliberately never touches a person's in-flight work.
                owner_type: "agent".into(),
                due: None,
                due_time: None,
                reviewer: None,
                shepherd: None,
                gate: vec![],
                depends_on: vec![],
                tags: vec!["autofix".into(), format!("detector:{kind_slug}")],
            };
            let row = bs::create_issue(conn, &new, now_s)?;
            conn.execute(
                "UPDATE issues SET source_ref = ?1 WHERE id = ?2",
                rusqlite::params![format!("autofix:{signature}"), row.id],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO session_events (ts, session, type, data, idem, source) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    now_s as f64,
                    owner.clone().unwrap_or_default(),
                    "autofix.filed",
                    json!({"card": row.id, "signature": signature, "detector": kind_slug}).to_string(),
                    idem,
                    "autofix"
                ],
            )?;
            let event = crate::db::PendingEvent {
                entity_type: amux_core::revision::EntityType::Task,
                entity_id: row.id.clone(),
                mutation: amux_core::revision::MutationKind::Created,
                payload: Some(row.snapshot()),
            };
            if let Ok(mut g) = sink.lock() {
                *g = Some(row.id.clone());
            }
            Ok(crate::db::WriteOutcome { applied: true, events: vec![event] })
        })
        .await?;
    let created = created_id.lock().ok().and_then(|g| g.clone());
    Ok(created)
}

/// "Stopped happening" is NOTED, never acted on.
///
/// For every open autofix card whose signature has produced nothing for
/// `quiet_h`, append one log line saying so. The card is not closed, not
/// archived, not moved: silence is evidence, not a verdict, and deciding what
/// it means is the job of the lane holding the card. One line per card, ever —
/// the idem key makes the note exactly-once so a quiet signature cannot
/// re-nag forever (which is its own well-documented failure here).
async fn note_quiet_signatures(
    state: &AppState,
    now: f64,
    ci: &[CiRun],
) -> anyhow::Result<Vec<(String, String)>> {
    let quiet_cut = now - quiet_h() * 3600.0;
    /// One pending note: the card, the short "what" for the report, the line to
    /// log, and the idem that makes it exactly-once.
    struct Note {
        card: String,
        what: String,
        line: String,
        idem: String,
    }
    let mut candidates: Vec<Note> = Vec::new();
    {
        let conn = state.store.read()?;
        let mut stmt = conn.prepare(
            "SELECT id, source_ref FROM issues \
             WHERE source_ref LIKE 'autofix:%' AND deleted IS NULL \
               AND status NOT IN ('done','verified','discarded') \
               AND COALESCE(archived,0)=0 LIMIT 500",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for (id, sref) in rows.flatten() {
            let sig = sref.trim_start_matches("autofix:").to_string();

            // CI is the second detector with a per-occurrence record to ask —
            // GitHub's own run list. Same rule as the 5xx sweep and stated in
            // `ci_recovered`: the card gets a LINE, never a status change.
            if sig.starts_with("ci|") {
                if let Some(line) = ci_recovered(ci, &sig) {
                    candidates.push(Note {
                        card: id.clone(),
                        what: "workflow is green again".into(),
                        line,
                        idem: format!("autofix-ci-green:{id}"),
                    });
                }
                continue;
            }

            // Only the request-log-backed detectors can be asked "has this
            // recurred?"; the others have no per-occurrence row to count, so
            // they are left alone rather than guessed at.
            let Some(rest) = sig.strip_prefix("5xx|") else { continue };
            let parts: Vec<&str> = rest.split('|').collect();
            if parts.len() != 4 {
                continue;
            }
            let (status, method, target) = (parts[0], parts[1], parts[3]);
            let Ok(status) = status.parse::<i64>() else { continue };
            let recent: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM _amux_request_log WHERE status=?1 AND method=?2 AND ts >= ?3",
                    rusqlite::params![status, method, quiet_cut],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            if recent == 0 {
                candidates.push(Note {
                    card: id.clone(),
                    what: format!("{method} {target} → {status}"),
                    line: format!(
                        "autofix: no occurrence of this signature for {:.0}h ({method} {target} → \
                         {status}). STOPPED HAPPENING IS NOT FIXED — it may be fixed, or the \
                         caller may have given up, or the path may now be unreachable. This card \
                         stays open; whoever holds it decides which.",
                        quiet_h()
                    ),
                    idem: format!("autofix-quiet:{id}"),
                });
            }
        }
    }
    let mut noted = Vec::new();
    for Note { card, what, line, idem } in candidates {
        let already = {
            let conn = state.store.read()?;
            conn.query_row("SELECT 1 FROM session_events WHERE idem=?1 LIMIT 1", rusqlite::params![idem], |_| Ok(()))
                .is_ok()
        };
        if already {
            continue;
        }
        let c2 = card.clone();
        let i2 = idem.clone();
        state
            .store
            .write_async(move |conn| {
                use rusqlite::OptionalExtension;
                let existing: Option<String> = conn
                    .query_row("SELECT log FROM issues WHERE id=?1", rusqlite::params![c2], |r| r.get(0))
                    .optional()?
                    .flatten();
                let hhmm = chrono::Local::now().format("%H:%M").to_string();
                let log = bs::append_log(existing.as_deref(), &hhmm, &line);
                conn.execute("UPDATE issues SET log=?1 WHERE id=?2", rusqlite::params![log, c2])?;
                conn.execute(
                    "INSERT OR IGNORE INTO session_events (ts, session, type, data, idem, source) \
                     VALUES (?1, '', 'autofix.quiet', ?2, ?3, 'autofix')",
                    rusqlite::params![unix_now(), json!({"card": c2}).to_string(), i2],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await?;
        noted.push((card, what));
    }
    Ok(noted)
}

// ---------------------------------------------------------------------------
// Spawn + debug surface.
// ---------------------------------------------------------------------------

/// `AMUX_AUTOFIX_SECS=0` disables the loop entirely (the repo's tick idiom).
/// Note the difference from the Settings toggle: 0 stops the JOB, the toggle
/// stops the WRITE and keeps the evidence.
pub fn spawn(state: AppState) -> Option<super::PeriodicTask> {
    let secs = std::env::var("AMUX_AUTOFIX_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(AUTOFIX_TICK_SECS);
    if secs == 0 {
        tracing::info!("autofix: disabled (AMUX_AUTOFIX_SECS=0)");
        return None;
    }
    let home = crate::runtime_jobs::autofix::amux_home();
    Some(super::spawn_periodic("autofix", secs, move || {
        let state = state.clone();
        let home = home.clone();
        async move {
            let r = autofix_tick(&state, &home).await;
            if !r.filed.is_empty() || !r.errors.is_empty() {
                tracing::info!(
                    filed = r.filed.len(),
                    suppressed = r.suppressed.len(),
                    errors = ?r.errors,
                    "autofix tick"
                );
            }
        }
    }))
}

pub fn amux_home() -> std::path::PathBuf {
    std::env::var("AMUX_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
    })
}

/// `GET /api/debug/autofix` — the answer to "why didn't it file?" in one
/// request. Carries the toggle state, every threshold, every signature seen,
/// every card filed, and every suppression WITH its reason.
async fn debug_autofix(axum::extract::State(state): axum::extract::State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let r = last_report();
    let on = state.store.read().map(|c| enabled(&c)).unwrap_or(true);
    let secs = std::env::var("AMUX_AUTOFIX_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(AUTOFIX_TICK_SECS);
    let body = json!({
        "enabled": on,
        "loop_running": secs != 0,
        "tick_secs": secs,
        "detectors": DetectorKind::all().iter().map(|k| k.slug()).collect::<Vec<_>>(),
        "thresholds": {
            "window_h": window_h(),
            "baseline_h": baseline_h(),
            "latency_mult": latency_mult(),
            "latency_min_samples": latency_min_samples(),
            "outlier_ms": outlier_ms(),
            "dead_route_min_hits": dead_route_min_hits(),
            "schedule_overdue_min": schedule_overdue_min(),
            "steering_stale_min": steering_stale_min(),
            "invariant_min_occurrences": invariant_min_occurrences(),
            "build_stale_h": build_stale_h(),
            "quiet_h": quiet_h(),
            "ci_repos": ci_repos(),
            "ci_poll_min": ci_poll_min(),
            "ci_min_failures": ci_min_failures(),
            "ci_run_limit": ci_run_limit(),
            "ci_timeout_s": ci_cmd_timeout_s(),
        },
        "fixer_session": fixer_session(),
        "ignore": ignore_list(),
        "last": r.as_ref().map(|r| json!({
            "at": r.at,
            "at_local": rl::local_when(r.at),
            "age_s": (unix_now() - r.at).max(0.0),
            "enabled": r.enabled,
            "took_ms": r.took_ms,
            "signatures_seen": r.signatures_seen.iter()
                .map(|(k, s)| json!({"detector": k, "signature": s})).collect::<Vec<_>>(),
            "filed": r.filed.iter()
                .map(|(c, s)| json!({"card": c, "signature": s})).collect::<Vec<_>>(),
            "already_filed": r.already_filed,
            "suppressed": r.suppressed.iter()
                .map(|(k, s, why)| json!({"detector": k, "signature": s, "reason": why}))
                .collect::<Vec<_>>(),
            "quiet_noted": r.quiet_noted.iter()
                .map(|(c, w)| json!({"card": c, "what": w})).collect::<Vec<_>>(),
            "errors": r.errors,
        })),
        "note": "suppressed lists EVERY decision not to file, with its reason — a detector that \
                 silently declines is the failure this job exists to end",
    });
    axum::Json::<Value>(body).into_response()
}

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route("/api/debug/autofix", axum::routing::get(debug_autofix))
}

/// Permissive timestamp parse for `schedules.next_run`, which is stored as
/// text in whatever the writer used. Returns None rather than guessing.
fn parse_ts(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<f64>() {
        // Epoch seconds, sanity-bounded so a bare year ("2026") cannot pass.
        if n > 1_000_000_000.0 {
            return Some(n);
        }
        return None;
    }
    use chrono::TimeZone;
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp() as f64);
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            if let Some(dt) = chrono::Local.from_local_datetime(&ndt).single() {
                return Some(dt.timestamp() as f64);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests. Each one is a property the design would be worthless without, and
// each is written so a plausible wrong implementation FAILS it — an N-cards
// filer, a refiling filer, a filer that buries the board in 501s, a card with
// no evidence on it.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn state() -> (AppState, tempfile::TempDir) {
        // NO TEST MAY TOUCH GITHUB. The CI detector shells out to `gh` inside
        // `autofix_tick`; seeding its cache with an empty, fresh result makes
        // `fetch_ci_runs` short-circuit, so every existing test stays offline
        // and deterministic. The CI tests below deliberately do NOT go through
        // the tick — they drive the pure decision and the real filing path
        // directly, which is the only way to assert on a 31-failure streak
        // without a network and without inventing one.
        *ci_cache().write().unwrap() = Some((unix_now(), Vec::new(), Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::db::Store::open(&dir.path().join("t.db")).unwrap());
        (
            AppState {
                store,
                started: std::time::Instant::now(),
                build_hash: "test".into(),
                auth_token: None,
            },
            dir,
        )
    }

    /// One request-log row. Grouped into a struct rather than ten positional
    /// arguments so a caller cannot silently swap `worker` and `ua` — which
    /// would quietly change what the dead-route detector concludes.
    struct Row<'a> {
        ts: f64,
        method: &'a str,
        path: &'a str,
        family: &'a str,
        status: i64,
        body: &'a str,
        worker: &'a str,
        ua: &'a str,
        ms: f64,
    }

    fn log_row(st: &AppState, r: Row<'_>) {
        let (ts, status, ms) = (r.ts, r.status, r.ms);
        let (m, p, f, b, w, u) = (
            r.method.to_string(), r.path.to_string(), r.family.to_string(),
            r.body.to_string(), r.worker.to_string(), r.ua.to_string(),
        );
        st.store
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO _amux_request_log (ts, method, path, family, status, latency_ms, \
                     client_ip, user_agent, amux_session, worker, answered_by, error_body) \
                     VALUES (?1,?2,?3,?4,?5,?6,'127.0.0.1',?7,'',?8,'native',?9)",
                    rusqlite::params![ts, m, p, f, status, ms, u, w, b],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
    }

    fn cards(st: &AppState) -> Vec<(String, String, String, String, String)> {
        let conn = st.store.read().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, title, desc, type, COALESCE(source_ref,'') FROM issues ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        rows.flatten().collect()
    }

    /// THE grouping property: 14 identical failures are ONE incident. A filer
    /// that files per-row would put 14 cards on the board for one bug, which
    /// is how a board stops being read (ethos rule 5 — at 100x volume, does
    /// this stay coherent?).
    #[tokio::test]
    async fn n_identical_5xx_produce_one_card() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..14 {
            log_row(&st, Row { ts: now - 60.0 - i as f64, method: "POST", path: &format!("/api/sessions/lane{i}/send"), family: "/api/sessions", status: 500, body: "{\"message\":\"boom\"}", worker: "lane", ua: "curl/8", ms: 12.0 });
        }
        let r = autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        let c = cards(&st);
        assert_eq!(c.len(), 1, "14 rows must be ONE card, got {}: {c:#?}", c.len());
        assert!(r.filed.len() == 1, "report must name the card it filed: {:?}", r.filed);
        // The 14 collapse because normalize_target folds the lane name into
        // the route pattern — the SAME collapse /api/logs/analyze uses.
        assert!(c[0].1.contains("14x"), "count belongs in the computed title: {}", c[0].1);
        assert!(c[0].1.contains("/api/sessions/{name}/{*verb}"), "title is the route, not a path: {}", c[0].1);
    }

    /// A restart must not refile. This is the property an in-memory dedupe
    /// would pass in a single process and fail in production, silently, on
    /// every deploy — and this server re-execs whenever anyone saves.
    #[tokio::test]
    async fn a_restart_does_not_refile() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..5 {
            log_row(&st, Row { ts: now - 60.0 - i as f64, method: "POST", path: "/api/sessions/x/send", family: "/api/sessions", status: 500, body: "boom", worker: "", ua: "curl/8", ms: 12.0 });
        }
        autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        assert_eq!(cards(&st).len(), 1);

        // A RESTART: brand-new AppState over the SAME database, and the
        // in-memory report cell explicitly cleared. Nothing survives except
        // what is on disk — which is the whole question.
        *last_report_cell().write().unwrap() = None;
        let restarted = AppState {
            store: st.store.clone(),
            started: std::time::Instant::now(),
            build_hash: "test-after-restart".into(),
            auth_token: None,
        };
        let r = autofix_tick(&restarted, std::path::Path::new("/nonexistent")).await;
        assert_eq!(cards(&restarted).len(), 1, "a restart refiled the same signature");
        assert_eq!(r.filed.len(), 0, "nothing new should be filed");
        assert_eq!(r.already_filed.len(), 1, "and the dedupe must SAY it deduped: {r:?}");
    }

    /// 501/503 name what is missing. They are amux being honest about an
    /// absent capability, and a card for one is a card no lane can act on.
    /// Asserts BOTH halves: nothing filed, and the reason is visible.
    #[tokio::test]
    async fn honest_degradations_file_nothing_but_say_why() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..8 {
            log_row(&st, Row { ts: now - 60.0 - i as f64, method: "GET", path: "/api/email/search", family: "/api/email", status: 501, body: "{\"error\":\"not a connected Gmail account\",\"connected_hint\":\"...\"}", worker: "", ua: "curl/8", ms: 1.0 });
            log_row(&st, Row { ts: now - 60.0 - i as f64, method: "GET", path: "/api/torrents", family: "/api/torrents", status: 503, body: "{\"error\":\"aria2c not running\",\"start\":\"aria2c --enable-rpc\"}", worker: "", ua: "curl/8", ms: 1.0 });
        }
        let r = autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        assert_eq!(cards(&st).len(), 0, "a 501/503 must never become a card");
        let reasons: Vec<&String> = r.suppressed.iter().map(|(_, _, why)| why).collect();
        assert!(
            reasons.iter().any(|w| w.contains("honest degradation")),
            "the suppression must be VISIBLE with its reason, not silent: {reasons:?}"
        );
        // Control: a real 500 in the same window still files, so the test
        // cannot pass by the detector being broken outright.
        log_row(&st, Row { ts: now - 30.0, method: "POST", path: "/api/foo", family: "/api/foo", status: 500, body: "kaboom", worker: "", ua: "curl/8", ms: 1.0 });
        autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        assert_eq!(cards(&st).len(), 1, "a genuine 500 must still file");
    }

    /// The card has to be startable. A worker picking this up should be able
    /// to reproduce the finding before reading a word of prose.
    #[tokio::test]
    async fn the_card_carries_the_evidence() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..6 {
            log_row(&st, Row { ts: now - 300.0 + i as f64, method: "POST", path: "/api/sessions/amux/send", family: "/api/sessions", status: 500, body: "{\"message\":\"session is in the background-conversation view\"}", worker: "amux", ua: "Mozilla/5.0", ms: 37.2 });
        }
        autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        let c = cards(&st);
        assert_eq!(c.len(), 1);
        let (id, title, desc, ty, sref) = &c[0];
        for field in [
            "verdict:", "sample_request:", "error_body:", "first_seen:", "last_seen:",
            "count:", "distinct_clients:", "signature:", "re-check",
        ] {
            assert!(desc.contains(field), "card {id} is missing `{field}`:\n{desc}");
        }
        assert!(desc.contains("background-conversation"), "the actual error body must be quoted");
        assert!(desc.contains("/api/logs/analyze"), "the recheck must be a runnable query");
        assert_eq!(sref, "autofix:5xx|500|POST|/api/sessions|/api/sessions/{name}/{*verb}");
        // NEVER `code`: an auto-filed report has no merged commit to claim, so
        // a code gate could only be exited by asserting something untrue.
        assert_eq!(ty, "investigation", "auto-filed faults must not be gated as code");
        assert!(!title.is_empty());
        // Owner comes from the request's own worker attribution.
        let conn = st.store.read().unwrap();
        let owner: Option<String> = conn
            .query_row("SELECT session FROM issues WHERE id=?1", rusqlite::params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(owner.as_deref(), Some("amux"));
        let ot: String = conn
            .query_row("SELECT owner_type FROM issues WHERE id=?1", rusqlite::params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(ot, "agent", "board_drive only picks up agent-owned cards");
    }

    /// The toggle stops the WRITE, never the WATCH. A disabled watcher that
    /// also goes blind loses the evidence for exactly the window it was off.
    #[tokio::test]
    async fn disabled_still_records_what_it_would_have_filed() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..4 {
            log_row(&st, Row { ts: now - 60.0 - i as f64, method: "POST", path: "/api/thing", family: "/api/thing", status: 500, body: "bang", worker: "", ua: "curl/8", ms: 3.0 });
        }
        st.store
            .write(|conn| {
                conn.execute("INSERT OR REPLACE INTO prefs (key,value) VALUES ('autofix_enabled','0')", [])?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
        let r = autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        assert!(!r.enabled);
        assert_eq!(cards(&st).len(), 0, "disabled must not write a card");
        assert_eq!(r.signatures_seen.len(), 1, "but it must still SEE the fault: {r:?}");
        assert!(
            r.suppressed.iter().any(|(_, _, why)| why.contains("autofix_enabled=0")),
            "and say why it did not file: {:?}", r.suppressed
        );
        // Flip it back on: the same fault now files, with no restart.
        st.store
            .write(|conn| {
                conn.execute("UPDATE prefs SET value='1' WHERE key='autofix_enabled'", [])?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
        let r2 = autofix_tick(&st, std::path::Path::new("/nonexistent")).await;
        assert!(r2.enabled);
        assert_eq!(cards(&st).len(), 1, "the pref must take effect live, without a restart");
    }

    /// A 404 nobody's browser asked for is a probe, not a broken feature.
    /// Both directions asserted: the curl 404 is suppressed WITH a reason, and
    /// the browser 404 on the same path files — otherwise "files nothing" would
    /// pass by the detector being dead.
    #[tokio::test]
    async fn dead_route_files_only_what_the_spa_actually_calls() {
        let (st, _d) = state();
        let now = unix_now();
        for i in 0..5 {
            log_row(&st, Row { ts: now - 100.0 - i as f64, method: "GET", path: "/api/burn", family: "/api/burn", status: 404, body: "{\"error\":\"not found\"}", worker: "", ua: "curl/8.7.1", ms: 1.0 });
        }
        let (f, s) = detect_dead_routes(&st.store.read().unwrap(), now);
        assert!(f.is_empty(), "a curl 404 must not file: {f:?}");
        assert!(
            s.iter().any(|x| x.reason.contains("probe")),
            "and the suppression must be visible: {s:?}"
        );

        for i in 0..5 {
            log_row(&st, Row { ts: now - 100.0 - i as f64, method: "GET", path: "/api/offline-origin", family: "/api/offline-origin", status: 404, body: "{\"error\":\"not found\"}", worker: "", ua: "Mozilla/5.0 (Macintosh)", ms: 1.0 });
        }
        let (f2, _) = detect_dead_routes(&st.store.read().unwrap(), now);
        assert_eq!(f2.len(), 1, "a path the SPA fetches must file: {f2:?}");
        let ev: BTreeMap<_, _> = f2[0].evidence.iter().cloned().collect();
        // The killer annotation, taken verbatim from the route table rather
        // than re-derived here.
        assert!(ev.contains_key("routed_methods"), "{ev:?}");
        assert!(ev.contains_key("nearest_routes"), "{ev:?}");
        assert!(ev["called_by"].contains("browser"));
    }

    /// A p95 over four requests is one request wearing a percentile. Both
    /// floors must hold, and when a floor blocks the call it has to SAY so.
    #[tokio::test]
    async fn latency_needs_a_real_sample_on_both_sides() {
        let (st, _d) = state();
        let now = unix_now();
        // A tiny baseline and a slow window: the multiple is enormous, but
        // there is no norm to compare against, so it must not file.
        for i in 0..3 {
            log_row(&st, Row { ts: now - 200_000.0 - i as f64, method: "GET", path: "/api/slow", family: "/api/slow", status: 200, body: "", worker: "", ua: "curl/8", ms: 5.0 });
        }
        for i in 0..60 {
            log_row(&st, Row { ts: now - 100.0 - i as f64, method: "GET", path: "/api/slow", family: "/api/slow", status: 200, body: "", worker: "", ua: "curl/8", ms: 4000.0 });
        }
        let (f, s) = detect_latency(&st.store.read().unwrap(), now);
        assert!(
            !f.iter().any(|x| x.signature == "latency|p95|/api/slow"),
            "a 3-sample baseline cannot support a p95 verdict: {f:?}"
        );
        assert!(
            s.iter().any(|x| x.reason.contains("baseline has")),
            "the sample-floor refusal must be visible: {s:?}"
        );

        // Now give it a real baseline. Same window; it must file, and the card
        // must state the threshold it used.
        for i in 0..60 {
            log_row(&st, Row { ts: now - 200_000.0 - i as f64, method: "GET", path: "/api/slow", family: "/api/slow", status: 200, body: "", worker: "", ua: "curl/8", ms: 5.0 });
        }
        let (f2, _) = detect_latency(&st.store.read().unwrap(), now);
        let hit = f2.iter().find(|x| x.signature == "latency|p95|/api/slow")
            .expect("a 800x p95 regression over 60 samples must file");
        let ev: BTreeMap<_, _> = hit.evidence.iter().cloned().collect();
        assert!(ev["verdict"].contains("Threshold"), "state the threshold: {ev:?}");
        assert!(ev.contains_key("window_samples") && ev.contains_key("baseline_samples"));
    }

    /// A single absurd request is not a percentile shift and must not be
    /// folded into one — 27s on an upload chunk is one request going wrong.
    #[tokio::test]
    async fn absurd_single_requests_file_on_their_own() {
        let (st, _d) = state();
        let now = unix_now();
        log_row(&st, Row { ts: now - 50.0, method: "PUT", path: "/api/upload/abc123/chunk/7", family: "/api/upload", status: 200, body: "", worker: "", ua: "Mozilla/5.0", ms: 27_000.0 });
        let (f, _) = detect_latency(&st.store.read().unwrap(), now);
        let hit = f.iter().find(|x| x.signature.starts_with("latency|outlier|PUT"))
            .expect("a 27s request must file on its own: {f:?}");
        assert!(hit.title.contains("27.0s"), "the number belongs in the title: {}", hit.title);
        let ev: BTreeMap<_, _> = hit.evidence.iter().cloned().collect();
        assert!(ev["verdict"].contains("not a percentile shift"));
    }

    /// Silence is evidence, not a verdict. The card must be ANNOTATED and left
    /// exactly where it was — an auto-close here would be an agent deciding a
    /// bug was fixed because nobody hit it (ethos rule 8).
    ///
    /// Deliberately drives `file_finding` + `note_quiet_signatures` directly
    /// rather than steering the tick with env vars: the first version of this
    /// test set AMUX_AUTOFIX_WINDOW_H, and since cargo runs tests as threads in
    /// ONE process that leaked into the latency test running beside it and
    /// failed it. A test that reaches for a process-global to set up its
    /// fixture is testing the other tests too.
    #[tokio::test]
    async fn a_quiet_signature_is_noted_never_closed() {
        let (st, _d) = state();
        let now = unix_now();
        // A filed card whose signature has produced NO request-log rows at all
        // — the purest form of "stopped happening".
        let f = Finding {
            kind: DetectorKind::Http5xx,
            signature: "5xx|500|POST|/api/quiet|/api/quiet".into(),
            title: "POST /api/quiet → HTTP 500 (3x)".into(),
            evidence: vec![("verdict".into(), "it broke".into())],
            recheck: "curl -sk $AMUX_URL/api/logs/analyze".into(),
            owner: None,
            count: 3,
            last_ts: now - 86_400.0,
        };
        let id = file_finding(&st, &f).await.unwrap().expect("card filed");

        let noted = note_quiet_signatures(&st, now, &[]).await.unwrap();
        assert_eq!(noted.len(), 1, "a quiet signature must be noted: {noted:?}");

        let conn = st.store.read().unwrap();
        let (status, log): (String, Option<String>) = conn
            .query_row("SELECT status, log FROM issues WHERE id=?1", rusqlite::params![id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(status, "todo", "the card must NOT be closed or moved");
        let log = log.unwrap_or_default();
        assert!(log.contains("STOPPED HAPPENING IS NOT FIXED"), "the note must say what it means: {log}");
        drop(conn);

        // And it must not re-nag: a second sweep adds nothing. A note that
        // fires every tick forever is its own well-documented failure here.
        let again = note_quiet_signatures(&st, now, &[]).await.unwrap();
        assert!(again.is_empty(), "the quiet note must be exactly-once, got {again:?}");
    }

    /// Every detector's item type must be closable honestly — i.e. never
    /// `code`, whose gates demand a merged commit and passing tests that an
    /// auto-filed report does not have.
    #[test]
    fn no_detector_files_a_code_card() {
        for k in DetectorKind::all() {
            assert_ne!(k.item_type(), "code", "{k:?} would be gated on a merge it cannot claim");
            assert!(!k.slug().is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // CI / deploy detector (AMUX-2780).
    //
    // These drive `ci_findings` (the pure decision) and `file_finding` (the
    // REAL filing path, dedupe included) rather than `autofix_tick`, because
    // the tick's CI input is a `gh` subprocess. Testing through the tick would
    // mean either hitting GitHub — which makes the suite non-deterministic and
    // makes a red suite mean "the network is down" — or mocking the tick, which
    // proves nothing about the code that ships.
    // -----------------------------------------------------------------------

    /// A run fixture. `mins_ago` keeps the timeline readable at the call site;
    /// getting the ORDER wrong is how a streak test silently measures nothing.
    fn ci_run(wf: &str, id: i64, conclusion: &str, mins_ago: f64) -> CiRun {
        CiRun {
            repo: "mixpeek/amux".into(),
            workflow: wf.into(),
            run_id: id,
            status: "completed".into(),
            conclusion: conclusion.into(),
            event: "push".into(),
            branch: "main".into(),
            on_default_branch: true,
            created_at: unix_now() - mins_ago * 60.0,
            url: format!("https://github.com/mixpeek/amux/actions/runs/{id}"),
            failing_step: None,
        }
    }

    fn card_row(st: &AppState, id: &str) -> (String, String, String) {
        let conn = st.store.read().unwrap();
        conn.query_row(
            "SELECT status, COALESCE(log,''), COALESCE(source_ref,'') FROM issues WHERE id=?1",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    /// THE test the whole card exists for. `deploy-cloud.yml` failed 31
    /// consecutive times over three days and produced NOTHING anyone could see.
    /// If the detector is removed, misfiltered, or its threshold is set above
    /// the streak, this fails — which is the property "nobody saw 29 failures"
    /// needed and did not have.
    #[tokio::test]
    async fn a_failing_workflow_produces_a_card_with_its_streak() {
        let (st, _d) = state();
        let mut runs = vec![ci_run("Deploy to cloud.amux.io", 31_182_322_406, "success", 4400.0)];
        for i in 0..31 {
            let mut r = ci_run("Deploy to cloud.amux.io", 31_200_000_000 + i, "failure", 4000.0 - i as f64 * 100.0);
            if i == 30 {
                r.failing_step = Some("deploy / Gateway env — admin emails".into());
            }
            runs.push(r);
        }
        let (f, _s) = ci_findings(&runs, unix_now());
        assert_eq!(f.len(), 1, "31 failures of one workflow are ONE incident, got {}", f.len());
        let ev: BTreeMap<&str, &str> =
            f[0].evidence.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(ev["consecutive_failures"], "31", "the count is the whole point");
        assert!(ev["first_failure_at"].contains("31200000000"), "first-seen names the run: {ev:?}");
        assert!(ev["latest_run_url"].contains("31200000030"), "run URL must be the NEWEST failure");
        assert!(ev["failing_step"].contains("Gateway env"), "the failing STEP is required evidence");
        assert!(ev["last_success"].contains("31182322406"), "last green run belongs on the card");
        assert!(f[0].recheck.contains("--log-failed"), "recheck must be runnable: {}", f[0].recheck);

        // …and it reaches the board through the real filing path.
        let card = file_finding(&st, &f[0]).await.unwrap().expect("a finding must file a card");
        let (status, _log, sref) = card_row(&st, &card);
        assert_eq!(status, "todo", "a filed fault is queued, not pre-closed");
        assert_eq!(sref, format!("autofix:{}", f[0].signature));
        let c = cards(&st);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].3, "blocker", "a red deploy is a blocker, not a code card");
        assert!(c[0].1.contains("31x"), "count belongs in the computed title: {}", c[0].1);
    }

    /// Dedupe, durably. A nightly failure files ONCE. This is the property that
    /// separates a detector from a nightly spam machine, and the one an
    /// in-memory dedupe passes in-process and fails in production on every
    /// re-exec.
    #[tokio::test]
    async fn a_nightly_failure_files_once_not_nightly() {
        let (st, _d) = state();
        let base = vec![
            ci_run("rust", 900, "success", 5000.0),
            ci_run("rust", 901, "failure", 400.0),
            ci_run("rust", 902, "failure", 300.0),
        ];
        let f1 = ci_findings(&base, unix_now()).0;
        assert_eq!(f1.len(), 1);
        assert!(file_finding(&st, &f1[0]).await.unwrap().is_some(), "first sighting files");

        // Night 2 and night 3: the SAME outage, more failures. Same anchor,
        // same signature, no new card.
        let mut later = base.clone();
        later.push(ci_run("rust", 903, "failure", 200.0));
        later.push(ci_run("rust", 904, "failure", 100.0));
        let f2 = ci_findings(&later, unix_now()).0;
        assert_eq!(f2[0].signature, f1[0].signature, "the streak anchor must not move mid-outage");
        assert!(file_finding(&st, &f2[0]).await.unwrap().is_none(), "night 2 must not refile");

        // A RESTART: new AppState over the same DB, in-memory report cleared.
        *last_report_cell().write().unwrap() = None;
        let restarted = AppState {
            store: st.store.clone(),
            started: std::time::Instant::now(),
            build_hash: "after-restart".into(),
            auth_token: None,
        };
        assert!(
            file_finding(&restarted, &f2[0]).await.unwrap().is_none(),
            "a restart refiled the same outage — the dedupe is not durable"
        );
        assert_eq!(cards(&st).len(), 1, "three nights and a restart are ONE card");
    }

    /// The other half of dedupe, which a workflow-keyed signature gets wrong:
    /// a workflow that is FIXED and breaks again later must file again. A key
    /// that never changes goes silent forever after the first fix.
    #[test]
    fn a_new_outage_after_a_green_run_is_a_new_card() {
        let old = vec![
            ci_run("rust", 80, "success", 5000.0),
            ci_run("rust", 90, "failure", 4000.0),
            ci_run("rust", 100, "failure", 3000.0),
        ];
        let sig_old = ci_findings(&old, unix_now()).0[0].signature.clone();

        let mut new = old.clone();
        new.push(ci_run("rust", 150, "success", 2000.0)); // fixed
        new.push(ci_run("rust", 200, "failure", 1000.0)); // …and broke again
        new.push(ci_run("rust", 210, "failure", 500.0));
        let sig_new = ci_findings(&new, unix_now()).0[0].signature.clone();

        assert_ne!(sig_old, sig_new, "a second outage must not be deduped against the first");
        assert!(sig_new.ends_with("|200"), "the anchor is the streak's OLDEST failure: {sig_new}");
    }

    /// "Stopped failing is not fixed." A green run appends a line and changes
    /// NOTHING else — not the status, not the archive flag. A detector that
    /// auto-closes on green declares a fix it did not verify, and the one time
    /// that is wrong is the time the failing step was quietly deleted.
    #[tokio::test]
    async fn a_green_run_is_noted_never_closed() {
        let (st, _d) = state();
        let broken = vec![
            ci_run("checks", 10, "success", 5000.0),
            ci_run("checks", 11, "failure", 400.0),
            ci_run("checks", 12, "failure", 300.0),
        ];
        let f = ci_findings(&broken, unix_now()).0;
        let card = file_finding(&st, &f[0]).await.unwrap().unwrap();

        // While red, there is nothing to say.
        let noted = note_quiet_signatures(&st, unix_now(), &broken).await.unwrap();
        assert!(noted.is_empty(), "a still-failing workflow must not be reported as recovered");

        let mut green = broken.clone();
        green.push(ci_run("checks", 13, "success", 10.0));
        let noted = note_quiet_signatures(&st, unix_now(), &green).await.unwrap();
        assert_eq!(noted.len(), 1, "a green run must be NOTED: {noted:?}");

        let (status, log, _) = card_row(&st, &card);
        assert_eq!(status, "todo", "green must not close, move or resolve the card");
        assert!(log.contains("GREEN again"), "the note must be on the card: {log}");
        assert!(log.contains("STOPPED FAILING IS NOT FIXED"), "and must say what green does NOT prove");

        // Exactly once, forever — a recovery that re-nags every tick is its
        // own well-documented failure in this repo.
        let again = note_quiet_signatures(&st, unix_now(), &green).await.unwrap();
        assert!(again.is_empty(), "the green note must be exactly-once, got {again:?}");
    }

    /// The suppressions the card asked for, and the rule that a decision NOT to
    /// file is published rather than silent.
    #[test]
    fn cancelled_and_pull_request_runs_are_not_evidence() {
        let now = unix_now();

        // A cancelled run is the NEWEST. It must neither file (it is not a
        // fault) nor reset the streak behind it (the 08-10 16:42 cancel sat in
        // the middle of a 31-failure outage).
        let mut runs = vec![
            ci_run("Deploy to cloud.amux.io", 1, "success", 900.0),
            ci_run("Deploy to cloud.amux.io", 2, "failure", 800.0),
            ci_run("Deploy to cloud.amux.io", 3, "failure", 700.0),
        ];
        runs.push(ci_run("Deploy to cloud.amux.io", 4, "cancelled", 600.0));
        let (f, s) = ci_findings(&runs, now);
        assert_eq!(f.len(), 1, "a cancelled run must not mask the outage behind it");
        assert!(f[0].signature.ends_with("|2"), "cancel must not move the anchor: {}", f[0].signature);
        assert!(
            s.iter().any(|x| x.reason.contains("cancelled")),
            "the decision to ignore a cancelled run must be PUBLISHED: {s:?}"
        );

        // A fork/PR run that fails is somebody else's branch, not main's CI.
        let mut pr = ci_run("rust", 50, "failure", 10.0);
        pr.event = "pull_request".into();
        pr.on_default_branch = false;
        pr.branch = "contributor:patch-1".into();
        let mut pr2 = pr.clone();
        pr2.run_id = 51;
        let (f, s) = ci_findings(&[pr, pr2], now);
        assert!(f.is_empty(), "PR/fork runs must never file against main: {f:?}");
        assert!(
            s.iter().any(|x| x.reason.contains("pull-request")),
            "the exclusion must be visible, not silent: {s:?}"
        );

        // One red run is below the threshold — and says so, rather than
        // vanishing. An unexplained non-filing is the failure this file exists
        // to end.
        let one = vec![ci_run("rust", 60, "success", 900.0), ci_run("rust", 61, "failure", 10.0)];
        let (f, s) = ci_findings(&one, now);
        assert!(f.is_empty());
        assert!(
            s.iter().any(|x| x.reason.contains("AMUX_CI_MIN_FAILURES")),
            "a sub-threshold streak must name the threshold that held it back: {s:?}"
        );
    }

    /// The probe against the REAL incident, run by hand.
    ///
    /// `#[ignore]` and honest about why: it shells out to `gh` and talks to
    /// GitHub, so in CI it would be a test that fails when the network hiccups
    /// or the token expires — and a check that goes red for reasons unrelated
    /// to the code is a check people learn to ignore. It is NOT decoration:
    /// this is the only thing that exercises `fetch_ci_runs`' parsing, the
    /// default-branch comparison and the failing-step API call against real
    /// GitHub JSON, none of which a fixture can prove. Run it after touching
    /// any of those:
    ///
    /// ```text
    /// cargo test -p amux-server --lib ci_detector_against_live_github -- --ignored --nocapture
    /// ```
    ///
    /// First run, 2026-08-10: reported `Deploy to cloud.amux.io` at 31
    /// consecutive failures since 2026-08-07, plus `rust` and `checks` — the
    /// exact outage that went unseen for three days.
    #[tokio::test]
    #[ignore = "hits live GitHub via the gh CLI; manual probe, not a CI gate"]
    async fn ci_detector_against_live_github() {
        *ci_cache().write().unwrap() = None;
        let now = unix_now();
        let (runs, sup) = fetch_ci_runs(now).await;
        println!("fetched {} runs; {} suppression(s)", runs.len(), sup.len());
        for s in &sup {
            println!("  SUPPRESSED {}: {}", s.signature, s.reason);
        }
        assert!(!runs.is_empty(), "gh returned nothing — is it installed and authenticated?");
        let (f, s2) = ci_findings(&runs, now);
        for x in &s2 {
            println!("  SUPPRESSED {}: {}", x.signature, x.reason);
        }
        for x in &f {
            println!("\n=== WOULD FILE: {}\nsignature: {}", x.title, x.signature);
            println!("{}", render_desc(x));
        }
    }

    /// Blindness must not read as an all-clear. This is the failure mode unique
    /// to the one detector whose instrument is off-machine: `gh` missing,
    /// logged out, rate-limited or offline all produce zero runs, which is
    /// byte-identical to a healthy CI unless the reason is published.
    #[test]
    fn no_runs_is_not_the_same_as_no_failures() {
        let (f, s) = ci_findings(&[], unix_now());
        assert!(f.is_empty());
        assert!(s.is_empty(), "an empty list alone says nothing — the REASON comes from the fetch");
        // The fetch is what knows why the list is empty, and it emits a
        // Suppressed naming the cause; `ci_findings` cannot and must not guess.
        // Guarded here so nobody later "helpfully" makes an empty list file a
        // card, which would fire on every network blip.
    }
}
